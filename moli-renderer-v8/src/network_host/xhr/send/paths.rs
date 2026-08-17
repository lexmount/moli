use super::super::delivery::{queue_xhr_failure_delivery, queue_xhr_response_delivery};
use super::super::*;
use super::request::PreparedXhrSendRequest;
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerFetchRequestMetadata, ServiceWorkerRequestDestination,
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestRedirectMode,
    should_request_be_blocked_due_to_bad_port,
};

pub(super) fn record_intercepted_xhr(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
) -> u64 {
    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.resolved_url,
        &prepared.method,
        prepared.credentials_mode,
    );
    host.record_pending_subresource_xhr(
        prepared.execution_context,
        v8::Global::new(scope, xhr),
        prepared.credentials_mode,
        prepared.network_partition_key,
        prepared.policy_context,
        PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: prepared.frame_id,
            document_url: prepared.document_url,
            url: prepared.resolved_url,
            websocket_socket_id: None,
            method: prepared.method,
            request_headers: prepared.request_headers,
            request_body: request_body_text(&prepared.send_body),
            request_body_bytes: prepared.send_body.clone(),
            resource_type: SubresourceResourceType::Xhr,
            request_cookie_report,
        },
    )
}

pub(super) fn dispatch_service_worker_xhr(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: &PreparedXhrSendRequest,
) -> Option<u64> {
    if !matches!(prepared.resolved_url.scheme(), "http" | "https") {
        return None;
    }
    let client_id = host.service_worker_client_id_for_subresource_owner(prepared.owner);
    host.service_worker_controller_for_fetch(
        client_id,
        &prepared.document_url,
        &prepared.resolved_url,
    )?;

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
        resource_type: SubresourceResourceType::Xhr,
        policy_context: prepared.policy_context,
    };
    let request_body_text = request_body_text(&prepared.send_body);
    let internal_id = host.record_async_subresource_xhr(
        prepared.execution_context.duplicate(scope),
        v8::Global::new(scope, xhr),
        Some(cancel_handle.clone()),
        prepared.credentials_mode,
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
            request_body_bytes: prepared.send_body.clone(),
            resource_type: SubresourceResourceType::Xhr,
            request_cookie_report: request_cookie_report.clone(),
        },
    );
    let request = host.service_worker_fetch_request(
        client_id,
        prepared.resolved_url.clone(),
        prepared.method.clone(),
        prepared.request_headers.clone(),
        prepared.send_body.clone(),
        ServiceWorkerRequestDestination::Empty,
        moli_fetch::RequestMode::Cors,
        prepared.credentials_mode,
        RequestRedirectMode::Follow,
        None,
        ServiceWorkerFetchRequestMetadata::default(),
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
        return Some(internal_id);
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
                parkable_image: None,
                result: Err("service worker xhr dispatch failed".to_owned()),
            });
    Some(internal_id)
}

pub(super) fn record_offline_xhr_failure(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
) {
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.send_body),
        SubresourceResourceType::Xhr,
        "Network emulation offline".to_owned(),
    ));
    queue_xhr_failure_delivery(scope, host, xhr);
}

pub(super) fn record_blocked_xhr_failure(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
) {
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.send_body),
        SubresourceResourceType::Xhr,
        BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
    ));
    queue_xhr_failure_delivery(scope, host, xhr);
}

pub(super) fn record_csp_xhr_failure(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
    message: String,
) {
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.send_body),
        SubresourceResourceType::Xhr,
        message,
    ));
    queue_xhr_failure_delivery(scope, host, xhr);
}

pub(super) fn record_url_policy_xhr_failure(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
    message: String,
) {
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.send_body),
        SubresourceResourceType::Xhr,
        message,
    ));
    queue_xhr_failure_delivery(scope, host, xhr);
}

pub(super) fn record_bad_port_xhr_failure(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
) {
    debug_assert!(should_request_be_blocked_due_to_bad_port(
        &prepared.resolved_url
    ));
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url.clone(),
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.send_body),
        SubresourceResourceType::Xhr,
        format!("xhr: blocked bad port for `{}`", prepared.resolved_url),
    ));
    queue_xhr_failure_delivery(scope, host, xhr);
}

pub(super) fn queue_local_xhr_response(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
    response: Response,
) {
    record_xhr_response_success(host, &prepared, &response);
    queue_xhr_response_delivery(scope, host, xhr, response);
}

pub(super) fn record_xhr_response_success(
    host: &mut JsContextHost,
    prepared: &PreparedXhrSendRequest,
    response: &Response,
) {
    host.record_subresource_network(
        SubresourceNetworkRecord::success_with_body(
            prepared.frame_id.clone(),
            prepared.document_url.clone(),
            prepared.resolved_url.clone(),
            prepared.method.clone(),
            prepared.request_headers.clone(),
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            response.request_cookie_report.clone(),
            response
                .redirect_chain
                .clone()
                .into_iter()
                .map(Into::into)
                .collect(),
            response.final_url.clone(),
            response.status,
            response.headers.clone(),
            crate::protocol_types::SubresourceResponseBody::from_fetch_response(response),
            response.cookie_set_reports.clone(),
        )
        .with_from_cache(response.from_cache)
        .with_negotiated_http_version(response.negotiated_http_version),
    );
}

pub(super) fn spawn_network_xhr_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
    request_client: ResourceRequestClient,
) -> u64 {
    let request = Request::new_bytes(
        &prepared.method,
        prepared.resolved_url.as_str(),
        prepared.send_body.clone(),
        prepared.request_headers.clone(),
    )
    .expect("xhr request url was already resolved")
    .with_initiator_url(&prepared.document_url)
    .with_credentials_mode(prepared.credentials_mode)
    .with_network_partition_key(prepared.network_partition_key.clone())
    .with_browser_request_metadata(BrowserRequestMetadata::Xhr)
    .with_subframe_context(prepared.frame_id.is_some());

    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.resolved_url,
        &prepared.method,
        prepared.credentials_mode,
    );
    let network_context = AsyncSubresourceNetworkContext {
        frame_id: prepared.frame_id.clone(),
        document_url: prepared.document_url.clone(),
        resource_type: SubresourceResourceType::Xhr,
        policy_context: prepared.policy_context,
    };
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let internal_id = host.record_async_subresource_xhr(
        prepared.execution_context,
        v8::Global::new(scope, xhr),
        Some(cancel_handle.clone()),
        prepared.credentials_mode,
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
            request_body: request_body_text(&prepared.send_body),
            request_body_bytes: prepared.send_body.clone(),
            resource_type: SubresourceResourceType::Xhr,
            request_cookie_report,
        },
    );
    spawn_async_subresource_fetch(
        prepared.resource_loader.task_runner(),
        host.resource_completion_sender(),
        request_client,
        request,
        Some(cancel_handle),
        prepared.cors_preflight_request_headers,
        internal_id,
        network_context,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.send_body),
    );
    internal_id
}

pub(super) fn request_body_text(body: &Option<Vec<u8>>) -> Option<String> {
    body.as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
}
