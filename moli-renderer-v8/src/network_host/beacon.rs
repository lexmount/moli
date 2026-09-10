use super::*;
use crate::webidl;
use moli_fetch::{
    FetchCancelHandle, RequestCredentialsMode, RequestMode, RequestRedirectMode,
    RequestResourceType, should_request_be_blocked_due_to_bad_port,
};

pub(crate) fn navigator_send_beacon_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    if args.length() < 1 {
        crate::util::throw_type_error(
            scope,
            &webidl::WebIdlError::missing_required(webidl::Context::argument(
                "Navigator.sendBeacon",
                1,
            ))
            .to_string(),
        );
        return;
    }

    let raw_url = match webidl::convert::<webidl::UsvString>(
        scope,
        args.get(0),
        webidl::Context::argument("Navigator.sendBeacon", 1),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            crate::util::throw_type_error(scope, &error.to_string());
            return;
        }
    };
    let Some((body, body_content_type)) = navigator_beacon_body(scope, &args) else {
        return;
    };

    let host = unsafe { &mut *host_ptr };
    let Some(request_context) = window_ping_request_context(scope, host) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let WindowPingRequestContext {
        execution_context,
        resource_loader,
        frame_id,
        document_url,
        network_partition_key,
    } = request_context;
    let resolved_url = match resolve_context_url(&document_url, &raw_url, None) {
        Ok(url) => url,
        Err(_) => {
            crate::util::throw_type_error(scope, "The URL argument is ill-formed or unsupported.");
            return;
        }
    };
    if !matches!(resolved_url.scheme(), "http" | "https") {
        crate::util::throw_type_error(scope, "Beacons are only supported over HTTP(S).");
        return;
    }

    let mut request_headers = Vec::new();
    append_default_body_content_type(&mut request_headers, body_content_type.as_deref());
    request_headers = filter_headers_for_guard(&request_headers, HeadersGuard::RequestNoCors);
    request_headers =
        merge_subresource_request_headers(host.extra_http_headers(), &request_headers);
    let request_body_text = request_body_text(&body);
    let request_cookie_report = observe_subresource_request_cookie_report(
        resource_loader.request_client(),
        &document_url,
        &resolved_url,
        "POST",
        RequestCredentialsMode::Include,
    );

    let info = PendingSubresourceFetchInfo {
        internal_id: 0,
        network_request_handle: None,
        frame_id: frame_id.clone(),
        document_url: document_url.clone(),
        url: resolved_url.clone(),
        websocket_socket_id: None,
        method: "POST".to_owned(),
        request_headers: request_headers.clone(),
        request_body: request_body_text.clone(),
        request_body_bytes: body.clone(),
        resource_type: SubresourceResourceType::Ping,
        request_cookie_report,
    };

    if host.should_intercept_subresource(SubresourceResourceType::Ping) {
        host.record_pending_subresource_beacon(
            execution_context,
            network_partition_key.clone(),
            info,
        );
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    }

    if host.network_offline() {
        record_beacon_failure(host, info, "Network emulation offline".to_owned());
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    }
    if host.is_url_blocked(&resolved_url) {
        record_beacon_failure(host, info, BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned());
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    }
    if should_request_be_blocked_due_to_bad_port(&resolved_url) {
        record_beacon_failure(
            host,
            info,
            format!("sendBeacon: blocked bad port for `{resolved_url}`"),
        );
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    }

    let loader = resource_loader.request_client().clone();
    let request =
        match Request::new_bytes("POST", resolved_url.as_str(), body, request_headers.clone()) {
            Ok(request) => request
                .with_initiator_url(&document_url)
                .with_resource_type(RequestResourceType::Beacon)
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Beacon)
                .with_request_mode(RequestMode::NoCors)
                .with_credentials_mode(RequestCredentialsMode::Include)
                .with_network_partition_key(network_partition_key.clone())
                .with_redirect_mode(RequestRedirectMode::Follow)
                .with_subframe_context(frame_id.is_some()),
            Err(error) => {
                crate::util::throw_type_error(scope, &error.to_string());
                return;
            }
        };
    let cancel_handle = FetchCancelHandle::new();
    let network_context = AsyncSubresourceNetworkContext {
        frame_id: info.frame_id.clone(),
        document_url: info.document_url.clone(),
        resource_type: info.resource_type,
        policy_context: Default::default(),
    };
    let internal_id = host.record_async_subresource_beacon(
        execution_context,
        Some(cancel_handle.clone()),
        network_partition_key,
        info,
    );
    spawn_async_subresource_fetch(
        resource_loader.task_runner(),
        host.resource_completion_sender(),
        loader,
        request,
        Some(cancel_handle),
        Vec::new(),
        internal_id,
        network_context,
        resolved_url,
        "POST".to_owned(),
        request_headers,
        request_body_text,
    );
    rv.set(v8::Boolean::new(scope, true).into());
}

pub(crate) fn send_link_audit_ping(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    ping_url: url::Url,
    destination_url: &str,
) {
    if !matches!(ping_url.scheme(), "http" | "https") {
        return;
    }
    let host = unsafe { &mut *host_ptr };
    let Some(request_context) = window_ping_request_context(scope, host) else {
        return;
    };
    let WindowPingRequestContext {
        execution_context,
        resource_loader,
        frame_id,
        document_url,
        network_partition_key,
    } = request_context;
    let mut request_headers = vec![
        ("Content-Type".to_owned(), "text/ping".to_owned()),
        ("Cache-Control".to_owned(), "max-age=0".to_owned()),
        ("Ping-To".to_owned(), destination_url.to_owned()),
    ];
    if document_url.scheme() == "http" || moli_url::same_origin(&document_url, &ping_url) {
        request_headers.push(("Ping-From".to_owned(), document_url.as_str().to_owned()));
    }
    request_headers =
        merge_subresource_request_headers(host.extra_http_headers(), &request_headers);
    let request_body = Some("PING".to_owned());
    let request_cookie_report = observe_subresource_request_cookie_report(
        resource_loader.request_client(),
        &document_url,
        &ping_url,
        "POST",
        RequestCredentialsMode::Include,
    );

    let info = PendingSubresourceFetchInfo {
        internal_id: 0,
        network_request_handle: None,
        frame_id: frame_id.clone(),
        document_url: document_url.clone(),
        url: ping_url.clone(),
        websocket_socket_id: None,
        method: "POST".to_owned(),
        request_headers: request_headers.clone(),
        request_body: request_body.clone(),
        request_body_bytes: request_body.as_ref().map(|body| body.as_bytes().to_vec()),
        resource_type: SubresourceResourceType::Ping,
        request_cookie_report,
    };

    if host.should_intercept_subresource(SubresourceResourceType::Ping) {
        host.record_pending_subresource_beacon(
            execution_context,
            network_partition_key.clone(),
            info,
        );
        return;
    }

    if host.network_offline() {
        record_beacon_failure(host, info, "Network emulation offline".to_owned());
        return;
    }
    if host.is_url_blocked(&ping_url) {
        record_beacon_failure(host, info, BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned());
        return;
    }
    if should_request_be_blocked_due_to_bad_port(&ping_url) {
        record_beacon_failure(
            host,
            info,
            format!("ping: blocked bad port for `{ping_url}`"),
        );
        return;
    }

    let loader = resource_loader.request_client().clone();
    let request = match Request::new_bytes(
        "POST",
        ping_url.as_str(),
        Some(b"PING".to_vec()),
        request_headers.clone(),
    ) {
        Ok(request) => request
            .with_initiator_url(&document_url)
            .with_resource_type(RequestResourceType::Ping)
            .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Ping)
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(RequestCredentialsMode::Include)
            .with_network_partition_key(network_partition_key.clone())
            .with_redirect_mode(RequestRedirectMode::Follow)
            .with_subframe_context(frame_id.is_some()),
        Err(error) => {
            record_beacon_failure(host, info, error.to_string());
            return;
        }
    };
    let cancel_handle = FetchCancelHandle::new();
    let network_context = AsyncSubresourceNetworkContext {
        frame_id: info.frame_id.clone(),
        document_url: info.document_url.clone(),
        resource_type: info.resource_type,
        policy_context: Default::default(),
    };
    let internal_id = host.record_async_subresource_beacon(
        execution_context,
        Some(cancel_handle.clone()),
        network_partition_key,
        info,
    );
    spawn_async_subresource_fetch(
        resource_loader.task_runner(),
        host.resource_completion_sender(),
        loader,
        request,
        Some(cancel_handle),
        Vec::new(),
        internal_id,
        network_context,
        ping_url,
        "POST".to_owned(),
        request_headers,
        request_body,
    );
}

struct WindowPingRequestContext {
    execution_context: crate::native_bridge::WindowExecutionContextIdentity,
    resource_loader: crate::network::context::DocumentResourceLoader,
    frame_id: Option<String>,
    document_url: url::Url,
    network_partition_key: Option<String>,
}

fn window_ping_request_context(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
) -> Option<WindowPingRequestContext> {
    let execution_context = host.current_runtime_window_execution_context_identity(scope)?;
    let owner = execution_context.dispatch_scope();
    let resource_loader = host
        .document_resource_loader_for_dispatch_scope(owner)?
        .clone();
    let (frame_id, document_url) = subresource_request_scope_for_owner(scope, host, owner)?;
    Some(WindowPingRequestContext {
        execution_context,
        resource_loader,
        frame_id,
        document_url,
        network_partition_key: active_subresource_network_partition_key(host, owner),
    })
}

fn navigator_beacon_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(Option<Vec<u8>>, Option<String>)> {
    if args.length() < 2 || args.get(1).is_null_or_undefined() {
        return Some((None, None));
    }
    let data = args.get(1);
    if let Ok(object) = v8::Local::<v8::Object>::try_from(data)
        && moli_webapi_declare::implements_interface(scope, object, "ReadableStream")
    {
        crate::util::throw_type_error(scope, "sendBeacon cannot have a ReadableStream body.");
        return None;
    }
    match body_init(
        scope,
        data,
        webidl::Context::argument("Navigator.sendBeacon", 2),
    ) {
        Ok(body) => Some((
            body.as_ref().map(|body| body.bytes.clone()),
            body.and_then(|body| body.content_type),
        )),
        Err(error) => {
            crate::util::throw_type_error(scope, &error.to_string());
            None
        }
    }
}

fn request_body_text(body: &Option<Vec<u8>>) -> Option<String> {
    body.as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
}

fn record_beacon_failure(
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
        info.resource_type,
        message,
    ));
}
