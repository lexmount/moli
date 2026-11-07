use std::{collections::HashSet, sync::Arc};

use parking_lot::Mutex;
use url::Url;

use super::{ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyUrlViolation};

type Check = dyn Fn(
        &Url,
        Option<&str>,
        ContentSecurityPolicyRedirectStatus,
    ) -> (
        Option<ContentSecurityPolicyUrlViolation>,
        Option<ContentSecurityPolicyUrlViolation>,
    ) + Send
    + Sync;

pub(crate) type StylesheetRequestCspCheck =
    Arc<dyn Fn(&Url, usize) -> anyhow::Result<()> + Send + Sync>;

/// Captured Document policy for one import graph. Network tasks only evaluate
/// policy; the exact Document owner receives the reports with graph completion.
#[derive(Clone)]
pub(crate) struct StylesheetFetchCspContext {
    check: Arc<Check>,
    violations: Arc<Mutex<Vec<ContentSecurityPolicyUrlViolation>>>,
}

impl StylesheetFetchCspContext {
    pub(crate) fn new(
        check: impl Fn(
            &Url,
            Option<&str>,
            ContentSecurityPolicyRedirectStatus,
        ) -> (
            Option<ContentSecurityPolicyUrlViolation>,
            Option<ContentSecurityPolicyUrlViolation>,
        ) + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            check: Arc::new(check),
            violations: Arc::default(),
        }
    }

    pub(crate) fn for_request(&self, nonce: Option<&str>) -> StylesheetRequestCspCheck {
        let context = self.clone();
        let nonce = nonce.map(str::to_owned);
        let checked_hops = Mutex::new(HashSet::new());
        Arc::new(move |url, ordinal| {
            let (report, enforce) = (context.check)(
                url,
                nonce.as_deref(),
                if ordinal == 0 {
                    ContentSecurityPolicyRedirectStatus::NoRedirect
                } else {
                    ContentSecurityPolicyRedirectStatus::FollowedRedirect
                },
            );
            let error = enforce.as_ref().map(|violation| {
                anyhow::anyhow!(
                    "Stylesheet request blocked by Content Security Policy directive `{}`",
                    violation.effective_directive
                )
            });
            // Transport retries and cached response validation may check a hop
            // again. They must enforce policy without reporting it twice.
            if checked_hops.lock().insert((ordinal, url.clone())) {
                let mut violations = context.violations.lock();
                violations.extend(report);
                violations.extend(enforce);
            }
            error.map_or(Ok(()), Err)
        })
    }

    pub(crate) fn take_violations(&self) -> Vec<ContentSecurityPolicyUrlViolation> {
        std::mem::take(&mut *self.violations.lock())
    }
}
