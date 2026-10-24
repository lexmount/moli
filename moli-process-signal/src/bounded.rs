//! Recording-only termination notification with a process-wide hard deadline.

use std::{
    io::{self, Read},
    os::{
        fd::{AsRawFd, IntoRawFd},
        unix::net::UnixStream,
    },
    sync::atomic::{AtomicI32, Ordering},
    thread,
    time::{Duration, Instant},
};

use tokio::sync::oneshot;

static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);
static FIRST_SIGNAL: AtomicI32 = AtomicI32::new(0);
const NOTICE_BYTES: usize = 3 * size_of::<i64>();

pub struct TerminationNotice {
    pub signal: i32,
    pub deadline: Instant,
}

pub struct BoundedTermination {
    notice: oneshot::Receiver<TerminationNotice>,
}

impl BoundedTermination {
    /// Install once, before application workers. The relay uses one socket pair
    /// and one OS thread for the process lifetime; a second signal exits at once.
    pub fn install(budget: Duration) -> io::Result<Self> {
        let (mut reader, writer) = UnixStream::pair()?;
        writer.set_nonblocking(true)?;
        SIGNAL_FD
            .compare_exchange(-1, writer.as_raw_fd(), Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "termination relay already installed",
                )
            })?;
        let (sender, notice) = oneshot::channel();
        // Never close/reuse the handler's fd while another thread could have
        // loaded it. This single process-lifetime descriptor is intentional.
        let _ = writer.into_raw_fd();
        thread::Builder::new()
            .name("moli-signal".into())
            .spawn(move || {
                let mut bytes = [0; NOTICE_BYTES];
                if reader.read_exact(&mut bytes).is_err() {
                    return;
                }
                let signal =
                    i64::from_ne_bytes(bytes[..8].try_into().expect("signal bytes")) as i32;
                let seconds = i64::from_ne_bytes(bytes[8..16].try_into().expect("clock seconds"));
                let nanos = i64::from_ne_bytes(bytes[16..].try_into().expect("clock nanos"));
                let sent_at = Duration::new(seconds as u64, nanos as u32);
                let mut clock = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                // SAFETY: clock is writable; CLOCK_MONOTONIC is supported on Unix.
                let elapsed = if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut clock) }
                    == 0
                {
                    Duration::new(clock.tv_sec as u64, clock.tv_nsec as u32).saturating_sub(sent_at)
                } else {
                    budget
                };
                let remaining = budget.saturating_sub(elapsed);
                let deadline = Instant::now() + remaining;
                let _ = sender.send(TerminationNotice { signal, deadline });
                thread::sleep(deadline.saturating_duration_since(Instant::now()));
                // The watchdog is independent of renderer tasks, blocking writes,
                // runtime shutdown and thread joins. Only process exit disarms it.
                // SAFETY: _exit terminates the process without invoking cleanup.
                unsafe { libc::_exit(128 + signal) }
            })?;
        super::install_immediate_exit_handlers()?;
        Ok(Self { notice })
    }

    pub async fn notified(&mut self) -> io::Result<TerminationNotice> {
        (&mut self.notice)
            .await
            .map_err(|_| io::Error::other("termination relay stopped"))
    }
}

pub(super) fn notify(signal: libc::c_int) -> bool {
    let fd = SIGNAL_FD.load(Ordering::Relaxed);
    if fd < 0
        || FIRST_SIGNAL
            .compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return false;
    }
    let mut clock = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime and write are async-signal-safe. All data is on the
    // signal stack; the nonblocking fd has room for this sole small notice.
    unsafe {
        if libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut clock) != 0 {
            return false;
        }
        #[allow(clippy::unnecessary_cast)]
        let notice = [i64::from(signal), clock.tv_sec as i64, clock.tv_nsec as i64];
        libc::write(fd, notice.as_ptr().cast(), NOTICE_BYTES) == NOTICE_BYTES as isize
    }
}
