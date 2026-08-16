//! CLI policy for replacing an HTTP error Document with its next navigation.
//!
//! HTTP status interpretation and timeout configuration belong to the CLI
//! layer. The renderer receives only the generic lifecycle-target decision.

use anyhow::Result;
use moli_core::runtime::{
    Browser, FetchDeadline, FetchedDocument, RenderedDomWaitUntil, RendererLifecycleDecision,
};
use moli_fetch::Request;
use std::time::Duration;

pub(super) fn is_http_error_status(status: u16) -> bool {
    (400..=599).contains(&status)
}

pub(super) async fn fetch_with_http_error_navigation(
    browser: &Browser,
    request: Request,
    wait_until: RenderedDomWaitUntil,
    deadline: FetchDeadline,
    navigation_grace: Duration,
) -> Result<FetchedDocument> {
    let navigation_grace_ms = navigation_grace.as_millis().min(u128::from(u64::MAX)) as u64;
    // The first DCL/load is delivered to this synchronous decision normally.
    // A 4xx/5xx may spend up to the configured grace waiting for a replacement
    // navigation. That wait and the successor's matching lifecycle stage both
    // consume the plan's absolute deadline; neither starts a fresh budget.
    browser
        .fetch_document_with_lifecycle_decider_and_deadline(
            request,
            wait_until,
            deadline,
            move |target| {
                Ok(if is_http_error_status(target.status) {
                    RendererLifecycleDecision::FollowNextDocument {
                        navigation_grace_ms,
                    }
                } else {
                    RendererLifecycleDecision::Finish
                })
            },
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::is_http_error_status;

    #[test]
    fn http_error_status_covers_only_four_hundred_and_five_hundred_ranges() {
        assert!(!is_http_error_status(399));
        assert!(is_http_error_status(400));
        assert!(is_http_error_status(499));
        assert!(is_http_error_status(500));
        assert!(is_http_error_status(599));
        assert!(!is_http_error_status(600));
    }
}
