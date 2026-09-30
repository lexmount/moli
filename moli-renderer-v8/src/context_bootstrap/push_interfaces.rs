//! Native Push objects shared by Window and Worker bindings.
//!
//! The existing subscription store is local only. Until a push transport and
//! encryption backend exist, advertise no content encodings or key material.

use super::*;
use crate::service_worker_runtime::ServiceWorkerPushSubscriptionSnapshot;
use crate::util::{get_private_value, set_private_value};
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiFunctionTemplate, WebApiObject};

const ENDPOINT: &str = "__moliPushSubscriptionEndpoint";
const EXPIRATION_TIME: &str = "__moliPushSubscriptionExpirationTime";
const OPTIONS: &str = "__moliPushSubscriptionOptions";
const USER_VISIBLE_ONLY: &str = "__moliPushUserVisibleOnly";
const APPLICATION_SERVER_KEY: &str = "__moliPushApplicationServerKey";
const CONTENT_ENCODINGS: &str = "__moliPushSupportedContentEncodings";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PushManager)]
struct PushManagerObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

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
#[webapi(interface = web_api_interfaces::PushSubscription, enumerable, receiver)]
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
#[webapi(interface = web_api_interfaces::PushSubscriptionOptions, enumerable, receiver)]
struct OptionsAttributes {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, USER_VISIBLE_ONLY))]
    user_visible_only: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, APPLICATION_SERVER_KEY))]
    application_server_key: (),
}

pub(super) fn install_template_bindings<'s>(
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
        "PushManager" => {
            let getter = v8::FunctionTemplate::builder(supported_content_encodings)
                .constructor_behavior(v8::ConstructorBehavior::Throw)
                .build(scope);
            getter.set_class_name(v8str(scope, "get supportedContentEncodings"));
            template.set_accessor_property(
                v8str(scope, "supportedContentEncodings").into(),
                Some(getter),
                None,
                v8::PropertyAttribute::NONE,
            );
        }
        _ => {}
    }
}

pub(crate) fn build_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = ensure_intrinsic_interface_prototype(scope, "PushManager").ok()?;
    PushManagerObject::new(prototype).bind(scope).ok()
}

pub(crate) fn build_subscription<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: &ServiceWorkerPushSubscriptionSnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = ensure_intrinsic_interface_prototype(scope, "PushSubscriptionOptions").ok()?;
    let options = PushSubscriptionOptionsObject::new(
        prototype,
        snapshot.user_visible_only,
        v8::null(scope).into(),
    )
    .bind(scope)
    .ok()?;
    let prototype = ensure_intrinsic_interface_prototype(scope, "PushSubscription").ok()?;
    PushSubscriptionObject::new(
        prototype,
        snapshot.endpoint.clone(),
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

fn supported_content_encodings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(constructor) = ensure_intrinsic_interface_constructor(scope, "PushManager") else {
        return;
    };
    if let Some(array) = get_private_value(scope, constructor.into(), CONTENT_ENCODINGS) {
        rv.set(array);
        return;
    }
    let array = v8::Array::new(scope, 0);
    if array.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return;
    }
    set_private_value(scope, constructor.into(), CONTENT_ENCODINGS, array.into());
    rv.set(array.into());
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

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PushSubscriptionOptionsInit")]
pub(crate) struct PushSubscriptionOptionsInit {
    #[webidl(name = "applicationServerKey", with = application_server_key_member)]
    pub(crate) has_application_server_key: bool,
    #[webidl(default = false)]
    pub(crate) user_visible_only: bool,
}

fn application_server_key_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    _key: &str,
) -> Result<bool, webidl::WebIdlError> {
    let context = webidl::Context::member("PushSubscriptionOptionsInit", "applicationServerKey");
    let Some(value) = webidl::property_result(scope, object, "applicationServerKey", context)?
    else {
        return Ok(false);
    };
    if value.is_null_or_undefined() {
        return Ok(false);
    }
    if value.is_array_buffer() || value.is_array_buffer_view() {
        webidl::convert::<webidl::BufferSource>(scope, value, context)?;
    } else {
        webidl::convert::<webidl::DomString>(scope, value, context)?;
    }
    Ok(true)
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PushManager")]
struct ManagerArgs {
    #[webidl(with = options_arg)]
    options: PushSubscriptionOptionsInit,
}

fn options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<PushSubscriptionOptionsInit, webidl::WebIdlError> {
    webidl::parse_dictionary(
        scope,
        args.get(index),
        webidl::Context::argument("PushManager", (index + 1) as usize),
    )
    .map(Option::unwrap_or_default)
}

pub(crate) fn parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<PushSubscriptionOptionsInit> {
    webidl::parse_args::<ManagerArgs>(scope, args).map(|parsed| parsed.options)
}
