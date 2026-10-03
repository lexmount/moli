//! Callable command runner for the Moli CLI.

mod readiness;
mod redirect_navigation;

use std::{
    fmt,
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

use crate::{
    cli::{Cli, Commands, FetchArgs, normalize_args_for_compat},
    config::AppConfig,
    cookie_cache, eval_output, fetch_dump, robots,
};
use anyhow::Result;
use anyhow::{Context, anyhow};
use clap::Parser;
use moli_core::runtime::{
    Browser, FetchReadinessTimeout, FetchedDocument, NavigationRuntimeConfig,
    storage_partition::StoragePartitionState,
};
use moli_fetch::{NetworkFetchFailureContext, Request};
use moli_protocol_server::ProtocolServer;

use self::readiness::ReadinessPlan;

pub async fn run_from_env() -> Result<()> {
    let cli = Cli::parse_from(normalize_args_for_compat(std::env::args_os()));
    let config = AppConfig::from_cli(&cli).context("failed to build app configuration")?;
    crate::telemetry::init(&config.log_filter);
    let mut stdout = std::io::stdout();
    run_cli_with_config(cli, config, &mut stdout).await
}

pub async fn run_cli<W: Write>(stdout: &mut W, cli: Cli) -> Result<()> {
    let config = AppConfig::from_cli(&cli).context("failed to build app configuration")?;
    run_cli_with_config(cli, config, stdout).await
}

pub async fn run_cli_with_config<W: Write>(
    cli: Cli,
    config: AppConfig,
    stdout: &mut W,
) -> Result<()> {
    match cli.command {
        Commands::Fetch(mut args) => {
            reject_multiple_stdin_script_sources(&args)?;
            let eval_expression = if let Some(path) = args.eval_file.as_deref() {
                Some(read_script_file_arg("--eval-file", path)?)
            } else {
                args.eval.take()
            };
            let readiness =
                ReadinessPlan::from_fetch_args(&args, config.fetch.response_wait.clone())?;
            let raw_document_output =
                fetch_dump::RawDocumentOutputPolicy::from_command(&config.fetch);
            let request = build_fetch_request(&args.url, &config)?;
            if config.browser.fetch().obey_robots() {
                // Checked before the browser starts so a refused fetch costs
                // nothing but the robots.txt request itself.
                robots::ensure_fetch_allowed(config.browser.fetch(), &request.url)
                    .await
                    .map_err(|error| with_fetch_context(error, &args.url))?;
            }
            let browser = Browser::new(config.browser.clone())?;
            load_cookie_state(&browser, &config)?;
            let fetch_result = readiness
                .fetch_document(&browser, request, raw_document_output.fetch_policy())
                .await;
            let fetched_document = match fetch_result {
                Ok(document) => document,
                Err(error) => {
                    finalize_fetch_browser(browser);
                    return Err(with_fetch_context(
                        raw_document_output.map_fetch_error(error),
                        &args.url,
                    ));
                }
            };

            let mut page = match fetched_document {
                FetchedDocument::Page(page) => page,
                FetchedDocument::Raw(raw_document) => {
                    if eval_expression.is_some() {
                        finalize_fetch_browser(browser);
                        return Err(with_fetch_context(
                            anyhow!(
                                "raw non-HTML document fetch does not support --eval or --eval-file"
                            ),
                            &args.url,
                        ));
                    }
                    if readiness.has_page_waits() || args.delay_ms > 0 {
                        finalize_fetch_browser(browser);
                        return Err(with_fetch_context(
                            anyhow!(
                                "raw non-HTML document fetch does not support page wait options"
                            ),
                            &args.url,
                        ));
                    }
                    let rendered =
                        fetch_dump::render_raw_document_output(&raw_document, raw_document_output)
                            .map_err(|error| with_fetch_context(error, &args.url))?;
                    stdout
                        .write_all(&rendered)
                        .context("failed to write raw fetch output")
                        .map_err(|error| with_fetch_context(error, &args.url))?;
                    let _ = stdout.flush();
                    finalize_fetch_browser(browser);
                    return Ok(());
                }
            };

            if let Err(error) = readiness.wait_for_page(&browser, &mut page).await {
                if let Err(close_error) = page.close_async().await {
                    tracing::warn!(
                        error = %close_error,
                        "failed to close fetched page after readiness failure"
                    );
                }
                finalize_fetch_browser(browser);
                return Err(with_fetch_context(error, &args.url));
            }

            if args.delay_ms > 0 {
                browser
                    .wait_for_page_delay(&mut page, std::time::Duration::from_millis(args.delay_ms))
                    .await
                    .context("failed while waiting for page delay")
                    .map_err(|error| with_fetch_context(error, &args.url))?;
            }

            let rendered = if let Some(expression) = eval_expression.as_deref() {
                if config.browser.layout_policy().uses_real_layout() {
                    page.publish_layout_async()
                        .await
                        .context("failed to publish layout before evaluating JavaScript")
                        .map_err(|error| with_fetch_context(error, &args.url))?;
                }
                eval_output::evaluate(&mut page, expression).await
            } else {
                fetch_dump::render_page_output_async(&mut page, &config.fetch).await
            }
            .map_err(|error| with_fetch_context(error, &args.url))?;
            stdout
                .write_all(&rendered)
                .context("failed to write fetch output")
                .map_err(|error| with_fetch_context(error, &args.url))?;
            let _ = stdout.flush();
            if let Err(error) = page.close_async().await {
                tracing::warn!(error = %error, "failed to close fetched page before browser shutdown");
            }
            finalize_fetch_browser(browser);
        }
        Commands::Serve(_) => {
            if config.browser.fetch().obey_robots() {
                // Protocol clients drive navigation themselves, so the CLI
                // cannot refuse a page on their behalf. Say so rather than let
                // the flag look enforced.
                tracing::warn!(
                    "--obey-robots is enforced for `moli fetch` only; \
                     protocol-server navigations are not checked against robots.txt"
                );
            }
            let storage_partition =
                Arc::new(StoragePartitionState::open(config.browser.profile_dir())?);
            storage_partition.import_cookies(load_cookie_state_cookies(&config)?)?;
            let server = ProtocolServer::new_with_storage_partition_and_runtime_config(
                config.server.clone(),
                storage_partition,
                NavigationRuntimeConfig::from(&config.browser),
            );
            serve_until_terminated(&server)
                .await
                .context("protocol server failed")?;
        }
        Commands::Import(args) => {
            let summary =
                moli_cookie_import::import_session_state(&moli_cookie_import::ImportRequest {
                    profile_dir: &args.profile_dir,
                    includes: args.includes,
                    chrome_profile_dir: args.chrome_profile_dir.as_deref(),
                    chrome_crypto_key: args.chrome_crypto_key.as_ref(),
                    firefox_profile_dir: args.firefox_profile_dir.as_deref(),
                    cookie_jars: &args.cookie_jar,
                })?;
            writeln!(stdout, "Imported session state: {summary}")
                .context("failed to write import summary")?;
        }
    }

    Ok(())
}

/// Serves until a browser-level `Browser.close` or a termination signal.
///
/// A termination signal triggers the same graceful drain-and-flush path; if the
/// drain does not complete within `GRACEFUL_SHUTDOWN_BUDGET`, the process is
/// force-terminated with the conventional `128 + signal` status.
async fn serve_until_terminated(server: &ProtocolServer) -> Result<()> {
    const GRACEFUL_SHUTDOWN_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

    // Install the signal handlers synchronously so a signal delivered right
    // after the listener is announced still takes the graceful path.
    let mut termination = moli_process_signal::TerminationStream::install()
        .context("failed to install termination signal handlers")?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let (drained_tx, drained_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        let signal = termination.recv().await;
        tracing::info!(
            signal,
            "received termination signal; draining protocol server"
        );
        let _ = shutdown_tx.send(());
        tokio::select! {
            _ = drained_rx => {}
            () = tokio::time::sleep(GRACEFUL_SHUTDOWN_BUDGET) => {
                tracing::warn!(
                    signal,
                    "graceful shutdown exceeded its budget; forcing exit"
                );
                moli_process_signal::force_exit_for_signal(signal);
            }
        }
    });

    let result = server
        .serve_with_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await;
    let _ = drained_tx.send(());
    result
}

fn reject_multiple_stdin_script_sources(args: &FetchArgs) -> Result<()> {
    if args.eval_file.as_deref() == Some(Path::new("-"))
        && args.wait_script_file.as_deref() == Some("-")
    {
        return Err(anyhow!(
            "`--eval-file -` and `--wait-script-file -` cannot both read from stdin"
        ));
    }
    Ok(())
}

fn read_script_file_arg(option: &str, path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut source = String::new();
        std::io::stdin()
            .read_to_string(&mut source)
            .with_context(|| format!("failed to read {option} `-` from stdin"))?;
        Ok(source)
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {option} `{}`", path.display()))
    }
}

fn build_fetch_request(url: &str, config: &AppConfig) -> Result<Request> {
    let mut request = Request::get(url)?;
    // Keep CLI-provided headers scoped to the initial document navigation.
    request.request_headers = config.fetch.request_headers.clone().into();
    Ok(request)
}

struct CliFetchFailureContext {
    url: String,
    reason: String,
}

impl fmt::Display for CliFetchFailureContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "failed to fetch `{}`", self.url)
    }
}

impl fmt::Debug for CliFetchFailureContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CliFetchFailureContext")
            .field("url", &self.url)
            .field("has_reason", &!self.reason.is_empty())
            .finish()
    }
}

fn with_fetch_context(error: anyhow::Error, url: &str) -> anyhow::Error {
    if error.is::<CliFetchFailureContext>() {
        return error;
    }
    let reason = if let Some(failure) = error.downcast_ref::<NetworkFetchFailureContext>() {
        failure.reason().to_owned()
    } else if let Some(timeout) = error.downcast_ref::<FetchReadinessTimeout>() {
        timeout.to_string()
    } else {
        format!("{error:#}")
    };
    with_fetch_context_reason(error, url, reason)
}

fn with_fetch_context_reason(
    error: anyhow::Error,
    url: &str,
    reason: impl Into<String>,
) -> anyhow::Error {
    error.context(CliFetchFailureContext {
        url: url.to_owned(),
        reason: reason.into(),
    })
}

/// Writes the stable, concise command-line error presentation.
///
/// Fetch errors carry their selected user-facing reason in a typed outer
/// context and deliberately render only that two-line presentation. Other CLI
/// failures retain their anyhow context chain so startup and configuration
/// diagnostics do not lose their actionable inner cause.
pub fn write_error_report<W: Write>(writer: &mut W, error: &anyhow::Error) -> std::io::Result<()> {
    if let Some(fetch) = error.downcast_ref::<CliFetchFailureContext>() {
        writeln!(writer, "Error: {fetch}")?;
        writeln!(writer, "Reason: {}", one_line_reason(&fetch.reason))?;
        return Ok(());
    }
    writeln!(writer, "Error: {error:#}")
}

fn one_line_reason(reason: &str) -> String {
    reason.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn load_cookie_state(browser: &Browser, config: &AppConfig) -> Result<()> {
    browser.import_cookies(load_cookie_state_cookies(config)?)?;
    Ok(())
}

fn load_cookie_state_cookies(config: &AppConfig) -> Result<Vec<moli_cookie_jar::StoredCookie>> {
    let mut cookies = Vec::new();
    for path in &config.fetch.cookie_files {
        let loaded = cookie_cache::load_cookie_file(path)
            .with_context(|| anyhow!("failed to load cookie file `{path}`"))?;
        cookies.extend(loaded);
    }
    Ok(cookies)
}

fn finalize_fetch_browser(browser: Browser) {
    // Fetch is a one-shot CLI path, but the browser must still be dropped in an
    // orderly way. Letting network threads survive until process exit can race
    // OpenSSL global cleanup with libcurl transfers still in progress.
    // Browser::drop owns profile cookie writeback when --profile-dir is set.
    drop(browser);
}

#[cfg(test)]
mod tests {
    use super::{with_fetch_context, write_error_report};
    use moli_core::runtime::{FetchReadinessTimeout, FetchTimeoutPhase};
    use std::time::Duration;

    #[test]
    fn fetch_report_has_one_reason_line_including_the_source_chain() {
        let error = with_fetch_context(
            anyhow::anyhow!("first failure line\nsecond failure line").context("request failed"),
            "https://example.test/",
        );
        let mut report = Vec::new();

        write_error_report(&mut report, &error).expect("report should write");
        let report = String::from_utf8(report).expect("report should be UTF-8");

        assert_eq!(
            report,
            "Error: failed to fetch `https://example.test/`\nReason: request failed: first failure line second failure line\n"
        );
        assert!(!report.contains("Caused by:"));
    }

    #[test]
    fn fetch_report_selects_the_typed_readiness_timeout_through_outer_context() {
        let error = anyhow::Error::new(FetchReadinessTimeout::new(
            Duration::from_millis(4000),
            FetchTimeoutPhase::WaitingForSelector,
        ))
        .context("failed while waiting for selector `#target`");
        let error = with_fetch_context(error, "https://example.test/");
        let mut report = Vec::new();

        write_error_report(&mut report, &error).expect("report should write");
        let report = String::from_utf8(report).expect("report should be UTF-8");

        assert_eq!(
            report,
            "Error: failed to fetch `https://example.test/`\n\
             Reason: fetch readiness timed out after 4000 ms while waiting for a selector\n"
        );
        assert!(!report.contains("failed while waiting for selector"));
        assert!(!report.contains("Caused by:"));
    }

    #[test]
    fn non_fetch_report_retains_the_anyhow_context_chain() {
        let error = anyhow::anyhow!("inner cause").context("outer context");
        let mut report = Vec::new();

        write_error_report(&mut report, &error).expect("report should write");
        let report = String::from_utf8(report).expect("report should be UTF-8");

        assert!(report.contains("outer context"));
        assert!(report.contains("inner cause"));
    }
}
