//! Callable command runner for the Moli CLI.

mod readiness;
mod redirect_navigation;

use std::{
    fmt,
    io::{BufReader, Read, Write},
    path::Path,
    process::ExitCode,
    sync::Arc,
    time::Duration,
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
    RawDocumentFetchPolicy, storage_partition::StoragePartitionState,
};
use moli_fetch::{CookieHeaderPolicy, NetworkFetchFailureContext, Request};
use moli_protocol_server::ProtocolServer;

use self::readiness::ReadinessPlan;

const READINESS_FAILURE_DIAGNOSTICS_TIMEOUT: Duration = Duration::from_millis(250);

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
    mut config: AppConfig,
    stdout: &mut W,
) -> Result<()> {
    match cli.command {
        Commands::Fetch(mut args) => {
            reject_conflicting_stdin_sources(&args)?;
            let initial_cookies = load_cookie_state_cookies(&config)?;
            let eval_expression = if let Some(path) = args.eval_file.as_deref() {
                Some(read_script_file_arg("--eval-file", path)?)
            } else {
                args.eval.take()
            };
            let readiness =
                ReadinessPlan::from_fetch_args(&args, config.fetch.response_wait.clone())?;
            let raw_document_output =
                fetch_dump::RawDocumentOutputPolicy::from_command(&config.fetch);
            // Preserve status metadata for HTTP error precedence, but never
            // consume a raw body that the selected output format cannot use.
            let raw_document_fetch_policy = match raw_document_output.fetch_policy() {
                RawDocumentFetchPolicy::RequirePage if args.fail || args.fail_with_body => {
                    RawDocumentFetchPolicy::MetadataOnly
                }
                _ if args.fail => RawDocumentFetchPolicy::MaterializeBelowStatus(400),
                policy => policy,
            };
            let request = build_fetch_request(&args.url, &config)?;
            if config.browser.fetch().obey_robots() {
                // Checked before the browser starts so a refused fetch costs
                // nothing but the robots.txt request itself.
                robots::ensure_fetch_allowed(config.browser.fetch(), &request.url)
                    .await
                    .map_err(|error| with_fetch_context(error, &args.url))?;
            }
            let browser = Browser::new(config.browser.clone())?;
            let result = async {
                browser.import_cookies(initial_cookies)?;
                let fetch_result = readiness
                    .fetch_document(&browser, request, raw_document_fetch_policy)
                    .await;
                let fetched_document = match fetch_result {
                    Ok(document) => document,
                    Err(error) => {
                        return Err(with_fetch_context(
                            raw_document_output.map_fetch_error(error),
                            &args.url,
                        ));
                    }
                };

                let mut page = match fetched_document {
                    FetchedDocument::Page(page) => page,
                    FetchedDocument::Raw(raw_document) => {
                        let http_failure = http_status_failure(
                            &args,
                            raw_document.status(),
                            raw_document.final_url().as_str(),
                        );
                        let with_output_context = |error| {
                            with_http_status_context(
                                error,
                                &args,
                                raw_document.status(),
                                raw_document.final_url().as_str(),
                            )
                        };
                        if !args.fail || http_failure.is_none() {
                            if eval_expression.is_some() {
                                return Err(with_output_context(anyhow!(
                                    "raw non-HTML document fetch does not support --eval or --eval-file"
                                )));
                            }
                            if readiness.has_page_waits() || args.delay_ms > 0 {
                                return Err(with_output_context(anyhow!(
                                    "raw non-HTML document fetch does not support page wait options"
                                )));
                            }
                            let rendered = fetch_dump::render_raw_document_output(
                                &raw_document,
                                raw_document_output,
                            )
                            .map_err(with_output_context)?;
                            stdout
                                .write_all(&rendered)
                                .context("failed to write raw fetch output")
                                .map_err(with_output_context)?;
                            stdout
                                .flush()
                                .context("failed to flush raw fetch output")
                                .map_err(with_output_context)?;
                        }
                        if let Some(error) = http_failure {
                            return Err(error);
                        }
                        return Ok(());
                    }
                };

                let readiness_result = async {
                    readiness.wait_for_page(&browser, &mut page).await?;
                    if args.delay_ms > 0 {
                        browser
                            .wait_for_page_delay(
                                &mut page,
                                std::time::Duration::from_millis(args.delay_ms),
                            )
                            .await
                            .context("failed while waiting for page delay")?;
                    }
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                if let Err(error) = readiness_result {
                    if args.fail || args.fail_with_body {
                        // Keep diagnostics bounded so Page close can interrupt a
                        // renderer still occupied by the failed readiness script.
                        match tokio::time::timeout(
                            READINESS_FAILURE_DIAGNOSTICS_TIMEOUT,
                            page.page_diagnostics_snapshot_async(),
                        )
                        .await
                        {
                            Ok(Ok(_)) => {}
                            Ok(Err(snapshot_error)) => {
                                tracing::debug!(
                                    error = %snapshot_error,
                                    "failed to refresh main-document state after readiness failure"
                                );
                            }
                            Err(_) => {
                                tracing::debug!(
                                    "timed out refreshing main-document state after readiness failure; using cached state"
                                );
                            }
                        }
                    }
                    let error = with_http_status_context(
                        error,
                        &args,
                        page.status(),
                        page.final_url().as_str(),
                    );
                    if let Err(close_error) = page.close_async().await {
                        tracing::warn!(
                            error = %close_error,
                            "failed to close fetched page after readiness failure"
                        );
                    }
                    return Err(error);
                }

                let page_status = page.status();
                let final_url = page.final_url().clone();
                let http_failure = http_status_failure(&args, page_status, final_url.as_str());
                let with_output_context =
                    |error| with_http_status_context(error, &args, page_status, final_url.as_str());
                if !args.fail || http_failure.is_none() {
                    let rendered = if let Some(expression) = eval_expression.as_deref() {
                        if config.browser.layout_policy().uses_real_layout() {
                            page.publish_layout_async()
                                .await
                                .context("failed to publish layout before evaluating JavaScript")
                                .map_err(with_output_context)?;
                        }
                        if args.dump.is_some() {
                            eval_output::execute(&mut page, expression)
                                .await
                                .map_err(with_output_context)?;
                            fetch_dump::render_page_output_async(&mut page, &config.fetch).await
                        } else {
                            eval_output::evaluate(&mut page, expression).await
                        }
                    } else {
                        fetch_dump::render_page_output_async(&mut page, &config.fetch).await
                    }
                    .map_err(with_output_context)?;
                    stdout
                        .write_all(&rendered)
                        .context("failed to write fetch output")
                        .map_err(with_output_context)?;
                    stdout
                        .flush()
                        .context("failed to flush fetch output")
                        .map_err(with_output_context)?;
                }
                if let Err(error) = page.close_async().await {
                    tracing::warn!(error = %error, "failed to close fetched page before browser shutdown");
                }
                if let Some(error) = http_failure {
                    return Err(error);
                }
                Ok(())
            }
            .await;
            finalize_fetch_browser(browser, args.cookie_jar.as_deref(), stdout, result)
                .map_err(|error| with_fetch_context(error, &args.url))?;
        }
        Commands::Serve(args) => {
            let additional_cookies = cookie_header_from_args(&config)?;
            config
                .browser
                .fetch_mut()
                .set_additional_cookies(additional_cookies);
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
            #[cfg(unix)]
            if args.remote_debugging_pipe {
                let (input, output) = take_inherited_pipe_descriptors()?;
                server
                    .serve_pipe(input, output)
                    .await
                    .context("protocol pipe failed")?;
                return Ok(());
            }
            server.serve().await.context("protocol server failed")?;
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

fn reject_conflicting_stdin_sources(args: &FetchArgs) -> Result<()> {
    if args.eval_file.as_deref() == Some(Path::new("-"))
        && args.wait_script_file.as_deref() == Some("-")
    {
        return Err(anyhow!(
            "`--eval-file -` and `--wait-script-file -` cannot both read from stdin"
        ));
    }
    anyhow::ensure!(
        !args.common.cookie_file.iter().any(|source| source == "-")
            || (args.eval_file.as_deref() != Some(Path::new("-"))
                && args.wait_script_file.as_deref() != Some("-")),
        "`--cookie -` cannot share stdin with `--eval-file -` or `--wait-script-file -`"
    );
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
    let mut request = Request::get(url)?
        .with_additional_cookies(cookie_header_from_args(config)?)
        .with_cookie_header_policy(CookieHeaderPolicy::PreserveExplicit);
    // Keep CLI-provided headers scoped to the initial document navigation.
    request.request_headers = config.fetch.request_headers.clone().into();
    Ok(request)
}

#[derive(Debug)]
struct CliHttpStatusFailure {
    status: u16,
    final_url: String,
}

impl fmt::Display for CliHttpStatusFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "final main document returned HTTP status {} at `{}`",
            self.status, self.final_url
        )
    }
}

impl std::error::Error for CliHttpStatusFailure {}

fn http_status_failure(args: &FetchArgs, status: u16, final_url: &str) -> Option<anyhow::Error> {
    if status < 400 || !(args.fail || args.fail_with_body) {
        return None;
    }
    Some(with_fetch_context(
        anyhow::Error::new(CliHttpStatusFailure {
            status,
            final_url: final_url.to_owned(),
        }),
        &args.url,
    ))
}

fn with_http_status_context(
    error: anyhow::Error,
    args: &FetchArgs,
    status: u16,
    final_url: &str,
) -> anyhow::Error {
    let Some(http_failure) = http_status_failure(args, status, final_url) else {
        return with_fetch_context(error, &args.url);
    };
    let reason = fetch_failure_reason(&error);
    let http_reason = fetch_failure_reason(&http_failure);
    if args.fail {
        with_fetch_context_reason(http_failure, &args.url, format!("{http_reason}; {reason}"))
    } else {
        // Keeping the processing error as the source preserves its exit code.
        // HTTP status is additional diagnostic context for --fail-with-body.
        with_fetch_context_reason(error, &args.url, format!("{reason}; {http_reason}"))
    }
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
    let reason = fetch_failure_reason(&error);
    with_fetch_context_reason(error, url, reason)
}

fn fetch_failure_reason(error: &anyhow::Error) -> String {
    if let Some(fetch) = error.downcast_ref::<CliFetchFailureContext>() {
        fetch.reason.clone()
    } else if let Some(failure) = error.downcast_ref::<NetworkFetchFailureContext>() {
        failure.reason().to_owned()
    } else if let Some(timeout) = error.downcast_ref::<FetchReadinessTimeout>() {
        timeout.to_string()
    } else {
        format!("{error:#}")
    }
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

/// Selects the curl-compatible HTTP failure code without changing other CLI errors.
pub fn error_exit_code(error: &anyhow::Error) -> ExitCode {
    if error.is::<CliHttpStatusFailure>() {
        ExitCode::from(22)
    } else {
        ExitCode::FAILURE
    }
}

fn one_line_reason(reason: &str) -> String {
    reason.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cookie_header_from_args(config: &AppConfig) -> Result<Option<String>> {
    let mut values = Vec::new();
    for source in &config.fetch.cookie_files {
        if source.contains('=') {
            anyhow::ensure!(
                !source.contains(['\r', '\n', '\0']),
                "--cookie data contains an invalid HTTP header character"
            );
            values.push(source.as_str());
        }
    }
    Ok((!values.is_empty()).then(|| values.join(";")))
}

fn load_cookie_state_cookies(config: &AppConfig) -> Result<Vec<moli_cookie_jar::StoredCookie>> {
    let mut cookies = Vec::new();
    for path in &config.fetch.cookie_files {
        if path.is_empty() || path.contains('=') {
            continue;
        }
        if path == "-" {
            cookies.extend(
                moli_cookie_cache::read_cookie_input_lossy(std::io::stdin().lock())
                    .context("failed to read --cookie from stdin")?,
            );
            continue;
        }
        // Like curl, a missing, unreadable, or directory input does not
        // turn the transfer into a file-reading failure.
        if let Ok(file) = std::fs::File::open(path) {
            match moli_cookie_cache::read_cookie_input_lossy(BufReader::new(file)) {
                Ok(loaded) => cookies.extend(loaded),
                Err(error) => tracing::debug!(error = %error, "could not read cookie input"),
            }
        }
    }
    Ok(cookies)
}

fn finalize_fetch_browser<W: Write>(
    browser: Browser,
    cookie_jar: Option<&str>,
    stdout: &mut W,
    result: Result<()>,
) -> Result<()> {
    if let Some(path) = cookie_jar {
        let saved = if path == "-" {
            cookie_cache::write_browser_cookie_file(&browser, stdout)
        } else {
            cookie_cache::save_browser_cookie_file(&browser, path)
        };
        match saved {
            Ok(report) if report.skipped_partitioned > 0 => tracing::warn!(
                skipped = report.skipped_partitioned,
                "omitted partitioned cookies from Netscape cookie jar"
            ),
            Err(error) => tracing::warn!(error = %error, "failed to save cookie jar"),
            Ok(_) => {}
        }
    }
    // Complete export before orderly browser teardown, including when fetching,
    // evaluation, rendering, or writing the response failed. As in curl, an
    // export failure does not replace the transfer's result.
    drop(browser);
    result
}

/// Claims descriptors 3 and 4, which Chromium's pipe protocol makes the parent
/// pass to the child, as the CDP input and output.
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
    // SAFETY: both descriptors are open pipes or sockets (checked above) and
    // this runs once per process, so each gets exactly one owner.
    Ok(unsafe { (OwnedFd::from_raw_fd(3), OwnedFd::from_raw_fd(4)) })
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
