//! One native loading/decision path for intercepted asynchronous and synchronous XHR.

mod stream;
pub(crate) use stream::{ControlledXhrBody, ControlledXhrCompletion, ControlledXhrStream};

use moli_fetch::{FetchCancelHandle, Request, Response, ResponseHead};
use tokio::sync::oneshot;

use crate::{
    document_runtime::DocumentConnectPolicySnapshot,
    network::{
        ResourceRequestClient,
        interception_control::{
            InterceptionPhase, ResourceInterceptionDecision, ResourceInterceptionPause,
        },
        interception_output::InterceptionOutput,
        loads::ResourceLoadLease,
    },
    runtime::{RendererDocumentLifecycleIdentity, RendererOwnerAction},
    types::{
        PendingSubresourceAuthInfo, PendingSubresourceContinueEvent, PendingSubresourceFetchInfo,
        PendingSubresourceResponseInfo, SubresourceResponseBody,
    },
};

pub(crate) struct ControlledXhrLoad {
    pub(crate) request: Request,
    pub(crate) info: PendingSubresourceFetchInfo,
    pub(crate) load: ResourceLoadLease,
    pub(crate) cancel: FetchCancelHandle,
    pub(crate) source_document: RendererDocumentLifecycleIdentity,
    pub(crate) output: InterceptionOutput,
    pub(crate) connect_policy: DocumentConnectPolicySnapshot,
    pub(crate) preflight_headers: Vec<(String, String)>,
    pub(crate) initial_decision: Option<oneshot::Receiver<ResourceInterceptionDecision>>,
}

#[derive(Debug)]
pub(crate) struct ControlledXhrResponse {
    pub(crate) result: Result<Response, String>,
    pub(crate) synthetic: bool,
    pub(crate) info: PendingSubresourceFetchInfo,
}

impl ControlledXhrLoad {
    /// The Page can publish request admission in the initiating turn. Network
    /// execution then awaits the same native port, preserving async XHR's
    /// existing request-notification boundary without running a Page task.
    pub(crate) fn prepare_initial_pause(&mut self) -> RendererOwnerAction {
        let (pause, decision) = self.pause(InterceptionPhase::Request);
        self.initial_decision = Some(decision);
        RendererOwnerAction::SubresourceFetchPause {
            source_document: self.source_document,
            info: Box::new(self.info.clone()),
            child_document: None,
            network_pause: Some(pause),
        }
    }

    pub(crate) async fn run(self) -> ControlledXhrResponse {
        match self.run_with_stream(None).await {
            ControlledXhrCompletion::Buffered(completed) => *completed,
            ControlledXhrCompletion::Streamed { .. } => {
                unreachable!("buffered XHR has no streaming sink")
            }
        }
    }

    pub(crate) async fn run_with_stream(
        mut self,
        mut stream: Option<ControlledXhrStream>,
    ) -> ControlledXhrCompletion {
        let result = self.run_inner(stream.as_mut()).await;
        // Network terminal facts belong to the load, including cancellation
        // after its Worker can no longer deliver an XHR callback. Page/Worker
        // delivery shares this same handle; protocol already commits each
        // request's terminal once.
        let record = match &result {
            Ok((loaded, _, network_headers)) => {
                let response = loaded.head();
                crate::types::SubresourceNetworkRecord::success_with_body(
                    self.info.frame_id.clone(),
                    self.info.document_url.clone(),
                    self.info.url.clone(),
                    self.info.method.clone(),
                    self.info.request_headers.clone(),
                    self.info.request_body.clone(),
                    self.info.resource_type,
                    response
                        .request_cookie_report
                        .clone()
                        .or_else(|| self.info.request_cookie_report.clone()),
                    // Each redirect was already announced with its next native pause.
                    Vec::new(),
                    response.final_url.clone(),
                    response.status,
                    response.headers.clone(),
                    loaded.body(),
                    response.cookie_set_reports.clone(),
                )
                .with_from_cache(response.from_cache)
                .with_negotiated_http_version(response.negotiated_http_version)
                .with_network_request_headers(network_headers.clone())
            }
            Err(error) => crate::types::SubresourceNetworkRecord::failure(
                self.info.frame_id.clone(),
                self.info.document_url.clone(),
                self.info.url.clone(),
                self.info.method.clone(),
                self.info.request_headers.clone(),
                self.info.request_body.clone(),
                self.info.resource_type,
                error.clone(),
            ),
        }
        .with_request_body_bytes(self.info.request_body_bytes.clone());
        if stream
            .as_ref()
            .is_none_or(|stream| stream.body_source_id.is_none())
            && let Some(handle) = self.info.network_request_handle
        {
            self.output.record_subresource_network(
                self.source_document,
                record.with_request_handle(handle),
            );
        }
        let _ = self
            .output
            .publish(RendererOwnerAction::SubresourceContinue {
                source_document: self.source_document,
                event: Box::new(PendingSubresourceContinueEvent::Completed {
                    internal_id: self.info.internal_id,
                }),
                network_pause: None,
            });
        if let Some(body_source_id) = stream.as_ref().and_then(|stream| stream.body_source_id) {
            return ControlledXhrCompletion::Streamed {
                internal_id: self.info.internal_id,
                body_source_id,
                result: result.map(|_| ()),
            };
        }
        let (result, synthetic) = match result {
            Ok((ControlledXhrBody::Buffered(response), synthetic, _)) => (Ok(response), synthetic),
            Ok((ControlledXhrBody::Streamed { .. }, _, _)) => {
                unreachable!("streamed body requires its registered sink")
            }
            Err(error) => (Err(error), false),
        };
        ControlledXhrCompletion::Buffered(Box::new(ControlledXhrResponse {
            result,
            synthetic,
            info: self.info,
        }))
    }

    fn pause(
        &self,
        phase: InterceptionPhase,
    ) -> (
        ResourceInterceptionPause,
        oneshot::Receiver<ResourceInterceptionDecision>,
    ) {
        let (pause, rx) = ResourceInterceptionPause::new(
            self.info.internal_id,
            phase,
            self.load.request_client().page_network_policy().clone(),
        );
        let redirect = (phase == InterceptionPhase::Request)
            .then(|| self.request.redirect_chain().last().cloned())
            .flatten();
        (pause.with_request_redirect(redirect), rx)
    }

    async fn decision(
        &self,
        rx: oneshot::Receiver<ResourceInterceptionDecision>,
    ) -> Result<ResourceInterceptionDecision, String> {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err("net::ERR_ABORTED".into()),
            result = rx => result.map_err(|_| "network interception owner was dropped".into()),
        }
    }

    fn sync_info(&mut self) {
        self.info.url = self.request.url.clone();
        self.info.method = self.request.method.clone();
        self.info.request_headers = self.request.request_headers.clone();
        self.info.request_body_bytes = self.request.body.clone();
        self.info.request_body = self
            .request
            .body
            .as_ref()
            .map(|body| String::from_utf8_lossy(body).into_owned());
    }

    fn check_request(&self) -> Result<(), String> {
        let redirect_status = if self.request.redirect_count() == 0 {
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
        } else {
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
        };
        if let Some(violation) = self.connect_policy.enforce_violation(
            &self.info.document_url,
            &self.request.url,
            redirect_status,
        ) {
            return Err(
                crate::document_runtime::document_content_security_policy_error_message(
                    &violation,
                    "XMLHttpRequest",
                ),
            );
        }
        moli_url_policy::ensure_http_network_transport_url(&self.request.url)
            .map_err(|error| error.to_string())
    }

    fn synthetic_response(
        &self,
        status: u16,
        headers: Vec<(String, Vec<u8>)>,
        body: crate::RendererSyntheticResponseBody,
    ) -> Response {
        body.into_fetch_response(ResponseHead {
            final_url: self.request.url.clone(),
            status,
            status_text: None,
            headers,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: self.request.redirect_count() != 0,
            redirect_chain: self.request.redirect_chain().to_vec(),
            from_cache: false,
            cache_state: Default::default(),
            negotiated_http_version: None,
        })
    }

    async fn run_inner(
        &mut self,
        mut stream: Option<&mut ControlledXhrStream>,
    ) -> Result<(ControlledXhrBody, bool, Option<Vec<(String, String)>>), String> {
        use ResourceInterceptionDecision as D;
        let mut next_request_pause = true;
        let mut intercept_response = false;
        let mut handle_auth_requests = false;
        let mut synthetic = None;
        let preflight_observer =
            super::preflight_events::CorsPreflightNetworkObserver::for_native_xhr(
                self.output.clone(),
                self.source_document,
                &self.info,
            );
        loop {
            if self.cancel.is_cancelled() || self.load.is_cancelled() {
                return Err("net::ERR_ABORTED".into());
            }
            if next_request_pause {
                self.sync_info();
                let rx = if let Some(rx) = self.initial_decision.take() {
                    rx
                } else {
                    let (pause, rx) = self.pause(InterceptionPhase::Request);
                    if !self
                        .output
                        .publish(RendererOwnerAction::SubresourceFetchPause {
                            source_document: self.source_document,
                            info: Box::new(self.info.clone()),
                            child_document: None,
                            network_pause: Some(pause),
                        })
                    {
                        return Err("network interception output owner is closed".into());
                    }
                    rx
                };
                match self.decision(rx).await? {
                    D::ContinueRequest {
                        url,
                        method,
                        body,
                        headers,
                        intercept_response: response,
                        handle_auth_requests: auth,
                    } => {
                        if let Some(url) = url {
                            self.request.url = url;
                        }
                        if let Some(method) = method {
                            self.request.method = method.clone();
                            self.request.cookie_context.set_request_method(&method);
                        }
                        self.request.cookie_context = self
                            .request
                            .cookie_context
                            .clone()
                            .recompute_site_context_for_request(&self.request.url);
                        if let Some(body) = body {
                            let text = body.as_ref().map(|bytes| {
                                std::sync::Arc::from(String::from_utf8_lossy(bytes).as_ref())
                            });
                            self.request.set_body(body, text);
                        }
                        if let Some(headers) = headers {
                            match headers {
                                moli_fetch::RequestHeaderOverride::CurrentRequest {
                                    headers,
                                    redirect_headers,
                                } => {
                                    self.request.set_redirect_headers(Some(
                                        redirect_headers.unwrap_or_else(|| {
                                            self.request.request_headers.clone()
                                        }),
                                    ));
                                    self.request.request_headers = headers;
                                }
                                moli_fetch::RequestHeaderOverride::RedirectChain(headers) => {
                                    self.request.set_redirect_headers(None);
                                    self.request.request_headers = headers;
                                }
                            }
                            self.preflight_headers = self.request.request_headers.to_byte_strings();
                        }
                        intercept_response = response;
                        handle_auth_requests = auth;
                        self.sync_info();
                    }
                    D::Fail(error) => return Err(error),
                    D::Fulfill {
                        status,
                        headers,
                        body,
                    } => {
                        synthetic = Some(self.synthetic_response(status, headers, body));
                        intercept_response = false;
                        handle_auth_requests = false;
                    }
                    _ => unreachable!("request pause accepts only request decisions"),
                }
            }
            self.check_request()?;
            let loader: ResourceRequestClient = self.load.request_client();
            let hop_cancel = self.cancel.child_for_subrequest();
            let (response, mut is_synthetic, network_request_headers) = match synthetic.take() {
                Some(response) => (ControlledXhrBody::Buffered(response), true, None),
                None => {
                    let observed = super::async_fetch::fetch_controlled_subresource_hop(
                        &loader,
                        self.request.clone(),
                        hop_cancel.clone(),
                        self.preflight_headers.clone(),
                        &preflight_observer,
                        if intercept_response {
                            None
                        } else {
                            stream.as_deref_mut().map(|stream| (stream, &self.info))
                        },
                        handle_auth_requests,
                    )
                    .await?;
                    let (response, observation) = observed.into_parts();
                    let headers = observation.map(|observation| observation.into_headers());
                    (response, false, headers)
                }
            };
            let mut response = match response {
                ControlledXhrBody::Buffered(response) => response,
                ControlledXhrBody::Streamed { mut head, body } => {
                    head.redirect_chain = self.request.redirect_chain().to_vec();
                    head.redirected = !head.redirect_chain.is_empty();
                    self.cancel.adopt_response_progress(&hop_cancel);
                    return Ok((
                        ControlledXhrBody::Streamed { head, body },
                        false,
                        network_request_headers,
                    ));
                }
            };
            if handle_auth_requests
                && matches!(response.status, 401 | 407)
                && let Some(challenge) =
                    crate::protocol_types::extract_subresource_auth_challenge(&response.headers)
            {
                let (pause, rx) = self.pause(InterceptionPhase::Auth);
                if !self
                    .output
                    .publish(RendererOwnerAction::SubresourceContinue {
                        source_document: self.source_document,
                        event: Box::new(PendingSubresourceContinueEvent::AuthRequired(
                            PendingSubresourceAuthInfo {
                                internal_id: self.info.internal_id,
                                url: self.request.url.clone(),
                                method: self.request.method.clone(),
                                request_headers: self.request.request_headers.clone(),
                                request_body: self.info.request_body.clone(),
                                resource_type: self.info.resource_type,
                                request_cookie_report: response.request_cookie_report.clone(),
                                network_request_headers: network_request_headers.clone(),
                                challenge,
                                intercept_response,
                                response_final_url: response.final_url.clone(),
                                response_status: response.status,
                                response_headers: response.headers.clone(),
                                response_body: SubresourceResponseBody::from_fetch_response(
                                    &response,
                                ),
                                response_from_cache: response.from_cache,
                                response_cache_state: response.cache_state,
                            },
                        )),
                        network_pause: Some(pause),
                    })
                {
                    return Err("network authentication output owner is closed".into());
                }
                match self.decision(rx).await? {
                    D::Auth(auth) => {
                        if !self.load.try_restart_with_auth() {
                            return Err(moli_fetch::AuthRetryBudget::ERROR_TEXT.into());
                        }
                        self.request = self.request.clone().with_auth(auth.into());
                        next_request_pause = false;
                        continue;
                    }
                    D::CancelAuth => {}
                    D::Fail(error) => return Err(error),
                    _ => unreachable!("auth pause accepts only auth decisions"),
                }
            }
            if intercept_response {
                let (pause, rx) = self.pause(InterceptionPhase::Response);
                if !self
                    .output
                    .publish(RendererOwnerAction::SubresourceContinue {
                        source_document: self.source_document,
                        event: Box::new(PendingSubresourceContinueEvent::ResponsePaused(
                            PendingSubresourceResponseInfo {
                                internal_id: self.info.internal_id,
                                url: self.request.url.clone(),
                                final_url: response.final_url.clone(),
                                method: self.request.method.clone(),
                                request_headers: self.request.request_headers.clone(),
                                request_body: self.info.request_body.clone(),
                                resource_type: self.info.resource_type,
                                request_cookie_report: response.request_cookie_report.clone(),
                                network_request_headers: network_request_headers.clone(),
                                response_status: response.status,
                                response_headers: response.headers.clone(),
                                response_body: SubresourceResponseBody::from_fetch_response(
                                    &response,
                                ),
                                from_cache: response.from_cache,
                                cache_state: response.cache_state,
                            },
                        )),
                        network_pause: Some(pause),
                    })
                {
                    return Err("network response output owner is closed".into());
                }
                match self.decision(rx).await? {
                    D::ContinueResponse { status, headers } => {
                        if let Some(status) = status {
                            response.status = status;
                            response.status_text = None;
                        }
                        if let Some(headers) = headers {
                            response.headers = headers;
                        }
                    }
                    D::Fulfill {
                        status,
                        headers,
                        body,
                    } => {
                        response = self.synthetic_response(status, headers, body);
                        is_synthetic = true;
                    }
                    D::Fail(error) => return Err(error),
                    _ => unreachable!("response pause accepts only response decisions"),
                }
            }
            if !is_synthetic {
                self.request
                    .validate_cors_response_for_url(&response.final_url, &response.headers)?;
            }
            let head = response.head();
            self.request.observe_redirect_requests(&head.redirect_chain);
            let Some(url) = crate::network_host::next_redirect_url(
                &head.final_url,
                head.status,
                &head.headers,
                self.request.redirect_count(),
            )?
            else {
                response.redirect_chain = self.request.redirect_chain().to_vec();
                response.redirected = !response.redirect_chain.is_empty();
                if !is_synthetic {
                    self.cancel.adopt_response_progress(&hop_cancel);
                }
                return Ok((
                    ControlledXhrBody::Buffered(response),
                    is_synthetic,
                    network_request_headers,
                ));
            };
            self.request
                .follow_redirect(moli_fetch::RedirectInfo {
                    source: moli_fetch::RedirectSource::Network,
                    from_url: head.final_url,
                    to_url: url,
                    status: head.status,
                    headers: head.headers,
                    network_extra_info_available: false,
                    request_extra_info: None,
                    dispatched_request: None,
                    response_extra_info: None,
                    redirect_has_extra_info: false,
                    request_cookie_report: head.request_cookie_report,
                    cookie_set_reports: head.cookie_set_reports,
                    from_cache: head.from_cache,
                    negotiated_http_version: head.negotiated_http_version,
                })
                .map_err(|error| error.to_string())?;
            self.preflight_headers = self.request.request_headers.to_byte_strings();
            // Retire the previous hop's protocol correlation before installing
            // its next one. The stable native network handle keeps Network ID.
            self.output
                .publish(RendererOwnerAction::SubresourceContinue {
                    source_document: self.source_document,
                    event: Box::new(PendingSubresourceContinueEvent::Completed {
                        internal_id: self.info.internal_id,
                    }),
                    network_pause: None,
                });
            next_request_pause = true;
        }
    }
}
