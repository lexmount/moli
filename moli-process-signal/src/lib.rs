//! Process-wide termination signal handling for Moli executables.
//!
//! Container runtimes deliver termination signals to PID 1. A PID namespace's
//! init process cannot rely on the ordinary default `SIGTERM` disposition, so
//! Moli installs explicit handlers and exits from them immediately.

use std::io;

/// Installs immediate-exit handlers for `SIGTERM`, `SIGINT`, and `SIGHUP`.
///
/// On Unix, a handled signal terminates the process with status
/// `128 + signal`. The handler calls `_exit`, so it does not run Rust
/// destructors, allocator teardown, async shutdown, or application cleanup.
/// Call this once at process startup, before worker threads are created.
///
/// On non-Unix platforms this function is a no-op.
pub fn install_immediate_exit_handlers() -> io::Result<()> {
    #[cfg(unix)]
    {
        unix::install_immediate_exit_handlers()
    }

    #[cfg(not(unix))]
    {
        Ok(())
    }
}

/// A stream of termination signals (`SIGTERM`, `SIGINT`, and `SIGHUP`).
///
/// [`TerminationStream::install`] synchronously installs tokio's signal
/// handling, replacing the immediate-exit handler installed by
/// [`install_immediate_exit_handlers`]. Awaiting [`TerminationStream::recv`]
/// yields the delivered signal. Callers are expected to drain gracefully and
/// then either return or force termination with [`force_exit_for_signal`].
pub struct TerminationStream {
    #[cfg(unix)]
    inner: unix::TerminationStream,
}

impl TerminationStream {
    /// Installs and returns a termination-signal stream.
    ///
    /// On non-Unix platforms this returns a stream whose `recv` never resolves.
    pub fn install() -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                inner: unix::TerminationStream::install()?,
            })
        }

        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }

    /// Awaits the next termination signal, returning its number.
    pub async fn recv(&mut self) -> i32 {
        #[cfg(unix)]
        {
            self.inner.recv().await
        }

        #[cfg(not(unix))]
        {
            std::future::pending().await
        }
    }
}

/// Terminates the process immediately with the conventional signal exit status
/// (`128 + signal`) without running destructors.
///
/// This is the bounded fallback used when graceful shutdown overruns its
/// configured budget.
#[cfg(unix)]
pub fn force_exit_for_signal(signal: i32) -> ! {
    unix::force_exit_for_signal(signal)
}

/// On non-Unix platforms there are no termination signals; this aborts.
#[cfg(not(unix))]
pub fn force_exit_for_signal(_signal: i32) -> ! {
    std::process::abort()
}

#[cfg(unix)]
mod unix {
    use std::{io, mem, ptr};

    const SIGNAL_EXIT_STATUS_BASE: libc::c_int = 128;
    const TERMINATION_SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

    pub(super) fn install_immediate_exit_handlers() -> io::Result<()> {
        for signal in TERMINATION_SIGNALS {
            install_immediate_exit_handler(signal)?;
        }
        Ok(())
    }

    fn install_immediate_exit_handler(signal: libc::c_int) -> io::Result<()> {
        // SAFETY: A zero-initialized sigaction is valid before its mask,
        // handler, and flags are explicitly initialized below.
        let mut action = unsafe { mem::zeroed::<libc::sigaction>() };
        action.sa_sigaction = exit_immediately as *const () as libc::sighandler_t;
        action.sa_flags = 0;

        // SAFETY: action owns a valid sigset_t and remains alive for the call.
        if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: action is fully initialized, the handler has the C signal
        // ABI, and the kernel copies the action before this function returns.
        if unsafe { libc::sigaction(signal, &action, ptr::null_mut()) } != 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    extern "C" fn exit_immediately(signal: libc::c_int) {
        // `_exit` is async-signal-safe and terminates the whole process on the
        // supported Unix targets without invoking user-space cleanup.
        // SAFETY: the status is derived from one of the installed signals and
        // `_exit` never returns.
        unsafe { libc::_exit(SIGNAL_EXIT_STATUS_BASE + signal) }
    }

    pub(super) struct TerminationStream {
        sigterm: tokio::signal::unix::Signal,
        sigint: tokio::signal::unix::Signal,
        sighup: tokio::signal::unix::Signal,
    }

    impl TerminationStream {
        pub(super) fn install() -> io::Result<Self> {
            use tokio::signal::unix::{SignalKind, signal};
            // tokio's signal registry chains to any previously-installed
            // handler. Reset the immediate-exit handlers first so a delivered
            // signal is not also forwarded to `_exit`.
            reset_termination_handlers_to_default()?;
            Ok(Self {
                sigterm: signal(SignalKind::terminate())?,
                sigint: signal(SignalKind::interrupt())?,
                sighup: signal(SignalKind::hangup())?,
            })
        }

        pub(super) async fn recv(&mut self) -> i32 {
            tokio::select! {
                _ = self.sigterm.recv() => libc::SIGTERM,
                _ = self.sigint.recv() => libc::SIGINT,
                _ = self.sighup.recv() => libc::SIGHUP,
            }
        }
    }

    pub(super) fn force_exit_for_signal(signal: i32) -> ! {
        // SAFETY: `_exit` is async-signal-safe and never returns.
        unsafe { libc::_exit(SIGNAL_EXIT_STATUS_BASE + signal) }
    }

    #[cfg(test)]
    pub(super) fn current_sigterm_handler() -> libc::sighandler_t {
        let mut old = unsafe { mem::zeroed::<libc::sigaction>() };
        // SAFETY: `old` is a valid, writable sigaction; querying installs
        // nothing because the new action is null.
        unsafe { libc::sigaction(libc::SIGTERM, ptr::null(), &mut old) };
        old.sa_sigaction
    }

    fn reset_termination_handlers_to_default() -> io::Result<()> {
        for signal in TERMINATION_SIGNALS {
            let mut action = unsafe { mem::zeroed::<libc::sigaction>() };
            action.sa_sigaction = libc::SIG_DFL;
            action.sa_flags = 0;
            if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: action is fully initialized and the kernel copies it.
            if unsafe { libc::sigaction(signal, &action, ptr::null_mut()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn termination_stream_replaces_immediate_handler() {
        install_immediate_exit_handlers().expect("immediate handlers");
        let before = unix::current_sigterm_handler();
        let _stream = TerminationStream::install().expect("termination stream");
        let after = unix::current_sigterm_handler();
        assert_ne!(
            before, after,
            "tokio signal registration should replace the immediate exit handler"
        );
    }
}
