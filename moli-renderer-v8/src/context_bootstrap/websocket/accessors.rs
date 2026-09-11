use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiFunctionTemplate;
use std::str::FromStr;

#[derive(Clone, Copy)]
struct WebSocketEventHandler {
    slot_name: &'static str,
    event_type: &'static str,
}

const WEBSOCKET_EVENT_HANDLERS: &[WebSocketEventHandler] = &[
    WebSocketEventHandler {
        slot_name: WEBSOCKET_ONOPEN_SLOT,
        event_type: "open",
    },
    WebSocketEventHandler {
        slot_name: WEBSOCKET_ONMESSAGE_SLOT,
        event_type: "message",
    },
    WebSocketEventHandler {
        slot_name: WEBSOCKET_ONERROR_SLOT,
        event_type: "error",
    },
    WebSocketEventHandler {
        slot_name: WEBSOCKET_ONCLOSE_SLOT,
        event_type: "close",
    },
];

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebSocket)]
struct WebSocketEventHandlerAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = websocket_event_handler_getter_function,
        setter = websocket_event_handler_setter_function,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    onopen: (),

    #[webapi(
        accessor_property,
        getter = websocket_event_handler_getter_function,
        setter = websocket_event_handler_setter_function,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    onmessage: (),

    #[webapi(
        accessor_property,
        getter = websocket_event_handler_getter_function,
        setter = websocket_event_handler_setter_function,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    onerror: (),

    #[webapi(
        accessor_property,
        getter = websocket_event_handler_getter_function,
        setter = websocket_event_handler_setter_function,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    onclose: (),
}

#[derive(Clone, Copy, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum WebSocketBinaryType {
    Blob,
    ArrayBuffer,
}

pub(super) fn websocket_url_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::helpers::set_websocket_string_slot_return_value(
        scope,
        args.this(),
        WEBSOCKET_URL_SLOT,
        "",
        &mut rv,
    );
}

pub(super) fn websocket_ready_state_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let ready_state = super::helpers::websocket_ready_state(scope, args.this());
    rv.set(v8::Number::new(scope, ready_state).into());
}

pub(super) fn websocket_buffered_amount_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let buffered_amount = super::helpers::websocket_buffered_amount(scope, args.this());
    rv.set(v8::Number::new(scope, buffered_amount).into());
}

pub(super) fn websocket_extensions_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::helpers::set_websocket_string_slot_return_value(
        scope,
        args.this(),
        WEBSOCKET_EXTENSIONS_SLOT,
        "",
        &mut rv,
    );
}

pub(super) fn websocket_protocol_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::helpers::set_websocket_string_slot_return_value(
        scope,
        args.this(),
        WEBSOCKET_PROTOCOL_SLOT,
        "",
        &mut rv,
    );
}

pub(super) fn websocket_binary_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::helpers::set_websocket_string_slot_return_value(
        scope,
        args.this(),
        WEBSOCKET_BINARY_TYPE_SLOT,
        "blob",
        &mut rv,
    );
}

pub(super) fn websocket_binary_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = match webidl::convert::<webidl::DomString>(
        scope,
        args.get(0),
        webidl::Context::member("WebSocket", "binaryType"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Ok(binary_type) = WebSocketBinaryType::from_str(&value) else {
        super::helpers::throw_websocket_dom_exception(
            scope,
            "SyntaxError",
            12,
            "Failed to set 'binaryType' on 'WebSocket': The provided value is not a valid enum value of type BinaryType.",
        );
        return;
    };

    super::helpers::set_websocket_string_slot(
        scope,
        args.this(),
        WEBSOCKET_BINARY_TYPE_SLOT,
        binary_type.into(),
    );
}

pub(super) fn websocket_stream_url_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::helpers::set_websocket_string_slot_return_value(
        scope,
        args.this(),
        WEBSOCKET_STREAM_URL_SLOT,
        "",
        &mut rv,
    );
}

pub(super) fn websocket_stream_opened_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::helpers::set_websocket_value_slot_return_value(
        scope,
        args.this(),
        WEBSOCKET_STREAM_OPENED_SLOT,
        &mut rv,
    );
}

pub(super) fn websocket_stream_closed_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::helpers::set_websocket_value_slot_return_value(
        scope,
        args.this(),
        WEBSOCKET_STREAM_CLOSED_SLOT,
        &mut rv,
    );
}

pub(super) fn install_websocket_event_handler_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    // The declared callback data indexes must stay aligned with
    // WEBSOCKET_EVENT_HANDLERS because the shared getter/setter resolves handler
    // metadata from args.data().
    WebSocketEventHandlerAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

fn websocket_event_handler_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler) = callback_data_item(
        scope,
        &args,
        WEBSOCKET_EVENT_HANDLERS,
        "WebSocket event handlers",
    ) else {
        rv.set_null();
        return;
    };
    let target = args.this();
    let value = get_private_value(scope, target, handler.slot_name)
        .or_else(|| target.get(scope, v8str(scope, handler.slot_name).into()))
        .unwrap_or_else(|| v8::null(scope).into());
    if value.is_null_or_undefined() {
        rv.set_null();
    } else {
        rv.set(value);
    }
}

fn websocket_event_handler_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(handler) = callback_data_item(
        scope,
        &args,
        WEBSOCKET_EVENT_HANDLERS,
        "WebSocket event handlers",
    ) else {
        return;
    };
    let value = args.get(0);
    let stored = if value.is_function() || value.is_object() {
        value
    } else {
        v8::null(scope).into()
    };
    super::helpers::set_websocket_value_slot(scope, args.this(), handler.slot_name, stored);
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        WEBSOCKET_LISTENERS_SLOT,
        handler.event_type,
        handler.slot_name,
        stored.is_function(),
    );
}
