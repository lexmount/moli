use parking_lot::Mutex;
use std::sync::Arc;

use moli_fetch::RequestRedirectCheck;
use url::Url;

use crate::{
    content_security_policy::{
        ContentSecurityPolicySourceLocation, ContentSecurityPolicyUrlViolation, csp_url_for_report,
    },
    document_runtime::{
        DocumentConnectPolicySnapshot, document_content_security_policy_error_message,
    },
};

/// The last redirect target checked by this request's network producer. The
/// response path still checks synthetic responses whose URL was not checked.
#[derive(Clone)]
pub(crate) struct FetchCspRedirectState(Option<Arc<Mutex<Option<Url>>>>);

impl FetchCspRedirectState {
    pub(crate) fn new(policy: &DocumentConnectPolicySnapshot) -> Self {
        Self(policy.has_policies().then(|| Arc::new(Mutex::new(None))))
    }

    pub(crate) fn was_checked(&self, url: &Url) -> bool {
        self.0
            .as_ref()
            .is_some_and(|checked| checked.lock().as_ref() == Some(url))
    }

    pub(crate) fn redirect_check(
        &self,
        policy: DocumentConnectPolicySnapshot,
        document_url: Url,
        request_url: Url,
        report: impl Fn(ContentSecurityPolicyUrlViolation) + Send + Sync + 'static,
    ) -> Option<RequestRedirectCheck> {
        let checked = self.0.clone()?;
        Some(RequestRedirectCheck::new(move |next_url| {
            let (report_only, enforced) = policy
                .check_redirect(&document_url, next_url)
                .into_violations();
            let mut failure = None;
            for (is_enforced, mut violation) in report_only
                .into_iter()
                .map(|v| (false, v))
                .chain(enforced.into_iter().map(|v| (true, v)))
            {
                violation.blocked_uri = csp_url_for_report(&request_url);
                ContentSecurityPolicySourceLocation::default().apply_to(&mut violation);
                if is_enforced && failure.is_none() {
                    failure = Some(document_content_security_policy_error_message(
                        &violation, "fetch",
                    ));
                }
                report(violation);
            }
            *checked.lock() = Some(next_url.clone());
            failure.map_or(Ok(()), Err)
        }))
    }
}
