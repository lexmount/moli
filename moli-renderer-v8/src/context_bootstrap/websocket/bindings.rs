use super::accessors::{
    install_websocket_event_handler_accessors, websocket_binary_type_getter_function,
    websocket_binary_type_setter_function, websocket_buffered_amount_getter_function,
    websocket_extensions_getter_function, websocket_protocol_getter_function,
    websocket_ready_state_getter_function, websocket_stream_closed_getter_function,
    websocket_stream_opened_getter_function, websocket_stream_url_getter_function,
    websocket_url_getter_function,
};
use super::methods::{
    websocket_close_callback, websocket_send_callback, websocket_stream_close_callback,
};
use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebSocket, enumerable)]
struct WebSocketTemplateMethodsDeclaration {
    #[webapi(constant = "CONNECTING", value = CONNECTING)]
    connecting: (),

    #[webapi(constant = "OPEN", value = OPEN)]
    open: (),

    #[webapi(constant = "CLOSING", value = CLOSING)]
    closing: (),

    #[webapi(constant = "CLOSED", value = CLOSED)]
    closed: (),

    #[webapi(method, length = 1, callback = websocket_send_callback)]
    send: (),

    #[webapi(method, length = 0, callback = websocket_close_callback)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebSocketStream, enumerable)]
struct WebSocketStreamTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = websocket_stream_close_callback)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebSocket)]
struct WebSocketPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = websocket_url_getter_function, enumerable)]
    url: (),
    #[webapi(
        accessor_property = "readyState",
        getter = websocket_ready_state_getter_function,
        enumerable
    )]
    ready_state: (),
    #[webapi(
        accessor_property = "bufferedAmount",
        getter = websocket_buffered_amount_getter_function,
        enumerable
    )]
    buffered_amount: (),
    #[webapi(accessor_property, getter = websocket_extensions_getter_function, enumerable)]
    extensions: (),
    #[webapi(accessor_property, getter = websocket_protocol_getter_function, enumerable)]
    protocol: (),
    #[webapi(
        accessor_property = "binaryType",
        getter = websocket_binary_type_getter_function,
        setter = websocket_binary_type_setter_function,
        enumerable
    )]
    binary_type: (),

    #[webapi(
        accessor_property,
        symbol = "toStringTag",
        getter = websocket_to_string_tag_getter
    )]
    to_string_tag: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebSocketStream)]
struct WebSocketStreamPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = websocket_stream_url_getter_function, enumerable)]
    url: (),
    #[webapi(accessor_property, getter = websocket_stream_opened_getter_function, enumerable)]
    opened: (),
    #[webapi(accessor_property, getter = websocket_stream_closed_getter_function, enumerable)]
    closed: (),
}

pub(in crate::context_bootstrap) fn install_websocket_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    WebSocketTemplateMethodsDeclaration::initialize_template(scope, template);
    WebSocketTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
    WebSocketPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
    install_websocket_event_handler_accessors(scope, prototype);
}

pub(in crate::context_bootstrap) fn install_websocket_stream_bindings(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<'_, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    WebSocketStreamTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
    WebSocketStreamPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

fn websocket_to_string_tag_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(prototype) = global_constructor_prototype(scope, "WebSocket") else {
        rv.set(v8str(scope, "WebSocket").into());
        return;
    };
    let tag = if args.this().strict_equals(prototype.into()) {
        "WebSocketPrototype"
    } else {
        "WebSocket"
    };
    rv.set(v8str(scope, tag).into());
}
