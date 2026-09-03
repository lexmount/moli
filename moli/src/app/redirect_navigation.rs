//! CLI policy for replacing a non-success Document with its next navigation.
//!
//! HTTP status interpretation and timeout configuration belong to the CLI
//! layer. The renderer receives only the generic lifecycle-target decision.

use anyhow::{Context, Result};
use moli_core::runtime::{
    Browser, FetchDeadline, FetchedDocument, RawDocumentFetchPolicy, RenderedDomWaitUntil,
    RendererLifecycleDecision,
};
use moli_fetch::Request;
use std::time::{Duration, Instant};

pub(super) fn uses_redirect_wait(status: u16) -> bool {
    (300..=599).contains(&status)
}

pub(super) async fn fetch_with_redirect_wait(
    browser: &Browser,
    request: Request,
    wait_until: RenderedDomWaitUntil,
    deadline: FetchDeadline,
    minimum_navigation_wait: Duration,
    raw_document_policy: RawDocumentFetchPolicy,
) -> Result<FetchedDocument> {
    let minimum_navigation_deadline = Instant::now()
        .checked_add(minimum_navigation_wait)
        .context("response replacement-navigation wait exceeds the supported range")?;
    // The first DCL/load is delivered to this synchronous decision normally.
    // A 3xx/4xx/5xx Document keeps running until the configured minimum time
    // from fetch start has elapsed, giving client-side responses a chance to
    // replace it. The initial lifecycle load therefore consumes this window
    // instead of receiving a fresh grace period afterward. The outer readiness
    // deadline still caps this wait and any successor lifecycle.
    browser
        .fetch_document_with_lifecycle_decider_and_deadline(
            request,
            wait_until,
            deadline,
            raw_document_policy,
            move |target| {
                Ok(if uses_redirect_wait(target.status) {
                    RendererLifecycleDecision::FollowNextDocumentOrFinish {
                        navigation_grace_ms: remaining_wait_milliseconds(
                            minimum_navigation_deadline,
                            Instant::now(),
                        ),
                    }
                } else {
                    RendererLifecycleDecision::Finish
                })
            },
        )
        .await
}

fn remaining_wait_milliseconds(deadline: Instant, now: Instant) -> u64 {
    deadline
        .saturating_duration_since(now)
        .as_nanos()
        .div_ceil(1_000_000)
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::{remaining_wait_milliseconds, uses_redirect_wait};
    use std::time::{Duration, Instant};

    #[test]
    fn redirect_wait_status_covers_three_hundred_through_five_hundred_ranges() {
        assert!(!uses_redirect_wait(299));
        assert!(uses_redirect_wait(300));
        assert!(uses_redirect_wait(304));
        assert!(uses_redirect_wait(399));
        assert!(uses_redirect_wait(400));
        assert!(uses_redirect_wait(499));
        assert!(uses_redirect_wait(500));
        assert!(uses_redirect_wait(599));
        assert!(!uses_redirect_wait(600));
    }

    #[test]
    fn replacement_wait_is_the_unconsumed_minimum_from_fetch_start() {
        let started = Instant::now();
        let deadline = started + Duration::from_millis(1_000);

        assert_eq!(remaining_wait_milliseconds(deadline, started), 1_000);
        assert_eq!(
            remaining_wait_milliseconds(deadline, started + Duration::from_millis(275)),
            725
        );
        assert_eq!(
            remaining_wait_milliseconds(deadline, started + Duration::from_millis(1_000)),
            0
        );
        assert_eq!(
            remaining_wait_milliseconds(deadline, started + Duration::from_millis(1_500)),
            0
        );
    }

    #[test]
    fn replacement_wait_rounds_up_a_partial_millisecond() {
        let now = Instant::now();
        let deadline = now + Duration::from_nanos(1_000_001);

        assert_eq!(remaining_wait_milliseconds(deadline, now), 2);
    }
}
