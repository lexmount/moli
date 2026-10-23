use super::*;
use crate::webidl;
use moli_websocket::{
    WebSocketCloseValidationError, WebSocketSubprotocolError, WebSocketUrlError,
    close_info_code_from_number, normalize_websocket_close_info,
    normalize_websocket_url as normalize_socket_url, validate_subprotocols,
};

pub(super) enum WebSocketProtocolParseError {
    Conversion(webidl::WebIdlError),
    Validation(String),
}

pub(super) fn normalize_websocket_url(
    base_url: &url::Url,
    input: &str,
) -> Result<url::Url, String> {
    normalize_socket_url(base_url, input).map_err(websocket_url_constructor_error)
}

pub(super) fn normalize_websocket_stream_url(
    base_url: &url::Url,
    input: &str,
) -> Result<url::Url, String> {
    normalize_socket_url(base_url, input).map_err(websocket_stream_url_constructor_error)
}

pub(super) fn parse_websocket_protocols<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Result<Vec<String>, WebSocketProtocolParseError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_undefined() {
        return Ok(Vec::new());
    }
    let protocols = websocket_dom_string_or_sequence(scope, value, "WebSocket", "protocols")
        .map_err(WebSocketProtocolParseError::Conversion)?;
    validate_subprotocols(&protocols)
        .map_err(websocket_subprotocol_constructor_error)
        .map_err(WebSocketProtocolParseError::Validation)?;
    Ok(protocols)
}

pub(super) fn validate_websocket_stream_protocols(protocols: &[String]) -> Result<(), String> {
    validate_subprotocols(protocols).map_err(websocket_stream_subprotocol_constructor_error)
}

fn websocket_dom_string_or_sequence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    prefix: &'static str,
    member: &'static str,
) -> Result<Vec<String>, webidl::WebIdlError> {
    if value.is_object() && !value.is_string() && websocket_value_has_iterator(scope, value)? {
        return webidl::convert::<webidl::Sequence<webidl::DomString>>(
            scope,
            value,
            webidl::Context::member(prefix, member),
        )
        .map(|sequence| {
            sequence
                .0
                .into_iter()
                .map(|value| value.0)
                .collect::<Vec<_>>()
        });
    }
    webidl::convert::<webidl::DomString>(scope, value, webidl::Context::member(prefix, member))
        .map(|value| vec![value.0])
}

fn websocket_value_has_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<bool, webidl::WebIdlError> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(false);
    };
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let iterator_key = v8::Symbol::get_iterator(&scope);
    let iterator = object.get(&scope, iterator_key.into());
    match iterator {
        Some(iterator) => Ok(!iterator.is_null_or_undefined()),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(webidl::WebIdlError::pending_exception(
                webidl::Context::member("WebSocket", "protocols"),
            ))
        }
        None => Ok(false),
    }
}

fn websocket_url_constructor_error(error: WebSocketUrlError) -> String {
    format!("Failed to construct 'WebSocket': {error}.")
}

fn websocket_stream_url_constructor_error(error: WebSocketUrlError) -> String {
    format!("Failed to construct 'WebSocketStream': {error}.")
}

fn websocket_subprotocol_constructor_error(error: WebSocketSubprotocolError) -> String {
    format!("Failed to construct 'WebSocket': {error}.")
}

fn websocket_stream_subprotocol_constructor_error(error: WebSocketSubprotocolError) -> String {
    format!("Failed to construct 'WebSocketStream': {error}.")
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebSocketCloseInfo")]
pub(super) struct WebSocketCloseInfo {
    // Finish all dictionary conversions before applying close-code semantics.
    // The protocol helper preserves Blink's observed truncating clamp behavior.
    #[webidl(converter = "unrestricted_double")]
    close_code: Option<f64>,
    #[webidl(default = "", converter = "usv_string")]
    reason: String,
}

impl WebSocketCloseInfo {
    pub(super) fn normalize(
        self,
    ) -> Result<moli_websocket::WebSocketCloseRequest, WebSocketCloseValidationError> {
        let code = self
            .close_code
            .map(close_info_code_from_number)
            .transpose()?;
        moli_websocket::normalize_websocket_close_info(code, self.reason)
    }
}

pub(super) fn throw_websocket_close_error(
    scope: &mut v8::PinScope<'_, '_>,
    operation: &'static str,
    error: WebSocketCloseValidationError,
) {
    let detail = match error {
        WebSocketCloseValidationError::InvalidCode => "invalid close code",
        WebSocketCloseValidationError::ReasonTooLong => "close reason is too long",
    };
    throw_websocket_dom_exception(
        scope,
        websocket_close_error_name(error),
        websocket_close_error_code(error),
        &format!("Failed to {operation}: {detail}."),
    );
}

fn websocket_close_error_name(error: WebSocketCloseValidationError) -> &'static str {
    match error {
        WebSocketCloseValidationError::InvalidCode => "InvalidAccessError",
        WebSocketCloseValidationError::ReasonTooLong => "SyntaxError",
    }
}

fn websocket_close_error_code(error: WebSocketCloseValidationError) -> i32 {
    match error {
        WebSocketCloseValidationError::InvalidCode => 15,
        WebSocketCloseValidationError::ReasonTooLong => 12,
    }
}

pub(super) fn websocket_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    websocket_number_slot(scope, socket, WEBSOCKET_ID_SLOT)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| value as u64)
}

pub(super) fn set_websocket_id(
    scope: &mut v8::PinScope<'_, '_>,
    socket: v8::Local<'_, v8::Object>,
    socket_id: u64,
) {
    set_websocket_number_slot(scope, socket, WEBSOCKET_ID_SLOT, socket_id as f64);
}

pub(super) fn add_buffered_amount<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    delta: f64,
) {
    let current = websocket_buffered_amount(scope, socket);
    set_websocket_buffered_amount(scope, socket, (current + delta).max(0.0));
}

pub(super) fn websocket_ready_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
) -> f64 {
    websocket_number_slot(scope, socket, WEBSOCKET_READY_STATE_SLOT).unwrap_or(CLOSED)
}

pub(super) fn set_websocket_ready_state(
    scope: &mut v8::PinScope<'_, '_>,
    socket: v8::Local<'_, v8::Object>,
    value: f64,
) {
    set_websocket_number_slot(scope, socket, WEBSOCKET_READY_STATE_SLOT, value);
}

pub(super) fn websocket_buffered_amount<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
) -> f64 {
    websocket_number_slot(scope, socket, WEBSOCKET_BUFFERED_AMOUNT_SLOT).unwrap_or(0.0)
}

pub(super) fn set_websocket_buffered_amount(
    scope: &mut v8::PinScope<'_, '_>,
    socket: v8::Local<'_, v8::Object>,
    value: f64,
) {
    set_websocket_number_slot(
        scope,
        socket,
        WEBSOCKET_BUFFERED_AMOUNT_SLOT,
        value.max(0.0),
    );
}

pub(super) fn websocket_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
) -> Option<String> {
    websocket_value_slot(scope, socket, slot_name)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn websocket_value_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, socket, slot_name)
}

pub(super) fn websocket_object_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    websocket_value_slot(scope, socket, slot_name)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn set_websocket_string_slot(
    scope: &mut v8::PinScope<'_, '_>,
    socket: v8::Local<'_, v8::Object>,
    slot_name: &'static str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_websocket_value_slot(scope, socket, slot_name, value.into());
    }
}

pub(super) fn set_websocket_value_slot(
    scope: &mut v8::PinScope<'_, '_>,
    socket: v8::Local<'_, v8::Object>,
    slot_name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, socket, slot_name, value);
}

fn websocket_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
) -> Option<f64> {
    get_private_value(scope, socket, slot_name).and_then(|value| value.number_value(scope))
}

fn set_websocket_number_slot(
    scope: &mut v8::PinScope<'_, '_>,
    socket: v8::Local<'_, v8::Object>,
    slot_name: &'static str,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_websocket_value_slot(scope, socket, slot_name, value.into());
}

pub(super) fn set_websocket_string_slot_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
    default: &str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let value =
        websocket_string_slot(scope, socket, slot_name).unwrap_or_else(|| default.to_owned());
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(super) fn set_websocket_value_slot_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    socket: v8::Local<'s, v8::Object>,
    slot_name: &'static str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let value = websocket_value_slot(scope, socket, slot_name)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

pub(super) fn throw_websocket_dom_exception(
    scope: &mut v8::PinScope<'_, '_>,
    name: &'static str,
    code: i32,
    message: &str,
) {
    let exception = websocket_dom_exception_value(scope, name, code, message);
    scope.throw_exception(exception);
}

pub(super) fn websocket_dom_exception_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &'static str,
    _code: i32,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    crate::context_bootstrap::new_dom_exception_value(scope, message, name)
}
