use super::*;
use crate::content_security_policy::{
    ContentSecurityPolicyViolationEventFields, content_security_policy_report_requests,
};
use crate::document_runtime::DomHandle;
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
    service_worker_fetch_request_metadata,
};
use moli_fetch::{
    FetchCancelHandle, Request, RequestCredentialsMode, should_request_be_blocked_due_to_bad_port,
};

pub(crate) fn send_content_security_policy_reports_for_window(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    owner_child_window: Option<DomHandle>,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
    report_uri_endpoints: &[String],
    report_to_endpoints: &[String],
) {
    let dispatch_scope = owner_child_window
        .map(crate::native_bridge::OwnerDispatchScope::Child)
        .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
    let owner = ContentSecurityPolicyReportOwner::new(
        crate::native_bridge::WindowDocumentOwner::Frame(document_owner),
        dispatch_scope,
    );
    let Some(request_context) =
        window_csp_report_request_context_for_identity(scope, host, owner.network_identity())
    else {
        return;
    };
    send_content_security_policy_reports_from_window_context(
        host,
        &request_context,
        fields,
        report_uri_endpoints,
        report_to_endpoints,
    );
}

#[derive(Clone, Copy)]
struct ContentSecurityPolicyReportOwner {
    document_owner: crate::native_bridge::WindowDocumentOwner,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
}

impl ContentSecurityPolicyReportOwner {
    fn new(
        document_owner: crate::native_bridge::WindowDocumentOwner,
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    ) -> Self {
        Self {
            document_owner,
            dispatch_scope,
        }
    }

    fn network_identity(self) -> crate::native_bridge::WindowDocumentNetworkRequestIdentity {
        crate::native_bridge::WindowDocumentNetworkRequestIdentity::new(
            self.document_owner,
            self.dispatch_scope,
        )
    }
}

#[derive(Clone)]
pub(crate) struct WindowCspReportRequestContext {
    identity: crate::native_bridge::WindowDocumentNetworkRequestIdentity,
    resource_loader: crate::network::context::DocumentResourceLoader,
    request_client: ResourceRequestClient,
    frame_id: Option<String>,
    document_url: url::Url,
    network_partition_key: Option<String>,
    policy_context: crate::types::SubresourcePolicyContext,
    client_id: crate::service_worker_runtime::ServiceWorkerClientId,
}

impl WindowCspReportRequestContext {
    pub(crate) fn identity(&self) -> crate::native_bridge::WindowDocumentNetworkRequestIdentity {
        self.identity
    }

    fn register_report_load(
        &self,
        cancel_handle: Option<FetchCancelHandle>,
    ) -> crate::network::loads::ResourceLoadLease {
        self.resource_loader
            .register_network_only_keepalive_load(
                crate::network::loads::ResourceLoadKind::CspReport,
                self.request_client.clone(),
                cancel_handle,
            )
            .expect("captured CSP report context must retain a resource authority")
    }
}

pub(crate) fn capture_window_csp_report_request_context(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
) -> Option<WindowCspReportRequestContext> {
    let document_owner = match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => {
            crate::native_bridge::WindowDocumentOwner::Frame(
                host.current_main_document_task_owner()?,
            )
        }
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            crate::native_bridge::WindowDocumentOwner::Frame(
                host.current_child_document_task_owner(handle)?,
            )
        }
    };
    window_csp_report_request_context_for_identity(
        scope,
        host,
        crate::native_bridge::WindowDocumentNetworkRequestIdentity::new(
            document_owner,
            dispatch_scope,
        ),
    )
}

pub(crate) fn send_content_security_policy_violation_report_from_window_context(
    host: &mut JsContextHost,
    request_context: &WindowCspReportRequestContext,
    violation: &crate::document_runtime::DocumentContentSecurityPolicyViolation,
) {
    let fields = ContentSecurityPolicyViolationEventFields::from(violation);
    send_content_security_policy_reports_from_window_context(
        host,
        request_context,
        &fields,
        &violation.report_uri_endpoints,
        &violation.report_to_endpoints,
    );
}

fn send_content_security_policy_reports_from_window_context(
    host: &mut JsContextHost,
    request_context: &WindowCspReportRequestContext,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
    report_uri_endpoints: &[String],
    report_to_endpoints: &[String],
) {
    for request in
        content_security_policy_report_requests(fields, report_uri_endpoints, report_to_endpoints)
    {
        send_content_security_policy_report_request(host, request_context, request);
    }
}

fn send_content_security_policy_report_request(
    host: &mut JsContextHost,
    request_context: &WindowCspReportRequestContext,
    request: Request,
) {
    if !matches!(request.url.scheme(), "http" | "https") {
        return;
    }

    let request = request
        .with_initiator_url(&request_context.document_url)
        .with_network_partition_key(request_context.network_partition_key.clone())
        .with_subframe_context(request_context.frame_id.is_some());
    let info = report_subresource_fetch_info(
        &request_context.request_client,
        request_context.frame_id.clone(),
        &request_context.document_url,
        &request,
    );

    if request_context
        .request_client
        .page_network_policy()
        .snapshot()
        .blocks_url(&request.url)
    {
        record_content_security_policy_report_failure(
            host,
            info,
            BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
        );
        return;
    }
    if should_request_be_blocked_due_to_bad_port(&request.url) {
        record_content_security_policy_report_failure(
            host,
            info,
            format!("csp report: blocked bad port for `{}`", request.url),
        );
        return;
    }

    if host.should_intercept_subresource(SubresourceResourceType::CspReport) {
        let load = request_context.register_report_load(None);
        host.record_pending_subresource_csp_report(
            request_context.identity,
            request_context.client_id,
            load,
            request_context.network_partition_key.clone(),
            request_context.policy_context,
            info,
        );
        return;
    }

    if request_context
        .request_client
        .page_network_policy()
        .snapshot()
        .network_offline()
    {
        record_content_security_policy_report_failure(
            host,
            info,
            "Network emulation offline".to_owned(),
        );
        return;
    }

    if dispatch_service_worker_content_security_policy_report(
        host,
        request_context,
        info.clone(),
        request.clone(),
    ) {
        return;
    }

    let loader = request_context.request_client.clone();
    spawn_content_security_policy_report_fetch(host, loader, request_context, info, request);
}

fn dispatch_service_worker_content_security_policy_report(
    host: &mut JsContextHost,
    request_context: &WindowCspReportRequestContext,
    info: PendingSubresourceFetchInfo,
    request: Request,
) -> bool {
    if host
        .service_worker_controller_for_fetch(
            request_context.client_id,
            &info.document_url,
            &request.url,
        )
        .is_none()
    {
        return false;
    }

    let cancel_handle = FetchCancelHandle::new();
    let load = request_context.register_report_load(Some(cancel_handle.clone()));
    let internal_id = host.record_async_subresource_csp_report(
        request_context.identity,
        request_context.client_id,
        load,
        request_context.network_partition_key.clone(),
        request_context.policy_context,
        info.clone(),
    );
    let request_body_text = report_request_body_text(&request);
    let dispatch = ServiceWorkerFetchDispatch {
        internal_id,
        request: host.service_worker_fetch_request(
            request_context.client_id,
            request.url.clone(),
            request.method.clone(),
            request.request_headers.clone(),
            request.body.clone(),
            ServiceWorkerRequestDestination::Report,
            request.request_mode,
            request.credentials_mode,
            request.redirect_mode,
            request.priority_hints.fetch_priority,
            service_worker_fetch_request_metadata(&request),
        ),
        request_body_text: request_body_text.clone(),
        cors_preflight_request_headers: Vec::new(),
        request_cookie_report: info.request_cookie_report.clone(),
        network_context: AsyncSubresourceNetworkContext {
            frame_id: info.frame_id.clone(),
            document_url: info.document_url.clone(),
            resource_type: SubresourceResourceType::CspReport,
            policy_context: request_context.policy_context,
        },
        completion_tx: host.resource_completion_sender(),
        request_client: request_context.request_client.clone(),
        resource_task_runner: request_context.resource_loader.task_runner(),
        cancel_handle,
        direct_completion_tx: None,
    };
    if host.dispatch_service_worker_fetch(dispatch) {
        return true;
    }

    let _ =
        host.resource_completion_sender()
            .send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url: request.url,
                request_method: request.method,
                request_headers: request.request_headers,
                request_body: request_body_text,
                response_status_text: None,
                skip_fetch_security_validation: true,
                response_filter: Default::default(),
                network_error_text: None,
                result: Err("service worker csp report fetch dispatch failed".to_owned()),
            });
    true
}

fn spawn_content_security_policy_report_fetch(
    host: &mut JsContextHost,
    request_client: ResourceRequestClient,
    request_context: &WindowCspReportRequestContext,
    info: PendingSubresourceFetchInfo,
    request: Request,
) {
    let cancel_handle = FetchCancelHandle::new();
    let load = request_context.register_report_load(Some(cancel_handle.clone()));
    let task_runner = load.task_runner();
    let internal_id = host.record_async_subresource_csp_report(
        request_context.identity,
        request_context.client_id,
        load,
        request_context.network_partition_key.clone(),
        request_context.policy_context,
        info.clone(),
    );
    let request_body_text = report_request_body_text(&request);
    spawn_async_subresource_fetch(
        task_runner,
        host.resource_completion_sender(),
        request_client,
        request.clone(),
        Some(cancel_handle),
        Vec::new(),
        internal_id,
        AsyncSubresourceNetworkContext {
            frame_id: info.frame_id,
            document_url: info.document_url,
            resource_type: SubresourceResourceType::CspReport,
            policy_context: request_context.policy_context,
        },
        request.url,
        request.method,
        request.request_headers,
        request_body_text,
    );
}

fn report_subresource_fetch_info(
    request_client: &crate::network::ResourceRequestClient,
    frame_id: Option<String>,
    document_url: &url::Url,
    request: &Request,
) -> PendingSubresourceFetchInfo {
    PendingSubresourceFetchInfo {
        internal_id: 0,
        network_request_handle: None,
        frame_id,
        document_url: document_url.clone(),
        url: request.url.clone(),
        websocket_socket_id: None,
        method: request.method.clone(),
        request_headers: request.request_headers.clone(),
        request_body: report_request_body_text(request),
        request_body_bytes: request.body.clone(),
        resource_type: SubresourceResourceType::CspReport,
        request_cookie_report: observe_subresource_request_cookie_report(
            request_client,
            document_url,
            &request.url,
            &request.method,
            RequestCredentialsMode::SameOrigin,
        ),
    }
}

fn window_csp_report_request_context_for_identity(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    identity: crate::native_bridge::WindowDocumentNetworkRequestIdentity,
) -> Option<WindowCspReportRequestContext> {
    if !host.window_document_owner_is_current_for_dispatch_scope(
        identity.owner(),
        identity.dispatch_scope(),
    ) {
        tracing::debug!(
            document_owner = ?identity.owner(),
            dispatch_scope = ?identity.dispatch_scope(),
            "discarded CSP report for retired Window document"
        );
        return None;
    }
    let resource_loader = host
        .document_resource_loader_for_window_owner(identity.owner())?
        .clone();
    let (frame_id, document_url) =
        subresource_request_scope_for_owner(scope, host, identity.dispatch_scope())?;
    let client_id = match identity.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Top => {
            host.service_worker_client_id_for_window_fetch(None)
        }
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.service_worker_client_id_for_window_fetch(Some(handle))
        }
    };
    Some(WindowCspReportRequestContext {
        identity,
        request_client: resource_loader.frozen_request_client(),
        resource_loader,
        frame_id,
        document_url,
        network_partition_key: active_subresource_network_partition_key(
            host,
            identity.dispatch_scope(),
        ),
        policy_context: effective_subresource_policy_context(
            scope,
            host,
            identity.dispatch_scope(),
        ),
        client_id,
    })
}

fn report_request_body_text(request: &Request) -> Option<String> {
    request
        .body
        .as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
}

fn record_content_security_policy_report_failure(
    host: &mut JsContextHost,
    info: PendingSubresourceFetchInfo,
    message: String,
) {
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        info.frame_id,
        info.document_url,
        info.url,
        info.method,
        info.request_headers,
        info.request_body,
        SubresourceResourceType::CspReport,
        message,
    ));
}
