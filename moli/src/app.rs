//! Callable command runner for the Moli CLI.

mod http_error_navigation;
mod readiness;

use std::{io::Write, sync::Arc};

use crate::{
    cli::{Cli, Commands, normalize_args_for_compat},
    config::AppConfig,
    cookie_cache, fetch_dump, robots,
};
use anyhow::Result;
use anyhow::{Context, anyhow};
use clap::Parser;
use moli_core::runtime::{
    Browser, FetchedDocument, NavigationRuntimeConfig, storage_partition::StoragePartitionState,
};
use moli_fetch::{NetworkFetchFailureContext, Request, ensure_http_status_success};
use moli_protocol_server::ProtocolServer;

use self::{http_error_navigation::is_http_error_status, readiness::ReadinessPlan};

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
        Commands::Fetch(args) => {
            let request = build_fetch_request(&args.url, &config)?;
            if config.browser.fetch().obey_robots() {
                // Checked before the browser starts so a refused fetch costs
                // nothing but the robots.txt request itself.
                robots::ensure_fetch_allowed(config.browser.fetch(), &request.url)
                    .await
                    .map_err(|error| with_fetch_context(error, &args.url))?;
            }
            let browser = Browser::new(config.browser.clone())
                .context("failed to initialize browser runtime")?;
            load_cookie_state(&browser, &config)?;
            let readiness =
                ReadinessPlan::from_fetch_args(&args, config.fetch.response_wait.clone())?;
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
                    if readiness.lifecycle_stage().is_some()
                        && is_http_error_status(raw_document.status())
                    {
                        let status_error = ensure_http_status_success(
                            raw_document.final_url().as_str(),
                            raw_document.status(),
                            false,
                        );
                        finalize_fetch_browser(browser);
                        return status_error
                            .context(
                                "HTTP error response is not an executable document and cannot navigate",
                            )
                            .with_context(|| anyhow!("failed to fetch `{}`", args.url));
                    }
                    if readiness.has_page_waits() || args.delay_ms > 0 {
                        finalize_fetch_browser(browser);
                        return Err(anyhow!(
                            "raw non-HTML document fetch does not support page wait options"
                        ));
                    }
                    let rendered =
                        fetch_dump::render_raw_document_dump(&raw_document, &config.fetch)
                            .context("failed to render raw fetch output")?;
                    stdout
                        .write_all(&rendered)
                        .context("failed to write raw fetch output")?;
                    let _ = stdout.flush();
                    finalize_fetch_browser(browser);
                    return Ok(());
                }
            };

            if readiness.lifecycle_stage().is_some() && is_http_error_status(page.status()) {
                let error = ensure_http_status_success(
                    page.final_url().as_str(),
                    page.status(),
                    false,
                )
                .context(
                    "navigation from the HTTP error document reached another HTTP error document",
                )
                .expect_err("HTTP error status must fail success validation");
                if let Err(close_error) = page.close_async().await {
                    tracing::warn!(
                        error = %close_error,
                        "failed to close fetched page after HTTP error navigation failure"
                    );
                }
                finalize_fetch_browser(browser);
                return Err(with_fetch_context(error, &args.url));
            }

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
                    .context("failed while waiting for page delay")?;
            }

            let rendered = fetch_dump::render_page_output_async(&mut page, &config.fetch)
                .await
                .context("failed to render fetch output")?;
            stdout
                .write_all(&rendered)
                .context("failed to write fetch output")?;
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

fn build_fetch_request(url: &str, config: &AppConfig) -> Result<Request> {
    let mut request = Request::get(url)?;
    // Keep CLI-provided headers scoped to the initial document navigation.
    request.request_headers = config.fetch.request_headers.clone();
    Ok(request)
}

fn with_fetch_context(error: anyhow::Error, url: &str) -> anyhow::Error {
    // Network failures already carry a URL-aware typed context. Adding the
    // same CLI context again would print two "failed to fetch" chain entries.
    if error.is::<NetworkFetchFailureContext>() {
        error
    } else {
        error.context(anyhow!("failed to fetch `{url}`"))
    }
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
