use super::*;
use crate::{
    native_bridge::{JsContextHost, WindowDocumentTaskTarget},
    response_links::{DocumentResponseLinks, ResponseScriptPreload},
    service_worker_runtime::{
        ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
        service_worker_fetch_request_metadata,
    },
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext, PendingResponsePreload,
        PendingSubresourceFetchInfo, SubresourceResourceType,
    },
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType,
};

pub(crate) fn start_document_response_links(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    target: WindowDocumentTaskTarget,
    links: &DocumentResponseLinks,
) {
    start_response_dictionary_loads(scope, host, target, &links.dictionaries);
    if !host.owner_scripting_enabled(target.dispatch_scope()) {
        return;
    }
    for preload in &links.scripts {
        if !host.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) {
            break;
        }
        if let Err(error) = start_script_preload(scope, host, target, preload) {
            tracing::debug!(url = %preload.url, error, "response script preload was not loaded");
        }
    }
}

pub(crate) fn apply_response_preload_request_metadata(
    request: Request,
    preload: &PendingResponsePreload,
) -> Request {
    request
        .with_browser_request_metadata(BrowserRequestMetadata::Script)
        .with_resource_type(RequestResourceType::Script)
        .with_link_preload()
        .with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
            cross_origin: preload.metadata.cross_origin.clone(),
            referrer_policy: preload.metadata.referrer_policy.clone(),
            document_referrer_policy: preload.document_referrer_policy.clone(),
            charset: None,
            integrity: preload.metadata.integrity.clone(),
            nonce: preload.metadata.nonce.clone(),
            fetch_priority: preload.metadata.fetch_priority,
            scheduler_priority: None,
        })
}

fn start_script_preload(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    target: WindowDocumentTaskTarget,
    preload: &ResponseScriptPreload,
) -> Result<(), String> {
    let timing = crate::context_bootstrap::FetchResourceTiming::for_initiator_type("link");
    let owner = target.dispatch_scope();
    let resource_loader = host
        .document_resource_loader_for_window_owner(target.owner())
        .ok_or("preload Document resource authority is unavailable")?;
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or("preload request environment is unavailable")?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        request_origin,
        frame_id,
        ..
    } = environment;
    let policy = host
        .document_connect_policy_snapshot_for_owner(owner)
        .ok_or("preload policy owner is unavailable")?
        .for_script_preload(preload.metadata.clone());
    let report_context = capture_window_csp_report_request_context(scope, host, owner)
        .ok_or("preload reporting owner is unavailable")?;
    let (reports, enforced) = policy
        .check_url(
            &document_url,
            &preload.url,
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .into_violations();
    let blocked = !enforced.is_empty();
    let host_ptr = host as *mut JsContextHost;
    for violation in reports.into_iter().chain(enforced) {
        send_content_security_policy_violation_report_from_window_context(
            host,
            &report_context,
            &violation,
        );
        host.dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(scope, host_ptr, report_context.identity(), &violation);
    }
    if blocked {
        host.record_document_resource_timing(scope, target, timing.failure(&preload.url));
        return Err("script preload blocked by Content Security Policy".into());
    }
    let request_mode = if preload.metadata.cross_origin.is_some() {
        RequestMode::Cors
    } else {
        RequestMode::NoCors
    };
    let credentials_mode = if preload
        .metadata
        .cross_origin
        .as_deref()
        .is_some_and(|value| !value.eq_ignore_ascii_case("use-credentials"))
    {
        RequestCredentialsMode::SameOrigin
    } else {
        RequestCredentialsMode::Include
    };
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let client_id = host.service_worker_client_id_for_subresource_owner(owner);
    let mut pending = PendingResponsePreload {
        metadata: preload.metadata.clone(),
        document_referrer_policy: host.document_response_preload_referrer_policy(owner),
        redirect_state: FetchCspRedirectState::new(&policy),
        policy,
        report_context,
        producer: None,
        timing,
    };
    let mut request = apply_response_preload_request_metadata(
        Request::get_with_url(preload.url.clone())
            .with_page_network_policy()
            .with_initiator_url(&document_url)
            .with_request_origin(request_origin.clone()),
        &pending,
    )
    .with_request_mode(request_mode)
    .with_credentials_mode(credentials_mode)
    .with_network_partition_key(network_partition_key.clone())
    .with_subframe_context(frame_id.is_some());
    pending.producer = resource_loader
        .request_client()
        .register_document_preload(&request);
    let cancel_handle = FetchCancelHandle::new();
    let (internal_id, load) = host
        .record_async_response_preload(
            target,
            pending,
            cancel_handle.clone(),
            network_partition_key,
            policy_context,
            request_mode,
            credentials_mode,
            PendingSubresourceFetchInfo {
                internal_id: 0,
                network_request_handle: None,
                frame_id: frame_id.clone(),
                document_url: document_url.clone(),
                url: preload.url.clone(),
                websocket_socket_id: None,
                method: "GET".into(),
                request_headers: Default::default(),
                request_body: None,
                request_body_bytes: None,
                resource_type: SubresourceResourceType::Script,
                request_cookie_report: None,
            },
        )
        .ok_or("preload Document was retired before request admission")?;
    let redirect_check = host.window_fetch_redirect_check(internal_id);
    if let Some(check) = redirect_check.clone() {
        request = request.with_redirect_check(check);
    }
    let loader = load.request_client();
    let network_context = AsyncSubresourceNetworkContext {
        frame_id,
        request_origin,
        document_url: document_url.clone(),
        resource_type: SubresourceResourceType::Script,
        policy_context,
    };
    if host
        .service_worker_controller_for_fetch(client_id, &document_url, &preload.url)
        .is_some()
    {
        let dispatch = ServiceWorkerFetchDispatch {
            redirect_check,
            internal_id,
            request: host.service_worker_fetch_request(
                client_id,
                preload.url.clone(),
                "GET".into(),
                Vec::new(),
                None,
                ServiceWorkerRequestDestination::Script,
                request_mode,
                credentials_mode,
                RequestRedirectMode::Follow,
                request.priority_hints.fetch_priority,
                service_worker_fetch_request_metadata(&request),
            ),
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context,
            completion_tx: host.resource_completion_sender(),
            request_client: loader,
            resource_task_runner: load.task_runner(),
            cancel_handle,
            direct_completion_tx: None,
        };
        if !host.dispatch_service_worker_fetch(dispatch) {
            let _ = host.resource_completion_sender().send_async_subresource(
                AsyncSubresourceFetchCompletion {
                    internal_id,
                    request_url: preload.url.clone(),
                    request_method: "GET".into(),
                    request_headers: Default::default(),
                    request_body: None,
                    response_status_text: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    network_error_text: None,
                    result: Err("service worker preload dispatch failed".to_owned()).into(),
                },
            );
        }
    } else if host.should_intercept_subresource(SubresourceResourceType::Script) {
        host.pause_pending_subresource_fetch(internal_id);
    } else {
        spawn_async_subresource_fetch(
            load.task_runner(),
            host.resource_completion_sender(),
            loader,
            request,
            Some(cancel_handle),
            Vec::new(),
            internal_id,
            network_context,
            preload.url.clone(),
            "GET".into(),
            Default::default(),
            None,
        );
    }
    Ok(())
}
