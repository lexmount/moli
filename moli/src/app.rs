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
            let fetch_result = readiness.fetch_document(&browser, request).await;
            let fetched_document = match fetch_result {
                Ok(document) => document,
                Err(error) => {
                    finalize_fetch_browser(browser);
                    return Err(with_fetch_context(error, &args.url));
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
                        fetch_dump::render_raw_document_output(&raw_document, &config.fetch)
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
            server.serve().await.context("protocol server failed")?;
        }
    }

    Ok(())
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
    request.request_headers = config.fetch.request_headers.clone();
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
        error.to_string()
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
    fn fetch_report_has_one_reason_line_without_rendering_the_source_chain() {
        let error = with_fetch_context(
            anyhow::anyhow!("first failure line\nsecond failure line"),
            "https://example.test/",
        );
        let mut report = Vec::new();

        write_error_report(&mut report, &error).expect("report should write");
        let report = String::from_utf8(report).expect("report should be UTF-8");

        assert_eq!(
            report,
            "Error: failed to fetch `https://example.test/`\nReason: first failure line second failure line\n"
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
