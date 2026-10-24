//! Process-wide termination signal handling for Moli executables.
//!
//! Container runtimes deliver termination signals to PID 1. A PID namespace's
//! init process cannot rely on the ordinary default `SIGTERM` disposition, so
//! Moli installs explicit handlers and exits from them immediately.

use std::io;

#[cfg(unix)]
mod bounded;
#[cfg(unix)]
pub use bounded::{BoundedTermination, TerminationNotice};

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
        if super::bounded::notify(signal) {
            return;
        }
        // `_exit` is async-signal-safe and terminates the whole process on the
        // supported Unix targets without invoking user-space cleanup.
        // SAFETY: the status is derived from one of the installed signals and
        // `_exit` never returns.
        unsafe { libc::_exit(SIGNAL_EXIT_STATUS_BASE + signal) }
    }
}
