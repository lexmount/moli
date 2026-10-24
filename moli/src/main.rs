//! Command-line entry point for Moli tooling.
//!
//! This binary wires the `moli` support crate and the public
//! `moli` facade into CLI commands such as page fetches and the embedded
//! protocol server.

use anyhow::Result;
use std::process::ExitCode;

mod recording_termination;

#[cfg(all(
    feature = "jemalloc",
    not(target_os = "windows"),
    not(any(
        all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"),
        all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))
))]
mod allocator;

fn run() -> Result<ExitCode> {
    let cli = moli::app::parse_cli_from_env();
    let recording =
        matches!(&cli.command, moli::cli::Commands::Serve(args) if args.screencast_video.is_some());
    let mut termination = if recording {
        Some(recording_termination::VideoTermination::install()?)
    } else {
        moli_process_signal::install_immediate_exit_handlers()?;
        None
    };
    let runtime_thread_budget = moli::runtime_thread_budget::tokio_runtime_thread_budget();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(runtime_thread_budget)
        .max_blocking_threads(runtime_thread_budget)
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let Some(termination) = termination.as_mut() else {
            return moli::app::run_parsed_cli(cli, None).await.map(|()| ExitCode::SUCCESS);
        };
        let control = moli_protocol_server::BrowserInstanceControl::new();
        let running = moli::app::run_parsed_cli(cli, Some(control.clone()));
        tokio::pin!(running);
        tokio::select! {
            result = &mut running => result.map(|()| ExitCode::SUCCESS),
            notice = termination.notified() => {
                let notice = notice?;
                control.request(moli_protocol_server::InstanceCommand::Shutdown);
                match tokio::time::timeout_at(tokio::time::Instant::from_std(notice.deadline), &mut running).await {
                    Ok(Ok(())) => {},
                    Ok(Err(error)) => report_error(&error),
                    Err(_) => report_error(&anyhow::anyhow!("video shutdown exceeded its signal deadline")),
                }
                Ok(ExitCode::from((128 + notice.signal) as u8))
            }
        }
    });
    if recording {
        runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    }
    result
}

fn report_error(error: &anyhow::Error) {
    let stderr = std::io::stderr();
    let _ = moli::app::write_error_report(&mut stderr.lock(), error);
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            report_error(&error);
            moli::app::error_exit_code(&error)
        }
    }
}
