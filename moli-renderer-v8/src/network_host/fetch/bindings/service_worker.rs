use super::super::*;
use super::request::PreparedWindowFetchRequest;
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerFetchRequestMetadata, ServiceWorkerRequestDestination,
};
use moli_fetch::FetchCancelHandle;

pub(super) fn dispatch_service_worker_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    prepared: &PreparedWindowFetchRequest,
) -> Result<Option<u64>, String> {
    if !matches!(prepared.resolved_url.scheme(), "http" | "https") {
        return Ok(None);
    }
    let client_id = host.service_worker_client_id_for_subresource_owner(prepared.request_scope());
    if host
        .service_worker_controller_for_fetch(
            client_id,
            &prepared.document_url,
            &prepared.resolved_url,
        )
        .is_none()
    {
        return Ok(None);
    }

    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.resolved_url,
        &prepared.method,
        prepared.credentials_mode,
    );
    let cancel_handle = FetchCancelHandle::new();
    let network_context = AsyncSubresourceNetworkContext {
        frame_id: prepared.frame_id.clone(),
        document_url: prepared.document_url.clone(),
        resource_type: SubresourceResourceType::Fetch,
        policy_context: prepared.policy_context,
    };
    let requires_preflight = prepared.request_mode != moli_fetch::RequestMode::NoCors
        && crate::network_host::cors_preflight_request_headers(
            &prepared.document_url,
            &prepared.resolved_url,
            &prepared.method,
            &prepared.cors_preflight_request_headers,
        )
        .is_some();
    let request_body_text = request_body_text(&prepared.body);
    let internal_id = host.record_async_subresource_fetch(
        prepared.fetch_context.duplicate(scope),
        v8::Global::new(scope, resolver),
        prepared.keepalive,
        prepared.connect_policy.clone(),
        prepared.csp_report_context.clone(),
        Some(cancel_handle.clone()),
        prepared.credentials_mode,
        prepared.request_mode,
        prepared.network_partition_key.clone(),
        prepared.policy_context,
        PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: prepared.frame_id.clone(),
            document_url: prepared.document_url.clone(),
            url: prepared.resolved_url.clone(),
            websocket_socket_id: None,
            method: prepared.method.clone(),
            request_headers: prepared.request_headers.clone(),
            request_body: request_body_text.clone(),
            request_body_bytes: prepared.body.clone(),
            resource_type: SubresourceResourceType::Fetch,
            request_cookie_report: request_cookie_report.clone(),
        },
        requires_preflight,
    );
    let request = host.service_worker_fetch_request(
        client_id,
        prepared.resolved_url.clone(),
        prepared.method.clone(),
        prepared.request_headers.clone(),
        prepared.body.clone(),
        ServiceWorkerRequestDestination::Empty,
        prepared.request_mode,
        prepared.credentials_mode,
        prepared.redirect_mode,
        prepared.priority,
        ServiceWorkerFetchRequestMetadata {
            cache: prepared.cache.clone(),
            referrer: prepared.referrer.clone(),
            referrer_policy: prepared.referrer_policy.clone(),
            integrity: prepared.integrity.clone(),
            keepalive: prepared.keepalive,
        },
    );
    let dispatch = ServiceWorkerFetchDispatch {
        internal_id,
        request,
        request_body_text: request_body_text.clone(),
        cors_preflight_request_headers: prepared.cors_preflight_request_headers.clone(),
        request_cookie_report,
        network_context,
        completion_tx: host.resource_completion_sender(),
        request_client: prepared.resource_loader.request_client().clone(),
        resource_task_runner: prepared.resource_loader.task_runner(),
        cancel_handle,
        direct_completion_tx: None,
    };
    if host.dispatch_service_worker_fetch(dispatch) {
        return Ok(Some(internal_id));
    }

    let _ =
        host.resource_completion_sender()
            .send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url: prepared.resolved_url.clone(),
                request_method: prepared.method.clone(),
                request_headers: prepared.request_headers.clone(),
                request_body: request_body_text,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: Err("service worker fetch dispatch failed".to_owned()).into(),
            });
    Ok(Some(internal_id))
}

fn request_body_text(body: &Option<Vec<u8>>) -> Option<String> {
    body.as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
}
