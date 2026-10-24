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
    native_bridge::{JsContextHost, OwnerDispatchScope},
    page_task_queue::RendererResourceCompletionSender,
};

use super::{WindowCspReportRequestContext, capture_window_csp_report_request_context};

/// The accepting Document's native policy and reporting address, without V8 roots.
pub(crate) struct WindowCspRequestContext {
    pub(crate) policy: DocumentConnectPolicySnapshot,
    pub(crate) redirect_state: FetchCspRedirectState,
    pub(crate) report_context: WindowCspReportRequestContext,
}

impl WindowCspRequestContext {
    pub(crate) fn capture(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        owner: OwnerDispatchScope,
    ) -> Option<Self> {
        let policy = host.document_connect_policy_snapshot_for_owner(owner)?;
        let report_context = capture_window_csp_report_request_context(scope, host, owner)?;
        Some(Self {
            redirect_state: FetchCspRedirectState::new(&policy),
            policy,
            report_context,
        })
    }

    pub(crate) fn redirect_check(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        document_url: &Url,
        request_url: &Url,
        completion_tx: RendererResourceCompletionSender,
    ) -> Option<RequestRedirectCheck> {
        window_csp_redirect_check(
            &self.policy,
            &self.redirect_state,
            &self.report_context,
            document_url,
            request_url,
            if self.policy.has_policies() {
                ContentSecurityPolicySourceLocation::capture(scope)
            } else {
                ContentSecurityPolicySourceLocation::default()
            },
            completion_tx,
        )
    }
}

pub(crate) fn window_csp_redirect_check(
    policy: &DocumentConnectPolicySnapshot,
    redirect_state: &FetchCspRedirectState,
    report_context: &WindowCspReportRequestContext,
    document_url: &Url,
    request_url: &Url,
    source_location: ContentSecurityPolicySourceLocation,
    completion_tx: RendererResourceCompletionSender,
) -> Option<RequestRedirectCheck> {
    if !policy.has_policies() {
        return None;
    }
    let report_context = report_context.clone();
    redirect_state.redirect_check(
        policy.clone(),
        document_url.clone(),
        request_url.clone(),
        move |mut violation| {
            source_location.apply_to(&mut violation);
            let _ = completion_tx.send_async_subresource_event(
                crate::types::AsyncSubresourceFetchEvent::ContentSecurityPolicyViolation {
                    report_context: Box::new(report_context.clone()),
                    violation: Box::new(violation),
                },
            );
        },
    )
}

struct CheckedCspRedirect {
    url: Url,
    blocked: bool,
}

/// The last redirect target checked by this request's network producer. The
/// response path still checks synthetic responses whose URL was not checked.
#[derive(Clone)]
pub(crate) struct FetchCspRedirectState(Option<Arc<Mutex<Option<CheckedCspRedirect>>>>);

impl FetchCspRedirectState {
    pub(crate) fn new(policy: &DocumentConnectPolicySnapshot) -> Self {
        Self(policy.has_policies().then(|| Arc::new(Mutex::new(None))))
    }

    pub(crate) fn was_checked(&self, url: &Url) -> bool {
        self.0.as_ref().is_some_and(|checked| {
            checked
                .lock()
                .as_ref()
                .is_some_and(|checked| &checked.url == url)
        })
    }

    pub(crate) fn was_blocked(&self) -> bool {
        self.0.as_ref().is_some_and(|checked| {
            checked
                .lock()
                .as_ref()
                .is_some_and(|checked| checked.blocked)
        })
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
            *checked.lock() = Some(CheckedCspRedirect {
                url: next_url.clone(),
                blocked: failure.is_some(),
            });
            failure.map_or(Ok(()), Err)
        }))
    }
}
