//! Core fetch bridge for synchronous renderer lifecycle-target decisions.
//!
//! The renderer owns the exact DCL/load boundary and any successor-navigation
//! grace period. The host keeps one deadline around the complete fetch, so a
//! lifecycle decision cannot reset or extend the caller's timeout budget.

use super::{
    Browser, FetchDeadline, FetchedDocument, RenderedDomWaitUntil, RendererLifecycleDecider,
    RendererLifecycleDecision, RendererLifecycleSnapshot, RendererReplyBoundary,
};
use anyhow::{Result, anyhow};
use moli_fetch::{NetworkFetchFailureContext, Request};
use std::time::Duration;

impl Browser {
    /// Fetches an executable document with a synchronous one-shot policy at
    /// the exact requested lifecycle target.
    ///
    /// The decision runs in the renderer owner turn that observes DCL/load;
    /// it does not expose an intermediate Page or require a second owner
    /// command. The original `timeout` covers the request, the first lifecycle
    /// target, any successor-navigation grace period, and the successor target.
    pub async fn fetch_document_with_lifecycle_decider<F>(
        &self,
        request: Request,
        wait_until: RenderedDomWaitUntil,
        timeout: Duration,
        decider: F,
    ) -> Result<FetchedDocument>
    where
        F: FnOnce(RendererLifecycleSnapshot) -> Result<RendererLifecycleDecision> + Send + 'static,
    {
        let deadline = FetchDeadline::new(timeout)?;
        self.fetch_document_with_lifecycle_decider_and_deadline(
            request, wait_until, deadline, decider,
        )
        .await
    }

    /// Applies a lifecycle decision using a caller-owned absolute deadline.
    /// The same deadline can then gate response, selector, and script waits.
    pub async fn fetch_document_with_lifecycle_decider_and_deadline<F>(
        &self,
        request: Request,
        wait_until: RenderedDomWaitUntil,
        deadline: FetchDeadline,
        decider: F,
    ) -> Result<FetchedDocument>
    where
        F: FnOnce(RendererLifecycleSnapshot) -> Result<RendererLifecycleDecision> + Send + 'static,
    {
        anyhow::ensure!(
            matches!(
                wait_until,
                RenderedDomWaitUntil::DomContentLoaded
                    | RenderedDomWaitUntil::Load
                    | RenderedDomWaitUntil::Done
            ),
            "a lifecycle decider requires DCL, load, or done"
        );
        let decider = RendererLifecycleDecider::new(decider);
        self.fetch_document_to_base_stage(
            request,
            wait_until,
            deadline,
            RendererReplyBoundary::Stage,
            Some(decider),
        )
        .await
        .map_err(|error| {
            // This typed context already supplies the caller-facing fetch
            // message. Keep it outermost instead of obscuring it with a
            // lifecycle layer that adds no actionable network information.
            if error.is::<NetworkFetchFailureContext>() {
                error
            } else {
                error.context(anyhow!(
                    "failed while applying the {wait_until:?} lifecycle-target decision or following its successor navigation"
                ))
            }
        })
    }
}
