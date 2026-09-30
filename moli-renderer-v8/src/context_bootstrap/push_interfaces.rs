//! Native Push objects shared by Window and Worker bindings.
//!
//! The existing subscription store is local only. Until a push transport and
//! encryption backend exist, advertise no content encodings or key material.

use super::*;
use crate::service_worker_runtime::ServiceWorkerPushSubscriptionSnapshot;
use crate::util::{get_private_value, set_private_value};
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::WebApiObject;

const CONTENT_ENCODINGS: &str = "__moliPushSupportedContentEncodings";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PushManager)]
struct PushManagerObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

pub(super) fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name != "PushManager" {
        moli_webapi::push_values::install_subscription_template_bindings(scope, template, name);
        return;
    }
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
    let options_prototype =
        ensure_intrinsic_interface_prototype(scope, "PushSubscriptionOptions").ok()?;
    let options = moli_webapi::push_values::new_subscription_options(
        scope,
        options_prototype,
        snapshot.user_visible_only,
    )?;
    let prototype = ensure_intrinsic_interface_prototype(scope, "PushSubscription").ok()?;
    moli_webapi::push_values::new_subscription(scope, prototype, &snapshot.endpoint, options)
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
