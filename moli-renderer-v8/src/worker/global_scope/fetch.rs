use super::*;
mod native_load;
use std::sync::atomic::AtomicU64;

use crate::network::interception_output::ResourceInterceptionContext;
use crate::network_host::{BlobUrlFetchEnvironment, CapturedBlobUrl};

use crate::network_host::{FetchArgumentError, combined_headers_entries, convert_fetch_arguments};
use crate::network_host::{HeadersGuard, validate_fetch_response_headers};
use crate::service_worker_runtime::ServiceWorkerFetchRequestMetadata;
use crate::types::{
    ScriptNetworkOutputItem, SubresourcePolicyContext, WebSocketLifecycleEvent,
    WebSocketNetworkEvent,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::worker) enum WorkerFetchKind {
    Fetch,
    Font,
}

impl WorkerFetchKind {
    fn resource_type(self) -> SubresourceResourceType {
        match self {
            Self::Fetch => SubresourceResourceType::Fetch,
            Self::Font => SubresourceResourceType::Font,
        }
    }

    fn csp_resource_kind(
        self,
    ) -> crate::content_security_policy::ContentSecurityPolicyResourceKind {
        use crate::content_security_policy::ContentSecurityPolicyResourceKind;
        match self {
            Self::Fetch => ContentSecurityPolicyResourceKind::WorkerConnect,
            Self::Font => ContentSecurityPolicyResourceKind::DocumentFont,
        }
    }

    fn browser_metadata(self) -> BrowserRequestMetadata {
        match self {
            Self::Fetch => BrowserRequestMetadata::Fetch,
            Self::Font => BrowserRequestMetadata::Font,
        }
    }
}

pub(in crate::worker) fn record_worker_subresource_failure(
    state: &WorkerGlobalState,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: moli_fetch::RequestHeaders,
    request_body: Option<String>,
    resource_type: SubresourceResourceType,
    error_text: String,
) {
    record_worker_subresource_failure_with_handle(
        state,
        None,
        document_url,
        url,
        method,
        request_headers,
        request_body.map(bytes::Bytes::from),
        resource_type,
        error_text,
    );
}

pub(in crate::worker) fn record_worker_subresource_failure_with_handle(
    state: &WorkerGlobalState,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    document_url: Url,
    url: Url,
    method: String,
    request_headers: moli_fetch::RequestHeaders,
    request_body_bytes: Option<bytes::Bytes>,
    resource_type: SubresourceResourceType,
    error_text: String,
) {
    let mut record = SubresourceNetworkRecord::failure(
        None,
        document_url,
        url,
        method,
        request_headers,
        request_body_text(&request_body_bytes),
        resource_type,
        error_text,
    )
    .with_request_body_bytes(request_body_bytes);
    if let Some(handle) = request_handle {
        record = record.with_request_handle(handle);
    }
    if let Some(output) = &state.interception_context {
        output.record_subresource_network(record);
    }


}

fn worker_fetch_script_metadata(
    document_referrer_policy: Option<String>,
    request_metadata: &ServiceWorkerFetchRequestMetadata,
) -> Option<moli_fetch::ScriptFetchRequestMetadata> {
    let referrer_policy = (!request_metadata.referrer_policy.is_empty())
        .then(|| request_metadata.referrer_policy.clone());
    let integrity =
        (!request_metadata.integrity.is_empty()).then(|| request_metadata.integrity.clone());
    if referrer_policy.is_none() && document_referrer_policy.is_none() && integrity.is_none() {
        return None;
    }
    Some(moli_fetch::ScriptFetchRequestMetadata {
        referrer_policy,
        document_referrer_policy,
        integrity,
        ..moli_fetch::ScriptFetchRequestMetadata::default()
    })
}

fn record_worker_websocket_subresource_failure(
    output: &ResourceInterceptionContext,
    socket_id: u64,
    document_url: Url,
    url: Url,
    request_headers: moli_fetch::RequestHeaders,
    error_text: String,
) {
    output.record_subresource_network(
        SubresourceNetworkRecord::failure(
            None,
            document_url,
            url,
            "GET".to_owned(),
            request_headers,
            None,
            SubresourceResourceType::WebSocket,
            error_text,
        )
        .with_websocket_socket_id(socket_id),
    );
}

#[must_use = "the caller must stop mixed-content processing when worker CSP blocks the request"]
pub(crate) enum WorkerWebSocketCspOutcome {
    Allowed,
    Blocked(String),
}

impl WorkerWebSocketCspOutcome {
    pub(crate) fn blocks_request(&self) -> bool {
        matches!(self, Self::Blocked(_))
    }

    fn into_failure_message(self) -> Option<String> {
        match self {
            Self::Allowed => None,
            Self::Blocked(message) => Some(message),
        }
    }
}

pub(crate) fn check_worker_websocket_csp(
    scope: &mut v8::PinScope<'_, '_>,
    document_url: &Url,
    url: &Url,
) -> Option<WorkerWebSocketCspOutcome> {
    let state = get_worker_state(scope)?;
    let (wake_tx, report_only_violation, enforced_violation) = {
        let state_ref = state.borrow();
        (
            state_ref.worker_wake_tx.clone(),
            worker_content_security_policy_report_only_violation(
                &state_ref,
                document_url,
                url,
                crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
            ),
            worker_content_security_policy_violation(
                &state_ref,
                document_url,
                url,
                crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
            ),
        )
    };
    let outcome = match &enforced_violation {
        Some(violation) => WorkerWebSocketCspOutcome::Blocked(
            worker_content_security_policy_error_message(violation, "WebSocket"),
        ),
        None => WorkerWebSocketCspOutcome::Allowed,
    };
    // CSP reports are tasks, including those produced by WebSocket construction.
    // They must not overtake earlier queued Trusted Types violation reports.
    for violation in [report_only_violation, enforced_violation]
        .into_iter()
        .flatten()
    {
        let _ = wake_tx.send(WorkerMessage::DispatchContentSecurityPolicyViolation(
            Box::new(violation),
        ));
    }
    Some(outcome)
}

pub(crate) fn register_worker_websocket<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    document_url: Url,
    url: Url,
    protocols: Vec<String>,
    csp_outcome: WorkerWebSocketCspOutcome,
) -> Option<u64> {
    let state = get_worker_state(scope)?;
    // Nested workers can publish into the same target's Network agent.
    static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);
    let socket_id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    assert!(socket_id < 1 << 53, "Worker WebSocket id space exhausted");
    let (loader, extra_http_headers, network_offline, blocked_url_patterns, websocket_event_tx) = {
        let state = state.borrow();
        (
            state.loader.clone(),
            state.extra_http_headers.clone(),
            state.network_offline,
            state.blocked_url_patterns.clone(),
            state.websocket_event_tx.clone(),
        )
    };

    let cookie_url = websocket_cookie_url(&url);
    let cookie_context = moli_cookie_jar::NetworkCookieRequestContext::subresource("GET")
        .with_initiator_url(&cookie_url, &document_url);
    let cookie_header = moli_fetch::cookie_header_for_request(
        &loader.request_client().cookie_store(),
        &cookie_url,
        cookie_context,
    );
    let cookie_header_for_context = cookie_header
        .as_ref()
        .ok()
        .and_then(|header| header.clone());
    let context = WebSocketConnectOptions {
        origin: moli_url::origin_ascii_serialization(&document_url),
        user_agent: loader
            .request_client()
            .effective_browser_identity()
            .user_agent()
            .to_owned(),
        extra_headers: extra_http_headers,
        http_proxy: loader.request_client().http_proxy().map(ToOwned::to_owned),
        http_no_proxy: loader
            .request_client()
            .http_no_proxy()
            .map(ToOwned::to_owned),
        http_host_resolve: loader.request_client().http_host_resolve().to_vec(),
        proxy_bearer_token: loader
            .request_client()
            .proxy_bearer_token()
            .map(ToOwned::to_owned),
        tls: loader.request_client().tls_config().clone(),
        cookie_header: cookie_header_for_context,
    };

    let csp_failure = csp_outcome.into_failure_message();
    let blocked = worker_url_blocked(&blocked_url_patterns, &url);
    let failure_message = if csp_failure.is_some() {
        csp_failure
    } else if blocked {
        Some(BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
    } else if network_offline {
        Some("Network emulation offline".to_owned())
    } else {
        cookie_header
            .err()
            .map(|error| format!("failed to build WebSocket cookie header: {error}"))
    };
    let handshake_recorded = failure_message.is_some();
    let (connection, load) = if let Some(error_text) = failure_message {
        record_worker_websocket_subresource_failure(
            state
                .borrow()
                .interception_context
                .as_ref()
                .expect("Worker WebSocket has a network output owner"),
            socket_id,
            document_url.clone(),
            url.clone(),
            context.extra_headers.clone(),
            error_text.clone(),
        );
        (
            spawn_failed_connection(socket_id, error_text, websocket_event_tx),
            None,
        )
    } else {
        let load = loader.register_load(
            ResourceLoadKind::WebSocket,
            ResourceLoadDisposition::Ordinary,
            None,
        )?;
        let connection = spawn_connection(
            loader.request_client().websocket_connector(),
            socket_id,
            url.to_string(),
            protocols,
            context,
            websocket_event_tx,
        );
        let cancel_tx = connection.clone();
        load.attach_consumer_cancel(move || {
            cancel_tx.cancel();
        });
        (connection, Some(load))
    };

    state.borrow_mut().websockets.insert(
        socket_id,
        WorkerWebSocketState {
            wrapper: v8::Global::new(scope, wrapper),
            stream_abort: None,
            connection,
            document_url,
            url,
            load,
            handshake_recorded,
        },
    );
    Some(socket_id)
}

pub(crate) fn set_worker_websocket_stream_abort_registration(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    registration: crate::context_bootstrap::WebSocketStreamAbortRegistration,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    let Some(connection) = state.websockets.get_mut(&socket_id) else {
        return false;
    };
    connection.stream_abort = Some(registration);
    true
}

pub(crate) fn take_worker_websocket_stream_abort_registration(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
) -> Option<crate::context_bootstrap::WebSocketStreamAbortRegistration> {
    let state = get_worker_state(scope)?;
    state
        .borrow_mut()
        .websockets
        .get_mut(&socket_id)?
        .stream_abort
        .take()
}

pub(crate) fn worker_websocket_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket_id: u64,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    if state.closed {
        return None;
    }
    state
        .websockets
        .get(&socket_id)
        .map(|entry| v8::Local::new(scope, &entry.wrapper))
}

pub(crate) fn send_worker_websocket_text(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    text: String,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    let connection = state
        .borrow()
        .websockets
        .get(&socket_id)?
        .connection
        .clone();
    Some(connection.send_text(text).is_ok())
}

pub(crate) fn send_worker_websocket_binary(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    bytes: Vec<u8>,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    let connection = state
        .borrow()
        .websockets
        .get(&socket_id)?
        .connection
        .clone();
    Some(connection.send_binary(bytes).is_ok())
}

pub(crate) fn send_worker_websocket_blob(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    data: moli_file_api::BlobData,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    let connection = state
        .borrow()
        .websockets
        .get(&socket_id)?
        .connection
        .clone();
    Some(connection.send_blob(data).is_ok())
}

pub(crate) fn close_worker_websocket(
    scope: &mut v8::PinScope<'_, '_>,
    socket_id: u64,
    code: Option<u16>,
    reason: String,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    let connection = state
        .borrow()
        .websockets
        .get(&socket_id)?
        .connection
        .clone();
    Some(connection.close(code, reason).is_ok())
}

pub(in crate::worker) fn dispatch_worker_websocket_event(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    event: &WebSocketEvent,
) -> crate::context_bootstrap::WebSocketDispatchResult {
    let socket_id = event.socket_id();
    let (socket, document_url, socket_url, output) = {
        let mut state = state.borrow_mut();
        let WorkerGlobalState {
            interception_context,
            loader,
            websockets,
            ..
        } = &mut *state;
        let Some(entry) = websockets.get_mut(&socket_id) else {
            return crate::context_bootstrap::WebSocketDispatchResult::Noop;
        };
        let output = interception_context
            .clone()
            .expect("Worker WebSocket has a network output owner");
        let document_url = &entry.document_url;
        let socket_url = &entry.url;
        let item = match event {
            WebSocketEvent::Open {
                request_headers,
                response_status,
                response_headers,
                ..
            } => {
                let cookie_set_reports =
                    store_worker_websocket_response_cookies(socket_url, loader, response_headers);
                entry.handshake_recorded = true;
                output.record_subresource_network(
                    SubresourceNetworkRecord::success(
                        None,
                        document_url.clone(),
                        socket_url.clone(),
                        "GET".to_owned(),
                        request_headers.clone(),
                        None,
                        SubresourceResourceType::WebSocket,
                        None,
                        Vec::new(),
                        socket_url.clone(),
                        *response_status,
                        response_headers.to_vec(),
                        String::new(),
                        cookie_set_reports,
                    )
                    .with_websocket_socket_id(socket_id),
                );
                None
            }
            WebSocketEvent::SendCompleted {
                opcode,
                payload_length,
                ..
            } => Some(ScriptNetworkOutputItem::WebSocketNetworkEvent(
                WebSocketNetworkEvent::new(
                    socket_id,
                    document_url.clone(),
                    socket_url.clone(),
                    WebSocketFrameDirection::Sent,
                    worker_websocket_frame_opcode(*opcode),
                    *payload_length,
                ),
            )),
            WebSocketEvent::Error { message, .. } => {
                if !entry.handshake_recorded {
                    entry.handshake_recorded = true;
                    record_worker_websocket_subresource_failure(
                        &output,
                        socket_id,
                        document_url.clone(),
                        socket_url.clone(),
                        Default::default(),
                        message.clone(),
                    );
                }
                Some(ScriptNetworkOutputItem::WebSocketLifecycleEvent(
                    WebSocketLifecycleEvent::error(
                        socket_id,
                        document_url.clone(),
                        socket_url.clone(),
                        message.clone(),
                    ),
                ))
            }
            WebSocketEvent::Close {
                code,
                reason,
                was_clean,
                ..
            } => Some(ScriptNetworkOutputItem::WebSocketLifecycleEvent(
                WebSocketLifecycleEvent::close(
                    socket_id,
                    document_url.clone(),
                    socket_url.clone(),
                    *code,
                    reason.clone(),
                    *was_clean,
                ),
            )),
            WebSocketEvent::HandshakeResponse { .. }
            | WebSocketEvent::Closing { .. }
            | WebSocketEvent::TextMessage { .. }
            | WebSocketEvent::BinaryMessage { .. } => None,
        };
        if let Some(item) = item {
            output.publish_network(item);
        }
        (
            v8::Local::new(scope, &entry.wrapper),
            document_url.clone(),
            socket_url.clone(),
            output,
        )
    };
    let dispatch_result = crate::context_bootstrap::dispatch_websocket_event(scope, socket, event);
    if dispatch_result != crate::context_bootstrap::WebSocketDispatchResult::Backpressured {
        let frame = match event {
            WebSocketEvent::TextMessage { data, .. } => {
                Some((WebSocketFrameOpcode::Text, data.len()))
            }
            WebSocketEvent::BinaryMessage { data, .. } => {
                Some((WebSocketFrameOpcode::Binary, data.len()))
            }
            _ => None,
        };
        if let Some((opcode, payload_length)) = frame {
            output.publish_network(ScriptNetworkOutputItem::WebSocketNetworkEvent(
                WebSocketNetworkEvent::new(
                    socket_id,
                    document_url,
                    socket_url,
                    WebSocketFrameDirection::Received,
                    opcode,
                    payload_length,
                ),
            ));
        }
    }
    if matches!(event, WebSocketEvent::Close { .. })
        && let Some(socket) = state.borrow_mut().websockets.remove(&socket_id)
        && let Some(load) = socket.load
    {
        load.finish();
    }
    dispatch_result
}

fn worker_websocket_frame_opcode(opcode: moli_websocket::FrameOpcode) -> WebSocketFrameOpcode {
    match opcode {
        moli_websocket::FrameOpcode::Text => WebSocketFrameOpcode::Text,
        moli_websocket::FrameOpcode::Binary => WebSocketFrameOpcode::Binary,
    }
}

fn store_worker_websocket_response_cookies(
    socket_url: &Url,
    request_client: &crate::network::context::WorkerResourceLoader,
    response_headers: &[(String, Vec<u8>)],
) -> Vec<StoredCookieSetReport> {
    if !response_headers
        .iter()
        .any(|(name, _)| response_header_name_is(name, &HeaderName::from_static("set-cookie")))
    {
        return Vec::new();
    }
    let response_cookie_url = websocket_cookie_url(socket_url);
    let cookie_store = request_client.request_client().cookie_store();
    let mut store = match cookie_store.lock() {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(%error, "WebSocket response cookies could not access storage");
            return response_headers
                .iter()
                .filter(|(name, _)| {
                    response_header_name_is(name, &HeaderName::from_static("set-cookie"))
                })
                .map(|_| StoredCookieSetReport::store_unavailable())
                .collect();
        }
    };
    store.store_response_headers_with_reports(&response_cookie_url, response_headers)
}

fn response_header_name_is(candidate: &str, expected: &HeaderName) -> bool {
    HeaderName::from_bytes(candidate.as_bytes()).is_ok_and(|candidate| candidate == *expected)
}

pub(in crate::worker) fn request_body_text(body: &Option<bytes::Bytes>) -> Option<String> {
    body.as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
}

pub(in crate::worker) fn make_rejected_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("worker fetch resolver");
    let promise = resolver.get_promise(scope);
    if let Some(message) = v8_string(scope, message) {
        resolver.reject(scope, v8::Exception::type_error(scope, message));
    } else {
        resolver.reject(scope, v8::undefined(scope).into());
    }
    promise
}

pub(in crate::worker) fn make_rejected_promise_with_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("worker fetch resolver");
    let promise = resolver.get_promise(scope);
    let _ = resolver.reject(scope, reason);
    promise
}

pub(in crate::worker) struct ResolvedWorkerFetchInput<'s> {
    resolved_url: Url,
    blob_url_entry: Option<CapturedBlobUrl>,
    method: String,
    body: Option<bytes::Bytes>,
    body_stream: Option<v8::Global<v8::Object>>,
    headers: Vec<(String, String)>,
    request_mode: moli_fetch::RequestMode,
    credentials_mode: RequestCredentialsMode,
    redirect_mode: RequestRedirectMode,
    priority: Option<moli_fetch::FetchPriorityHint>,
    metadata: ServiceWorkerFetchRequestMetadata,
    signal: Option<v8::Local<'s, v8::Object>>,
}

impl ResolvedWorkerFetchInput<'_> {
    pub(super) fn font(url: Url) -> Self {
        Self {
            blob_url_entry: CapturedBlobUrl::capture(&url),
            resolved_url: url,
            method: "GET".to_owned(),
            body: None,
            body_stream: None,
            headers: Vec::new(),
            request_mode: RequestMode::Cors,
            credentials_mode: RequestCredentialsMode::SameOrigin,
            redirect_mode: RequestRedirectMode::Follow,
            priority: None,
            metadata: ServiceWorkerFetchRequestMetadata::default(),
            signal: None,
        }
    }
}

pub(in crate::worker) fn resolve_worker_fetch_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    base_url: &Url,
) -> Result<ResolvedWorkerFetchInput<'s>, FetchArgumentError> {
    if args.length() < 1 {
        return Err(
            webidl::WebIdlError::missing_required(webidl::Context::argument("fetch", 1)).into(),
        );
    }
    let args = webidl::try_parse_args::<crate::network_host::FetchArgs>(scope, args)?;
    let inherit_body = !args.init.body_is_present();
    let (url, inherited, source) = args.input.materialize_for_fetch(scope, inherit_body)?;
    let mut request_like = None;
    let mut consumes_request_body = false;
    let signal;
    let body_stream;
    let has_stream_body;
    let blob_url_entry = inherited
        .as_ref()
        .and_then(|request| request.blob_url_entry.clone());
    let (
        url_input,
        method,
        body,
        headers,
        request_mode,
        credentials_mode,
        redirect_mode,
        priority,
        mut metadata,
        init_validation,
    ) = if let Some(inherited) = inherited {
        let req_obj = source.expect("request input source");
        request_like = Some(req_obj);
        let init = parse_fetch_init(scope, args.init)?;
        if inherited.body_unusable && !init.body_present {
            return Err(crate::network_host::request_body_already_used_error().into());
        }
        let body = if init.body_present {
            init.body.clone()
        } else {
            inherited.body_bytes_for_fetch(scope, req_obj)?
        };
        signal = init.signal.unwrap_or(inherited.signal);
        body_stream = if init.body_present {
            init.body_stream.clone()
        } else {
            crate::network_host::body_stream_object(scope, req_obj)
                .map(|stream| v8::Global::new(scope, stream))
        };
        consumes_request_body =
            !init.body_present && (inherited.body.is_some() || body_stream.is_some());
        has_stream_body =
            init.body_stream.is_some() || !init.body_present && inherited.has_stream_body;
        let method = if init.method_present {
            init.method.clone()
        } else {
            inherited.method.clone()
        };
        let mut headers = if init.headers_present {
            init.headers.clone()
        } else {
            inherited.headers.clone()
        };
        if init.body_present && !init.headers_present {
            append_default_body_content_type(&mut headers, init.body_content_type.as_deref());
        }
        let inherited_credentials = request_object_credentials_mode(scope, req_obj)?;
        let request_mode = init
            .validation
            .mode
            .or_else(|| moli_fetch::RequestMode::from_str(&inherited.mode).ok())
            .unwrap_or(moli_fetch::RequestMode::Cors);
        validate_worker_no_cors_method(request_mode, &method)?;
        crate::network_host::validate_request_method_body(&method, body.as_ref())?;
        let headers = if init.headers_present || request_mode == moli_fetch::RequestMode::NoCors {
            filter_headers_for_guard(
                &headers,
                request_headers_guard_for_mode(request_mode.as_ref()),
            )
        } else {
            filter_headers_for_guard(&headers, HeadersGuard::Request)
        };
        let credentials_mode = init
            .credentials_mode
            .or(inherited_credentials)
            .unwrap_or(RequestCredentialsMode::SameOrigin);
        let redirect_mode = init
            .redirect_mode
            .or_else(|| crate::network_host::parse_request_redirect_mode_label(&inherited.redirect))
            .unwrap_or(RequestRedirectMode::Follow);
        let priority = init.priority.or_else(|| {
            (inherited.priority != moli_fetch::FetchPriorityHint::Auto)
                .then_some(inherited.priority)
        });
        let metadata = ServiceWorkerFetchRequestMetadata {
            mixed_content_policy: Default::default(),
            cache: init.cache.unwrap_or(inherited.cache),
            referrer: inherited.referrer,
            referrer_policy: init.referrer_policy.unwrap_or(inherited.referrer_policy),
            integrity: init.integrity.unwrap_or(inherited.integrity),
            keepalive: init.keepalive.unwrap_or(inherited.keepalive),
            is_history_navigation: false,
            use_cors_preflight: false,
            link_preload: false,
        };
        // Validate the merged signal without rereading the legacy input getter.
        if let Some(signal) = signal.as_ref() {
            let signal = v8::Local::new(scope, signal);
            webidl::convert::<crate::abort_signal_route::ResolvedAbortSignal>(
                scope,
                signal.into(),
                webidl::Context::member("Request", "signal"),
            )?;
        }
        (
            url,
            method,
            body,
            headers,
            request_mode,
            credentials_mode,
            redirect_mode,
            priority,
            metadata,
            init.validation,
        )
    } else {
        let init = parse_fetch_init(scope, args.init)?;
        signal = init.signal.flatten();
        body_stream = init.body_stream.clone();
        has_stream_body = body_stream.is_some();
        let request_mode = init
            .validation
            .mode
            .unwrap_or(moli_fetch::RequestMode::Cors);
        validate_worker_no_cors_method(request_mode, &init.method)?;
        crate::network_host::validate_request_method_body(&init.method, init.body.as_ref())?;
        let headers = filter_headers_for_guard(
            &init.headers,
            request_headers_guard_for_mode(request_mode.as_ref()),
        );
        let credentials_mode = init
            .credentials_mode
            .unwrap_or(RequestCredentialsMode::SameOrigin);
        let redirect_mode = init.redirect_mode.unwrap_or(RequestRedirectMode::Follow);
        let metadata = ServiceWorkerFetchRequestMetadata {
            mixed_content_policy: Default::default(),
            cache: init.cache.unwrap_or_else(|| "default".to_owned()),
            referrer: "about:client".to_owned(),
            referrer_policy: init.referrer_policy.unwrap_or_default(),
            integrity: init.integrity.unwrap_or_default(),
            keepalive: init.keepalive.unwrap_or(false),
            is_history_navigation: false,
            use_cors_preflight: false,
            link_preload: false,
        };
        (
            url,
            init.method,
            init.body,
            headers,
            request_mode,
            credentials_mode,
            redirect_mode,
            init.priority,
            metadata,
            init.validation,
        )
    };
    let resolved_url = resolve_context_url(base_url, url_input.as_str(), None)?;
    url_input.validate_credentials(&resolved_url)?;
    crate::network_host::validate_fetch_body(
        body.is_some() || body_stream.is_some(),
        has_stream_body,
        &method,
        request_mode,
    )?;
    if let Some(referrer) =
        init_validation.validate(scope, request_mode.as_ref(), &metadata.cache)?
    {
        metadata.referrer = referrer;
    }
    let signal = signal.as_ref().map(|signal| v8::Local::new(scope, signal));
    let blob_url_entry = blob_url_entry.or_else(|| CapturedBlobUrl::capture(&resolved_url));
    if consumes_request_body && let Some(request_like) = request_like {
        crate::network_host::mark_request_input_body_used_for_fetch(scope, request_like);
    }
    Ok(ResolvedWorkerFetchInput {
        resolved_url,
        blob_url_entry,
        method,
        body,
        body_stream,
        headers,
        request_mode,
        credentials_mode,
        redirect_mode,
        priority,
        metadata,
        signal,
    })
}

pub(in crate::worker) fn validate_worker_no_cors_method(
    request_mode: RequestMode,
    method: &str,
) -> Result<(), FetchArgumentError> {
    if request_mode == RequestMode::NoCors && !moli_fetch::is_cors_safelisted_method(method) {
        return Err(FetchArgumentError::UnsupportedNoCorsMethod {
            method: method.to_owned(),
            interface: Some("DedicatedWorkerGlobalScope"),
        });
    }
    Ok(())
}

pub(in crate::worker) fn worker_fetch_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(document_url) = worker_current_script_url(scope) else {
        rv.set(make_rejected_promise(scope, "fetch: worker script url is unavailable").into());
        return;
    };
    let input = match convert_fetch_arguments(scope, |scope| {
        resolve_worker_fetch_input(scope, &args, &document_url)
    }) {
        Ok(input) => input,
        Err(exception) => {
            rv.set(make_rejected_promise_with_value(scope, exception).into());
            return;
        }
    };
    if let Some(promise) = start_worker_resource_fetch(scope, input, None) {
        rv.set(promise.into());
    }
}

pub(crate) fn fetch_native_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let function = v8::Function::new(scope, worker_fetch_callback)?;
    let global = scope.get_current_context().global(scope);
    v8::Local::<v8::Promise>::try_from(crate::script_execution::call_function(
        scope,
        function,
        global.into(),
        &[request.into()],
    )?)
    .ok()
}

pub(super) fn start_worker_resource_fetch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: ResolvedWorkerFetchInput<'s>,
    font_face: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let kind = if font_face.is_some() {
        WorkerFetchKind::Font
    } else {
        WorkerFetchKind::Fetch
    };
    let Some(state) = get_worker_state(scope) else {
        return Some(make_rejected_promise(
            scope,
            "fetch: worker runtime state is unavailable",
        ));
    };

    let (
        web_security_disabled,
        request_origin,
        document_url,
        extra_http_headers,
        blocked_url_patterns,
        policy_context,
    ) = {
        let state = state.borrow();
        let Some(document_url) = state.current_script_url.clone() else {
            return Some(make_rejected_promise(
                scope,
                "fetch: worker script url is unavailable",
            ));
        };
        (
            state.loader.request_client().web_security_disabled(),
            state.request_origin(),
            document_url,
            state.extra_http_headers.clone(),
            state.blocked_url_patterns.clone(),
            state.policy_context,
        )
    };

    let ResolvedWorkerFetchInput {
        resolved_url,
        blob_url_entry,
        method,
        body,
        body_stream,
        headers: request_headers,
        request_mode,
        credentials_mode,
        redirect_mode,
        priority,
        metadata: mut request_metadata,
        signal,
    } = input;
    request_metadata.mixed_content_policy = state.borrow().mixed_content_policy();
    let blob_environment = BlobUrlFetchEnvironment::for_worker(scope);
    let blob_url_entry =
        blob_url_entry.map(|entry| entry.for_environment(blob_environment.as_ref()));
    let network_credentials_mode =
        policy_context.network_credentials_mode(request_mode, credentials_mode);
    // Use the same normalized header list as Request before policy checks and
    // context overrides can otherwise discard a repeated Range field.
    let request_headers = combined_headers_entries(&request_headers);
    let headers = merge_worker_request_headers(
        &extra_http_headers,
        &moli_fetch::RequestHeaders::from_byte_strings(&request_headers)
            .expect("validated worker Fetch headers are ByteStrings"),
    );
    if let Some(signal) = signal
        && worker_abort_signal_aborted(scope, signal)
    {
        let reason = worker_abort_signal_reason(scope, signal)
            .unwrap_or_else(|| worker_abort_error_value(scope));
        let rejected = make_rejected_promise_with_value(scope, reason);
        if let Some(stream) = body_stream {
            let stream = v8::Local::new(scope, stream);
            crate::context_bootstrap::cancel_readable_stream_for_fetch(scope, stream, reason);
        }
        return Some(rejected);
    }

    let request_origin = worker_global_origin(scope).unwrap_or(moli_url::WebOrigin::Opaque);
    let timing = WorkerResourceTiming::new(
        scope,
        resolved_url.clone(),
        request_origin,
        WorkerTimingRequest::Fetch {
            method: method.clone(),
            mode: request_mode,
            credentials: network_credentials_mode,
            redirect: redirect_mode,
            policy: policy_context,
            font: kind == WorkerFetchKind::Font,
        },
    );

    if let Some(message) = check_worker_request_csp(
        scope,
        &state,
        &document_url,
        &resolved_url,
        kind.csp_resource_kind(),
        "fetch",
        crate::content_security_policy::ContentSecurityPolicyDisposition::Enforce,
    ) {
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            kind.resource_type(),
            message.clone(),
        );
        return Some(make_rejected_promise(scope, &message));
    }

    if let Err(error) = moli_url_policy::route_fetch_url(&resolved_url) {
        let message = error.to_string();
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            kind.resource_type(),
            message.clone(),
        );
        return Some(make_rejected_promise(scope, &message));
    }

    if let Err(message) = request_metadata.mixed_content_policy.check(&resolved_url) {
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            kind.resource_type(),
            message.clone(),
        );
        return Some(make_rejected_promise(scope, &message));
    }

    if should_request_be_blocked_due_to_bad_port(&resolved_url) {
        let message = format!("fetch: blocked bad port for `{resolved_url}`");
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            kind.resource_type(),
            message.clone(),
        );
        return Some(make_rejected_promise(scope, &message));
    }

    if !web_security_disabled
        && !blob_url_entry.as_ref().is_some_and(|entry| entry.is_authorized_fetch(&resolved_url))
        && let Err(error) = moli_fetch::FetchUrlList::new(&resolved_url, &[])
            .validate_request_mode(request_mode, &request_origin)
    {
        return Some(make_rejected_promise(scope, &error.to_string()));
    }

    if worker_url_blocked(&blocked_url_patterns, &resolved_url) {
        let message = BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned();
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            kind.resource_type(),
            message.clone(),
        );
        return Some(make_rejected_promise(scope, &message));
    }

    if let Err(error) = crate::network_host::validate_no_cors_http_redirect_mode(
        &request_origin,
        &resolved_url,
        request_mode,
        redirect_mode,
    ) {
        let message = error.to_string();
        record_worker_subresource_failure(
            &state.borrow(),
            document_url,
            resolved_url,
            method,
            headers,
            request_body_text(&body),
            kind.resource_type(),
            message.clone(),
        );
        return Some(make_rejected_promise(scope, &message));
    }

    if kind == WorkerFetchKind::Font
        && matches!(resolved_url.scheme(), "http" | "https")
        && !state
            .borrow()
            .loader
            .request_client()
            .optional_resource_fetch_enabled(SubresourceResourceType::Font)
    {
        return Some(make_rejected_promise(
            scope,
            "Font requests are disabled by the resource policy",
        ));
    }


    {
        let metadata =
            worker_fetch_script_metadata(state.borrow().referrer_policy.clone(), &request_metadata);
        let request = Request::new_browser(&method, resolved_url, body, headers, request_origin)
            .with_mixed_content_policy(request_metadata.mixed_content_policy)
            .with_initiator_url(&document_url)
            .with_request_mode(request_mode)
            .with_credentials_mode(credentials_mode)
            .with_redirect_mode(redirect_mode)
            .with_cache_mode(request_metadata.cache.parse().unwrap_or_default())
            .with_network_partition_key(state.borrow().network_partition_key.clone())
            .with_fetch_priority_hint(priority)
            .with_browser_request_metadata(kind.browser_metadata());
        let request = if kind == WorkerFetchKind::Font {
            request.with_resource_type(moli_fetch::RequestResourceType::Font)
        } else {
            request
        };
        let request = match request.with_fetch_referrer(&request_metadata.referrer) {
            Ok(mut request) => {
                if let Some(metadata) = metadata {
                    request = request.with_script_fetch_metadata(metadata);
                }
                request
            }
            Err(error) => {
                return Some(make_rejected_promise(scope, &error.to_string()));
            }
        };
        let signal_id = signal.and_then(|signal| worker_abort_signal_id(scope, signal));
        native_load::start(
            scope,
            &state,
            native_load::NativeWorkerFetchRequest { request, blob_url_entry, kind, font_face, timing },


            document_url,
            signal_id,
            request_metadata,
            policy_context,
        )

    }



}

pub(in crate::worker) fn reject_worker_fetches_for_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal_id: u32,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let rejected = {
        let mut state = state.borrow_mut();
        let fetch_ids = state
            .pending_fetches
            .iter()
            .filter_map(|(fetch_id, pending)| {
                (pending.signal_id == Some(signal_id)).then_some(*fetch_id)
            })
            .collect::<Vec<_>>();
        let mut rejected = Vec::with_capacity(fetch_ids.len());
        for fetch_id in fetch_ids {
            if let Some(pending) = state.pending_fetches.remove(&fetch_id) {
                rejected.push((fetch_id, pending));
            }
        }
        rejected
    };
    for (_, pending) in rejected {
        pending.timing.abort(scope);
        pending.load.cancel();
        if let Some(body_source_id) = pending.streaming_body_source_id {

            error_pending_network_body_stream_with_reason(
                scope,
                body_source_id,
                "net::ERR_ABORTED".into(),
                reason,
            );
        }
        let resolver = v8::Local::new(scope, &pending.resolver);
        let _ = resolver.reject(scope, reason);
    }
}


pub(in crate::worker) fn drain_worker_fetch_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    event: WorkerFetchEvent,
) {
    match event {
        WorkerFetchEvent::RedirectAdmission {
            fetch_id,
            admission,
        } => {
            if state
                .borrow()
                .pending_fetches
                .get(&fetch_id)
                .is_some_and(|pending| !pending.load.is_cancelled())
            {
                admission.grant();
            }
        }
        WorkerFetchEvent::ResourceTiming { fetch_id, timing } => {
            if !state.borrow().pending_fetches.contains_key(&fetch_id) {
                timing.publish(scope);
            }
        }
        WorkerFetchEvent::PolicyViolations(violations) => {
            for violation in violations {
                dispatch_worker_content_security_policy_violation_event_for_state(
                    scope, state, &violation,
                );
            }

        }
        WorkerFetchEvent::Completion(completion) => {
            drain_worker_fetch_completion_result(scope, state, *completion)
        }
        WorkerFetchEvent::StreamingStarted(started) => {
            start_worker_streaming_fetch(scope, state, started)
        }
        WorkerFetchEvent::StreamingChunk(chunk) => {
            let deliver = state
                .borrow()
                .pending_fetches
                .get(&chunk.fetch_id)
                .is_some_and(|pending| {
                    pending.streaming_body_source_id == Some(chunk.body_source_id)
                        && !pending.streaming_needs_orb_body_validation
                });
            if deliver {
                enqueue_pending_network_body_chunk(scope, chunk.body_source_id, chunk.bytes);
            }
        }
        WorkerFetchEvent::StreamingFinished(finished) => {
            finish_worker_streaming_fetch(scope, state, finished)
        }
    }
}

pub(in crate::worker) fn start_worker_streaming_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    started: WorkerFetchStreamingStarted,
) {
    let (resolver, method) = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_fetches.get_mut(&started.fetch_id) else {
            return;
        };
        pending.streaming_body_source_id = Some(started.body_source_id);
        (pending.resolver.clone(), pending.request_method.clone())
    };
    // Security checks, exposed headers and immutable metadata belong to the
    // native load. This task only constructs the Response in its Worker realm.
    let response = crate::network_host::build_fetch_response_object_from_stream_with_metadata(
        scope,
        &method,
        started.head,
        started.body_source_id,
        &started.metadata,
    );
    let resolver = v8::Local::new(scope, &resolver);
    let _ = resolver.resolve(scope, response.into());
}

pub(in crate::worker) fn finish_worker_streaming_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    finished: WorkerFetchStreamingFinished,
) {
    let Some(pending) = state
        .borrow_mut()
        .pending_fetches
        .remove(&finished.fetch_id)
    else {
        return;
    };
    pending.load.finish();
    match finished.result {
        Ok(entry) => {
            crate::context_bootstrap::record_resource_performance_entry(scope, entry);
            close_pending_network_body_stream(scope, finished.body_source_id);
        }
        Err(error_text) => {
            pending.timing.publish(scope);
            let reason = v8_string(scope, &error_text)
                .map(|message| v8::Exception::type_error(scope, message))
                .unwrap_or_else(|| v8::undefined(scope).into());
            error_pending_network_body_stream_with_reason(
                scope,
                finished.body_source_id,
                error_text,
                reason,
            );
        }
    }
}

pub(in crate::worker) fn drain_worker_fetch_completion_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerFetchCompletion,
) {


    let Some(pending) = state
        .borrow_mut()
        .pending_fetches
        .remove(&completion.fetch_id)
    else {
        return;
    };
    pending.load.finish();
    let resolver = v8::Local::new(scope, &pending.resolver);
    match completion.result {
        Ok(response) => {
            crate::context_bootstrap::record_resource_performance_entry(
                scope,
                response.resource_timing,
            );
            if let Some(face) = pending.font_face.as_ref() {
                let usable = (200..300).contains(&response.head.status)
                    && response.metadata.response_type.is_readable();
                let bytes = usable.then(|| response.body.as_materialized_bytes()).flatten();
                let face = v8::Local::new(scope, face);
                crate::context_bootstrap::finish_font_face_url_load(scope, face, bytes);
                // Font bytes stay native; author ArrayBuffer.then is never read.
                let _ = resolver.resolve(scope, v8::undefined(scope).into());
                return;
            }
            let response_obj =
                crate::network_host::build_fetch_response_object_from_body_source_with_metadata(
                    scope,
                    &pending.request_method,
                    response.head,
                    response.body,
                    &response.metadata,
                );
            let _ = resolver.resolve(scope, response_obj.into());
        }
        Err(message) => {
            pending.timing.reject(scope);

            if let Some(message) = v8_string(scope, &message) {
                let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
            } else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
            }
        }
    }
}
