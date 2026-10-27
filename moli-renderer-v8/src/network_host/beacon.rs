use super::*;
use crate::web_api_interfaces;
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

    let execution_context = {
        let host = unsafe { &*host_ptr };
        let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
            .expect("binding validated Navigator receiver");
        receiver.get_creation_context(scope).and_then(|context| {
            host.window_execution_context_identity_for_v8_context(scope, context)
        })
    };

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
    let Some(execution_context) = execution_context else {
        rv.set_bool(false);
        return;
    };
    let Some(document_target) = host
        .current_window_document_task_target_for_dispatch_scope(execution_context.dispatch_scope())
    else {
        rv.set_bool(false);
        return;
    };
    let Some(request_context) =
        window_ping_request_context(scope, host, execution_context, document_target)
    else {
        rv.set_bool(false);
        return;
    };
    let WindowPingRequestContext {
        resource_loader,
        frame_id,
        document_url,
        base_url,
        request_origin,
        network_partition_key,
        ..
    } = &request_context;
    let resolved_url = match resolve_context_url(base_url, &raw_url, None) {
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
    let request_mode = if body_content_type.as_ref().is_some_and(|content_type| {
        !moli_fetch::is_cors_safelisted_request_header("Content-Type", content_type)
    }) {
        RequestMode::Cors
    } else {
        RequestMode::NoCors
    };
    let preflight_request_headers = request_headers.clone();
    let request_headers =
        merge_subresource_request_headers(host.extra_http_headers(), &request_headers);
    let request_body_text = request_body_text(&body);
    let request_cookie_report = observe_subresource_request_cookie_report(
        resource_loader.request_client(),
        document_url,
        request_origin,
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

    let request =
        match Request::new_bytes("POST", resolved_url.as_str(), body, request_headers.clone()) {
            Ok(request) => request
                .with_initiator_url(document_url)
                .with_request_origin(request_origin.clone())
                .with_resource_type(RequestResourceType::Beacon)
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Beacon)
                .with_request_mode(request_mode)
                .with_credentials_mode(RequestCredentialsMode::Include)
                .with_network_partition_key(network_partition_key.clone())
                .with_redirect_mode(RequestRedirectMode::Follow)
                .with_subframe_context(frame_id.is_some()),
            Err(error) => {
                crate::util::throw_type_error(scope, &error.to_string());
                return;
            }
        };
    let request = match request
        .with_subresource_request_metadata(moli_fetch::SubresourceRequestMetadata {
            document_referrer_policy: request_context.referrer_policy.clone(),
            ..Default::default()
        })
        .with_fetch_referrer(&request_context.referrer_for(&resolved_url))
    {
        Ok(request) => request,
        Err(error) => {
            crate::util::throw_type_error(scope, &error.to_string());
            return;
        }
    };
    rv.set_bool(start_window_ping(
        scope,
        host,
        &request_context,
        request,
        info,
        preflight_request_headers,
        "beacon",
    ));
}

pub(crate) fn send_link_audit_ping(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    link: crate::document_runtime::DomHandle,
    ping_url: url::Url,
    destination_url: &str,
) {
    if !matches!(ping_url.scheme(), "http" | "https") {
        return;
    }
    let host = unsafe { &mut *host_ptr };
    let Some(owner) = host.owner_dispatch_scope_for_node(link) else {
        return;
    };
    let Some(execution_context) = host.current_registered_window_execution_context_identity(owner)
    else {
        return;
    };
    let Some(document_target) = host
        .current_window_document_task_target_for_dispatch_scope(execution_context.dispatch_scope())
    else {
        return;
    };
    let Some(request_context) =
        window_ping_request_context(scope, host, execution_context, document_target)
    else {
        return;
    };
    let WindowPingRequestContext {
        resource_loader,
        frame_id,
        document_url,
        request_origin,
        network_partition_key,
        ..
    } = &request_context;
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
        request_origin,
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
        request_body_bytes: request_body
            .as_ref()
            .map(|body| bytes::Bytes::copy_from_slice(body.as_bytes())),
        resource_type: SubresourceResourceType::Ping,
        request_cookie_report,
    };

    let request = match Request::new_bytes(
        "POST",
        ping_url.as_str(),
        Some(bytes::Bytes::from_static(b"PING")),
        request_headers.clone(),
    ) {
        Ok(request) => request
            .with_initiator_url(document_url)
            .without_inferred_referrer()
            .with_request_origin(request_origin.clone())
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
    let _ = start_window_ping(
        scope,
        host,
        &request_context,
        request,
        info,
        Vec::new(),
        "ping",
    );
}

struct WindowPingRequestContext {
    execution_context: crate::native_bridge::WindowExecutionContextIdentity,
    document_target: crate::native_bridge::WindowDocumentTaskTarget,
    policy_context: crate::types::SubresourcePolicyContext,
    referrer_policy: Option<String>,
    referrer_source: Option<url::Url>,
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
    execution_context: crate::native_bridge::WindowExecutionContextIdentity,
    document_target: crate::native_bridge::WindowDocumentTaskTarget,
) -> Option<WindowPingRequestContext> {
    let owner = execution_context.dispatch_scope();
    if !host.window_execution_context_owner_is_current(execution_context.owner(), owner)
        || !host.window_document_owner_is_current_for_dispatch_scope(document_target.owner(), owner)
    {
        return None;
    }
    let resource_loader =
        host.document_resource_loader_for_window_owner(document_target.owner())?;
    let environment = host.subresource_request_environment(&resource_loader, owner)?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        base_url,
        request_origin,
        frame_id,
    } = environment;
    Some(WindowPingRequestContext {
        execution_context,
        document_target,
        policy_context: effective_subresource_policy_context(scope, host, owner),
        referrer_policy: effective_subresource_referrer_policy(host, owner),
        referrer_source: host.document_client_referrer_source(owner),
        resource_loader,
        frame_id,
        document_url,
        base_url,
        request_origin,
        network_partition_key: active_subresource_network_partition_key(host, owner),
    })
}

impl WindowPingRequestContext {
    fn referrer_for(&self, url: &url::Url) -> String {
        self.referrer_source
            .as_ref()
            .and_then(|source| {
                moli_fetch::referrer_value(source, url, self.referrer_policy.as_deref(), None)
            })
            .unwrap_or_default()
    }
}

fn start_window_ping(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    context: &WindowPingRequestContext,
    mut request: Request,
    info: PendingSubresourceFetchInfo,
    preflight_request_headers: Vec<(String, String)>,
    initiator_type: &'static str,
) -> bool {
    let owner = context.execution_context.dispatch_scope();
    let Some(policy) = host.document_connect_policy_snapshot_for_owner(owner) else {
        return false;
    };
    let Some(report_context) = capture_window_csp_report_request_context(scope, host, owner) else {
        return false;
    };
    if report_context.identity() != context.document_target {
        return false;
    }
    let (reports, enforced) = policy
        .check_url(
            &context.document_url,
            &request.url,
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .into_violations();
    let policy_error = enforced.first().map(|violation| {
        crate::document_runtime::document_content_security_policy_error_message(
            violation,
            initiator_type,
        )
    });
    let host_ptr = host as *mut JsContextHost;
    for violation in reports.into_iter().chain(enforced) {
        send_content_security_policy_violation_report_from_window_context(
            host,
            &report_context,
            &violation,
        );
        host.dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(
            scope, host_ptr, context.document_target, &violation,
        );
    }
    if let Some(error) = policy_error {
        // Blink's Beacon path stops before constructing a ResourceRequest.
        // Hyperlink auditing instead exposes a failed ping timing, even when
        // the keepalive quota is already full. Neither reaches transport.
        if request.resource_type == RequestResourceType::Ping {
            let timing = crate::context_bootstrap::FetchResourceTiming::for_initiator_type("ping");
            host.record_window_resource_timing(
                scope,
                context.execution_context,
                timing.failure(&info.url),
            );
            record_beacon_failure(host, info, error);
        }
        return true;
    }
    let beacon = crate::types::PendingBeacon {
        request: request.clone(),
        policy_context: context.policy_context,
        redirect_state: FetchCspRedirectState::new(&policy),
        policy,
        report_context,
        timing: Some(
            crate::context_bootstrap::FetchResourceTiming::for_initiator_type(initiator_type),
        ),
    };
    let cancel_handle = FetchCancelHandle::new();
    let Some(internal_id) = host.record_async_subresource_beacon(
        context.execution_context,
        beacon,
        Some(cancel_handle.clone()),
        info.clone(),
    ) else {
        return false;
    };
    let error = if host.network_offline() {
        Some("Network emulation offline".to_owned())
    } else if host.is_url_blocked(&request.url) {
        Some(BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
    } else if should_request_be_blocked_due_to_bad_port(&request.url) {
        Some(format!(
            "{initiator_type}: blocked bad port for `{}`",
            request.url
        ))
    } else {
        None
    };
    if let Some(error) = error {
        let _ = host.resource_completion_sender().send_async_subresource(
            crate::types::AsyncSubresourceFetchCompletion {
                internal_id,
                request_url: request.url,
                request_method: request.method,
                request_headers: request.request_headers,
                request_body: info.request_body,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: Err(error).into(),
            },
        );
        return true;
    }
    if host.should_intercept_subresource(SubresourceResourceType::Ping) {
        host.pause_pending_subresource_fetch(internal_id);
        return true;
    }
    if let Some(check) = host.window_fetch_redirect_check(internal_id) {
        request = request.with_redirect_check(check);
    }
    spawn_async_subresource_fetch(
        context.resource_loader.task_runner(),
        host.resource_completion_sender(),
        context.resource_loader.request_client().clone(),
        request,
        Some(cancel_handle),
        preflight_request_headers,
        internal_id,
        AsyncSubresourceNetworkContext {
            frame_id: info.frame_id,
            request_origin: context.request_origin.clone(),
            document_url: info.document_url,
            resource_type: info.resource_type,
            policy_context: context.policy_context,
        },
        info.url,
        info.method,
        info.request_headers,
        info.request_body,
        None,
    );
    true
}

fn navigator_beacon_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(Option<bytes::Bytes>, Option<String>)> {
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
        Ok(Some(body)) => {
            let bytes = match body.source {
                PreparedBodySource::Bytes(bytes) => bytes,
                PreparedBodySource::Blob(blob) => {
                    let blob = v8::Local::new(scope, blob);
                    blob::blob_bytes_from_object(scope, blob)?.into()
                }
                PreparedBodySource::Stream(_) => unreachable!("sendBeacon rejects streams"),
            };
            Some((Some(bytes), body.content_type))
        }
        Ok(None) => Some((None, None)),
        Err(error) => {
            crate::util::throw_type_error(scope, &error.to_string());
            None
        }
    }
}

fn request_body_text(body: &Option<bytes::Bytes>) -> Option<String> {
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
