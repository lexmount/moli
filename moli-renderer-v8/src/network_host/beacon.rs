use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, RequestCredentialsMode, RequestMode,
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
    let resource_loader = &request_context.resource_loader;
    let document_url = &request_context.document_url;
    let frame_id = &request_context.frame_id;
    let resolved_url = match resolve_context_url(&request_context.base_url, &raw_url, None) {
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
    let request_headers =
        merge_subresource_request_headers(host.extra_http_headers(), &request_headers);
    let request_body_text = request_body_text(&body);
    let request_cookie_report = observe_subresource_request_cookie_report(
        resource_loader.request_client(),
        document_url,
        &request_context.request_origin,
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

    send_ping(host, request_context, info, BrowserRequestMetadata::Beacon);
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
    let resource_loader = &request_context.resource_loader;
    let document_url = &request_context.document_url;
    let frame_id = &request_context.frame_id;
    let mut request_headers = vec![
        ("Content-Type".to_owned(), "text/ping".to_owned()),
        ("Cache-Control".to_owned(), "max-age=0".to_owned()),
        ("Ping-To".to_owned(), destination_url.to_owned()),
    ];
    if document_url.scheme() == "http" || moli_url::same_origin(document_url, &ping_url) {
        request_headers.push(("Ping-From".to_owned(), document_url.as_str().to_owned()));
    }
    let request_headers =
        merge_subresource_request_headers(host.extra_http_headers(), &request_headers);
    let request_body = Some("PING".to_owned());
    let request_cookie_report = observe_subresource_request_cookie_report(
        resource_loader.request_client(),
        document_url,
        &request_context.request_origin,
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

    send_ping(host, request_context, info, BrowserRequestMetadata::Ping);
}

fn send_ping(
    host: &mut JsContextHost,
    context: WindowPingRequestContext,
    mut info: PendingSubresourceFetchInfo,
    metadata: BrowserRequestMetadata,
) {
    let Some(request_network) = host
        .document_network_reporter()
        .and_then(|source| source.start_request())
    else {
        return;
    };
    info.network_request_handle = Some(request_network.handle());
    let observer = host.resource_completion_sender().network_observer();
    let (network, started) = crate::network::ResourceTransfer::start(
        request_network,
        move |event| observer(event),
        |request| keepalive_request_started(request, &info),
    );
    if host.should_intercept_subresource(SubresourceResourceType::Ping) {
        let load = context
            .resource_loader
            .register_load(
                crate::network::loads::ResourceLoadKind::Beacon,
                crate::network::loads::ResourceLoadDisposition::Keepalive,
                None,
            )
            .expect("the sending Document has an active resource loader");
        host.record_pending_subresource_beacon(
            context.execution_context,
            context.request_origin.clone(),
            load,
            network,
            context.network_partition_key,
            info,
        );
        host.record_native_resource_observation(started);
        return;
    }
    host.record_native_resource_observation(started);
    let failure = if host.network_offline() {
        Some("Network emulation offline".to_owned())
    } else if host.is_url_blocked(&info.url) {
        Some(BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
    } else if should_request_be_blocked_due_to_bad_port(&info.url) {
        Some(format!(
            "{}: blocked bad port for `{}`",
            if matches!(metadata, BrowserRequestMetadata::Beacon) {
                "sendBeacon"
            } else {
                "ping"
            },
            info.url
        ))
    } else {
        None
    };
    if let Some(message) = failure {
        network.failed(&crate::network::ResourceResponseFailure::Request(message));
        return;
    }
    let request = match Request::new_bytes(
        "POST",
        info.url.as_str(),
        info.request_body_bytes,
        info.request_headers,
    ) {
        Ok(request) => request
            .with_initiator_url(&info.document_url)
            .with_request_origin(context.request_origin.clone())
            .with_resource_type(if matches!(metadata, BrowserRequestMetadata::Beacon) {
                RequestResourceType::Beacon
            } else {
                RequestResourceType::Ping
            })
            .with_browser_request_metadata(metadata)
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(RequestCredentialsMode::Include)
            .with_network_partition_key(context.network_partition_key)
            .with_subframe_context(info.frame_id.is_some()),
        Err(error) => {
            network.failed(&crate::network::ResourceResponseFailure::Request(
                error.to_string(),
            ));
            return;
        }
    };
    let cancel = FetchCancelHandle::new();
    let load = context
        .resource_loader
        .register_load(
            crate::network::loads::ResourceLoadKind::Beacon,
            crate::network::loads::ResourceLoadDisposition::Keepalive,
            Some(cancel.clone()),
        )
        .expect("the sending Document has an active resource loader");
    KeepaliveResource::new(network, load).fetch(
        context.resource_loader.request_client().clone(),
        request,
        cancel,
    );
}

struct WindowPingRequestContext {
    execution_context: crate::native_bridge::WindowExecutionContextIdentity,
    resource_loader: crate::network::context::DocumentResourceLoader,
    frame_id: Option<String>,
    document_url: url::Url,
    base_url: url::Url,
    request_origin: moli_url::WebOrigin,
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
    let environment = host.subresource_request_environment(&resource_loader, owner)?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        base_url,
        request_origin,
        frame_id,
    } = environment;
    Some(WindowPingRequestContext {
        execution_context,
        resource_loader,
        frame_id,
        document_url,
        base_url,
        request_origin,
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
        && web_api_interfaces::ReadableStream::is_instance(scope, object)
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
