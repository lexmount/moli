//! Immutable Push message and option values, shared by Window and Worker embeddings.
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    array_buffer_from_bytes, get_private_value, new_uint8_array_from_bytes, set_private_value,
    v8_string, v8str,
};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;
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

const ENDPOINT: &str = "__moliPushSubscriptionEndpoint";
const EXPIRATION_TIME: &str = "__moliPushSubscriptionExpirationTime";
const OPTIONS: &str = "__moliPushSubscriptionOptions";
const USER_VISIBLE_ONLY: &str = "__moliPushUserVisibleOnly";
const APPLICATION_SERVER_KEY: &str = "__moliPushApplicationServerKey";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PushSubscription)]
struct PushSubscriptionObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = ENDPOINT)]
    endpoint: String,
    #[webapi(slot = EXPIRATION_TIME)]
    expiration_time: v8::Local<'s, v8::Value>,
    #[webapi(slot = OPTIONS)]
    options: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PushSubscriptionOptions)]
struct PushSubscriptionOptionsObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = USER_VISIBLE_ONLY)]
    user_visible_only: bool,
    #[webapi(slot = APPLICATION_SERVER_KEY)]
    application_server_key: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PushSubscription, enumerable)]
struct SubscriptionAttributes {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, ENDPOINT))]
    endpoint: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, EXPIRATION_TIME))]
    expiration_time: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, OPTIONS))]
    options: (),
    #[webapi(method, callback = get_key, length = 1)]
    get_key: (),
    #[webapi(method = "toJSON", callback = to_json, length = 0)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PushSubscriptionOptions, enumerable)]
struct OptionsAttributes {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, USER_VISIBLE_ONLY))]
    user_visible_only: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, APPLICATION_SERVER_KEY))]
    application_server_key: (),
}

pub fn install_subscription_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "PushSubscription" => {
            SubscriptionAttributes::initialize_prototype_template(scope, prototype)
        }
        "PushSubscriptionOptions" => {
            OptionsAttributes::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

pub fn new_subscription_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    user_visible_only: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    PushSubscriptionOptionsObject::new(prototype, user_visible_only, v8::null(scope).into())
        .bind(scope)
        .ok()
}

pub fn new_subscription<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    endpoint: &str,
    options: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    PushSubscriptionObject::new(
        prototype,
        endpoint.to_owned(),
        v8::null(scope).into(),
        options,
    )
    .bind(scope)
    .ok()
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "PushEncryptionKeyName")]
enum PushEncryptionKeyName {
    #[webidl(token = "p256dh")]
    P256dh,
    #[webidl(token = "auth")]
    Auth,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PushSubscription.getKey")]
struct GetKeyArgs {
    #[webidl(required, converter = "enum")]
    name: PushEncryptionKeyName,
}

fn get_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(GetKeyArgs { name: _name }) = webidl::parse_args::<GetKeyArgs>(scope, &args) else {
        return;
    };
    rv.set_null();
}

fn to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let object = ObjectLiteralDeclaration::bind(scope);
    for (name, slot) in [("endpoint", ENDPOINT), ("expirationTime", EXPIRATION_TIME)] {
        let Some(value) = get_private_value(scope, args.this(), slot) else {
            return;
        };
        object.set_string_property(scope, name, value);
    }
    let keys = ObjectLiteralDeclaration::bind(scope);
    object.set_string_property(scope, "keys", keys.into_value());
    rv.set(object.into_value());
}
