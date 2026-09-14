use super::*;
use crate::content_security_policy::ContentSecurityPolicyRedirectStatus;
use crate::content_security_policy::{
    ContentSecurityPolicyViolationEventFields, content_security_policy_report_requests,
};
use crate::document_runtime::DomHandle;
use crate::native_bridge::WorkerOwnerScope;
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerFetchResultSender, ServiceWorkerRequestDestination,
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

pub(crate) fn send_content_security_policy_reports_for_lightweight_popup(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    popup_id: u64,
    document_owner: crate::native_bridge::LightweightPopupDocumentOwner,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
    report_uri_endpoints: &[String],
    report_to_endpoints: &[String],
) {
    let owner = ContentSecurityPolicyReportOwner::new(
        crate::native_bridge::WindowDocumentOwner::LightweightPopup(document_owner),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id),
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

/// Frozen at fetch admission and retained with the physical response, so a
/// retired Window cannot discard or replace its response policy.
pub(crate) struct WindowFetchResponsePolicy {
    connect: crate::document_runtime::DocumentConnectPolicySnapshot,
    reports: WindowCspReportRequestContext,
    redirect_failure: std::sync::OnceLock<Option<String>>,
}

impl WindowFetchResponsePolicy {
    pub(crate) fn new(
        connect: crate::document_runtime::DocumentConnectPolicySnapshot,
        reports: WindowCspReportRequestContext,
    ) -> Self {
        Self {
            connect,
            reports,
            redirect_failure: Default::default(),
        }
    }

    pub(crate) fn check_redirect(
        &self,
        final_url: &url::Url,
        mut report: impl FnMut(
            &WindowCspReportRequestContext,
            &crate::document_runtime::DocumentContentSecurityPolicyViolation,
        ),
    ) -> Option<String> {
        self.redirect_failure
            .get_or_init(|| {
                let redirect = ContentSecurityPolicyRedirectStatus::FollowedRedirect;
                if let Some(violation) = self.connect.report_only_violation(
                    &self.reports.document_url,
                    final_url,
                    redirect,
                ) {
                    report(&self.reports, &violation);
                }
                let violation = self.connect.enforce_violation(
                    &self.reports.document_url,
                    final_url,
                    redirect,
                )?;
                report(&self.reports, &violation);
                Some(
                    crate::document_runtime::document_content_security_policy_error_message(
                        &violation, "fetch",
                    ),
                )
            })
            .clone()
    }

    pub(crate) fn check_unclaimed_response(
        &self,
        request: &crate::runtime::RendererNetworkRequest,
        final_url: &url::Url,
    ) -> Option<String> {
        self.check_redirect(final_url, |context, violation| {
            let fields = ContentSecurityPolicyViolationEventFields::from(violation);
            for report in content_security_policy_report_requests(
                &fields,
                &violation.report_uri_endpoints,
                &violation.report_to_endpoints,
            ) {
                send_content_security_policy_report_request(
                    None,
                    context,
                    request.dependent_request(),
                    report,
                );
            }
        })
    }
}

#[derive(Clone)]
pub(crate) struct WindowCspReportRequestContext {
    identity: crate::native_bridge::WindowDocumentNetworkRequestIdentity,
    network: crate::runtime::RendererDocumentNetworkReporter,
    completion_tx: crate::page_task_queue::RendererResourceCompletionSender,
    resource_loader: crate::network::context::DocumentResourceLoader,
    browser_context: crate::runtime::RendererBrowserContextRuntime,
    request_client: ResourceRequestClient,
    frame_id: Option<String>,
    document_url: url::Url,
    request_origin: moli_url::WebOrigin,
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
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            crate::native_bridge::WindowDocumentOwner::LightweightPopup(
                host.current_lightweight_popup_document_owner(popup_id)?,
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
        let Some(network) = request_context.network.start_request() else {
            return;
        };
        send_content_security_policy_report_request(Some(host), request_context, network, request);
    }
}

fn send_content_security_policy_report_request(
    mut host: Option<&mut JsContextHost>,
    request_context: &WindowCspReportRequestContext,
    request_network: crate::runtime::RendererNetworkRequest,
    request: Request,
) {
    if !matches!(request.url.scheme(), "http" | "https") {
        return;
    }

    let request = request
        .with_initiator_url(&request_context.document_url)
        .with_request_origin(request_context.request_origin.clone())
        .with_network_partition_key(request_context.network_partition_key.clone())
        .with_subframe_context(request_context.frame_id.is_some());
    let mut info = report_subresource_fetch_info(
        &request_context.request_client,
        request_context.frame_id.clone(),
        &request_context.document_url,
        &request,
    );

    info.network_request_handle = Some(request_network.handle());
    let completion_tx = request_context.completion_tx.clone();
    let (network, started) = crate::network::ResourceTransfer::start(
        request_network,
        move |observation| {
            let _ = completion_tx.send_async_subresource_event(
                AsyncSubresourceFetchEvent::NativeNetwork(observation),
            );
        },
        |network| keepalive_request_started(network, &info),
    );
    if let Some(host) = host.as_deref_mut() {
        host.record_native_resource_observation(started);
    } else {
        network.observe(started);
    }
    if request_context
        .request_client
        .page_network_policy()
        .snapshot()
        .blocks_url(&request.url)
    {
        network.failed(&crate::network::ResourceResponseFailure::Request(
            BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
        ));
        return;
    }
    if should_request_be_blocked_due_to_bad_port(&request.url) {
        network.failed(&crate::network::ResourceResponseFailure::Request(format!(
            "csp report: blocked bad port for `{}`",
            request.url
        )));
        return;
    }

    if let Some(host) = host.as_deref_mut()
        && host.should_intercept_subresource(SubresourceResourceType::CspReport)
    {
        let load = request_context.register_report_load(None);
        host.record_pending_subresource_csp_report(
            request_context.identity,
            request_context.request_origin.clone(),
            request_context.client_id,
            load,
            network,
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
        network.failed(&crate::network::ResourceResponseFailure::Request(
            "Network emulation offline".to_owned(),
        ));
        return;
    }

    let cancel_handle = FetchCancelHandle::new();
    let load = request_context.register_report_load(Some(cancel_handle.clone()));
    let resource = KeepaliveResource::new(network, load);
    let controller = match host {
        Some(host) => host.service_worker_controller_for_fetch(
            request_context.client_id,
            &info.document_url,
            &request.url,
        ),
        None if !request_context.request_client.bypass_service_worker() => request_context
            .browser_context
            .service_worker_controller_for_fetch(request_context.client_id, &request.url),
        None => None,
    };
    if controller.is_some() {
        let dispatch = ServiceWorkerFetchDispatch {
            internal_id: 0,
            request: crate::service_worker_runtime::ServiceWorkerFetchRequest {
                client_id: request_context.client_id,
                resulting_client_id: None,
                url: request.url.clone(),
                method: request.method.clone(),
                headers: request.request_headers.to_byte_strings(),
                body: request.body.clone(),
                destination: ServiceWorkerRequestDestination::Report,
                request_mode: request.request_mode,
                credentials_mode: request.credentials_mode,
                redirect_mode: request.redirect_mode,
                priority: request.priority_hints.fetch_priority,
                is_reload: false,
                metadata: service_worker_fetch_request_metadata(&request),
            },
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: info.request_cookie_report,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: info.frame_id,
                request_origin: request_context.request_origin.clone(),
                document_url: info.document_url,
                resource_type: SubresourceResourceType::CspReport,
                policy_context: request_context.policy_context,
            },
            result_tx: ServiceWorkerFetchResultSender::CspReport(resource.clone()),
            request_client: request_context.request_client.clone(),
            resource_task_runner: request_context.resource_loader.task_runner(),
            cancel_handle,
        };
        if !request_context
            .browser_context
            .dispatch_service_worker_fetch(dispatch)
        {
            resource.fail("service worker csp report fetch dispatch failed".into());
        }
    } else {
        resource.fetch(
            request_context.request_client.clone(),
            request,
            cancel_handle,
        );
    }
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
            request
                .browser_origin()
                .expect("CSP reports carry an explicit origin"),
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
    let environment =
        host.subresource_request_environment(&resource_loader, identity.dispatch_scope())?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        request_origin,
        frame_id,
        ..
    } = environment;
    let client_id = match identity.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Top => {
            host.service_worker_client_id_for_window_fetch(None)
        }
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.service_worker_client_id_for_window_fetch(Some(handle))
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => host
            .service_worker_client_id_for_worker_owner(WorkerOwnerScope::LightweightPopup(
                popup_id,
            )),
    };
    Some(WindowCspReportRequestContext {
        identity,
        network: host.document_network_reporter()?,
        completion_tx: host.resource_completion_sender(),
        browser_context: host.browser_context_runtime(),
        request_client: resource_loader.frozen_request_client(),
        resource_loader,
        frame_id,
        document_url,
        request_origin,
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
