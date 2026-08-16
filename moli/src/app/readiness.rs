//! CLI fetch-readiness policy and ordering.
//!
//! The renderer keeps ownership of the individual lifecycle, response,
//! selector, and script wait state machines. This outer plan supplies all of
//! them with one absolute deadline and preserves the CLI's established order:
//! response first, then selector, then script. Completed response records are
//! retained by the Page, so starting that wait after lifecycle completion does
//! not lose an early matching response.

use super::http_error_navigation::fetch_with_http_error_navigation;
use crate::cli::{FetchArgs, FetchWaitUntil};
use anyhow::{Context, Result, anyhow, bail};
use moli_core::{
    page::{Page, SubresourceResponseWaitCriteria},
    runtime::{Browser, FetchDeadline, FetchedDocument, PageVmInitStage, RenderedDomWaitUntil},
};
use moli_fetch::Request;
use std::time::Duration;

#[derive(Debug)]
pub(super) struct ReadinessPlan {
    wait_until: RenderedDomWaitUntil,
    deadline: FetchDeadline,
    redirect_wait: Duration,
    response: Option<SubresourceResponseWaitCriteria>,
    selector: Option<String>,
    script: Option<String>,
}

impl ReadinessPlan {
    pub(super) fn from_fetch_args(
        args: &FetchArgs,
        response: Option<SubresourceResponseWaitCriteria>,
    ) -> Result<Self> {
        let script = resolve_wait_script(args)?;
        Ok(Self {
            wait_until: rendered_wait_until(args.wait_until),
            deadline: FetchDeadline::new(Duration::from_millis(args.timeout))
                .context("failed to create fetch readiness deadline")?,
            redirect_wait: Duration::from_millis(args.redirect_wait_ms),
            response,
            selector: args.wait_selector.clone(),
            script,
        })
    }

    pub(super) fn lifecycle_stage(&self) -> Option<PageVmInitStage> {
        // Only concrete Document lifecycle milestones opt into HTTP-error
        // navigation recovery. `done` retains the historical load boundary;
        // network-idle and DOM-stable retain best-effort HTTP-error dumps.
        match self.wait_until {
            RenderedDomWaitUntil::DomContentLoaded => Some(PageVmInitStage::DomContentLoaded),
            RenderedDomWaitUntil::Load | RenderedDomWaitUntil::Done => Some(PageVmInitStage::Load),
            RenderedDomWaitUntil::NetworkIdle | RenderedDomWaitUntil::DomStable => None,
        }
    }

    pub(super) fn has_page_waits(&self) -> bool {
        self.response.is_some() || self.selector.is_some() || self.script.is_some()
    }

    pub(super) async fn fetch_document(
        &self,
        browser: &Browser,
        request: Request,
    ) -> Result<FetchedDocument> {
        match self.wait_until {
            RenderedDomWaitUntil::DomContentLoaded
            | RenderedDomWaitUntil::Load
            | RenderedDomWaitUntil::Done => {
                fetch_with_http_error_navigation(
                    browser,
                    request,
                    self.wait_until,
                    self.deadline,
                    self.redirect_wait,
                )
                .await
            }
            RenderedDomWaitUntil::NetworkIdle | RenderedDomWaitUntil::DomStable => {
                browser
                    .fetch_request_document_allow_http_error_with_wait_until_deadline(
                        request,
                        self.wait_until,
                        self.deadline,
                    )
                    .await
            }
        }
    }

    pub(super) async fn wait_for_page(&self, browser: &Browser, page: &mut Page) -> Result<()> {
        if let Some(response) = self.response.clone() {
            browser
                .wait_for_subresource_response_with_deadline(page, response, self.deadline)
                .await
                .context("failed while waiting for subresource response")?;
        }

        if let Some(selector) = self.selector.as_deref() {
            browser
                .wait_for_selector_with_deadline(page, selector, self.deadline)
                .await
                .with_context(|| anyhow!("failed while waiting for selector `{selector}`"))?;
        }

        if let Some(script) = self.script.as_deref() {
            browser
                .wait_for_script_truthy_with_deadline(page, script, self.deadline)
                .await
                .context("failed while waiting for script to become truthy")?;
        }

        Ok(())
    }
}

fn rendered_wait_until(wait_until: FetchWaitUntil) -> RenderedDomWaitUntil {
    match wait_until {
        FetchWaitUntil::DomContentLoaded => RenderedDomWaitUntil::DomContentLoaded,
        FetchWaitUntil::Load => RenderedDomWaitUntil::Load,
        FetchWaitUntil::NetworkIdle => RenderedDomWaitUntil::NetworkIdle,
        FetchWaitUntil::DomStable => RenderedDomWaitUntil::DomStable,
        FetchWaitUntil::Done => RenderedDomWaitUntil::Done,
    }
}

fn resolve_wait_script(args: &FetchArgs) -> Result<Option<String>> {
    match (
        args.wait_script.as_deref(),
        args.wait_script_file.as_deref(),
    ) {
        (Some(_), Some(_)) => {
            bail!("`--wait-script` and `--wait-script-file` are mutually exclusive")
        }
        (Some(script), None) => Ok(Some(script.to_owned())),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map(Some)
            .with_context(|| anyhow!("failed to read wait script file `{path}`")),
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadinessPlan, resolve_wait_script};
    use crate::cli::{Cli, Commands};
    use anyhow::Result;
    use clap::Parser;

    fn fetch_args(extra: &[&str]) -> Box<crate::cli::FetchArgs> {
        let mut raw = vec!["moli", "fetch"];
        raw.extend_from_slice(extra);
        raw.push("https://example.test/");
        match Cli::try_parse_from(raw)
            .expect("fetch arguments should parse")
            .command
        {
            Commands::Fetch(args) => args,
            command => panic!("expected fetch command, got {command:?}"),
        }
    }

    #[test]
    fn plan_collects_every_post_lifecycle_wait() -> Result<()> {
        let args = fetch_args(&[
            "--wait-selector",
            "#ready",
            "--wait-script",
            "globalThis.ready",
        ]);
        let plan = ReadinessPlan::from_fetch_args(&args, Some(Default::default()))?;

        assert!(plan.has_page_waits());
        assert_eq!(plan.selector.as_deref(), Some("#ready"));
        assert_eq!(plan.script.as_deref(), Some("globalThis.ready"));
        assert!(plan.response.is_some());
        Ok(())
    }

    #[test]
    fn plan_without_response_selector_or_script_has_no_page_waits() -> Result<()> {
        let args = fetch_args(&[]);
        let plan = ReadinessPlan::from_fetch_args(&args, None)?;

        assert!(!plan.has_page_waits());
        Ok(())
    }

    #[test]
    fn wait_script_sources_are_mutually_exclusive_before_fetch() {
        let args = fetch_args(&[
            "--wait-script",
            "true",
            "--wait-script-file",
            "/does/not/matter.js",
        ]);
        let error = resolve_wait_script(&args).unwrap_err();

        assert_eq!(
            error.to_string(),
            "`--wait-script` and `--wait-script-file` are mutually exclusive"
        );
    }
}
