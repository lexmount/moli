//! CLI fetch-readiness policy and ordering.
//!
//! The renderer keeps ownership of the individual lifecycle, response,
//! selector, and script wait state machines. This outer plan supplies all of
//! them with one absolute deadline and preserves the CLI's established order:
//! response first, then selector, then script. Completed response records are
//! retained by the Page, so starting that wait after lifecycle completion does
//! not lose an early matching response.

use super::redirect_navigation::{
    fetch_with_redirect_wait, remaining_wait_milliseconds, uses_redirect_wait,
};
use crate::cli::{FetchArgs, FetchWaitUntil};
use anyhow::{Context, Result, anyhow, bail};
use moli_core::{
    page::{Page, SubresourceResponseWaitCriteria},
    runtime::{
        Browser, FetchDeadline, FetchReadinessTimeout, FetchTimeoutPhase, FetchedDocument,
        RawDocumentFetchPolicy, RenderedDomWaitUntil,
    },
};
use moli_fetch::Request;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) struct ReadinessPlan {
    wait_until: RenderedDomWaitUntil,
    deadline: FetchDeadline,
    minimum_navigation_deadline: Instant,
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
        let minimum_navigation_deadline = Instant::now()
            .checked_add(Duration::from_millis(args.redirect_wait_ms))
            .context("response replacement-navigation wait exceeds the supported range")?;
        Ok(Self {
            wait_until: rendered_wait_until(args.wait_until),
            deadline: FetchDeadline::new(Duration::from_millis(args.timeout))
                .context("failed to create fetch readiness deadline")?,
            minimum_navigation_deadline,
            response,
            selector: args.wait_selector.clone(),
            script,
        })
    }

    pub(super) fn has_page_waits(&self) -> bool {
        self.response.is_some() || self.selector.is_some() || self.script.is_some()
    }

    pub(super) async fn fetch_document(
        &self,
        browser: &Browser,
        request: Request,
        raw_document_policy: RawDocumentFetchPolicy,
    ) -> Result<FetchedDocument> {
        if !self.has_page_waits()
            && self.response.is_none()
            && matches!(request.url.scheme(), "http" | "https")
        {
            return browser
                .fetch_request_document_allow_http_error_at_document_commit_with_deadline(
                    request,
                    self.wait_until,
                    self.deadline,
                    raw_document_policy,
                    self.minimum_navigation_deadline
                        .saturating_duration_since(Instant::now()),
                )
                .await;
        }
        match self.wait_until {
            RenderedDomWaitUntil::DomContentLoaded
            | RenderedDomWaitUntil::Load
            | RenderedDomWaitUntil::Done => {
                fetch_with_redirect_wait(
                    browser,
                    request,
                    self.wait_until,
                    self.deadline,
                    Duration::from_millis(remaining_wait_milliseconds(
                        self.minimum_navigation_deadline,
                        Instant::now(),
                    )),
                    raw_document_policy,
                )
                .await
            }
            RenderedDomWaitUntil::NetworkIdle | RenderedDomWaitUntil::DomStable => {
                browser
                    .fetch_request_document_allow_http_error_with_wait_until_deadline(
                        request,
                        self.wait_until,
                        self.deadline,
                        raw_document_policy,
                    )
                    .await
            }
        }
    }

    pub(super) async fn wait_for_page(&self, browser: &Browser, page: &mut Page) -> Result<bool> {
        let response_status = page.status();
        if !self.has_page_waits() && self.response.is_none() {
            if let Some(artifacts) = page.take_page_creation_artifacts() {
                let snapshot = artifacts.lifecycle_snapshot;
                if lifecycle_reached(snapshot, self.wait_until) {
                    if matches!(
                        self.wait_until,
                        RenderedDomWaitUntil::DomContentLoaded
                            | RenderedDomWaitUntil::Load
                            | RenderedDomWaitUntil::Done
                    ) && uses_redirect_wait(response_status)
                    {
                        browser
                            .wait_for_page_delay(
                                page,
                                Duration::from_millis(remaining_wait_milliseconds(
                                    self.minimum_navigation_deadline,
                                    Instant::now(),
                                ))
                                .min(self.deadline.remaining()),
                            )
                            .await
                            .context("failed while waiting for response replacement navigation")?;
                    }
                    return Ok(false);
                }
            }
            self.arm_lifecycle_wait(page).await?;
            page.release_committed_document_parser();
            let lifecycle_ready = self.wait_for_lifecycle(browser, page).await?;
            if lifecycle_ready {
                browser
                    .wait_for_page_readiness_with_deadline(page, self.wait_until, self.deadline)
                    .await
                    .context("failed while waiting for page readiness")?;
                if matches!(
                    self.wait_until,
                    RenderedDomWaitUntil::DomContentLoaded
                        | RenderedDomWaitUntil::Load
                        | RenderedDomWaitUntil::Done
                ) && uses_redirect_wait(response_status)
                {
                    let redirect_wait = Duration::from_millis(remaining_wait_milliseconds(
                        self.minimum_navigation_deadline,
                        Instant::now(),
                    ))
                    .min(self.deadline.remaining());
                    browser
                        .wait_for_page_delay(page, redirect_wait)
                        .await
                        .context("failed while waiting for response replacement navigation")?;
                }
            }
            return Ok(!lifecycle_ready);
        }

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

        Ok(false)
    }

    async fn arm_lifecycle_wait(&self, page: &mut Page) -> Result<()> {
        let (target, event, marker) = match self.wait_until {
            RenderedDomWaitUntil::DomContentLoaded | RenderedDomWaitUntil::DomStable => {
                ("document", "DOMContentLoaded", "__moliFetchDclReached")
            }
            RenderedDomWaitUntil::Load
            | RenderedDomWaitUntil::Done
            | RenderedDomWaitUntil::NetworkIdle => ("window", "load", "__moliFetchLoadReached"),
        };
        let expression = format!(
            "{target}.addEventListener('{event}', () => {{ globalThis.{marker} = true; }}, {{ once: true }}); true"
        );
        page.evaluate_runtime_expression_async(&expression)
            .await
            .context("failed to arm document lifecycle observation")?;
        Ok(())
    }

    async fn wait_for_lifecycle(&self, _browser: &Browser, page: &mut Page) -> Result<bool> {
        if let Some(artifacts) = page.take_page_creation_artifacts() {
            let snapshot = artifacts.lifecycle_snapshot;
            let reached = match self.wait_until {
                RenderedDomWaitUntil::DomContentLoaded | RenderedDomWaitUntil::DomStable => {
                    snapshot.dom_content_loaded.is_some() || snapshot.load.is_some()
                }
                RenderedDomWaitUntil::Load
                | RenderedDomWaitUntil::Done
                | RenderedDomWaitUntil::NetworkIdle => snapshot.load.is_some(),
            };
            if reached {
                return Ok(true);
            }
        }
        let mut ready_state = "loading".to_owned();
        while !self.deadline.remaining().is_zero() {
            let observation = page
                .evaluate_runtime_expression_async(&lifecycle_observation(self.wait_until))
                .await
                .context("failed while observing document lifecycle")?;
            let observation = observation
                .get("value")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow!("document lifecycle observation returned a non-string"))?;
            let observation: serde_json::Value = serde_json::from_str(observation)
                .context("failed to decode document lifecycle observation")?;
            ready_state = observation["readyState"]
                .as_str()
                .unwrap_or("loading")
                .to_owned();
            if observation["reached"].as_bool() == Some(true) {
                return Ok(true);
            }
            tokio::time::sleep(self.deadline.remaining().min(Duration::from_millis(25))).await;
        }

        let creation_phase = page.page_creation_phase();
        let phase_allows_snapshot = page.page_creation_is_waiting_for_lifecycle();
        if phase_allows_snapshot && matches!(ready_state.as_str(), "interactive" | "complete") {
            let timeout = FetchReadinessTimeout::new(
                self.deadline.timeout(),
                lifecycle_timeout_phase(self.wait_until),
            );
            tracing::warn!(
                page_id = page.page_id(),
                url = %page.requested_url(),
                final_url = %page.final_url(),
                wait_until = ?self.wait_until,
                ready_state = %ready_state,
                timeout_ms = self.deadline.timeout().as_millis(),
                remaining_ms = 0_u128,
                error = %timeout,
                "fetch readiness wait timed out; returning best-effort page"
            );
            Ok(false)
        } else {
            Err(anyhow::Error::new(FetchReadinessTimeout::new(
                self.deadline.timeout(),
                creation_phase
                    .map(timeout_phase_for_creation)
                    .unwrap_or_else(|| lifecycle_timeout_phase(self.wait_until)),
            )))
            .context("failed while waiting for document lifecycle readiness")
        }
    }
}

fn timeout_phase_for_creation(
    phase: moli_core::page::RendererPageCreationPhase,
) -> FetchTimeoutPhase {
    match phase {
        moli_core::page::RendererPageCreationPhase::StreamingMainBody => {
            FetchTimeoutPhase::StreamingMainBody
        }
        moli_core::page::RendererPageCreationPhase::ProcessingMainDocument => {
            FetchTimeoutPhase::ProcessingMainDocument
        }
        moli_core::page::RendererPageCreationPhase::WaitingForParserBlockingScript => {
            FetchTimeoutPhase::WaitingForParserBlockingScript
        }
        moli_core::page::RendererPageCreationPhase::WaitingForParserBlockingStylesheet => {
            FetchTimeoutPhase::WaitingForParserBlockingStylesheet
        }
        moli_core::page::RendererPageCreationPhase::WaitingForDomContentLoaded => {
            FetchTimeoutPhase::WaitingForDomContentLoaded
        }
        moli_core::page::RendererPageCreationPhase::WaitingForLoad => {
            FetchTimeoutPhase::WaitingForLoad
        }
    }
}

fn lifecycle_event(wait_until: RenderedDomWaitUntil) -> (&'static str, &'static str, &'static str) {
    match wait_until {
        RenderedDomWaitUntil::DomContentLoaded | RenderedDomWaitUntil::DomStable => {
            ("document", "DOMContentLoaded", "__moliFetchDclReached")
        }
        RenderedDomWaitUntil::Load
        | RenderedDomWaitUntil::Done
        | RenderedDomWaitUntil::NetworkIdle => ("window", "load", "__moliFetchLoadReached"),
    }
}

fn lifecycle_reached(
    snapshot: moli_core::page::RendererDocumentLifecycleSnapshot,
    wait_until: RenderedDomWaitUntil,
) -> bool {
    match wait_until {
        RenderedDomWaitUntil::DomContentLoaded | RenderedDomWaitUntil::DomStable => {
            snapshot.dom_content_loaded.is_some() || snapshot.load.is_some()
        }
        RenderedDomWaitUntil::Load
        | RenderedDomWaitUntil::Done
        | RenderedDomWaitUntil::NetworkIdle => snapshot.load.is_some(),
    }
}

fn lifecycle_observation(wait_until: RenderedDomWaitUntil) -> String {
    let (target, event, marker) = lifecycle_event(wait_until);
    let completed_dcl = matches!(
        wait_until,
        RenderedDomWaitUntil::DomContentLoaded | RenderedDomWaitUntil::DomStable
    );
    format!(
        "(() => {{ if (!globalThis.{marker}Listener) {{ globalThis.{marker}Listener = true; {target}.addEventListener('{event}', () => {{ globalThis.{marker} = true; }}, {{ once: true }}); }} const readyState = document.readyState; return JSON.stringify({{readyState, reached: Boolean(globalThis.{marker}{})}}); }})()",
        if completed_dcl {
            " || readyState === 'complete'"
        } else {
            ""
        }
    )
}

fn lifecycle_timeout_phase(wait_until: RenderedDomWaitUntil) -> FetchTimeoutPhase {
    match wait_until {
        RenderedDomWaitUntil::DomContentLoaded | RenderedDomWaitUntil::DomStable => {
            FetchTimeoutPhase::WaitingForDomContentLoaded
        }
        RenderedDomWaitUntil::Load
        | RenderedDomWaitUntil::Done
        | RenderedDomWaitUntil::NetworkIdle => FetchTimeoutPhase::WaitingForLoad,
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
        (None, Some(path)) => {
            super::read_script_file_arg("--wait-script-file", Path::new(path)).map(Some)
        }
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
