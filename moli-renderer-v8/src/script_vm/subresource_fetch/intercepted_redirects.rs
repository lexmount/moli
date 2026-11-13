use super::*;
#[cfg(any(test, feature = "devtools"))]
use crate::RendererSyntheticResponseBody;
use moli_fetch::{RedirectInfo, Request, ResponseHead};

pub(super) fn follow_intercepted_redirect(
    mut request: Request,
    response: &crate::protocol_types::NavigationResponse,
    _document_url: &Url,
    web_security_disabled: bool,
) -> Result<Option<(Request, RedirectInfo)>, String> {
    let head = response.head();
    // An intercepted error-mode snapshot without Location is final. Preserve
    // main's admission behavior, including checking CORS before accepting it.
    if matches!(head.status, 301 | 302 | 303 | 307 | 308)
        && request.redirect_mode() == moli_fetch::RequestRedirectMode::Error
        && !head
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("location"))
    {
        if !web_security_disabled { request.validate_cors_response_for_url(&head.final_url, &head.headers)?; }
        return Ok(None);
    }
    let transition = crate::network_host::advance_subresource_response(
        &mut request,
        head,
        response.network_response_extra_info().cloned(),
        moli_fetch::RedirectSource::Network,
        web_security_disabled,
    )?;
    if transition != crate::network_host::SubresourceResponseTransition::FollowedRedirect {
        return Ok(None);
    }
    if moli_fetch::should_request_be_blocked_due_to_bad_port(&request.url) {
        return Err(format!("fetch: invalid redirect target `{}`", request.url));
    }
    let dispatched = request.capture_dispatched_request(request.request_headers.to_byte_strings());
    let mut chain = request.redirect_chain().to_vec();
    let redirect = chain.last_mut().expect("native hop followed a redirect");
    redirect.dispatched_request = Some(dispatched);
    let redirect = redirect.clone();
    Ok(Some((request.with_redirect_chain(chain), redirect)))
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
            request_include_credentials: request.allows_credentials_for_url(&request.url),
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
        // This head is used only to check the actual accepted redirect target
        // against the captured Document policy, before its next native pause.
        let redirect_head = ResponseHead {
            request_include_credentials: request.allows_credentials_for_url(&request.url),
            final_url: request.url.clone(),
            status: redirect.status,
            status_text: None,
            headers: redirect.headers.clone(),
            request_cookie_report: redirect.request_cookie_report.clone(),
            cookie_set_reports: redirect.cookie_set_reports.clone(),
            redirected: true,
            redirect_chain: request.redirect_chain().to_vec(),
            from_cache: redirect.from_cache,
            cache_state: Default::default(),
            negotiated_http_version: redirect.negotiated_http_version,
            preload_state: Default::default(),
        };
        let entered_window_realm = pending.execution_context.context_global().is_some();
        let failure = if let Some(context) = pending.execution_context.context_global() {
            self.renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = v8::Local::new(scope, context);
                    let scope = &mut v8::ContextScope::new(scope, context);
                    Ok(document_connect_csp_response_failure_message(
                        scope,
                        &context_host,
                        &pending,
                        &redirect_head,
                        true,
                        None,
                    ))
                })?
        } else {
            detached_window_fetch_csp_response_failure_message(
                &context_host,
                &pending,
                &redirect_head,
                true,
                None,
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
        if let Some((_, state, _)) = pending.continuation.csp_request_policy() {
            state.record_checked_url(&request.url, false);
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
