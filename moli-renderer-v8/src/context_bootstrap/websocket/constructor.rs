use super::abort::install_websocket_stream_abort;
use super::helpers::{
    WebSocketCloseInfo, WebSocketProtocolParseError, normalize_websocket_stream_url,
    normalize_websocket_url, parse_websocket_protocols, throw_websocket_close_error,
    throw_websocket_dom_exception, validate_websocket_stream_protocols,
};
use super::realm::{effective_websocket_document_scope, websocket_constructor_relevant_context};
use super::stream::reject_websocket_stream_abort;
use super::*;
use crate::abort_signal_route::ResolvedAbortSignal;
use crate::context_bootstrap::constructors::initialize_websocket_error;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;
use moli_websocket::websocket_url_is_potentially_trustworthy;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebSocketError")]
struct WebSocketErrorConstructorArgs {
    #[webidl(default = Vec::new(), converter = "dom_string16")]
    message: Vec<u16>,
    #[webidl(dictionary)]
    init: WebSocketCloseInfo,
}

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

    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebSocketStream)]
struct WebSocketStreamObjectDeclaration<'scope> {
    #[webapi(slot = WEBSOCKET_STREAM_CLOSE_REQUESTED_SLOT, init = false)]
    close_requested: (),
    #[webapi(slot = WEBSOCKET_STREAM_URL_SLOT)]
    url: v8::Local<'scope, v8::String>,
    #[webapi(slot = WEBSOCKET_STREAM_OPENED_SLOT)]
    opened: v8::Local<'scope, v8::Promise>,
    #[webapi(slot = WEBSOCKET_STREAM_OPENED_RESOLVER_SLOT)]
    opened_resolver: v8::Local<'scope, v8::PromiseResolver>,
    #[webapi(slot = WEBSOCKET_STREAM_CLOSED_SLOT)]
    closed: v8::Local<'scope, v8::Promise>,
    #[webapi(slot = WEBSOCKET_STREAM_CLOSED_RESOLVER_SLOT)]
    closed_resolver: v8::Local<'scope, v8::PromiseResolver>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::WebSocketStream)]
struct WebSocketStreamRegisteredSocketDeclaration {
    #[webapi(slot = WEBSOCKET_ID_SLOT)]
    socket_id: f64,
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
    #[webidl(index = 1, dictionary)]
    options: WebSocketStreamOptionsMembers<'s>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "WebSocketStreamOptions")]
struct WebSocketStreamOptionsMembers<'s> {
    #[webidl(sequence, converter = "usv_string", default = Vec::new())]
    protocols: Vec<String>,
    #[webidl(converter = "raw")]
    signal: Option<ResolvedAbortSignal<'s>>,
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
    let Some(parsed) = webidl::parse_args::<WebSocketErrorConstructorArgs>(scope, &args) else {
        return;
    };
    let close = match parsed.init.normalize() {
        Ok(close) => close,
        Err(error) => {
            throw_websocket_close_error(scope, "construct 'WebSocketError'", error);
            return;
        }
    };
    let Some(message) =
        v8::String::new_from_two_byte(scope, &parsed.message, v8::NewStringType::Normal)
    else {
        return;
    };
    initialize_websocket_error(scope, args.this(), message, close.code, &close.reason);
    rv.set(args.this().into());
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
    let options = parsed.options;
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        let stream = args.this();
        if construct_worker_websocket_stream(scope, stream, &parsed.url, options).is_some() {
            rv.set(stream.into());
        }
        return;
    };
    let Some((owner, frame_id, document_url)) =
        effective_websocket_document_scope(scope, unsafe { &mut *host_ptr })
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
    let csp_outcome = unsafe { &mut *host_ptr }.check_document_connect_csp_for_owner(
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
    if let Err(message) = validate_websocket_stream_protocols(&options.protocols) {
        throw_websocket_dom_exception(scope, "SyntaxError", 12, &message);
        return;
    }
    let protocols = options.protocols;
    let signal = options.signal;
    let stream = args.this();
    if initialize_websocket_stream_object(scope, stream, &url).is_none() {
        rv.set_undefined();
        return;
    }
    if signal.is_some_and(|signal| signal.is_aborted(scope)) {
        reject_websocket_stream_abort(scope, stream);
        rv.set(stream.into());
        return;
    }
    let socket_id = unsafe { &mut *host_ptr }.register_websocket_for_document(
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
        install_websocket_stream_abort(scope, socket_id, signal);
    }
    rv.set(stream.into());
}

fn initialize_websocket_stream_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    url: &url::Url,
) -> Option<()> {
    let opened = v8::PromiseResolver::new(scope)?;
    let closed = v8::PromiseResolver::new(scope)?;
    WebSocketStreamObjectDeclaration::new(
        v8_string(scope, url.as_str()).unwrap_or_else(|| v8::String::empty(scope)),
        opened.get_promise(scope),
        opened,
        closed.get_promise(scope),
        closed,
    )
    .initialize(scope, stream)
    .expect("WebSocketStream object declaration should initialize");
    Some(())
}

fn construct_worker_websocket_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    raw_url: &str,
    options: WebSocketStreamOptionsMembers<'s>,
) -> Option<()> {
    let Some(document_url) = crate::worker::worker_current_script_url(scope) else {
        throw_type_error(
            scope,
            "Failed to construct 'WebSocketStream': runtime is unavailable.",
        );
        return None;
    };
    let url = match normalize_websocket_stream_url(&document_url, raw_url) {
        Ok(url) => url,
        Err(message) => {
            throw_websocket_dom_exception(scope, "SyntaxError", 12, &message);
            return None;
        }
    };
    let csp_outcome = crate::worker::check_worker_websocket_csp(scope, &document_url, &url)?;
    if !csp_outcome.blocks_request() && websocket_mixed_content_is_blocked(&document_url, &url) {
        throw_websocket_dom_exception(
            scope,
            "SecurityError",
            18,
            "Failed to construct 'WebSocketStream': An insecure WebSocket connection may not be initiated from a secure origin.",
        );
        return None;
    }
    if let Err(message) = validate_websocket_stream_protocols(&options.protocols) {
        throw_websocket_dom_exception(scope, "SyntaxError", 12, &message);
        return None;
    }
    initialize_websocket_stream_object(scope, stream, &url)?;
    if options
        .signal
        .is_some_and(|signal| signal.is_aborted(scope))
    {
        reject_websocket_stream_abort(scope, stream);
        return Some(());
    }
    let Some(socket_id) = crate::worker::register_worker_websocket(
        scope,
        stream,
        document_url,
        url,
        options.protocols,
        csp_outcome,
    ) else {
        throw_type_error(
            scope,
            "Failed to construct 'WebSocketStream': runtime is unavailable.",
        );
        return None;
    };
    WebSocketStreamRegisteredSocketDeclaration::new(socket_id as f64)
        .initialize(scope, stream)
        .expect("WebSocketStream registered socket declaration should initialize");
    if let Some(signal) = options.signal {
        install_websocket_stream_abort(scope, socket_id, signal);
    }
    Some(())
}
