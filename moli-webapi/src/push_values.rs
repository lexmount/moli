//! Immutable Push message and option values, shared by Window and Worker embeddings.
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    array_buffer_from_bytes, get_private_value, new_uint8_array_from_bytes, set_private_value,
    v8_string, v8str,
};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject};
const SERVICE_WORKER_PUSH_MESSAGE_DATA_BYTES_SLOT: &str = "__lmServiceWorkerPushMessageDataBytes";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PushMessageData)]
struct ServiceWorkerPushMessageDataDeclaration {
    #[webapi(method = "arrayBuffer", length = 0, callback = push_message_data_array_buffer_callback)]
    array_buffer: (),
    #[webapi(method = "bytes", length = 0, callback = push_message_data_bytes_callback)]
    bytes: (),
    #[webapi(method = "text", length = 0, callback = push_message_data_text_callback)]
    text: (),
    #[webapi(method = "json", length = 0, callback = push_message_data_json_callback)]
    json: (),
}
pub fn new_push_message_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: &[u8],
) -> Option<v8::Local<'s, v8::Object>> {
    let object = ServiceWorkerPushMessageDataDeclaration {
        array_buffer: (),
        bytes: (),
        text: (),
        json: (),
    }
    .bind(scope)
    .ok()?;
    let buffer = array_buffer_from_bytes(scope, bytes.to_vec())?;
    set_private_value(
        scope,
        object,
        SERVICE_WORKER_PUSH_MESSAGE_DATA_BYTES_SLOT,
        buffer.into(),
    );
    Some(object)
}

fn push_message_data_bytes_from_this<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    this: v8::Local<'s, v8::Object>,
) -> Option<Vec<u8>> {
    let value = get_private_value(scope, this, SERVICE_WORKER_PUSH_MESSAGE_DATA_BYTES_SLOT)?;
    let buffer = v8::Local::<v8::ArrayBuffer>::try_from(value).ok()?;
    let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())?;
    let mut bytes = vec![0; view.byte_length()];
    let written = view.copy_contents(&mut bytes);
    bytes.truncate(written);
    Some(bytes)
}

fn push_message_data_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes) = push_message_data_bytes_from_this(scope, args.this()) else {
        return;
    };
    if let Some(buffer) = array_buffer_from_bytes(scope, bytes) {
        rv.set(buffer.into());
    }
}

fn push_message_data_bytes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes) = push_message_data_bytes_from_this(scope, args.this()) else {
        return;
    };
    let Some(array) = new_uint8_array_from_bytes(scope, bytes) else {
        return;
    };
    rv.set(array.into());
}

fn push_message_data_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes) = push_message_data_bytes_from_this(scope, args.this()) else {
        return;
    };
    let text = String::from_utf8_lossy(&bytes);
    let Some(value) = v8_string(scope, text.as_ref()) else {
        return;
    };
    rv.set(value.into());
}

fn push_message_data_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes) = push_message_data_bytes_from_this(scope, args.this()) else {
        return;
    };
    let text = String::from_utf8_lossy(&bytes);
    let Some(source) = v8_string(scope, text.as_ref()) else {
        return;
    };
    let Some(value) = v8::json::parse(scope, source) else {
        let message = v8str(
            scope,
            "PushMessageData.json() could not parse the payload as JSON.",
        );
        let exception = v8::Exception::syntax_error(scope, message);
        scope.throw_exception(exception);
        return;
    };
    rv.set(value);
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PushSubscriptionOptions, prototype = "Object")]
struct ServiceWorkerPushSubscriptionOptionsDeclaration<'scope> {
    #[webapi(data_property = "userVisibleOnly", readonly)]
    user_visible_only: bool,

    #[webapi(data_property = "applicationServerKey", readonly)]
    application_server_key: v8::Local<'scope, v8::Value>,
}
pub fn new_subscription_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    user_visible_only: bool,
    application_server_key: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    ServiceWorkerPushSubscriptionOptionsDeclaration::new(user_visible_only, application_server_key)
        .bind(scope)
        .ok()
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PushSubscription)]
struct PushSubscriptionDeclaration<'s> {
    #[webapi(data_property, readonly)]
    endpoint: String,
    #[webapi(data_property = "expirationTime", readonly)]
    expiration_time: v8::Local<'s, v8::Value>,
    #[webapi(data_property, readonly)]
    options: v8::Local<'s, v8::Object>,
    #[webapi(data_property)]
    unsubscribe: v8::Local<'s, v8::Function>,
    #[webapi(method = "toJSON", callback = subscription_to_json, length = 0)]
    to_json: (),
}

/// Build the subscription surface with the caller's own registration operation.
pub fn new_subscription<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    endpoint: &str,
    user_visible_only: bool,
    unsubscribe: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Option<v8::Local<'s, v8::Object>> {
    let options = new_subscription_options(scope, user_visible_only, v8::null(scope).into())?;
    let unsubscribe = v8::Function::builder(unsubscribe)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)?;
    unsubscribe.set_name(v8str(scope, "unsubscribe"));
    PushSubscriptionDeclaration {
        endpoint: endpoint.to_owned(),
        expiration_time: v8::null(scope).into(),
        options,
        unsubscribe,
        to_json: (),
    }
    .bind(scope)
    .ok()
}

fn subscription_to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let object = ObjectLiteralDeclaration::bind(scope);
    for name in ["endpoint", "expirationTime", "options"] {
        let value = args
            .this()
            .get(scope, v8str(scope, name).into())
            .unwrap_or_else(|| v8::undefined(scope).into());
        object.set_string_property(scope, name, value);
    }
    rv.set(object.into_value());
}
