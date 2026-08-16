//! Core fetch bridge for synchronous renderer lifecycle-target decisions.
//!
//! The renderer owns the exact DCL/load boundary and any successor-navigation
//! grace period. The host keeps one deadline around the complete fetch, so a
//! lifecycle decision cannot reset or extend the caller's timeout budget.

use super::{
    Browser, FetchedDocument, RenderedDomWaitUntil, RendererLifecycleDecider,
    RendererLifecycleDecision, RendererLifecycleSnapshot, RendererReplyBoundary,
};
use anyhow::{Context, Result, anyhow};
use moli_fetch::Request;
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
        self.fetch_document_with_wait(
            request,
            wait_until,
            timeout,
            RendererReplyBoundary::Stage,
            Some(decider),
        )
        .await
        .with_context(|| {
            anyhow!(
                "failed while applying the {wait_until:?} lifecycle-target decision or following its successor navigation"
            )
        })
    }
}
