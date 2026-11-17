//! WebSocket dictionaries and error values; sockets and transport stay with the host.
use crate::dom_exception::initialize_websocket_error;
use moli_v8_util::throw_type_error;
use moli_webidl as webidl;
use moli_websocket_types::{WebSocketCloseValidationError, close_info_code_from_number};

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebSocketCloseInfo")]
pub struct WebSocketCloseInfo {
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
