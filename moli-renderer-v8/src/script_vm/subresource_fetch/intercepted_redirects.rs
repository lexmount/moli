use super::*;
#[cfg(any(test, feature = "devtools"))]
use crate::RendererSyntheticResponseBody;
#[cfg(any(test, feature = "devtools"))]
use moli_fetch::ResponseHead;
use moli_fetch::{RedirectInfo, Request, RequestRedirectMode};

pub(super) fn follow_intercepted_redirect(
    mut request: Request,
    response: &crate::protocol_types::NavigationResponse,
    document_url: &Url,
    web_security_disabled: bool,
) -> Result<Option<(Request, RedirectInfo)>, String> {
    // A manual response does not follow Location, so even an invalid URL must
    // remain opaque. The completion path still checks the response's CORS policy.
    if request.redirect_mode() == RequestRedirectMode::Manual {
        return Ok(None);
    }
    let head = response.head();
    if matches!(head.status, 301 | 302 | 303 | 307 | 308) {
        // A fulfilled network response has the same CORS gate as a real hop.
        // Preserve that failure before rejecting an error-mode redirect.
        if !web_security_disabled {
            request.validate_cors_response_for_url(&head.final_url, &head.headers)?;
        }
        if request.redirect_mode() == RequestRedirectMode::Error
            && head
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("location"))
        {
            return Err("fetch: redirect mode is error".to_owned());
        }
    }
    let Some(mut target) = crate::network_host::next_redirect_url(
        &head.final_url,
        head.status,
        &head.headers,
        request.redirect_count(),
    )?
    else {
        return Ok(None);
    };
    let includes_credentials = !target.username().is_empty() || target.password().is_some();
    // Like Chromium's credentialed-subresource check, allow credentials already
    // present in the same-origin embedding document. Fetch's CORS taint remains
    // sticky even when a later hop returns to that origin.
    let credentials_blocked = includes_credentials
        && (request
            .request_origin()
            .is_none_or(|origin| !origin.same_origin(&(&target).into()))
            || target.username() != document_url.username()
            || target.password() != document_url.password()
            || (request.request_mode == moli_fetch::RequestMode::Cors
                && request.has_cross_origin_url(&head.final_url)));
    if !matches!(target.scheme(), "http" | "https")
        || credentials_blocked
        || moli_fetch::should_request_be_blocked_due_to_bad_port(&target)
    {
        return Err(format!("fetch: invalid redirect target `{target}`"));
    }
    if target.fragment().is_none() {
        target.set_fragment(head.final_url.fragment());
    }
    let cross_origin = !moli_url::same_origin(&head.final_url, &target);
    let response_extra_info = response.network_response_extra_info().cloned();
    let mut redirect = head.into_redirect(
        target,
        moli_fetch::RedirectSource::Network,
        response_extra_info.is_some(),
    );
    redirect.request_extra_info = response.network_request_extra_info().cloned();
    redirect.response_extra_info = response_extra_info;
    request
        .follow_redirect_with_web_security(redirect, web_security_disabled)
        .map_err(|error| error.to_string())?;
    if cross_origin {
        request
            .request_headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
        let mut chain = request.redirect_chain().to_vec();
        chain
            .last_mut()
            .expect("followed redirect")
            .dispatched_request =
            Some(request.capture_dispatched_request(request.request_headers.to_byte_strings()));
        request = request.with_redirect_chain(chain);
    }
    let redirect = request
        .redirect_chain()
        .last()
        .expect("followed redirect")
        .clone();
    Ok(Some((request, redirect)))
}

impl ScriptVm {
    #[cfg(any(test, feature = "devtools"))]
    pub(in crate::script_vm::subresource_fetch) fn fulfill_intercepted_window_fetch(
        &mut self,
        pending: PendingSubresourceFetchState,
        status: u16,
        headers: Vec<(String, Vec<u8>)>,
        body: RendererSyntheticResponseBody,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let request = super::pending_fetch::request_for_pending_subresource(
            &pending,
            &pending.info.url,
            &pending.info.method,
            pending.info.request_headers.clone(),
            pending.info.request_body_bytes.clone(),
        );
        let head = ResponseHead {
            final_url: request.url.clone(),
            status,
            status_text: None,
            headers,
            request_cookie_report: pending.info.request_cookie_report.clone(),
            cookie_set_reports: Vec::new(),
            redirected: !request.redirect_chain().is_empty(),
            redirect_chain: request.redirect_chain().to_vec(),
            from_cache: false,
            cache_state: Default::default(),
            negotiated_http_version: None,
            preload_state: Default::default(),
        };
        let info = pending.info.clone();
        let activity = self.resolve_pending_subresource_fetch_body(
            pending,
            info.url,
            info.method,
            info.request_headers,
            info.request_body,
            None,
            false,
            None,
            None,
            Ok(body.into_navigation_response(head)),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    pub(super) fn continue_intercepted_window_redirect(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        request: Request,
        redirect: RedirectInfo,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let context_host = self._context_host.clone();
        let entered_window_realm = pending.execution_context.context_global().is_some();
        let failure = if let Some(context) = pending.execution_context.context_global() {
            self.renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = v8::Local::new(scope, context);
                    let scope = &mut v8::ContextScope::new(scope, context);
                    Ok(document_connect_csp_redirect_failure_message(
                        scope,
                        &context_host,
                        &pending,
                        &request.url,
                    ))
                })?
        } else {
            detached_window_fetch_csp_redirect_failure_message(
                &context_host,
                &pending,
                &request.url,
            )
        };
        let failure = failure.or_else(|| {
            self._context_host
                .borrow()
                .is_url_blocked(&request.url)
                .then(|| crate::network_host::BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
        });
        if let Some(message) = failure {
            let info = pending.info.clone();
            let activity = self.resolve_pending_subresource_fetch_body(
                pending,
                info.url,
                info.method,
                info.request_headers,
                info.request_body,
                None,
                false,
                None,
                None,
                Err(message),
            )?;
            return Ok(if entered_window_realm {
                AsyncSubresourceFetchBodyActivity::WindowRealmEntered
            } else {
                activity
            });
        }
        let loader = pending.load.request_client();
        let request = loader.bind_browser_site_context(request);
        pending.info.request_cookie_report = if request.allows_credentials_for_url(&request.url) {
            moli_fetch::observe_cookie_access_report_for_request(
                &loader.cookie_store(),
                &request.url,
                request.network_cookie_context(),
            )
            .ok()
            .flatten()
        } else {
            None
        };
        pending.update_request_snapshot(request);
        self._context_host
            .borrow_mut()
            .repause_window_fetch_after_redirect(pending, redirect);
        Ok(if entered_window_realm {
            AsyncSubresourceFetchBodyActivity::WindowRealmEntered
        } else {
            AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered
        })
    }
}
