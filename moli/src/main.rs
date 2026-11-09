//! Command-line entry point for Moli tooling.
//!
//! This binary wires the `moli` support crate and the public
//! `moli` facade into CLI commands such as page fetches and the embedded
//! protocol server.

use anyhow::Result;
use clap::Parser;
use moli::cli::{Cli, Commands, normalize_args_for_compat};
use std::process::ExitCode;

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

fn run() -> Result<()> {
    let cli = Cli::parse_from(normalize_args_for_compat(std::env::args_os()));
    if !matches!(cli.command, Commands::Serve(_)) {
        moli_process_signal::install_immediate_exit_handlers()?;
    }
    let config = moli::config::AppConfig::from_cli(&cli)?;
    moli::telemetry::init(&config.log_filter);
    let runtime_thread_budget = moli::runtime_thread_budget::tokio_runtime_thread_budget();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(runtime_thread_budget)
        .max_blocking_threads(runtime_thread_budget)
        .enable_all()
        .build()?;
    let result = runtime.block_on(moli::app::run_cli_with_config(
        cli,
        config,
        &mut std::io::stdout(),
    ));
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    result
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
