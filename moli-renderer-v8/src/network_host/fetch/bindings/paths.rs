use super::super::*;
use super::request::PreparedWindowFetchRequest;
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, should_request_be_blocked_due_to_bad_port,
};

pub(super) fn record_intercepted_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    prepared: PreparedWindowFetchRequest,
) {
    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.request_origin,
        &prepared.resolved_url,
        &prepared.method,
        prepared.credentials_mode,
    );
    host.record_pending_subresource_fetch(
        prepared.fetch_context,
        v8::Global::new(scope, resolver),
        prepared.options,
        prepared.connect_policy,
        prepared.csp_report_context,
        prepared.credentials_mode,
        prepared.request_mode,
        prepared.request_origin.clone(),
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
            request_body: request_body_text(&prepared.body),
            request_body_bytes: prepared.body.clone(),
            resource_type: SubresourceResourceType::Fetch,
            request_cookie_report,
        },
    );
}

pub(super) fn reject_offline_fetch(
    host: &mut JsContextHost,
    prepared: PreparedWindowFetchRequest,
) -> String {
    let message = "Network emulation offline".to_owned();
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.body),
        SubresourceResourceType::Fetch,
        message.clone(),
    ));
    message
}

pub(super) fn reject_blocked_fetch(
    host: &mut JsContextHost,
    prepared: PreparedWindowFetchRequest,
) -> String {
    let message = BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned();
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.body),
        SubresourceResourceType::Fetch,
        message.clone(),
    ));
    message
}

pub(super) fn reject_csp_fetch(
    host: &mut JsContextHost,
    prepared: PreparedWindowFetchRequest,
    message: String,
) -> String {
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.body),
        SubresourceResourceType::Fetch,
        message.clone(),
    ));
    message
}

pub(super) fn reject_url_policy_fetch(
    host: &mut JsContextHost,
    prepared: PreparedWindowFetchRequest,
    message: String,
) -> String {
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.body),
        SubresourceResourceType::Fetch,
        message.clone(),
    ));
    message
}

pub(super) fn reject_bad_port_fetch(
    host: &mut JsContextHost,
    prepared: PreparedWindowFetchRequest,
) -> String {
    debug_assert!(should_request_be_blocked_due_to_bad_port(
        &prepared.resolved_url
    ));
    let message = format!("fetch: blocked bad port for `{}`", prepared.resolved_url);
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.body),
        SubresourceResourceType::Fetch,
        message.clone(),
    ));
    message
}

pub(super) fn local_fetch_error_text(error: &LocalUrlError) -> String {
    match error {
        LocalUrlError::BlobUnavailable { .. } => FILE_NOT_FOUND_ERROR_TEXT.to_owned(),
        _ => error.to_string(),
    }
}

pub(super) fn resolve_local_fetch(
    host: &mut JsContextHost,
    prepared: &PreparedWindowFetchRequest,
) -> Result<Option<Response>, LocalUrlError> {
    let Some(result) = local_url_response_result(&prepared.resolved_url, &prepared.method) else {
        return Ok(None);
    };
    let response = result.inspect_err(|error| {
        host.record_subresource_network(SubresourceNetworkRecord::failure(
            prepared.frame_id.clone(),
            prepared.document_url.clone(),
            prepared.resolved_url.clone(),
            prepared.method.clone(),
            prepared.request_headers.clone(),
            request_body_text(&prepared.body),
            SubresourceResourceType::Fetch,
            local_fetch_error_text(error),
        ));
    })?;
    host.record_subresource_network(
        SubresourceNetworkRecord::success_with_body(
            prepared.frame_id.clone(),
            prepared.document_url.clone(),
            prepared.resolved_url.clone(),
            prepared.method.clone(),
            prepared.request_headers.clone(),
            request_body_text(&prepared.body),
            SubresourceResourceType::Fetch,
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
            crate::protocol_types::SubresourceResponseBody::from_fetch_response(&response),
            response.cookie_set_reports.clone(),
        )
        .with_from_cache(response.from_cache)
        .with_negotiated_http_version(response.negotiated_http_version),
    );
    Ok(Some(response))
}

pub(super) fn spawn_network_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    prepared: PreparedWindowFetchRequest,
) -> u64 {
    let loader = prepared.resource_loader.request_client().clone();
    let request = Request::new_browser(
        &prepared.method,
        prepared.resolved_url.clone(),
        prepared.body.clone(),
        prepared.request_headers.clone(),
        prepared.request_origin.clone(),
    )
    .with_initiator_url(&prepared.document_url)
    .with_request_mode(prepared.request_mode)
    .with_credentials_mode(prepared.credentials_mode)
    .with_network_partition_key(prepared.network_partition_key.clone());
    let request = prepared
        .options
        .apply(request)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch)
        .with_subframe_context(prepared.frame_id.is_some());

    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.request_origin,
        &prepared.resolved_url,
        &prepared.method,
        prepared.credentials_mode,
    );
    let cancel_handle = FetchCancelHandle::new();
    let internal_id = host.record_async_subresource_fetch(
        prepared.fetch_context,
        v8::Global::new(scope, resolver),
        prepared.options,
        prepared.connect_policy,
        prepared.csp_report_context,
        Some(cancel_handle.clone()),
        prepared.credentials_mode,
        prepared.request_mode,
        prepared.request_origin.clone(),
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
            request_body: request_body_text(&prepared.body),
            request_body_bytes: prepared.body.clone(),
            resource_type: SubresourceResourceType::Fetch,
            request_cookie_report,
        },
    );
    spawn_async_subresource_fetch(
        prepared.resource_loader.task_runner(),
        host.resource_completion_sender(),
        loader,
        request,
        Some(cancel_handle),
        prepared.cors_preflight_request_headers,
        internal_id,
        host.pending_subresource_response_stream(internal_id),
        host.pending_subresource_preflight_observer(internal_id),
        prepared.resolved_url,
    );
    internal_id
}

fn request_body_text(body: &Option<Vec<u8>>) -> Option<String> {
    body.as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
}
