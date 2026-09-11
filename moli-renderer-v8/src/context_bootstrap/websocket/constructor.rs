use super::helpers::{
    WebSocketProtocolParseError, normalize_websocket_stream_url, normalize_websocket_url,
    parse_websocket_protocols, parse_websocket_stream_protocols, throw_websocket_close_error,
    throw_websocket_dom_exception, websocket_close_info_code, websocket_close_info_reason,
    websocket_id, websocket_object_slot,
};
use super::realm::{effective_websocket_document_scope, websocket_constructor_relevant_context};
use super::stream::{new_websocket_stream_promise, reject_websocket_stream_abort};
use super::*;
use crate::context_bootstrap::constructors::initialize_websocket_error;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;
use moli_websocket::{normalize_websocket_close_info, websocket_url_is_potentially_trustworthy};

const WEBSOCKET_STREAM_ABORT_LISTENER_STREAM_SLOT: &str = "__moliWebSocketStreamAbortStream";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebSocket)]
struct WebSocketObjectDeclaration {
    #[webapi(slot = WEBSOCKET_URL_SLOT)]
    url: String,

    #[webapi(slot = WEBSOCKET_READY_STATE_SLOT)]
    ready_state: f64,

    #[webapi(slot = WEBSOCKET_BUFFERED_AMOUNT_SLOT, init = 0)]
    buffered_amount: (),

    #[webapi(slot = WEBSOCKET_EXTENSIONS_SLOT, init = "")]
    extensions: (),

    #[webapi(slot = WEBSOCKET_PROTOCOL_SLOT, init = "")]
    protocol: (),

    #[webapi(slot = WEBSOCKET_BINARY_TYPE_SLOT, init = string("blob"))]
    binary_type: (),

    #[webapi(slot = WEBSOCKET_ONOPEN_SLOT, init = "null")]
    onopen: (),

    #[webapi(slot = WEBSOCKET_ONMESSAGE_SLOT, init = "null")]
    onmessage: (),

    #[webapi(slot = WEBSOCKET_ONERROR_SLOT, init = "null")]
    onerror: (),

    #[webapi(slot = WEBSOCKET_ONCLOSE_SLOT, init = "null")]
    onclose: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = WEBSOCKET_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(method, enumerable, callback = simple_event_target_add_event_listener_callback)]
    add_event_listener: (),

    #[webapi(
        method,
        enumerable,
        callback = simple_event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),

    #[webapi(method, enumerable, callback = simple_event_target_dispatch_event_callback)]
    dispatch_event: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebSocketStream)]
struct WebSocketStreamObjectDeclaration<'scope> {
    #[webapi(slot = WEBSOCKET_STREAM_URL_SLOT)]
    url: v8::Local<'scope, v8::String>,
    #[webapi(slot = WEBSOCKET_STREAM_OPENED_SLOT)]
    opened: v8::Local<'scope, v8::Promise>,
    #[webapi(slot = WEBSOCKET_STREAM_OPENED_RESOLVE_SLOT)]
    opened_resolve: v8::Local<'scope, v8::Function>,
    #[webapi(slot = WEBSOCKET_STREAM_OPENED_REJECT_SLOT)]
    opened_reject: v8::Local<'scope, v8::Function>,
    #[webapi(slot = WEBSOCKET_STREAM_CLOSED_SLOT)]
    closed: v8::Local<'scope, v8::Promise>,
    #[webapi(slot = WEBSOCKET_STREAM_CLOSED_RESOLVE_SLOT)]
    closed_resolve: v8::Local<'scope, v8::Function>,
    #[webapi(slot = WEBSOCKET_STREAM_CLOSED_REJECT_SLOT)]
    closed_reject: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebSocketStream)]
struct WebSocketStreamRegisteredSocketDeclaration {
    #[webapi(slot = WEBSOCKET_ID_SLOT)]
    socket_id: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WebSocketStreamAbortListenerDataDeclaration<'scope> {
    #[webapi(slot = WEBSOCKET_STREAM_ABORT_LISTENER_STREAM_SLOT)]
    stream: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WebSocketStreamAbortListenerOptionsDeclaration {
    #[webapi(data_property, enumerable)]
    once: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebSocket")]
struct WebSocketConstructorArgs<'s> {
    #[webidl(
        required,
        name = "url",
        converter = "usv_string",
        missing_message = "Failed to construct 'WebSocket': 1 argument required, but only 0 present."
    )]
    url: String,
    #[webidl(index = 1, converter = "raw")]
    protocols: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebSocketStream")]
struct WebSocketStreamConstructorArgs<'s> {
    #[webidl(
        required,
        name = "url",
        converter = "usv_string",
        missing_message = "Failed to construct 'WebSocketStream': 1 argument required, but only 0 present."
    )]
    url: String,
    #[webidl(index = 1, converter = "raw")]
    options: Option<v8::Local<'s, v8::Value>>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "WebSocketStreamOptions")]
struct WebSocketStreamOptionsMembers<'s> {
    #[webidl(legacy_nullish, converter = "raw")]
    protocols: Option<v8::Local<'s, v8::Value>>,
    #[webidl(legacy_nullish, converter = "raw")]
    signal: Option<v8::Local<'s, v8::Object>>,
}

pub(in crate::context_bootstrap) fn websocket_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.is_construct_call() {
        let Some(relevant_context) =
            websocket_constructor_relevant_context(scope, args.new_target(), "WebSocket")
        else {
            return;
        };
        let scope = &mut v8::ContextScope::new(scope, relevant_context);
        websocket_constructor_callback_inner(scope, args, rv);
        return;
    }
    websocket_constructor_callback_inner(scope, args, rv);
}

pub(in crate::context_bootstrap) fn websocket_stream_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.is_construct_call() {
        let Some(relevant_context) =
            websocket_constructor_relevant_context(scope, args.new_target(), "WebSocketStream")
        else {
            return;
        };
        let scope = &mut v8::ContextScope::new(scope, relevant_context);
        websocket_stream_constructor_callback_inner(scope, args, rv);
        return;
    }
    websocket_stream_constructor_callback_inner(scope, args, rv);
}

pub(in crate::context_bootstrap) fn websocket_error_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'WebSocketError': Please use the 'new' operator.",
        );
        return;
    }
    let message = if args.length() > 0 && !args.get(0).is_undefined() {
        args.get(0)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let init = if args.length() > 1 && !args.get(1).is_null_or_undefined() {
        match v8::Local::<v8::Object>::try_from(args.get(1)) {
            Ok(init) => Some(init),
            Err(_) => {
                throw_type_error(
                    scope,
                    "Failed to construct 'WebSocketError': the provided init value is not an object.",
                );
                return;
            }
        }
    } else {
        None
    };
    let close_code = match websocket_close_info_code(scope, init) {
        Some(Ok(close_code)) => Some(close_code),
        Some(Err(error)) => {
            throw_websocket_close_error(scope, "construct 'WebSocketError'", error);
            return;
        }
        None => None,
    };
    let reason = websocket_close_info_reason(scope, init);
    let close = match normalize_websocket_close_info(close_code, reason) {
        Ok(close) => close,
        Err(error) => {
            throw_websocket_close_error(scope, "construct 'WebSocketError'", error);
            return;
        }
    };

    initialize_websocket_error(scope, args.this(), &message, close.code, &close.reason);
    rv.set(args.this().into());
}

fn websocket_stream_abort_signal_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(entry) = v8::Local::<v8::Object>::try_from(args.data()).ok() else {
        rv.set_undefined();
        return;
    };
    let Some(stream) =
        websocket_object_slot(scope, entry, WEBSOCKET_STREAM_ABORT_LISTENER_STREAM_SLOT)
    else {
        rv.set_undefined();
        return;
    };
    if websocket_object_slot(scope, stream, WEBSOCKET_STREAM_READABLE_SLOT).is_some() {
        rv.set_undefined();
        return;
    }
    if let Some(socket_id) = websocket_id(scope, stream)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let host = unsafe { &mut *host_ptr };
        let _ = host.close_websocket(socket_id, None, String::new());
    }
    reject_websocket_stream_abort(scope, stream);
    rv.set_undefined();
}

fn websocket_constructor_callback_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'WebSocket': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<WebSocketConstructorArgs<'s>>(scope, &args) else {
        return;
    };
    let protocols = match parse_websocket_protocols(scope, parsed.protocols) {
        Ok(protocols) => protocols,
        Err(WebSocketProtocolParseError::Validation(message)) => {
            throw_websocket_dom_exception(scope, "SyntaxError", 12, &message);
            return;
        }
        Err(WebSocketProtocolParseError::Conversion(error)) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        let Some(document_url) = crate::worker::worker_current_script_url(scope) else {
            throw_type_error(
                scope,
                "Failed to construct 'WebSocket': runtime is unavailable.",
            );
            return;
        };
        let url = match normalize_websocket_url(&document_url, &parsed.url) {
            Ok(url) => url,
            Err(message) => {
                throw_websocket_dom_exception(scope, "SyntaxError", 12, &message);
                return;
            }
        };
        let Some(csp_outcome) =
            crate::worker::check_worker_websocket_csp(scope, &document_url, &url)
        else {
            throw_type_error(
                scope,
                "Failed to construct 'WebSocket': runtime is unavailable.",
            );
            return;
        };
        if !csp_outcome.blocks_request() && websocket_mixed_content_is_blocked(&document_url, &url)
        {
            throw_websocket_dom_exception(
                scope,
                "SecurityError",
                18,
                "Failed to construct 'WebSocket': An insecure WebSocket connection may not be initiated from a secure origin.",
            );
            return;
        }
        let socket = args.this();
        initialize_websocket_object(scope, socket, &url);
        let Some(socket_id) = crate::worker::register_worker_websocket(
            scope,
            socket,
            document_url,
            url,
            protocols,
            csp_outcome,
        ) else {
            throw_type_error(
                scope,
                "Failed to construct 'WebSocket': runtime is unavailable.",
            );
            return;
        };
        super::helpers::set_websocket_id(scope, socket, socket_id);
        rv.set(socket.into());
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some((owner, frame_id, document_url)) = effective_websocket_document_scope(scope, host)
    else {
        throw_type_error(
            scope,
            "Failed to construct 'WebSocket': the owning document is no longer active.",
        );
        return;
    };
    let url = match normalize_websocket_url(&document_url, &parsed.url) {
        Ok(url) => url,
        Err(message) => {
            throw_websocket_dom_exception(scope, "SyntaxError", 12, &message);
            return;
        }
    };
    let csp_outcome = host.check_document_connect_csp_for_owner(
        scope,
        owner.dispatch_scope(),
        &document_url,
        &url,
    );
    if !csp_outcome.blocks_request() && websocket_mixed_content_is_blocked(&document_url, &url) {
        throw_websocket_dom_exception(
            scope,
            "SecurityError",
            18,
            "Failed to construct 'WebSocket': An insecure WebSocket connection may not be initiated from a secure origin.",
        );
        return;
    }
    let socket = args.this();
    initialize_websocket_object(scope, socket, &url);
    let socket_id = host.register_websocket_for_document(
        scope,
        socket,
        owner,
        frame_id,
        document_url,
        url,
        protocols,
        csp_outcome,
    );
    super::helpers::set_websocket_id(scope, socket, socket_id);

    rv.set(socket.into());
}

fn websocket_mixed_content_is_blocked(document_url: &url::Url, websocket_url: &url::Url) -> bool {
    moli_url::is_potentially_trustworthy_url(document_url)
        && websocket_url.scheme() == "ws"
        && !websocket_url_is_potentially_trustworthy(websocket_url)
}

fn initialize_websocket_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    url: &url::Url,
) {
    WebSocketObjectDeclaration::new(url.as_str().to_owned(), CONNECTING)
        .initialize(scope, socket)
        .expect("WebSocket declaration should initialize");
}

fn websocket_stream_constructor_callback_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'WebSocketStream': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<WebSocketStreamConstructorArgs<'s>>(scope, &args)
    else {
        return;
    };
    let Some(options) = websocket_stream_options(scope, parsed.options) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_type_error(
            scope,
            "Failed to construct 'WebSocketStream': runtime is unavailable.",
        );
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some((owner, frame_id, document_url)) = effective_websocket_document_scope(scope, host)
    else {
        throw_type_error(
            scope,
            "Failed to construct 'WebSocketStream': the owning document is no longer active.",
        );
        return;
    };
    let url = match normalize_websocket_stream_url(&document_url, &parsed.url) {
        Ok(url) => url,
        Err(message) => {
            throw_websocket_dom_exception(scope, "SyntaxError", 12, &message);
            return;
        }
    };
    let csp_outcome = host.check_document_connect_csp_for_owner(
        scope,
        owner.dispatch_scope(),
        &document_url,
        &url,
    );
    if !csp_outcome.blocks_request() && websocket_mixed_content_is_blocked(&document_url, &url) {
        throw_websocket_dom_exception(
            scope,
            "SecurityError",
            18,
            "Failed to construct 'WebSocketStream': An insecure WebSocket connection may not be initiated from a secure origin.",
        );
        return;
    }
    let protocols = match parse_websocket_stream_protocols(scope, options.protocols) {
        Ok(protocols) => protocols,
        Err(WebSocketProtocolParseError::Validation(message)) => {
            throw_websocket_dom_exception(scope, "SyntaxError", 12, &message);
            return;
        }
        Err(WebSocketProtocolParseError::Conversion(error)) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let signal = options.signal;
    let Some((opened_promise, opened_resolve, opened_reject)) = new_websocket_stream_promise(scope)
    else {
        rv.set_undefined();
        return;
    };
    let Some((closed_promise, closed_resolve, closed_reject)) = new_websocket_stream_promise(scope)
    else {
        rv.set_undefined();
        return;
    };
    let stream = args.this();
    WebSocketStreamObjectDeclaration::new(
        v8_string(scope, url.as_str()).unwrap_or_else(|| v8::String::empty(scope)),
        opened_promise,
        opened_resolve,
        opened_reject,
        closed_promise,
        closed_resolve,
        closed_reject,
    )
    .initialize(scope, stream)
    .expect("WebSocketStream object declaration should initialize");
    if signal.is_some_and(|signal| host.abort_signal_aborted(scope, signal)) {
        reject_websocket_stream_abort(scope, stream);
        rv.set(stream.into());
        return;
    }
    let socket_id = host.register_websocket_for_document(
        scope,
        stream,
        owner,
        frame_id,
        document_url,
        url,
        protocols,
        csp_outcome,
    );
    WebSocketStreamRegisteredSocketDeclaration::new(socket_id as f64)
        .initialize(scope, stream)
        .expect("WebSocketStream registered socket declaration should initialize");
    if let Some(signal) = signal {
        install_websocket_stream_abort_listener(scope, stream, signal);
    }
    rv.set(stream.into());
}

fn websocket_stream_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Option<WebSocketStreamOptionsMembers<'s>> {
    let Some(value) = value else {
        return Some(WebSocketStreamOptionsMembers::default());
    };
    match webidl::parse_dictionary::<WebSocketStreamOptionsMembers<'s>>(
        scope,
        value,
        webidl::Context::argument("WebSocketStream", 2),
    ) {
        Ok(options) => Some(options.unwrap_or_default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn install_websocket_stream_abort_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    signal: v8::Local<'s, v8::Object>,
) {
    let entry = WebSocketStreamAbortListenerDataDeclaration::new(stream)
        .bind(scope)
        .expect("WebSocketStream abort listener data declaration should bind");
    let Some(listener) = v8::Function::builder(websocket_stream_abort_signal_callback)
        .data(entry.into())
        .length(1)
        .build(scope)
    else {
        return;
    };
    let Some(add_event_listener) = signal
        .get(scope, v8str(scope, "addEventListener").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let options = WebSocketStreamAbortListenerOptionsDeclaration::new(true)
        .bind(scope)
        .expect("WebSocketStream abort listener options declaration should bind");
    let _ = add_event_listener.call(
        scope,
        signal.into(),
        &[
            v8str(scope, "abort").into(),
            listener.into(),
            options.into(),
        ],
    );
}
