use super::*;
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
    service_worker_fetch_request_metadata,
};
use crate::{
    native_bridge::JsContextHost,
    types::{
        AsyncSubresourceNetworkContext, PendingSubresourceFetchInfo,
        SubresourceRequestInitiatorType, SubresourceResourceType,
    },
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
    RequestResourceType,
};

pub(crate) enum FontFaceFetchStart {
    Local(Vec<u8>),
    Pending,
}

/// Explicit CSS Font Loading requests share document ownership, CORS and the
/// renderer network queue with other resources. Never call page-defined fetch.
pub(crate) fn start_font_face_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    face: v8::Local<'_, v8::Object>,
    source: &str,
) -> Result<FontFaceFetchStart, String> {
    let host_ptr = crate::util::context_host_ptr_from_global_bridge(scope)
        .ok_or_else(|| "FontFace URL loader is unavailable in this realm".to_owned())?;
    // SAFETY: the current live realm owns the bridge. No author callback runs
    // while the host is used except its existing CSP notification boundary.
    let host: &mut JsContextHost = unsafe { &mut *host_ptr };
    let (frame_id, document_url, owner) = effective_subresource_request_scope(scope, host, None);
    let base_url = owner
        .child_window()
        .and_then(|child| host.child_browsing_context_document_handle(child))
        .map(|doc| host.document_base_url_for_handle(doc))
        .unwrap_or_else(|| host.document_base_url_for_handle(host.document_handle()));
    let request_url = base_url.join(source).map_err(|e| e.to_string())?;
    if !host.font_request_allowed_by_csp(
        scope,
        owner,
        &document_url,
        &request_url,
        crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
    ) {
        return Err("Font request blocked by Content Security Policy".into());
    }
    if let Some(response) = local_url_response(&request_url) {
        let response: crate::types::NavigationResponse = response.into();
        let result = if (200..300).contains(&response.status) {
            Ok(FontFaceFetchStart::Local(response.body_bytes().to_vec()))
        } else {
            Err("Font request failed".to_owned())
        };
        host.record_get_subresource_network_result_with_initiator(
            frame_id,
            document_url,
            request_url,
            SubresourceResourceType::Font,
            SubresourceRequestInitiatorType::Script,
            &Ok(response),
        );
        return result;
    }
    let resource_loader = host
        .document_resource_loader_for_dispatch_scope(owner)
        .ok_or_else(|| "Font request has no live document resource loader".to_owned())?;
    let loader = resource_loader.request_client().clone();
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let request_cookie_report = observe_subresource_request_cookie_report(
        &loader,
        &document_url,
        &request_url,
        "GET",
        RequestCredentialsMode::SameOrigin,
    );
    let request = Request::new("GET", request_url.as_str(), None, Vec::new())
        .map_err(|e| e.to_string())?
        .with_initiator_url(&document_url)
        .with_resource_type(RequestResourceType::Font)
        .with_page_network_policy()
        .with_request_mode(RequestMode::Cors)
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_network_partition_key(network_partition_key.clone())
        .with_browser_request_metadata(BrowserRequestMetadata::Font)
        .with_subframe_context(frame_id.is_some());
    let cancel_handle = FetchCancelHandle::new();
    let internal_id = host.record_async_font_face_fetch(
        v8::Global::new(scope, scope.get_current_context()),
        v8::Global::new(scope, face),
        owner,
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
            method: "GET".into(),
            request_headers: Vec::new(),
            request_body: None,
            request_body_bytes: None,
            resource_type: SubresourceResourceType::Font,
            request_cookie_report: request_cookie_report.clone(),
        },
    );
    let client_id = host.service_worker_client_id_for_subresource_owner(owner);
    if matches!(request_url.scheme(), "http" | "https")
        && host
            .service_worker_controller_for_fetch(client_id, &document_url, &request_url)
            .is_some()
    {
        let dispatch = ServiceWorkerFetchDispatch {
            internal_id,
            request: host.service_worker_fetch_request(
                client_id,
                request_url.clone(),
                "GET".into(),
                Vec::new(),
                None,
                ServiceWorkerRequestDestination::Font,
                RequestMode::Cors,
                RequestCredentialsMode::SameOrigin,
                moli_fetch::RequestRedirectMode::Follow,
                request.priority_hints.fetch_priority,
                service_worker_fetch_request_metadata(&request),
            ),
            request_body_text: None,
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context: AsyncSubresourceNetworkContext {
                frame_id,
                document_url,
                resource_type: SubresourceResourceType::Font,
                policy_context,
            },
            completion_tx: host.resource_completion_sender(),
            request_client: loader,
            resource_task_runner: resource_loader.task_runner(),
            cancel_handle,
            direct_completion_tx: None,
        };
        if !host.dispatch_service_worker_fetch(dispatch) {
            let _ = host.resource_completion_sender().send_async_subresource(
                crate::types::AsyncSubresourceFetchCompletion {
                    internal_id,
                    request_url,
                    request_method: "GET".into(),
                    request_headers: Vec::new(),
                    request_body: None,
                    response_status_text: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    network_error_text: None,
                    result: Err("service worker font request dispatch failed".into()),
                },
            );
        }
        return Ok(FontFaceFetchStart::Pending);
    }
    spawn_async_subresource_fetch(
        resource_loader.task_runner(),
        host.resource_completion_sender(),
        loader,
        request,
        Some(cancel_handle),
        Vec::new(),
        internal_id,
        AsyncSubresourceNetworkContext {
            frame_id,
            document_url,
            resource_type: SubresourceResourceType::Font,
            policy_context,
        },
        request_url,
        "GET".into(),
        Vec::new(),
        None,
    );
    Ok(FontFaceFetchStart::Pending)
}
