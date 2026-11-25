//! Command-line entry point for Moli tooling.
//!
//! This binary wires the `moli` support crate and the public
//! `moli` facade into CLI commands such as page fetches and the embedded
//! protocol server.

use anyhow::Result;
use clap::Parser;
use moli::{
    app::ServeTransport,
    cli::{Cli, normalize_args_for_compat},
};
use std::process::ExitCode;

#[cfg(all(
    feature = "jemalloc",
    not(target_os = "windows"),
    not(target_env = "ohos"),
    not(any(
        all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"),
        all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))
))]
mod allocator;

fn run() -> Result<()> {
    let cli = Cli::parse_from(normalize_args_for_compat(std::env::args_os()));
    #[cfg(unix)]
    let transport = match &cli.command {
        moli::cli::Commands::Serve(args) if args.remote_debugging_pipe => {
            let (input, output) = take_inherited_pipe_descriptors()?;
            ServeTransport::Pipe(input, output)
        }
        _ => ServeTransport::Tcp,
    };
    #[cfg(not(unix))]
    let transport = ServeTransport::Tcp;
    moli_process_signal::install_immediate_exit_handlers()?;
    let runtime_thread_budget = moli::runtime_thread_budget::tokio_runtime_thread_budget();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(runtime_thread_budget)
        .max_blocking_threads(runtime_thread_budget)
        .enable_all()
        .build()?
        .block_on(moli::app::run_with_transport(cli, transport))
}

/// Claims the launcher's inherited CDP endpoints once, before starting Tokio
/// or opening application resources that could reuse a missing descriptor.
#[cfg(unix)]
fn take_inherited_pipe_descriptors() -> Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::{FromRawFd, OwnedFd};

    for fd in [3, 4] {
        // SAFETY: `fstat` only writes to the zeroed buffer it is given.
        let mut status: libc::stat = unsafe { std::mem::zeroed() };
        let kind =
            (unsafe { libc::fstat(fd, &mut status) } == 0).then_some(status.st_mode & libc::S_IFMT);
        anyhow::ensure!(
            matches!(kind, Some(libc::S_IFIFO | libc::S_IFSOCK)),
            "--remote-debugging-pipe needs the parent to pass pipes as file descriptors 3 and 4"
        );
    }
    // SAFETY: the executable calls this private helper exactly once at startup,
    // before any runtime or application resource can own these inherited FDs.
    // The launcher supplies both endpoints exclusively to this child process.
    Ok(unsafe { (OwnedFd::from_raw_fd(3), OwnedFd::from_raw_fd(4)) })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let stderr = std::io::stderr();
            let _ = moli::app::write_error_report(&mut stderr.lock(), &error);
            moli::app::error_exit_code(&error)
        }
    }
}
