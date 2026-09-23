use super::*;
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
    service_worker_fetch_request_metadata,
};
use crate::types::{
    AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext, PendingSubresourceFetchInfo,
    SubresourceResourceType,
};
use crate::util::context_host_ptr_from_global_bridge;
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType,
};

pub(crate) fn font_face_base_url(scope: &mut v8::PinScope<'_, '_>) -> Option<url::Url> {
    let host = unsafe { &*context_host_ptr_from_global_bridge(scope)? };
    let binding = host.current_runtime_window_execution_context_binding(scope)?;
    let loader = host.document_resource_loader_for_dispatch_scope(binding.dispatch_scope())?;
    Some(
        host.subresource_request_environment(&loader, binding.dispatch_scope())?
            .base_url,
    )
}

pub(crate) fn start_font_face_resource_fetch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    request_url: url::Url,
) -> Result<(), String> {
    let host_ptr =
        context_host_ptr_from_global_bridge(scope).ok_or("FontFace has no resource host")?;
    let host = unsafe { &mut *host_ptr };
    let binding = host
        .current_runtime_window_execution_context_binding(scope)
        .ok_or("FontFace Window is no longer active")?;
    let owner = binding.dispatch_scope();
    let resource_loader = host
        .document_resource_loader_for_dispatch_scope(owner)
        .ok_or("FontFace Document resource authority is unavailable")?;
    let environment = host
        .subresource_request_environment(&resource_loader, owner)
        .ok_or("FontFace request environment is unavailable")?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        request_origin,
        frame_id,
        ..
    } = environment;
    let policy = host
        .document_connect_policy_snapshot_for_owner(owner)
        .ok_or("FontFace policy owner is unavailable")?
        .for_resource_kind(
            crate::content_security_policy::ContentSecurityPolicyResourceKind::DocumentFont,
        );
    let report_context = capture_window_csp_report_request_context(scope, host, owner)
        .ok_or("FontFace reporting owner is unavailable")?;
    let (reports, enforced) = policy
        .check_url(
            &document_url,
            &request_url,
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .into_violations();
    let blocked = !enforced.is_empty();
    for violation in reports.into_iter().chain(enforced) {
        send_content_security_policy_violation_report_from_window_context(
            host,
            &report_context,
            &violation,
        );
        host.dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(
            scope, host_ptr, report_context.identity(), &violation,
        );
    }
    if blocked {
        return Err("FontFace request blocked by Content Security Policy".to_owned());
    }
    let credentials_mode = RequestCredentialsMode::SameOrigin;
    let request_mode = RequestMode::Cors;
    moli_fetch::FetchUrlList::new(&request_url, &[])
        .validate_request_mode(request_mode, &request_origin)?;
    let loader = resource_loader.request_client().clone();
    if matches!(request_url.scheme(), "http" | "https")
        && !loader.optional_resource_fetch_enabled(SubresourceResourceType::Font)
    {
        return Err("FontFace requests are disabled by the resource policy".to_owned());
    }
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let request_cookie_report = observe_subresource_request_cookie_report(
        &loader,
        &document_url,
        &request_origin,
        &request_url,
        "GET",
        credentials_mode,
    );
    let mut request = Request::new("GET", request_url.as_str(), None, Vec::new())
        .map_err(|error| error.to_string())?
        .with_initiator_url(&document_url)
        .with_request_origin(request_origin.clone())
        .with_resource_type(RequestResourceType::Font)
        .with_page_network_policy()
        .with_request_mode(request_mode)
        .with_credentials_mode(credentials_mode)
        .with_network_partition_key(network_partition_key.clone())
        .with_redirect_mode(RequestRedirectMode::Follow)
        .with_browser_request_metadata(BrowserRequestMetadata::Font)
        .with_subframe_context(frame_id.is_some());
    let cancel_handle = FetchCancelHandle::new();
    let font = crate::types::PendingFontFaceFetch {
        face: v8::Global::new(scope, face),
        redirect_state: FetchCspRedirectState::new(&policy),
        policy,
        report_context,
    };
    let internal_id = host
        .record_async_font_face_fetch(
            binding,
            &resource_loader,
            font,
            cancel_handle.clone(),
            network_partition_key,
            policy_context,
            PendingSubresourceFetchInfo {
                internal_id: 0,
                network_request_handle: None,
                frame_id: frame_id.clone(),
                document_url: document_url.clone(),
                url: request_url.clone(),
                websocket_socket_id: None,
                method: "GET".to_owned(),
                request_headers: Default::default(),
                request_body: None,
                request_body_bytes: None,
                resource_type: SubresourceResourceType::Font,
                request_cookie_report: request_cookie_report.clone(),
            },
        )
        .ok_or("FontFace Document was retired")?;

    let redirect_check = host.window_fetch_redirect_check(internal_id);
    if let Some(check) = redirect_check.clone() {
        request = request.with_redirect_check(check);
    }

    // Even a data/blob response completes through the resource task queue.
    // FontFace.load() must return while its status is still loading.
    let blob = CapturedBlobUrl::capture(&request_url);
    if let Some(response) =
        local_url_response_with_blob_entry(&request_url, "GET", &[], blob.as_ref())
    {
        let result = response
            .map(Into::into)
            .map_err(|error| error.into_message());
        let _ = host.resource_completion_sender().send_async_subresource(
            AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method: "GET".to_owned(),
                request_headers: Default::default(),
                request_body: None,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: result.into(),
            },
        );
        return Ok(());
    }

    let network_context = AsyncSubresourceNetworkContext {
        frame_id,
        request_origin,
        document_url: document_url.clone(),
        resource_type: SubresourceResourceType::Font,
        policy_context,
    };
    let client_id = host.service_worker_client_id_for_subresource_owner(owner);
    if matches!(request_url.scheme(), "http" | "https")
        && host
            .service_worker_controller_for_fetch(client_id, &document_url, &request_url)
            .is_some()
    {
        let dispatch = ServiceWorkerFetchDispatch {
            redirect_check,
            internal_id,
            request: host.service_worker_fetch_request(
                client_id,
                request_url.clone(),
                "GET".to_owned(),
                Vec::new(),
                None,
                ServiceWorkerRequestDestination::Font,
                request_mode,
                credentials_mode,
                RequestRedirectMode::Follow,
                request.priority_hints.fetch_priority,
                service_worker_fetch_request_metadata(&request),
            ),
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context,
            completion_tx: host.resource_completion_sender(),
            request_client: loader,
            resource_task_runner: resource_loader.task_runner(),
            cancel_handle,
            direct_completion_tx: None,
        };
        if !host.dispatch_service_worker_fetch(dispatch) {
            let _ = host.resource_completion_sender().send_async_subresource(
                AsyncSubresourceFetchCompletion {
                    internal_id,
                    request_url,
                    request_method: "GET".to_owned(),
                    request_headers: Default::default(),
                    request_body: None,
                    response_status_text: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    network_error_text: None,
                    result: Err("service worker font fetch dispatch failed".to_owned()).into(),
                },
            );
        }
        return Ok(());
    }
    spawn_async_subresource_fetch(
        resource_loader.task_runner(),
        host.resource_completion_sender(),
        loader,
        request,
        Some(cancel_handle),
        Vec::new(),
        internal_id,
        network_context,
        request_url,
        "GET".to_owned(),
        Default::default(),
        None,
    );
    Ok(())
}
