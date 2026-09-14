use std::cell::RefCell;
use std::rc::Rc;

use url::Url;

use crate::RendererSyntheticResponseBody;
use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyRedirectStatus,
    ContentSecurityPolicyResourceKind, ContentSecurityPolicyUrlViolation,
    ContentSecurityPolicyViolationEventFields, content_security_policy_report_requests,
    content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints,
    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints,
    create_security_policy_violation_event,
};
use crate::context_bootstrap::dispatch_simple_event_target_event;
use crate::network::ResourceResponseFailure;
use crate::network::ResourceTransfer;
use crate::network::loads::{ResourceLoadDisposition, ResourceLoadKind};
use crate::protocol_types::{PendingSubresourceFetchInfo, SubresourceRequestStarted};
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerFetchRequest, ServiceWorkerFetchResultSender,
    ServiceWorkerRequestDestination, service_worker_fetch_request_metadata,
};
use crate::types::{AsyncSubresourceNetworkContext, SubresourceResourceType};
use crate::worker::WorkerPendingFetchContinue;
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, Request, RequestResourceType,
    should_request_be_blocked_due_to_bad_port,
};

use super::{
    WORKER_GLOBAL_LISTENERS_SLOT, WorkerCspReport, WorkerGlobalState, next_fetch_id,
    request_body_text, worker_response_from_body,
};

pub(in crate::worker) fn dispatch_worker_content_security_policy_violation_event_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    violation: &ContentSecurityPolicyUrlViolation,
) {
    let global = scope.get_current_context().global(scope);
    let Some(event) = create_worker_content_security_policy_violation_event(scope, violation)
    else {
        return;
    };
    send_worker_content_security_policy_reports_for_state(state, violation);
    dispatch_simple_event_target_event(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        "securitypolicyviolation",
        event,
    );
}

pub(super) fn dispatch_worker_trusted_types_sink_violation_event_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    sink: &str,
    sample: &str,
) {
    let violation = {
        let state_ref = state.borrow();
        let Some(protected_url) = state_ref.current_script_url.as_ref() else {
            return;
        };
        content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints(
                &state_ref.content_security_policies,
                protected_url,
                sink,
                sample,
                ContentSecurityPolicyDisposition::Enforce,
                &state_ref.content_security_reporting_endpoints,
        )
    };
    if let Some(violation) = violation {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
}

fn create_worker_content_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    violation: &ContentSecurityPolicyUrlViolation,
) -> Option<v8::Local<'s, v8::Object>> {
    create_security_policy_violation_event(
        scope,
        &ContentSecurityPolicyViolationEventFields::from_url_violation(violation),
    )
}

fn send_worker_content_security_policy_reports_for_state(
    state: &Rc<RefCell<WorkerGlobalState>>,
    violation: &ContentSecurityPolicyUrlViolation,
) {
    let fields = ContentSecurityPolicyViolationEventFields::from_url_violation(violation);
    for request in content_security_policy_report_requests(
        &fields,
        &violation.report_uri_endpoints,
        &violation.report_to_endpoints,
    ) {
        send_worker_content_security_policy_report_for_state(state, request);
    }
}

fn send_worker_content_security_policy_report_for_state(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: Request,
) {
    if !matches!(request.url.scheme(), "http" | "https") {
        return;
    }
    let report = {
        let state = state.borrow();
        let Some(document_url) = state.current_script_url.clone() else {
            return;
        };
        let request = request
            .with_initiator_url(&document_url)
            .with_request_origin(moli_url::WebOrigin::from_url(&document_url))
            .with_network_partition_key(state.network_partition_key.clone())
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let Some(network) = ResourceTransfer::for_worker(
            state.global_kind.network(),
            state.parent_tx.network_observer(),
            |network| report_request_started(network, &document_url, &request),
        ) else {
            return;
        };
        let Some(load) = state.loader.register_load(
            ResourceLoadKind::CspReport,
            ResourceLoadDisposition::Keepalive,
            None,
        ) else {
            network.failed(&ResourceResponseFailure::Request(
                "csp report: worker global is shutting down".into(),
            ));
            return;
        };
        WorkerCspReport {
            network,
            load,
            document_url,
            request,
            policy_context: state.policy_context,
            service_worker_runtime: state.service_worker_runtime.clone(),
            service_worker_client_id: state.service_worker_client_id,
        }
    };
    if report.blocked() {
        return;
    }
    let intercept = {
        let state = state.borrow();
        state.fetch_subresource_interception_enabled
            && state
                .fetch_subresource_interception_resource_type
                .is_none_or(|expected| {
                    expected
                        .has_same_cdp_fetch_interception_type(SubresourceResourceType::CspReport)
                })
    };
    if !intercept {
        report.start_loading();
        return;
    }
    let handle = report.network.handle();
    let info = PendingSubresourceFetchInfo {
        internal_id: handle.get(),
        network_request_handle: Some(handle),
        frame_id: None,
        document_url: report.document_url.clone(),
        url: report.request.url.clone(),
        websocket_socket_id: None,
        method: report.request.method.clone(),
        request_headers: report.request.request_headers.clone(),
        request_body: request_body_text(&report.request.body),
        request_body_bytes: report.request.body.clone(),
        resource_type: SubresourceResourceType::CspReport,
        request_cookie_report: None,
    };
    let load = report.load.clone();
    let mut state = state.borrow_mut();
    let report_id = next_fetch_id(&mut state);
    state.pending_csp_reports.insert(report_id, report);
    super::publish_worker_fetch_pause(
        &state,
        crate::runtime::WorkerFetchTarget::CspReport(report_id),
        handle,
        load,
        crate::runtime::RendererWorkerFetchStage::Request(Box::new(info)),
    );
}

fn report_request_started(
    network: &crate::runtime::RendererNetworkRequest,
    document_url: &Url,
    request: &Request,
) -> SubresourceRequestStarted {
    super::worker_request_started(
        network,
        document_url,
        &request.url,
        &request.method,
        &request.request_headers,
        &request.body,
        SubresourceResourceType::CspReport,
    )
    .with_keepalive(true)
}

impl WorkerCspReport {
    fn fail(&self, message: String) {
        self.network
            .failed(&ResourceResponseFailure::Request(message));
        self.load.finish();
    }

    fn blocked(&self) -> bool {
        let error = if should_request_be_blocked_due_to_bad_port(&self.request.url) {
            Some(format!(
                "csp report: blocked bad port for `{}`",
                self.request.url
            ))
        } else if self.load.blocks_url(&self.request.url) {
            Some(crate::network_host::BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
        } else {
            None
        };
        if let Some(error) = error {
            self.fail(error);
            return true;
        }
        false
    }

    fn start_loading(mut self) {
        if self.load.network_offline() {
            self.fail("Network emulation offline".into());
            return;
        }
        if let (Some(runtime), Some(client_id)) = (
            self.service_worker_runtime.take(),
            self.service_worker_client_id,
        ) && runtime
            .matching_controller_for_client_fetch(client_id, &self.request.url)
            .is_some()
        {
            self.dispatch_to_service_worker(runtime, client_id);
            return;
        }
        self.spawn_network();
    }

    fn dispatch_to_service_worker(
        self,
        runtime: crate::service_worker_runtime::ServiceWorkerRuntimeService,
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
    ) {
        let cancel_handle = FetchCancelHandle::new();
        self.load.attach_cancel_handle(cancel_handle.clone());
        let request_client = self.load.request_client();
        let resource_task_runner = self.load.task_runner();
        let internal_id = self.load.id_for_diagnostics();
        let resource = crate::network_host::KeepaliveResource::new(self.network, self.load);
        let dispatch = ServiceWorkerFetchDispatch {
            internal_id,
            request: ServiceWorkerFetchRequest {
                client_id,
                resulting_client_id: None,
                url: self.request.url.clone(),
                method: self.request.method.clone(),
                headers: self.request.request_headers.to_byte_strings(),
                body: self.request.body.clone(),
                destination: ServiceWorkerRequestDestination::Report,
                request_mode: self.request.request_mode,
                credentials_mode: self.request.credentials_mode,
                redirect_mode: self.request.redirect_mode,
                priority: self.request.priority_hints.fetch_priority,
                is_reload: false,
                metadata: service_worker_fetch_request_metadata(&self.request),
            },
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                request_origin: moli_url::WebOrigin::from_url(&self.document_url),
                frame_id: None,
                document_url: self.document_url.clone(),
                resource_type: SubresourceResourceType::CspReport,
                policy_context: self.policy_context,
            },
            result_tx: ServiceWorkerFetchResultSender::CspReport {
                resource: resource.clone(),
                request: Box::new(self.request),
            },
            request_client,
            resource_task_runner,
            cancel_handle,
        };
        if !runtime.dispatch_controlled_fetch(dispatch) {
            resource.fail("service worker csp report fetch dispatch failed".into());
        }
    }

    fn spawn_network(self) {
        let Self {
            load,
            network,
            request,
            ..
        } = self;
        let cancel = FetchCancelHandle::new();
        load.attach_cancel_handle(cancel.clone());
        let loader = load.request_client();
        crate::network_host::KeepaliveResource::new(network, load).fetch(loader, request, cancel);
    }
}

pub(in crate::worker) fn continue_pending_worker_csp_report(
    state: &Rc<RefCell<WorkerGlobalState>>,
    continuation: WorkerPendingFetchContinue,
) {
    let Some(mut report) = state
        .borrow_mut()
        .pending_csp_reports
        .remove(&continuation.fetch_id)
    else {
        return;
    };
    let request = match Request::new_bytes(
        &continuation.method,
        continuation.url.as_str(),
        continuation.body,
        continuation.headers,
    ) {
        Ok(request) => request
            .with_redirect_headers(continuation.redirect_headers)
            .with_initiator_url(&report.document_url)
            .with_request_origin(moli_url::WebOrigin::from_url(&report.document_url))
            .with_resource_type(RequestResourceType::CspReport)
            .with_request_mode(report.request.request_mode)
            .with_credentials_mode(report.request.credentials_mode)
            .with_redirect_mode(report.request.redirect_mode)
            .with_network_partition_key(report.request.network_partition_key().map(str::to_owned))
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch),
        Err(error) => {
            report.fail(format!("csp report: {error}"));
            return;
        }
    };
    if request.url != report.request.url
        || request.method != report.request.method
        || request.request_headers != report.request.request_headers
        || request.body != report.request.body
    {
        report.network.update_request(|network| {
            report_request_started(network, &report.document_url, &request)
        });
    }
    report.request = request;
    if !report.blocked() {
        report.start_loading();
    }
}

pub(in crate::worker) fn fail_pending_worker_csp_report(
    state: &Rc<RefCell<WorkerGlobalState>>,
    continuation: WorkerPendingFetchContinue,
    error_text: String,
) {
    if let Some(report) = state
        .borrow_mut()
        .pending_csp_reports
        .remove(&continuation.fetch_id)
    {
        report.fail(error_text);
    }
}

pub(in crate::worker) fn fulfill_pending_worker_csp_report(
    state: &Rc<RefCell<WorkerGlobalState>>,
    continuation: WorkerPendingFetchContinue,
    response_code: u16,
    response_headers: Vec<(String, Vec<u8>)>,
    response_body: RendererSyntheticResponseBody,
) {
    if let Some(report) = state
        .borrow_mut()
        .pending_csp_reports
        .remove(&continuation.fetch_id)
    {
        report
            .network
            .response_completed(&worker_response_from_body(
                continuation.url,
                response_code,
                response_headers,
                response_body,
            ));
        report.load.finish();
    }
}

pub(super) fn worker_content_security_policy_violation(
    state: &WorkerGlobalState,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
) -> Option<ContentSecurityPolicyUrlViolation> {
    worker_content_security_policy_violation_with_redirect_status(
        state,
        protected_url,
        request_url,
        kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
}

pub(super) fn worker_content_security_policy_violation_with_redirect_status(
    state: &WorkerGlobalState,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
        &state.content_security_policies,
        protected_url,
        request_url,
        kind,
        redirect_status,
        ContentSecurityPolicyDisposition::Enforce,
        &state.content_security_reporting_endpoints,
    )
}

pub(super) fn worker_content_security_policy_violation_for_checked_url_with_redirect_status(
    state: &WorkerGlobalState,
    protected_url: &Url,
    checked_url: &Url,
    blocked_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
        &state.content_security_policies,
        protected_url,
        checked_url,
        blocked_url,
        kind,
        redirect_status,
        ContentSecurityPolicyDisposition::Enforce,
        &state.content_security_reporting_endpoints,
    )
}

pub(super) fn worker_content_security_policy_report_only_violation(
    state: &WorkerGlobalState,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
) -> Option<ContentSecurityPolicyUrlViolation> {
    worker_content_security_policy_report_only_violation_with_redirect_status(
        state,
        protected_url,
        request_url,
        kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
}

pub(super) fn worker_content_security_policy_report_only_violation_with_redirect_status(
    state: &WorkerGlobalState,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
        &state.content_security_report_only_policies,
        protected_url,
        request_url,
        kind,
        redirect_status,
        ContentSecurityPolicyDisposition::Report,
        &state.content_security_reporting_endpoints,
    )
}

pub(super) fn worker_content_security_policy_report_only_violation_for_checked_url_with_redirect_status(
    state: &WorkerGlobalState,
    protected_url: &Url,
    checked_url: &Url,
    blocked_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
        &state.content_security_report_only_policies,
        protected_url,
        checked_url,
        blocked_url,
        kind,
        redirect_status,
        ContentSecurityPolicyDisposition::Report,
        &state.content_security_reporting_endpoints,
    )
}

pub(super) fn dispatch_worker_content_security_policy_report_only_violation_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
) {
    dispatch_worker_content_security_policy_report_only_violation_with_redirect_status_for_state(
        scope,
        state,
        protected_url,
        request_url,
        kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    );
}

pub(super) fn dispatch_worker_content_security_policy_report_only_violation_with_redirect_status_for_state<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) {
    let violation = {
        let state_ref = state.borrow();
        worker_content_security_policy_report_only_violation_with_redirect_status(
            &state_ref,
            protected_url,
            request_url,
            kind,
            redirect_status,
        )
    };
    if let Some(violation) = violation {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
}

pub(super) fn dispatch_worker_content_security_policy_report_only_violation_for_checked_url_with_redirect_status_for_state<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    protected_url: &Url,
    checked_url: &Url,
    blocked_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) {
    let violation = {
        let state_ref = state.borrow();
        worker_content_security_policy_report_only_violation_for_checked_url_with_redirect_status(
            &state_ref,
            protected_url,
            checked_url,
            blocked_url,
            kind,
            redirect_status,
        )
    };
    if let Some(violation) = violation {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
}

pub(super) fn worker_content_security_policy_error_message(
    violation: &ContentSecurityPolicyUrlViolation,
    operation: &'static str,
) -> String {
    format!(
        "{operation}: blocked by Content Security Policy for `{}`.",
        violation.blocked_uri
    )
}
