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

/// An import graph reads the Document's delivered policy when each request is
/// prepared. That request keeps its own snapshot through redirects and retries;
/// the exact Document owner receives reports with graph completion.
#[derive(Clone)]
pub(crate) struct StylesheetFetchCspContext {
    capture_check: Arc<dyn Fn() -> Box<Check> + Send + Sync>,
    violations: Arc<Mutex<Vec<ContentSecurityPolicyUrlViolation>>>,
}

impl StylesheetFetchCspContext {
    pub(crate) fn new<C>(capture_check: impl Fn() -> C + Send + Sync + 'static) -> Self
    where
        C: Fn(
                &Url,
                Option<&str>,
                ContentSecurityPolicyRedirectStatus,
            ) -> (
                Option<ContentSecurityPolicyUrlViolation>,
                Option<ContentSecurityPolicyUrlViolation>,
            ) + Send
            + Sync
            + 'static,
    {
        Self {
            capture_check: Arc::new(move || Box::new(capture_check())),
            violations: Arc::default(),
        }
    }

    pub(crate) fn for_request(&self, nonce: Option<&str>) -> StylesheetRequestCspCheck {
        let check = (self.capture_check)();
        let violations = self.violations.clone();
        let nonce = nonce.map(str::to_owned);
        let checked_hops = Mutex::new(HashSet::new());
        Arc::new(move |url, ordinal| {
            let (report, enforce) = check(
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
                let mut violations = violations.lock();
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
