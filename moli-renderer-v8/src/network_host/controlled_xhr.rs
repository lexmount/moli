//! One native loading/decision path for intercepted asynchronous and synchronous XHR.

mod context;
mod result;
mod stream;
pub(crate) use context::{ControlledXhrContext, XhrInterceptionRoute};
use result::BufferedXhrResponse;
pub(crate) use result::{XhrLoadResult, start_buffered_xhr};
pub(crate) use stream::{ControlledXhrCompletion, ControlledXhrStream};

use moli_fetch::{FetchCancelHandle, Request, Response, ResponseHead};
use tokio::sync::oneshot;

use crate::{
    document_runtime::DocumentConnectPolicySnapshot,
    network::{
        ResourceInterceptionBody, ResourceRequestClient,
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
    request: Request,
    info: PendingSubresourceFetchInfo,
    load: ResourceLoadLease,
    cancel: FetchCancelHandle,
    source_document: RendererDocumentLifecycleIdentity,
    output: InterceptionOutput,
    connect_policy: DocumentConnectPolicySnapshot,
    preflight_headers: Vec<(String, String)>,
    initial_decision: Option<oneshot::Receiver<ResourceInterceptionDecision>>,
}

/// A hop is complete at headers. Its owner cancels any unread body on redirect,
/// authentication retry, fail or replacement, even if a protocol read holds a
/// clone of the body capability.
pub(super) struct LoadedXhrResponse {
    head: ResponseHead,
    body: ResourceInterceptionBody,
}

impl LoadedXhrResponse {
    pub(super) fn from_response(response: Response) -> Self {
        Self {
            head: response.head(),
            body: ResourceInterceptionBody::from_complete(
                SubresourceResponseBody::from_fetch_response(&response),
            ),
        }
    }

    pub(super) fn from_stream(
        response: moli_fetch::StreamingRawResponse,
        disk_pool: Option<moli_disk_pool::DiskPool>,
    ) -> Self {
        Self {
            head: response.head(),
            body: ResourceInterceptionBody::new(response, disk_pool),
        }
    }

    async fn deliver(
        &self,
        info: &PendingSubresourceFetchInfo,
        stream: Option<&mut ControlledXhrStream>,
        network_request_headers: Option<Vec<(String, String)>>,
    ) -> Result<Option<BufferedXhrResponse>, String> {
        if let Some(stream) = stream {
            stream.start(
                info,
                self.head.clone(),
                network_request_headers,
                self.body.clone(),
            )?;
            let mut offset = 0;
            loop {
                let (bytes, eof) = self
                    .body
                    .read_chunk(offset, 64 * 1024)
                    .await
                    .map_err(|error| format!("{error:#}"))?;
                if !bytes.is_empty() {
                    offset += bytes.len();
                    stream.chunk(bytes)?;
                }
                if eof {
                    break;
                }
            }
            // The stream consumer already owns delivery; no buffered Response is needed.
            return Ok(None);
        }
        let body = self
            .body
            .capture(usize::MAX)
            .await
            .map_err(|error| format!("{error:#}"))?;
        let bytes = body
            .materialize_bytes()
            .map_err(|error| error.to_string())?;
        Ok(Some(BufferedXhrResponse {
            response: Response::from_head_and_lossy_body_bytes(self.head.clone(), bytes),
            body,
        }))
    }
}

impl Drop for LoadedXhrResponse {
    fn drop(&mut self) {
        self.body.cancel();
    }
}

impl ControlledXhrLoad {
    /// The Page can publish request admission in the initiating turn. Network
    /// execution then awaits the same native port, preserving async XHR's
    /// existing request-notification boundary without running a Page task.
    pub(crate) fn prepare_initial_pause(&mut self) -> RendererOwnerAction {
        let (pause, decision) = self.pause(InterceptionPhase::Request);
        self.initial_decision = Some(decision);
        RendererOwnerAction::SubresourceFetchPause {
            redirect: None,
            source_document: self.source_document,
            info: Box::new(self.info.clone()),
            child_document: None,
            network_pause: Some(pause),
        }
    }

    pub(crate) async fn run(self) -> XhrLoadResult {
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
        if let Some(body_source_id) = stream.as_ref().and_then(|stream| stream.body_source_id) {
            self.publish_completed();
            return ControlledXhrCompletion::Streamed {
                internal_id: self.info.internal_id,
                body_source_id,
                result: result.map(|_| ()),
            };
        }
        let (result, network_headers) = match result {
            Ok((Some(response), headers)) => (Ok(response), headers),
            Ok((None, _)) => {
                unreachable!("streamed body requires its registered sink")
            }
            Err(error) => (Err(error), None),
        };
        let completed = XhrLoadResult::new(self.info.clone(), result, network_headers);
        if completed.info.network_request_handle.is_some() {
            // Redirects already crossed their own native request-pause boundary.
            // Diagnostics retain the full chain; protocol sees only the terminal.
            self.output.record_subresource_network(
                self.source_document,
                completed
                    .network_record
                    .clone()
                    .with_redirect_chain(Vec::new()),
            );
        }
        self.publish_completed();
        ControlledXhrCompletion::Buffered(Box::new(completed))
    }

    fn publish_completed(&self) {
        let _ = self
            .output
            .publish(RendererOwnerAction::SubresourceContinue {
                source_document: self.source_document,
                event: Box::new(PendingSubresourceContinueEvent::Completed {
                    internal_id: self.info.internal_id,
                }),
                network_pause: None,
            });
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
    ) -> Result<(Option<BufferedXhrResponse>, Option<Vec<(String, String)>>), String> {
        use ResourceInterceptionDecision as D;
        let mut next_request_pause = true;
        let mut intercept_response = false;
        let mut handle_auth_requests = false;
        let mut synthetic = None;
        let mut network_request_headers = None;
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
                // ExtraInfo describes the first on-wire request of this hop.
                // Authentication retries retain it; a redirect starts a new hop.
                network_request_headers = None;
                self.sync_info();
                let rx = if let Some(rx) = self.initial_decision.take() {
                    rx
                } else {
                    let (pause, rx) = self.pause(InterceptionPhase::Request);
                    if !self
                        .output
                        .publish(RendererOwnerAction::SubresourceFetchPause {
                            redirect: None,
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
            let (response, mut is_synthetic, observed_request_headers) = match synthetic.take() {
                Some(response) => (LoadedXhrResponse::from_response(response), true, None),
                None => {
                    let observed = super::async_fetch::fetch_controlled_subresource_hop(
                        &loader,
                        self.request.clone(),
                        hop_cancel.clone(),
                        self.preflight_headers.clone(),
                        &preflight_observer,
                    )
                    .await?;
                    let (response, observation) = observed.into_parts();
                    let headers = observation.map(|observation| observation.into_headers());
                    (response, false, headers)
                }
            };
            if network_request_headers.is_none() {
                network_request_headers = observed_request_headers;
            }
            let mut response = response;
            if handle_auth_requests
                && matches!(response.head.status, 401 | 407)
                && let Some(challenge) = crate::protocol_types::extract_subresource_auth_challenge(
                    &response.head.headers,
                )
            {
                let (pause, rx) = self.pause(InterceptionPhase::Auth);
                let pause = pause.with_response_body(response.body.clone());
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
                                request_cookie_report: response.head.request_cookie_report.clone(),
                                network_request_headers: network_request_headers.clone(),
                                challenge,
                                intercept_response,
                                response_final_url: response.head.final_url.clone(),
                                response_status: response.head.status,
                                response_headers: response.head.headers.clone(),
                                response_body: SubresourceResponseBody::from_bytes(Vec::new()),
                                response_from_cache: response.head.from_cache,
                                response_cache_state: response.head.cache_state,
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
                let pause = pause.with_response_body(response.body.clone());
                if !self
                    .output
                    .publish(RendererOwnerAction::SubresourceContinue {
                        source_document: self.source_document,
                        event: Box::new(PendingSubresourceContinueEvent::ResponsePaused(
                            PendingSubresourceResponseInfo {
                                internal_id: self.info.internal_id,
                                url: self.request.url.clone(),
                                final_url: response.head.final_url.clone(),
                                method: self.request.method.clone(),
                                request_headers: self.request.request_headers.clone(),
                                request_body: self.info.request_body.clone(),
                                resource_type: self.info.resource_type,
                                request_cookie_report: response.head.request_cookie_report.clone(),
                                network_request_headers: network_request_headers.clone(),
                                response_status: response.head.status,
                                response_headers: response.head.headers.clone(),
                                response_body: SubresourceResponseBody::from_bytes(Vec::new()),
                                from_cache: response.head.from_cache,
                                cache_state: response.head.cache_state,
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
                            response.head.status = status;
                            response.head.status_text = None;
                        }
                        if let Some(headers) = headers {
                            response.head.headers = headers;
                        }
                    }
                    D::Fulfill {
                        status,
                        headers,
                        body,
                    } => {
                        response = LoadedXhrResponse::from_response(
                            self.synthetic_response(status, headers, body),
                        );
                        is_synthetic = true;
                    }
                    D::Fail(error) => return Err(error),
                    _ => unreachable!("response pause accepts only response decisions"),
                }
            }
            use super::response_hop::{
                SubresourceResponseTransition, advance_subresource_response,
            };
            let transition =
                advance_subresource_response(&mut self.request, response.head.clone(), false)?;
            if let SubresourceResponseTransition::ManualResponse { .. } = transition {
                return Err("XHR requests must follow redirects".into());
            }
            if transition == SubresourceResponseTransition::FinalResponse {
                response.head.redirect_chain = self.request.redirect_chain().to_vec();
                response.head.redirected = !response.head.redirect_chain.is_empty();
                // Client-provided responses have the same CORS obligations as
                // transport responses. Validate before any success is published.
                super::validate_cors_response_chain(
                    self.request
                        .request_origin()
                        .expect("XHR has a requesting origin"),
                    &response.head,
                    self.request.credentials_mode,
                )?;
                if !is_synthetic {
                    self.cancel.adopt_response_progress(&hop_cancel);
                }
                let delivered = response
                    .deliver(
                        &self.info,
                        if is_synthetic {
                            None
                        } else {
                            stream.as_deref_mut()
                        },
                        network_request_headers.clone(),
                    )
                    .await?;
                return Ok((delivered, network_request_headers));
            }
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
