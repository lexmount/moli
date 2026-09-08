//! Native interception transport for resource loads.

mod context;
mod fetch;
mod result;
mod service_worker;
mod stream;
pub(crate) use context::{ControlledXhrContext, XhrInterceptionRoute};
use result::BufferedXhrResponse;
pub(crate) use result::{XhrLoadResult, start_buffered_xhr};
pub(crate) use stream::{ControlledResourceStream, ControlledXhrCompletion};

#[cfg(any(test, feature = "devtools"))]
use moli_fetch::ResponseHead;
use moli_fetch::{FetchCancelHandle, NetworkObservationJournal, Request, Response};
use tokio::sync::oneshot;

use crate::{
    document_runtime::DocumentConnectPolicySnapshot,
    network::{
        ResourceRequestClient,
        interception_control::{
            InterceptionPhase, ResourceInterceptionDecision, ResourceInterceptionPause,
        },
        loads::ResourceLoadLease,
    },
    runtime::RendererOwnerAction,
    types::{
        PendingSubresourceAuthInfo, PendingSubresourceContinueEvent, PendingSubresourceFetchInfo,
        PendingSubresourceResponseInfo, SubresourceRequestInitiatorType, SubresourceResponseBody,
    },
};

#[derive(Clone, Copy)]
enum ResourceResponsePolicy {
    Xhr,
    Fetch(crate::types::SubresourcePolicyContext),
}

pub(crate) struct ControlledResourceLoad {
    response_policy: ResourceResponsePolicy,
    service_worker: Option<service_worker::WorkerServiceWorkerFetch>,
    response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    request: Request,
    info: PendingSubresourceFetchInfo,
    load: ResourceLoadLease,
    cancel: FetchCancelHandle,
    interception: crate::network::interception_output::ResourceInterceptionContext,
    connect_policy: Option<DocumentConnectPolicySnapshot>,
    csp_violations: Vec<crate::content_security_policy::ContentSecurityPolicyUrlViolation>,
    preflight_headers: Vec<(String, String)>,
    initial_decision: Option<oneshot::Receiver<ResourceInterceptionDecision>>,
    request_initiator_type: SubresourceRequestInitiatorType,
}

use super::loaded_response::LoadedResponse;

impl LoadedResponse {
    async fn deliver(
        &self,
        info: &PendingSubresourceFetchInfo,
        stream: Option<&mut ControlledResourceStream>,
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
        let (response, body) = self.materialize().await?;
        Ok(Some(BufferedXhrResponse { response, body }))
    }
}

impl ControlledResourceLoad {
    pub(crate) fn with_fetch_policy(
        mut self,
        policy: crate::types::SubresourcePolicyContext,
    ) -> Self {
        self.response_policy = ResourceResponsePolicy::Fetch(policy);
        self
    }

    pub(crate) fn new(
        interception: crate::network::interception_output::ResourceInterceptionContext,
        request: Request,
        info: PendingSubresourceFetchInfo,
        load: ResourceLoadLease,
        cancel: FetchCancelHandle,
        request_initiator_type: SubresourceRequestInitiatorType,
    ) -> Self {
        Self {
            response_policy: ResourceResponsePolicy::Xhr,
            service_worker: None,
            response_filter: None,
            request,
            info,
            load,
            cancel,
            interception,
            connect_policy: None,
            csp_violations: Vec::new(),
            preflight_headers: Vec::new(),
            initial_decision: None,
            request_initiator_type,
        }
    }

    /// The Page can publish request admission in the initiating turn. Network
    /// execution then awaits the same native port, preserving async XHR's
    /// existing request-notification boundary without running a Page task.
    pub(crate) fn prepare_initial_pause(&mut self) -> RendererOwnerAction {
        let (pause, decision) = self.pause(InterceptionPhase::Request);
        self.initial_decision = Some(decision);
        self.interception.request_action(self.info.clone(), pause)
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
        mut stream: Option<ControlledResourceStream>,
    ) -> ControlledXhrCompletion {
        let result = self.run_inner(stream.as_mut()).await;
        let result = self.normalize_cancellation(result);
        self.load.finish();
        if let Some(body_source_id) = stream.as_ref().and_then(|stream| stream.body_source_id()) {
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
        let mut completed = XhrLoadResult::new(self.info.clone(), result, network_headers);
        completed.csp_violations = std::mem::take(&mut self.csp_violations);
        completed.network_record = completed
            .network_record
            .with_request_initiator_type(self.request_initiator_type);
        if completed.info.network_request_handle.is_some() {
            // Keep each hop's transport metadata. Protocol delivery skips
            // redirect announcements already published at the native boundary.
            self.interception
                .record_subresource_network(completed.network_record.clone());
        }
        self.publish_completed();
        ControlledXhrCompletion::Buffered(Box::new(completed))
    }

    fn normalize_cancellation<T>(&self, result: Result<T, String>) -> Result<T, String> {
        if result.is_err() && (self.cancel.is_cancelled() || self.load.is_cancelled()) {
            Err("net::ERR_ABORTED".into())
        } else {
            result
        }
    }

    fn publish_completed(&self) {
        let _ = self.interception.publish_continue(
            PendingSubresourceContinueEvent::Completed {
                internal_id: self.info.internal_id,
            },
            None,
        );
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
        (
            pause
                .with_worker_network(self.interception.is_worker())
                .with_request_redirect(redirect)
                .with_request_initiator_type(self.request_initiator_type),
            rx,
        )
    }

    async fn decision(
        &self,
        rx: oneshot::Receiver<ResourceInterceptionDecision>,
        phase: InterceptionPhase,
    ) -> Result<ResourceInterceptionDecision, String> {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err("net::ERR_ABORTED".into()),
            () = self.interception.worker_control_shutdown() => if self.load.disposition() != crate::network::loads::ResourceLoadDisposition::Keepalive {
                Err("net::ERR_ABORTED".into())
            } else { Ok(match phase {
                InterceptionPhase::Request => ResourceInterceptionDecision::ContinueRequest {url:None,method:None,body:None,headers:None,intercept_response:false,handle_auth_requests:false},
                InterceptionPhase::Response => ResourceInterceptionDecision::ContinueResponse {status:None,headers:None},
                InterceptionPhase::Auth => ResourceInterceptionDecision::CancelAuth,
            }) },
            result = rx => result.map_err(|_| "network interception owner was dropped".into()),
        }
    }

    fn sync_info(&mut self) {
        self.info.url = self.request.url.clone();
        self.info.method = self.request.method.clone();
        self.info.request_headers = self.request.interception_request_headers_with_web_security(
            self.load.request_client().web_security_disabled(),
        );
        self.info.request_body_bytes = self.request.body.clone();
        self.info.request_body = self
            .request
            .body
            .as_ref()
            .map(|body| String::from_utf8_lossy(body).into_owned());
    }

    fn apply_request_overrides(&mut self, overrides: moli_fetch::RequestOverrides) {
        let request_changed = overrides.url.is_some()
            || overrides.method.is_some()
            || overrides.body.is_some()
            || overrides.headers.is_some();
        let headers_changed = overrides.headers.is_some();
        overrides.apply(&mut self.request);
        if headers_changed {
            self.preflight_headers = self.request.request_headers.to_byte_strings();
        }
        self.sync_info();
        if request_changed && self.interception.is_worker() {
            self.interception.publish_network(
                crate::types::ScriptNetworkOutputItem::SubresourceRequestUpdated(Box::new(
                    self.worker_request_snapshot(None),
                )),
            );
        }
    }

    fn publish_worker_request(&self, redirect: Option<crate::types::NavigationRedirect>) {
        self.interception.publish_network(
            crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(
                self.worker_request_snapshot(redirect),
            )),
        );
    }

    fn worker_request_snapshot(
        &self,
        redirect: Option<crate::types::NavigationRedirect>,
    ) -> crate::types::SubresourceRequestSnapshot {
        crate::types::SubresourceRequestSnapshot::new(
            self.info
                .network_request_handle
                .expect("native resource load has a handle"),
            self.info.frame_id.clone(),
            self.info.document_url.clone(),
            self.info.url.clone(),
            self.info.method.clone(),
            self.info.request_headers.clone(),
            self.info.request_body.clone(),
            self.info.resource_type,
            self.request_initiator_type,
            self.info.request_cookie_report.clone(),
        )
        .with_request_body_bytes(self.info.request_body_bytes.clone())
        .with_redirect(redirect)
        .with_keepalive(
            self.load.disposition() == crate::network::loads::ResourceLoadDisposition::Keepalive,
        )
    }

    fn check_request(&mut self) -> Result<(), String> {
        let redirect_status = if self.request.redirect_count() == 0 {
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
        } else {
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
        };
        if self.interception.is_worker()
            && self.request.redirect_count() != 0
            && let Some(mut violation) = self.connect_policy.as_ref().and_then(|policy| {
                policy.report_only_violation(
                    &self.info.document_url,
                    &self.request.url,
                    redirect_status,
                )
            })
        {
            violation.blocked_uri = self.request.redirect_chain()[0].from_url.to_string();
            self.csp_violations.push(violation);
        }
        if let Some(policy) = &self.connect_policy
            && let Some(mut violation) = policy.enforce_violation(
                &self.info.document_url,
                &self.request.url,
                redirect_status,
            )
        {
            if self.request.redirect_count() != 0 {
                violation.blocked_uri = self
                    .request
                    .redirect_chain()
                    .first()
                    .expect("redirect chain is nonempty")
                    .from_url
                    .to_string();
            }
            let message = crate::document_runtime::document_content_security_policy_error_message(
                &violation,
                "XMLHttpRequest",
            );
            if self.interception.is_worker() {
                self.csp_violations.push(violation);
            }
            return Err(message);
        }
        if self.connect_policy.is_none() {
            self.request
                .admit_request_url_with_web_security(
                    &self.request.url,
                    self.load.request_client().web_security_disabled(),
                )
                .map_err(|error| error.to_string())?;
        }
        if matches!(self.response_policy, ResourceResponsePolicy::Fetch(_))
            && matches!(self.request.url.scheme(), "blob" | "data")
        {
            return Ok(());
        }
        moli_url_policy::ensure_http_network_transport_url(&self.request.url)
            .map_err(|error| error.to_string())
    }

    #[cfg(any(test, feature = "devtools"))]
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
            preload_state: Default::default(),
        })
    }

    async fn run_inner(
        &mut self,
        mut stream: Option<&mut ControlledResourceStream>,
    ) -> Result<(Option<BufferedXhrResponse>, Option<Vec<(String, String)>>), String> {
        use ResourceInterceptionDecision as D;
        let mut next_request_pause = self.initial_decision.is_some()
            || self
                .load
                .request_client()
                .page_network_policy()
                .should_intercept_subresource(self.info.resource_type);
        let mut announce_request = self.interception.is_worker();
        let mut intercept_response = false;
        let mut handle_auth_requests = false;
        let mut synthetic = None;
        let mut network_request_headers = None;
        let preflight_observer =
            super::preflight_events::CorsPreflightNetworkObserver::for_native_xhr(
                self.interception.clone(),
                &self.info,
            );
        loop {
            if self.cancel.is_cancelled() || self.load.is_cancelled() {
                return Err("net::ERR_ABORTED".into());
            }
            if announce_request {
                self.sync_info();
                self.publish_worker_request(
                    self.request
                        .redirect_chain()
                        .last()
                        .cloned()
                        .map(Into::into),
                );
            }
            if self.interception.worker_control_closed() {
                if self.load.disposition()
                    != crate::network::loads::ResourceLoadDisposition::Keepalive
                {
                    return Err("net::ERR_ABORTED".into());
                }
                next_request_pause = false;
            }
            if next_request_pause {
                self.sync_info();
                let rx = if let Some(rx) = self.initial_decision.take() {
                    rx
                } else {
                    let (pause, rx) = self.pause(InterceptionPhase::Request);
                    if !self.interception.publish_request(self.info.clone(), pause)
                        && !self.interception.worker_control_closed()
                    {
                        return Err("network interception output owner is closed".into());
                    }
                    rx
                };
                match self.decision(rx, InterceptionPhase::Request).await? {
                    D::ContinueRequest {
                        url,
                        method,
                        body,
                        headers,
                        intercept_response: response,
                        handle_auth_requests: auth,
                    } => {
                        self.apply_request_overrides(moli_fetch::RequestOverrides {
                            url,
                            method,
                            body,
                            headers,
                        });
                        intercept_response = response;
                        handle_auth_requests = auth;
                    }
                    #[cfg(any(test, feature = "devtools"))]
                    D::Fail(error) => return Err(error),
                    #[cfg(any(test, feature = "devtools"))]
                    D::Fulfill {
                        request_overrides,
                        status,
                        headers,
                        body,
                    } => {
                        if let Some(overrides) = request_overrides {
                            self.apply_request_overrides(*overrides);
                        }
                        synthetic = Some(self.synthetic_response(status, headers, body));
                        intercept_response = false;
                        handle_auth_requests = false;
                    }
                    _ => unreachable!("request pause accepts only request decisions"),
                }
            }
            self.check_request()?;
            if synthetic.is_none() && matches!(self.request.url.scheme(), "blob" | "data") {
                synthetic = Some(
                    crate::network_host::local_url_response_result(
                        &self.request.url,
                        &self.request.method,
                    )
                    .expect("local resource routing")
                    .map_err(|error| error.to_string())?,
                );
            }
            self.response_filter = None;
            if synthetic.is_none()
                && let Some(source) = self.service_worker.clone()
                && let Some(response) = self.fetch_service_worker_hop(source).await?
            {
                synthetic = Some(response);
            }
            let loader: ResourceRequestClient = self.load.request_client();
            let hop_cancel = self.cancel.child_for_subrequest();
            let (response, is_synthetic, observations) = match synthetic.take() {
                Some(response) => (
                    LoadedResponse::from_response(response),
                    true,
                    NetworkObservationJournal::default(),
                ),
                None => {
                    if self.load.network_offline() {
                        return Err("Network emulation offline".into());
                    }
                    let mut request = self.request.clone();
                    if self.interception.observes_request_headers()
                        && let Some(handle) = self.info.network_request_handle
                    {
                        let output = self.interception.clone();
                        let snapshot = (!output.is_worker())
                            .then(|| self.info.request_snapshot(
                                self.request_initiator_type,
                                self.load.disposition() == crate::network::loads::ResourceLoadDisposition::Keepalive,
                            ))
                            .flatten()
                            .map(Box::new);
                        request = request.with_network_request_observer(move |redirects, info| {
                            output.publish_network(
                                crate::types::ScriptNetworkOutputItem::SubresourceRequestExtraInfo {
                                    handle,
                                    request: snapshot.clone(),
                                    redirects: redirects.into_iter().map(Into::into).collect(),
                                    info: Box::new(info),
                                },
                            );
                        });
                    }
                    let observed = super::async_fetch::fetch_subresource_hop(
                        &loader,
                        request,
                        hop_cancel.clone(),
                        self.preflight_headers.clone(),
                        Some(&preflight_observer),
                    )
                    .await?;
                    let (response, observations) = observed.into_parts_with_observation_journal();
                    (response, false, observations)
                }
            };
            #[cfg(any(test, feature = "devtools"))]
            let mut is_synthetic = is_synthetic;
            if network_request_headers.is_none() {
                network_request_headers = observations
                    .final_request_observation()
                    .map(|request| request.headers().to_vec());
            }
            let network_response_extra_info =
                super::response_hop::network_response_extra_info(&response.head, &observations);
            let mut response = response;
            if handle_auth_requests
                && matches!(response.head.status, 401 | 407)
                && let Some(challenge) = crate::protocol_types::extract_subresource_auth_challenge(
                    &response.head.headers,
                )
            {
                let (pause, rx) = self.pause(InterceptionPhase::Auth);
                let pause = pause.with_response_body(response.body.clone());
                if !self.interception.publish_continue(
                    PendingSubresourceContinueEvent::AuthRequired(PendingSubresourceAuthInfo {
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
                    }),
                    Some(pause),
                ) && !self.interception.worker_control_closed()
                {
                    return Err("network authentication output owner is closed".into());
                }
                match self.decision(rx, InterceptionPhase::Auth).await? {
                    #[cfg(any(test, feature = "devtools"))]
                    D::Auth(auth) => {
                        if !self.load.try_restart_with_auth() {
                            return Err(moli_fetch::AuthRetryBudget::ERROR_TEXT.into());
                        }
                        self.request = self.request.clone().with_auth(auth.into());
                        announce_request = false;
                        next_request_pause = false;
                        continue;
                    }
                    D::CancelAuth => {}
                    #[cfg(any(test, feature = "devtools"))]
                    D::Fail(error) => return Err(error),
                    _ => unreachable!("auth pause accepts only auth decisions"),
                }
            }
            if intercept_response {
                let (pause, rx) = self.pause(InterceptionPhase::Response);
                let pause = pause.with_response_body(response.body.clone());
                if !self.interception.publish_continue(
                    PendingSubresourceContinueEvent::ResponsePaused(
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
                    ),
                    Some(pause),
                ) && !self.interception.worker_control_closed()
                {
                    return Err("network response output owner is closed".into());
                }
                match self.decision(rx, InterceptionPhase::Response).await? {
                    D::ContinueResponse { status, headers } => {
                        if let Some(status) = status {
                            response.head.status = status;
                            response.head.status_text = None;
                        }
                        if let Some(headers) = headers {
                            response.head.headers = headers;
                        }
                    }
                    #[cfg(any(test, feature = "devtools"))]
                    D::Fulfill {
                        request_overrides: _,
                        status,
                        headers,
                        body,
                    } => {
                        response = LoadedResponse::from_response(
                            self.synthetic_response(status, headers, body),
                        );
                        self.response_filter = None;
                        is_synthetic = true;
                    }
                    #[cfg(any(test, feature = "devtools"))]
                    D::Fail(error) => return Err(error),
                    _ => unreachable!("response pause accepts only response decisions"),
                }
            }
            use super::response_hop::{
                SubresourceResponseTransition, advance_subresource_response,
            };
            let transition = if matches!(self.response_policy, ResourceResponsePolicy::Fetch(_))
                && is_synthetic
                && self.request.redirect_mode() == moli_fetch::RequestRedirectMode::Error
                && !response
                    .head
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("location"))
            {
                if self.response_filter.is_none()
                    && !self.load.request_client().web_security_disabled()
                {
                    self.request.validate_cors_response_for_url(
                        &response.head.final_url,
                        &response.head.headers,
                    )?;
                }
                SubresourceResponseTransition::FinalResponse
            } else if self.response_filter.is_some() {
                super::response_hop::advance_validated_subresource_response(
                    &mut self.request,
                    response.head.clone(),
                    network_response_extra_info,
                    moli_fetch::RedirectSource::Network,
                    self.load.request_client().web_security_disabled(),
                )?
            } else {
                advance_subresource_response(
                    &mut self.request,
                    response.head.clone(),
                    network_response_extra_info,
                    moli_fetch::RedirectSource::Network,
                    self.load.request_client().web_security_disabled(),
                )?
            };
            let manual = matches!(
                transition,
                SubresourceResponseTransition::ManualResponse { .. }
            );
            if manual && matches!(self.response_policy, ResourceResponsePolicy::Xhr) {
                return Err("XHR requests must follow redirects".into());
            }
            if manual || transition == SubresourceResponseTransition::FinalResponse {
                response.head.redirect_chain = self.request.redirect_chain().to_vec();
                response.head.redirected = !response.head.redirect_chain.is_empty();
                // Client-provided responses have the same CORS obligations as
                // transport responses. Validate before any success is published.
                if self.response_filter.is_none()
                    && self.request.request_mode == moli_fetch::RequestMode::Cors
                {
                    super::validate_cors_response_chain(
                        self.request
                            .request_origin()
                            .expect("XHR has a requesting origin"),
                        &response.head,
                        self.request.credentials_mode,
                        self.load.request_client().web_security_disabled(),
                    )?;
                }
                if !is_synthetic {
                    self.cancel.adopt_response_progress(&hop_cancel);
                }
                if let Some(stream) = stream.as_deref_mut() {
                    stream.policy_violations(std::mem::take(&mut self.csp_violations));
                }
                // Whole-body security checks are needed for opaque responses,
                // not for every request authored with mode: "no-cors".
                let opaque_response =
                    matches!(self.response_policy, ResourceResponsePolicy::Fetch(_))
                        && super::network_response_filter(
                            self.request
                                .browser_origin()
                                .expect("native Fetch has a requesting origin"),
                            &response.head,
                            self.request.request_mode,
                            self.load.request_client().web_security_disabled(),
                        )
                        .is_some();
                let delivered = response
                    .deliver(
                        &self.info,
                        if is_synthetic || manual || opaque_response {
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
            // ExtraInfo describes the first on-wire request of the current hop.
            // Authentication retries retain it; a redirect starts a new hop.
            network_request_headers = None;
            // Retire the previous hop's protocol correlation before installing
            // its next one. The stable native network handle keeps Network ID.
            self.interception.publish_continue(
                PendingSubresourceContinueEvent::Completed {
                    internal_id: self.info.internal_id,
                },
                None,
            );
            next_request_pause = self
                .load
                .request_client()
                .page_network_policy()
                .should_intercept_subresource(self.info.resource_type);
            announce_request = self.interception.is_worker();
        }
    }
}
