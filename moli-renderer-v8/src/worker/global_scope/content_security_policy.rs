use std::cell::RefCell;
use std::rc::Rc;

use url::Url;

use crate::RendererSyntheticResponseBody;
use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyNonUrlKind,
    ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyResourceKind,
    ContentSecurityPolicyUrlViolation, ContentSecurityPolicyViolationEventFields,
    content_security_policy_non_url_violation_with_source, content_security_policy_report_requests,
    content_security_policy_trusted_types_policy_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints,
    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints,
    create_security_policy_violation_event, send_content_security_policy_reports,
};
use crate::context_bootstrap::dispatch_simple_event_target_event;
use crate::network::loads::{ResourceLoadDisposition, ResourceLoadKind, ResourceLoadLease};
use crate::protocol_types::{
    PendingSubresourceContinueEvent, PendingSubresourceFetchInfo, SubresourceNetworkRecord,
    SubresourceNetworkRequestHandle, SubresourceResponseBody,
};
use crate::service_worker_runtime::{
    ServiceWorkerDirectFetchResult, ServiceWorkerFetchDispatch, ServiceWorkerFetchRequest,
    ServiceWorkerRequestDestination, service_worker_fetch_request_metadata,
};
use crate::types::{AsyncSubresourceNetworkContext, SubresourceResourceType};
use crate::worker::handle::WorkerPendingSubresourceFetch;
use crate::worker::{WorkerPendingFetchContinue, WorkerToParentMessage};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, Request, RequestResourceType,
    should_request_be_blocked_due_to_bad_port,
};

use super::{
    PendingWorkerCspReport, WORKER_GLOBAL_LISTENERS_SLOT, WorkerGlobalState, next_fetch_id,
    record_worker_subresource_failure_with_handle, request_body_text,
};

pub(super) fn dispatch_worker_content_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request_client: &crate::network::context::WorkerResourceLoader,
    violation: &ContentSecurityPolicyUrlViolation,
) {
    let global = scope.get_current_context().global(scope);
    let Some(event) = create_worker_content_security_policy_violation_event(scope, violation)
    else {
        return;
    };
    let fields = ContentSecurityPolicyViolationEventFields::from_url_violation(violation);
    send_content_security_policy_reports(
        request_client.request_client(),
        &fields,
        &violation.report_uri_endpoints,
        &violation.report_to_endpoints,
    );
    dispatch_simple_event_target_event(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        "securitypolicyviolation",
        event,
    );
}

pub(super) fn dispatch_worker_content_security_policy_violation_event_for_state<'s>(
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

pub(super) fn allows_worker_trusted_type_policy_name_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    policy_name: &str,
    is_duplicate: bool,
) -> bool {
    let (report_only_violation, enforced_violation) = {
        let state_ref = state.borrow();
        let Some(protected_url) = state_ref.current_script_url.as_ref() else {
            return true;
        };
        let report_only_violation =
            content_security_policy_trusted_types_policy_violation_with_disposition_and_reporting_endpoints(
                &state_ref.content_security_report_only_policies,
                protected_url,
                policy_name,
                is_duplicate,
                ContentSecurityPolicyDisposition::Report,
                &state_ref.content_security_reporting_endpoints,
            );
        let enforced_violation =
            content_security_policy_trusted_types_policy_violation_with_disposition_and_reporting_endpoints(
                &state_ref.content_security_policies,
                protected_url,
                policy_name,
                is_duplicate,
                ContentSecurityPolicyDisposition::Enforce,
                &state_ref.content_security_reporting_endpoints,
            );
        (report_only_violation, enforced_violation)
    };
    let allowed = enforced_violation.is_none();
    // Match window policy creation reporting: enforce response policies are
    // modeled before report-only policies in the partitioned policy state.
    if let Some(violation) = enforced_violation {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
    if let Some(violation) = report_only_violation {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
    allowed
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
    let request_body = request_body_text(&request.body);
    let Some((
        document_url,
        loader,
        network_partition_key,
        policy_context,
        service_worker_runtime,
        service_worker_client_id,
    )) = worker_content_security_policy_report_context(state)
    else {
        return;
    };
    let request = request
        .with_initiator_url(&document_url)
        .with_network_partition_key(network_partition_key.clone())
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
    let Some(load) = loader.register_load(
        ResourceLoadKind::CspReport,
        ResourceLoadDisposition::Keepalive,
        None,
    ) else {
        record_worker_content_security_policy_report_failure(
            state,
            None,
            None,
            document_url,
            request,
            request_body,
            "csp report: worker global is shutting down".to_owned(),
        );
        return;
    };

    if should_request_be_blocked_due_to_bad_port(&request.url) {
        let message = format!("csp report: blocked bad port for `{}`", request.url);
        record_worker_content_security_policy_report_failure(
            state,
            None,
            None,
            document_url,
            request,
            request_body,
            message,
        );
        return;
    }
    if load.blocks_url(&request.url) {
        record_worker_content_security_policy_report_failure(
            state,
            None,
            None,
            document_url,
            request,
            request_body,
            crate::network_host::BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
        );
        return;
    }

    if should_intercept_worker_content_security_policy_report(state) {
        pause_worker_content_security_policy_report_for_fetch_interception(
            state,
            load,
            policy_context,
            service_worker_runtime,
            service_worker_client_id,
            document_url,
            request,
            request_body,
        );
        return;
    }

    if load.network_offline() {
        record_worker_content_security_policy_report_failure(
            state,
            None,
            None,
            document_url,
            request,
            request_body,
            "Network emulation offline".to_owned(),
        );
        return;
    }

    if let (Some(runtime), Some(client_id)) = (service_worker_runtime, service_worker_client_id)
        && runtime
            .matching_controller_for_client_fetch(client_id, &request.url)
            .is_some()
    {
        dispatch_worker_content_security_policy_report_to_service_worker(
            state.borrow().parent_tx.clone(),
            runtime,
            client_id,
            load,
            policy_context,
            None,
            None,
            document_url,
            request,
            request_body,
        );
        return;
    }

    spawn_worker_content_security_policy_report_network(
        state.borrow().parent_tx.clone(),
        load,
        None,
        None,
        document_url,
        request,
        request_body,
    );
}

fn should_intercept_worker_content_security_policy_report(
    state: &Rc<RefCell<WorkerGlobalState>>,
) -> bool {
    let state = state.borrow();
    state.fetch_subresource_interception_enabled
        && state
            .fetch_subresource_interception_resource_type
            .is_none_or(|expected| {
                expected.has_same_cdp_fetch_interception_type(SubresourceResourceType::CspReport)
            })
}

fn pause_worker_content_security_policy_report_for_fetch_interception(
    state: &Rc<RefCell<WorkerGlobalState>>,
    load: ResourceLoadLease,
    policy_context: crate::types::SubresourcePolicyContext,
    service_worker_runtime: Option<crate::service_worker_runtime::ServiceWorkerRuntimeService>,
    service_worker_client_id: Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    document_url: Url,
    request: Request,
    request_body: Option<String>,
) {
    let request_body_bytes = request.body.clone();
    let info = PendingSubresourceFetchInfo {
        internal_id: 0,
        network_request_handle: None,
        frame_id: None,
        document_url: document_url.clone(),
        url: request.url.clone(),
        websocket_socket_id: None,
        method: request.method.clone(),
        request_headers: request.request_headers.clone(),
        request_body: request_body.clone(),
        request_body_bytes,
        resource_type: SubresourceResourceType::CspReport,
        request_cookie_report: None,
    };
    let mut state = state.borrow_mut();
    let report_id = next_fetch_id(&mut state);
    let network_partition_key = request.network_partition_key().map(str::to_owned);
    let credentials_mode = request.credentials_mode;
    let request_mode = request.request_mode;
    state.pending_csp_reports.insert(
        report_id,
        PendingWorkerCspReport {
            load: load.clone(),
            document_url,
            request,
            request_body,
            policy_context,
            service_worker_runtime,
            service_worker_client_id,
        },
    );
    let _ = state
        .parent_tx
        .send(WorkerToParentMessage::PendingSubresourceFetch(
            WorkerPendingSubresourceFetch {
                fetch_id: report_id,
                load,
                credentials_mode,
                request_mode,
                network_partition_key,
                info,
            },
        ));
}

type WorkerContentSecurityPolicyReportContext = (
    Url,
    crate::network::context::WorkerResourceLoader,
    Option<String>,
    crate::types::SubresourcePolicyContext,
    Option<crate::service_worker_runtime::ServiceWorkerRuntimeService>,
    Option<crate::service_worker_runtime::ServiceWorkerClientId>,
);

fn worker_content_security_policy_report_context(
    state: &Rc<RefCell<WorkerGlobalState>>,
) -> Option<WorkerContentSecurityPolicyReportContext> {
    let state = state.borrow();
    Some((
        state.current_script_url.clone()?,
        state.loader.clone(),
        state.network_partition_key.clone(),
        state.policy_context,
        state.service_worker_runtime.clone(),
        state.service_worker_client_id,
    ))
}

#[allow(clippy::too_many_arguments)]
fn dispatch_worker_content_security_policy_report_to_service_worker(
    parent_tx: tokio::sync::mpsc::UnboundedSender<WorkerToParentMessage>,
    runtime: crate::service_worker_runtime::ServiceWorkerRuntimeService,
    client_id: crate::service_worker_runtime::ServiceWorkerClientId,
    load: ResourceLoadLease,
    policy_context: crate::types::SubresourcePolicyContext,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    continue_internal_id: Option<u64>,
    document_url: Url,
    request: Request,
    request_body: Option<String>,
) {
    let internal_id = load.id_for_diagnostics();
    let (direct_completion_tx, direct_completion_rx) = tokio::sync::oneshot::channel();
    let cancel_handle = FetchCancelHandle::new();
    load.attach_cancel_handle(cancel_handle.clone());
    let request_metadata = service_worker_fetch_request_metadata(&request);
    let dispatch = ServiceWorkerFetchDispatch {
        internal_id,
        request: ServiceWorkerFetchRequest {
            client_id,
            resulting_client_id: None,
            url: request.url.clone(),
            method: request.method.clone(),
            headers: request.request_headers.clone(),
            body: request.body.clone(),
            destination: ServiceWorkerRequestDestination::Report,
            request_mode: request.request_mode,
            credentials_mode: request.credentials_mode,
            redirect_mode: request.redirect_mode,
            priority: request.priority_hints.fetch_priority,
            is_reload: false,
            metadata: request_metadata,
        },
        request_body_text: request_body.clone(),
        cors_preflight_request_headers: Vec::new(),
        request_cookie_report: None,
        network_context: AsyncSubresourceNetworkContext {
            frame_id: None,
            document_url: document_url.clone(),
            resource_type: SubresourceResourceType::CspReport,
            policy_context,
        },
        completion_tx:
            crate::page_task_queue::RendererResourceCompletionSender::direct_completion_only(),
        request_client: load.request_client(),
        resource_task_runner: load.task_runner(),
        cancel_handle,
        direct_completion_tx: Some(direct_completion_tx),
    };

    if !runtime.dispatch_controlled_fetch(dispatch) {
        load.finish();
        send_worker_content_security_policy_report_failure(
            parent_tx,
            request_handle,
            continue_internal_id,
            document_url,
            request,
            request_body,
            "service worker csp report fetch dispatch failed".to_owned(),
        );
        return;
    }

    let task_runner = load.task_runner();
    task_runner.spawn(async move {
        match direct_completion_rx.await {
            Ok(ServiceWorkerDirectFetchResult::Fallback) => {
                spawn_worker_content_security_policy_report_network(
                    parent_tx,
                    load,
                    request_handle,
                    continue_internal_id,
                    document_url,
                    request,
                    request_body,
                );
            }
            Ok(ServiceWorkerDirectFetchResult::Response(response)) => {
                let response: moli_fetch::Response = (*response.response).into();
                let head = response.head();
                let body = SubresourceResponseBody::from_fetch_response(&response);
                load.finish();
                send_worker_content_security_policy_report_success(
                    parent_tx,
                    request_handle,
                    continue_internal_id,
                    document_url,
                    request,
                    request_body,
                    head,
                    body,
                );
            }
            Ok(ServiceWorkerDirectFetchResult::Failure(message)) => {
                load.finish();
                send_worker_content_security_policy_report_failure(
                    parent_tx,
                    request_handle,
                    continue_internal_id,
                    document_url,
                    request,
                    request_body,
                    message,
                );
            }
            Err(_) => {
                load.finish();
                send_worker_content_security_policy_report_failure(
                    parent_tx,
                    request_handle,
                    continue_internal_id,
                    document_url,
                    request,
                    request_body,
                    "service worker csp report fetch completion channel closed".to_owned(),
                );
            }
        }
    });
}

fn spawn_worker_content_security_policy_report_network(
    parent_tx: tokio::sync::mpsc::UnboundedSender<WorkerToParentMessage>,
    load: ResourceLoadLease,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    continue_internal_id: Option<u64>,
    document_url: Url,
    request: Request,
    request_body: Option<String>,
) {
    let callback_document_url = document_url.clone();
    let callback_request = request.clone();
    let callback_request_body = request_body.clone();
    let callback_load = load.clone();
    let callback_parent_tx = parent_tx.clone();
    if let Err(error) = load
        .request_client()
        .fetch_text_callback(request.clone(), move |result| {
            callback_load.finish();
            match result {
                Ok(response) => {
                    let head = response.head();
                    let body = SubresourceResponseBody::from_fetch_response(&response);
                    send_worker_content_security_policy_report_success(
                        callback_parent_tx,
                        request_handle,
                        continue_internal_id,
                        callback_document_url,
                        callback_request,
                        callback_request_body,
                        head,
                        body,
                    );
                }
                Err(error) => send_worker_content_security_policy_report_failure(
                    callback_parent_tx,
                    request_handle,
                    continue_internal_id,
                    callback_document_url,
                    callback_request,
                    callback_request_body,
                    format!("csp report: {error}"),
                ),
            }
        })
    {
        load.finish();
        send_worker_content_security_policy_report_failure(
            parent_tx,
            request_handle,
            continue_internal_id,
            document_url,
            request,
            request_body,
            format!("csp report: {error}"),
        );
    }
}

pub(in crate::worker) fn continue_pending_worker_csp_report(
    state: &Rc<RefCell<WorkerGlobalState>>,
    continuation: WorkerPendingFetchContinue,
) {
    let Some(pending) = state
        .borrow_mut()
        .pending_csp_reports
        .remove(&continuation.fetch_id)
    else {
        return;
    };
    let request_body = continuation.body.clone();
    let request = match Request::new(
        &continuation.method,
        continuation.url.as_str(),
        continuation.body.clone(),
        continuation.headers.clone(),
    ) {
        Ok(request) => request
            .with_initiator_url(&pending.document_url)
            .with_resource_type(RequestResourceType::CspReport)
            .with_request_mode(pending.request.request_mode)
            .with_credentials_mode(pending.request.credentials_mode)
            .with_redirect_mode(pending.request.redirect_mode)
            .with_network_partition_key(pending.request.network_partition_key().map(str::to_owned))
            .with_browser_request_metadata(BrowserRequestMetadata::Fetch),
        Err(error) => {
            record_worker_content_security_policy_report_failure(
                state,
                continuation.network_request_handle,
                Some(continuation.internal_id),
                pending.document_url,
                pending.request,
                pending.request_body,
                format!("csp report: {error}"),
            );
            return;
        }
    };

    let parent_tx = state.borrow().parent_tx.clone();
    let document_url = pending.document_url;
    let load = pending.load;
    let policy_context = pending.policy_context;
    let service_worker_runtime = pending.service_worker_runtime;
    let service_worker_client_id = pending.service_worker_client_id;

    if should_request_be_blocked_due_to_bad_port(&request.url) {
        let message = format!("csp report: blocked bad port for `{}`", request.url);
        record_worker_content_security_policy_report_failure(
            state,
            continuation.network_request_handle,
            Some(continuation.internal_id),
            document_url,
            request,
            request_body,
            message,
        );
        return;
    }
    if load.blocks_url(&request.url) {
        record_worker_content_security_policy_report_failure(
            state,
            continuation.network_request_handle,
            Some(continuation.internal_id),
            document_url,
            request,
            request_body,
            crate::network_host::BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
        );
        return;
    }
    if load.network_offline() {
        record_worker_content_security_policy_report_failure(
            state,
            continuation.network_request_handle,
            Some(continuation.internal_id),
            document_url,
            request,
            request_body,
            "Network emulation offline".to_owned(),
        );
        return;
    }

    if let (Some(runtime), Some(client_id)) = (service_worker_runtime, service_worker_client_id)
        && runtime
            .matching_controller_for_client_fetch(client_id, &request.url)
            .is_some()
    {
        dispatch_worker_content_security_policy_report_to_service_worker(
            parent_tx,
            runtime,
            client_id,
            load,
            policy_context,
            continuation.network_request_handle,
            Some(continuation.internal_id),
            document_url,
            request,
            request_body,
        );
        return;
    }

    spawn_worker_content_security_policy_report_network(
        parent_tx,
        load,
        continuation.network_request_handle,
        Some(continuation.internal_id),
        document_url,
        request,
        request_body,
    );
}

pub(in crate::worker) fn fail_pending_worker_csp_report(
    state: &Rc<RefCell<WorkerGlobalState>>,
    continuation: WorkerPendingFetchContinue,
    _error_text: String,
) {
    state
        .borrow_mut()
        .pending_csp_reports
        .remove(&continuation.fetch_id);
}

pub(in crate::worker) fn fulfill_pending_worker_csp_report(
    state: &Rc<RefCell<WorkerGlobalState>>,
    continuation: WorkerPendingFetchContinue,
    _response_code: u16,
    _response_headers: Vec<(String, String)>,
    _response_body: RendererSyntheticResponseBody,
) {
    state
        .borrow_mut()
        .pending_csp_reports
        .remove(&continuation.fetch_id);
}

fn record_worker_content_security_policy_report_failure(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    continue_internal_id: Option<u64>,
    document_url: Url,
    request: Request,
    request_body: Option<String>,
    message: String,
) {
    let state = state.borrow();
    record_worker_subresource_failure_with_handle(
        &state,
        request_handle,
        document_url,
        request.url,
        request.method,
        request.request_headers,
        request_body,
        SubresourceResourceType::CspReport,
        message,
    );
    send_worker_content_security_policy_report_continue_completed(
        &state.parent_tx,
        continue_internal_id,
    );
}

fn send_worker_content_security_policy_report_success(
    parent_tx: tokio::sync::mpsc::UnboundedSender<WorkerToParentMessage>,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    continue_internal_id: Option<u64>,
    document_url: Url,
    request: Request,
    request_body: Option<String>,
    head: moli_fetch::ResponseHead,
    body: SubresourceResponseBody,
) {
    let mut record = SubresourceNetworkRecord::success_with_body(
        None,
        document_url,
        request.url,
        request.method,
        request.request_headers,
        request_body,
        SubresourceResourceType::CspReport,
        head.request_cookie_report.clone(),
        head.redirect_chain
            .clone()
            .into_iter()
            .map(Into::into)
            .collect(),
        head.final_url.clone(),
        head.status,
        head.headers.clone(),
        body,
        head.cookie_set_reports.clone(),
    )
    .with_from_cache(head.from_cache)
    .with_negotiated_http_version(head.negotiated_http_version);
    if let Some(handle) = request_handle {
        record = record.with_request_handle(handle);
    }
    let _ = parent_tx.send(WorkerToParentMessage::SubresourceNetwork(record));
    send_worker_content_security_policy_report_continue_completed(&parent_tx, continue_internal_id);
}

fn send_worker_content_security_policy_report_failure(
    parent_tx: tokio::sync::mpsc::UnboundedSender<WorkerToParentMessage>,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    continue_internal_id: Option<u64>,
    document_url: Url,
    request: Request,
    request_body: Option<String>,
    message: String,
) {
    let mut record = SubresourceNetworkRecord::failure(
        None,
        document_url,
        request.url,
        request.method,
        request.request_headers,
        request_body,
        SubresourceResourceType::CspReport,
        message,
    );
    if let Some(handle) = request_handle {
        record = record.with_request_handle(handle);
    }
    let _ = parent_tx.send(WorkerToParentMessage::SubresourceNetwork(record));
    send_worker_content_security_policy_report_continue_completed(&parent_tx, continue_internal_id);
}

fn send_worker_content_security_policy_report_continue_completed(
    parent_tx: &tokio::sync::mpsc::UnboundedSender<WorkerToParentMessage>,
    continue_internal_id: Option<u64>,
) {
    let Some(internal_id) = continue_internal_id else {
        return;
    };
    let _ = parent_tx.send(WorkerToParentMessage::SubresourceContinue(
        PendingSubresourceContinueEvent::Completed { internal_id },
    ));
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

pub(super) fn worker_eval_content_security_policy_violation(
    state: &WorkerGlobalState,
    protected_url: &Url,
    allow_trusted_types_eval: bool,
    source: Option<&str>,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<ContentSecurityPolicyUrlViolation> {
    let policies = match disposition {
        ContentSecurityPolicyDisposition::Enforce => &state.content_security_policies,
        ContentSecurityPolicyDisposition::Report => &state.content_security_report_only_policies,
    };
    let kind = if allow_trusted_types_eval {
        ContentSecurityPolicyNonUrlKind::TrustedTypesEval
    } else {
        ContentSecurityPolicyNonUrlKind::Eval
    };
    policies.iter().find_map(|policy| {
        content_security_policy_non_url_violation_with_source(
            policy,
            protected_url,
            kind,
            source,
            disposition,
            &state.content_security_reporting_endpoints,
        )
    })
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
