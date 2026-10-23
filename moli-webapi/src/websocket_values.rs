//! WebSocket dictionaries and error values; sockets and transport stay with the host.
use crate::dom_exception::initialize_websocket_error;
use crate::events::fallback::PlainEventDeclaration;
use moli_v8_util::{throw_type_error, v8_string, v8str};
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;
use moli_websocket_types::{WebSocketCloseValidationError, close_info_code_from_number};

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebSocketCloseInfo")]
pub struct WebSocketCloseInfo {
    // Finish all dictionary conversions before applying close-code semantics.
    // The protocol helper preserves Blink's observed truncating clamp behavior.
    #[webidl(converter = "unrestricted_double")]
    close_code: Option<f64>,
    #[webidl(default = "", converter = "usv_string")]
    reason: String,
}

impl WebSocketCloseInfo {
    pub fn normalize(
        self,
    ) -> Result<moli_websocket_types::WebSocketCloseRequest, WebSocketCloseValidationError> {
        let code = self
            .close_code
            .map(close_info_code_from_number)
            .transpose()?;
        moli_websocket_types::normalize_websocket_close_info(code, self.reason)
    }
}

pub fn throw_websocket_close_error(
    scope: &mut v8::PinScope<'_, '_>,
    operation: &'static str,
    error: WebSocketCloseValidationError,
) {
    let detail = match error {
        WebSocketCloseValidationError::InvalidCode => "invalid close code",
        WebSocketCloseValidationError::ReasonTooLong => "close reason is too long",
    };
    let name = match error {
        WebSocketCloseValidationError::InvalidCode => "InvalidAccessError",
        WebSocketCloseValidationError::ReasonTooLong => "SyntaxError",
    };
    let exception = crate::dom_exception::new_dom_exception_value(
        scope,
        &format!("Failed to {operation}: {detail}."),
        name,
    );
    scope.throw_exception(exception);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebSocketError")]
struct WebSocketErrorConstructorArgs {
    #[webidl(default = Vec::new(), converter = "dom_string16")]
    message: Vec<u16>,
    #[webidl(dictionary)]
    init: WebSocketCloseInfo,
}

pub fn websocket_error_constructor_callback<'s>(
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

#[derive(WebApiObject)]
#[webapi(plain)]
struct WebSocketStreamOpenInfoDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    readable: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    writable: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    protocol: Option<v8::Local<'scope, v8::String>>,
    #[webapi(data_property, enumerable)]
    extensions: Option<v8::Local<'scope, v8::String>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct WebSocketStreamCloseInfoDeclaration<'scope> {
    close_code: f64,
    reason: Option<v8::Local<'scope, v8::String>>,
}

pub fn new_websocket_stream_open_info<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    readable: v8::Local<'s, v8::Object>,
    writable: v8::Local<'s, v8::Object>,
    protocol: &str,
    extensions: &str,
) -> v8::Local<'s, v8::Object> {
    WebSocketStreamOpenInfoDeclaration::new(
        readable,
        writable,
        v8_string(scope, protocol),
        v8_string(scope, extensions),
    )
    .bind(scope)
    .expect("WebSocketStream open info declaration should bind")
}

pub fn new_websocket_stream_close_info<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    close_code: u16,
    reason: &str,
) -> v8::Local<'s, v8::Object> {
    WebSocketStreamCloseInfoDeclaration::new(close_code as f64, v8_string(scope, reason))
        .bind(scope)
        .expect("WebSocketStream close info declaration should bind")
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct WebSocketCloseEventInitDeclaration<'scope> {
    code: u16,
    reason: v8::Local<'scope, v8::String>,
    was_clean: bool,
}

pub fn new_websocket_simple_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(ctor) = global
        .get(scope, v8str(scope, "Event").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        && let Some(event_type) = v8_string(scope, event_type)
        && let Some(event) = ctor.new_instance(scope, &[event_type.into()])
    {
        return event;
    }
    let event_type = v8_string(scope, event_type).expect("WebSocket event type should allocate");
    PlainEventDeclaration::new(event_type)
        .bind(scope)
        .expect("WebSocket simple event fallback declaration should bind")
}

pub fn new_websocket_close_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: u16,
    reason: &str,
    was_clean: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let reason = v8_string(scope, reason)?;
    if let Some(ctor) = global
        .get(scope, v8str(scope, "CloseEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let event_type = v8_string(scope, "close")?;
        let init = WebSocketCloseEventInitDeclaration::new(code, reason, was_clean)
            .bind(scope)
            .ok()?;
        return ctor.new_instance(scope, &[event_type.into(), init.into()]);
    }
    let event = new_websocket_simple_event(scope, "close");
    WebSocketCloseEventInitDeclaration::new(code, reason, was_clean)
        .initialize(scope, event)
        .ok()?;
    Some(event)
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebSocketStream")]
pub struct WebSocketStreamConstructorArgs<'s> {
    #[webidl(
        required,
        name = "url",
        converter = "usv_string",
        missing_message = "Failed to construct 'WebSocketStream': 1 argument required, but only 0 present."
    )]
    pub url: String,
    #[webidl(index = 1, dictionary)]
    pub options: WebSocketStreamOptionsMembers<'s>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "WebSocketStreamOptions")]
pub struct WebSocketStreamOptionsMembers<'s> {
    #[webidl(sequence, converter = "usv_string", default = Vec::new())]
    pub protocols: Vec<String>,
    #[webidl(interface = crate::interfaces::AbortSignal)]
    pub signal: Option<v8::Local<'s, v8::Object>>,
}
