use super::super::window_runtime::{
    MEDIA_DEVICES_BRAND_SLOT, navigator_media_devices_get_user_media_callback,
};
use super::super::*;
use crate::util::{get_private_value, set_private_value, throw_type_error};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const MEDIA_DEVICES_LISTENERS_SLOT: &str = "__moliMediaDevicesListeners";
const MEDIA_DEVICES_ONDEVICECHANGE_SLOT: &str = "__moliMediaDevicesOndevicechange";

#[derive(Default, WebApiObject)]
#[webapi(interface = "MediaDevices")]
struct MediaDevicesObjectDeclaration {
    #[webapi(slot = MEDIA_DEVICES_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = MEDIA_DEVICES_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),

    #[webapi(slot = MEDIA_DEVICES_ONDEVICECHANGE_SLOT, init = "null")]
    ondevicechange: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "MediaDevices", enumerable)]
struct MediaDevicesPrototypeDeclaration {
    #[webapi(method, length = 0, callback = enumerate_devices_callback)]
    enumerate_devices: (),

    #[webapi(method, length = 1, callback = navigator_media_devices_get_user_media_callback)]
    get_user_media: (),

    #[webapi(accessor_property, getter = ondevicechange_getter, setter = ondevicechange_setter)]
    ondevicechange: (),
}

pub(super) fn build_media_devices_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    MediaDevicesObjectDeclaration::default().bind(scope).ok()
}

pub(super) fn install_media_devices_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    MediaDevicesPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn receiver_is_media_devices<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, MEDIA_DEVICES_BRAND_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn enumerate_devices_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    if !receiver_is_media_devices(scope, args.this()) {
        let message = v8str(scope, "Illegal invocation");
        let error = v8::Exception::type_error(scope, message);
        let _ = resolver.reject(scope, error);
        return;
    }

    // Enumeration waits while its associated document is not fully active.
    // In particular, retaining a MediaDevices object must not make a discarded
    // iframe's enumeration resolve against the caller's active document.
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    // SAFETY: all Window realms use the host owned by this isolate's bridge.
    let host = unsafe { &*host_ptr };
    let Some(identity) = host.window_execution_context_identity_for_v8_context(scope, context)
    else {
        return;
    };
    if !host.window_execution_context_identity_is_current(identity) {
        return;
    }

    // The headless media backend currently has no input or output devices.
    let devices = v8::Array::new(scope, 0);
    let _ = resolver.resolve(scope, devices.into());
}

fn ondevicechange_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_is_media_devices(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = get_private_value(scope, args.this(), MEDIA_DEVICES_ONDEVICECHANGE_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

fn ondevicechange_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_is_media_devices(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(
        scope,
        args.this(),
        MEDIA_DEVICES_ONDEVICECHANGE_SLOT,
        stored,
    );
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        MEDIA_DEVICES_LISTENERS_SLOT,
        "devicechange",
        MEDIA_DEVICES_ONDEVICECHANGE_SLOT,
        stored.is_function(),
    );
}
