use super::super::media_queries::{
    mark_simple_event_target_slot, simple_event_target_add_event_listener_callback,
    simple_event_target_dispatch_event_callback,
    simple_event_target_remove_event_listener_callback,
};
use super::super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
    get_private_value, set_private_value, throw_type_error,
};
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const SCREEN_BRAND_SLOT: &str = "__moliScreenBrand";
const SCREEN_WIDTH_SLOT: &str = "__moliScreenWidth";
const SCREEN_HEIGHT_SLOT: &str = "__moliScreenHeight";
const SCREEN_AVAIL_WIDTH_SLOT: &str = "__moliScreenAvailWidth";
const SCREEN_AVAIL_HEIGHT_SLOT: &str = "__moliScreenAvailHeight";
const SCREEN_AVAIL_LEFT_SLOT: &str = "__moliScreenAvailLeft";
const SCREEN_AVAIL_TOP_SLOT: &str = "__moliScreenAvailTop";
const SCREEN_COLOR_DEPTH_SLOT: &str = "__moliScreenColorDepth";
const SCREEN_PIXEL_DEPTH_SLOT: &str = "__moliScreenPixelDepth";
const SCREEN_ORIENTATION_SLOT: &str = "__moliScreenOrientation";

const SCREEN_ORIENTATION_BRAND_SLOT: &str = "__moliScreenOrientationBrand";
const SCREEN_ORIENTATION_TYPE_SLOT: &str = "__moliScreenOrientationType";
const SCREEN_ORIENTATION_ANGLE_SLOT: &str = "__moliScreenOrientationAngle";
const SCREEN_ORIENTATION_ONCHANGE_SLOT: &str = "__moliScreenOrientationOnchange";

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "OrientationLockType", rename_all = "kebab-case")]
enum OrientationLockType {
    Any,
    Natural,
    Landscape,
    Portrait,
    PortraitPrimary,
    PortraitSecondary,
    LandscapePrimary,
    LandscapeSecondary,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ScreenOrientation.lock")]
struct ScreenOrientationLockArgs {
    #[webidl(required, converter = "enum")]
    orientation: OrientationLockType,
}

#[derive(WebApiObject)]
#[webapi(interface = "Screen")]
struct ScreenObjectDeclaration {
    #[webapi(slot = SCREEN_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = SCREEN_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = SCREEN_HEIGHT_SLOT)]
    height: f64,
    #[webapi(slot = SCREEN_AVAIL_WIDTH_SLOT)]
    avail_width: f64,
    #[webapi(slot = SCREEN_AVAIL_HEIGHT_SLOT)]
    avail_height: f64,
    #[webapi(slot = SCREEN_AVAIL_LEFT_SLOT)]
    avail_left: f64,
    #[webapi(slot = SCREEN_AVAIL_TOP_SLOT)]
    avail_top: f64,
    #[webapi(slot = SCREEN_COLOR_DEPTH_SLOT)]
    color_depth: f64,
    #[webapi(slot = SCREEN_PIXEL_DEPTH_SLOT)]
    pixel_depth: f64,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "Screen")]
struct ScreenPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 0), enumerable)]
    avail_width: (),
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 1), enumerable)]
    avail_height: (),
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 2), enumerable)]
    avail_left: (),
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 3), enumerable)]
    avail_top: (),
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 4), enumerable)]
    width: (),
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 5), enumerable)]
    height: (),
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 6), enumerable)]
    color_depth: (),
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 7), enumerable)]
    pixel_depth: (),
    #[webapi(accessor_property, getter = screen_attribute_getter_callback, data = callback_data_index_value(scope, 8), enumerable)]
    orientation: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "ScreenOrientation")]
struct ScreenOrientationObjectDeclaration {
    #[webapi(slot = SCREEN_ORIENTATION_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = SCREEN_ORIENTATION_TYPE_SLOT)]
    orientation_type: &'static str,
    #[webapi(slot = SCREEN_ORIENTATION_ANGLE_SLOT)]
    angle: f64,
    #[webapi(slot = SCREEN_ORIENTATION_ONCHANGE_SLOT, init = "null")]
    onchange: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "ScreenOrientation")]
struct ScreenOrientationPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, name = "type", getter = screen_orientation_attribute_getter_callback, data = callback_data_index_value(scope, 0), enumerable)]
    orientation_type: (),
    #[webapi(accessor_property, getter = screen_orientation_attribute_getter_callback, data = callback_data_index_value(scope, 1), enumerable)]
    angle: (),
    #[webapi(accessor_property, getter = screen_orientation_onchange_getter_callback, setter = screen_orientation_onchange_setter_callback, enumerable)]
    onchange: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "ScreenOrientation")]
struct ScreenOrientationPrototypeMethodsDeclaration {
    #[webapi(method, enumerable, length = 1, callback = screen_orientation_lock_callback)]
    lock: (),
    #[webapi(method, enumerable, length = 0, callback = screen_orientation_unlock_callback)]
    unlock: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "Screen", enumerable)]
struct ScreenEventTargetMethodsDeclaration {
    #[webapi(method, length = 2, callback = screen_event_target_add_event_listener_callback)]
    add_event_listener: (),
    #[webapi(
        method,
        length = 2,
        callback = screen_event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),
    #[webapi(method, length = 1, callback = screen_event_target_dispatch_event_callback)]
    dispatch_event: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "ScreenOrientation", enumerable)]
struct ScreenOrientationEventTargetMethodsDeclaration {
    #[webapi(
        method,
        length = 2,
        callback = screen_orientation_event_target_add_event_listener_callback
    )]
    add_event_listener: (),
    #[webapi(
        method,
        length = 2,
        callback = screen_orientation_event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),
    #[webapi(
        method,
        length = 1,
        callback = screen_orientation_event_target_dispatch_event_callback
    )]
    dispatch_event: (),
}

fn new_not_supported_dom_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    crate::context_bootstrap::new_dom_exception_value(scope, message, "NotSupportedError")
}

fn screen_orientation_lock_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<ScreenOrientationLockArgs, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut conversion_scope = try_catch.init();
    match webidl::try_parse_args::<ScreenOrientationLockArgs>(&mut conversion_scope, args) {
        Ok(parsed) => Ok(parsed),
        Err(error) => {
            webidl::throw_error(&mut conversion_scope, &error);
            let exception = conversion_scope
                .exception()
                .unwrap_or_else(|| v8::undefined(&conversion_scope).into());
            conversion_scope.reset();
            Err(exception)
        }
    }
}

fn screen_orientation_lock_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_ORIENTATION_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let promise = resolver.get_promise(scope);
    let parsed = match screen_orientation_lock_args(scope, &args) {
        Ok(parsed) => parsed,
        Err(exception) => {
            let _ = resolver.reject(scope, exception);
            rv.set(promise.into());
            return;
        }
    };
    let _orientation = parsed.orientation;
    let exception = new_not_supported_dom_exception(
        scope,
        "screen.orientation.lock() is not available on this device.",
    );
    let _ = resolver.reject(scope, exception);
    rv.set(promise.into());
}

fn screen_orientation_unlock_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_ORIENTATION_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(v8::undefined(scope).into());
}

fn screen_event_target_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_add_event_listener_callback(scope, args, rv);
}

fn screen_event_target_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_remove_event_listener_callback(scope, args, rv);
}

fn screen_event_target_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_dispatch_event_callback(scope, args, rv);
}

fn screen_orientation_event_target_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_ORIENTATION_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_add_event_listener_callback(scope, args, rv);
}

fn screen_orientation_event_target_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_ORIENTATION_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_remove_event_listener_callback(scope, args, rv);
}

fn screen_orientation_event_target_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_ORIENTATION_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    simple_event_target_dispatch_event_callback(scope, args, rv);
}

pub(in crate::context_bootstrap) fn install_screen_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Screen" => {
            ScreenPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
            ScreenEventTargetMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "ScreenOrientation" => {
            ScreenOrientationPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            ScreenOrientationEventTargetMethodsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            ScreenOrientationPrototypeMethodsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn build_window_screen<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    let profile = &DEFAULT_WINDOW_SURFACE_PROFILE;
    let screen = ScreenObjectDeclaration {
        brand: (),
        width: profile.screen_width,
        height: profile.screen_height,
        avail_width: profile.screen_avail_width,
        avail_height: profile.screen_avail_height,
        avail_left: 0.0,
        avail_top: 0.0,
        color_depth: profile.color_depth,
        pixel_depth: profile.pixel_depth,
    }
    .bind(scope)?;
    mark_simple_event_target_slot(scope, screen, SCREEN_EVENT_LISTENERS_SLOT);
    Ok(screen)
}

fn screen_owner_realm_is_current(
    scope: &mut v8::PinScope<'_, '_>,
    screen: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(context) = screen.get_creation_context(scope) else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    host.window_execution_context_identity_for_access_check(context)
        .is_some_and(|identity| host.window_execution_context_identity_is_current(identity))
}

fn screen_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(scope, &args, SCREEN_ATTRIBUTE_SLOTS, "Screen slots")
    else {
        rv.set_undefined();
        return;
    };
    if slot == SCREEN_ORIENTATION_SLOT {
        match ensure_screen_orientation(scope, args.this()) {
            Ok(orientation) => rv.set(orientation.into()),
            Err(error) => throw_error(
                scope,
                &format!("Failed to materialize Screen.orientation: {error}"),
            ),
        }
        return;
    }
    if matches!(
        slot,
        SCREEN_WIDTH_SLOT
            | SCREEN_HEIGHT_SLOT
            | SCREEN_AVAIL_WIDTH_SLOT
            | SCREEN_AVAIL_HEIGHT_SLOT
            | SCREEN_AVAIL_LEFT_SLOT
            | SCREEN_AVAIL_TOP_SLOT
    ) && !screen_owner_realm_is_current(scope, args.this())
    {
        rv.set(v8::Number::new(scope, 0.0).into());
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn ensure_screen_orientation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    screen: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    if let Some(orientation) = get_private_value(scope, screen, SCREEN_ORIENTATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Ok(orientation);
    }
    let relevant_context = screen
        .get_creation_context(scope)
        .ok_or_else(|| anyhow!("Screen receiver has no creation context"))?;
    if relevant_context == scope.get_current_context() {
        return build_screen_orientation_in_current_realm(scope, screen);
    }
    let screen = v8::Global::new(scope, screen);
    let orientation = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let target_screen = v8::Local::new(target_scope, &screen);
        let orientation = build_screen_orientation_in_current_realm(target_scope, target_screen)?;
        v8::Global::new(target_scope, orientation)
    };
    Ok(v8::Local::new(scope, &orientation))
}

fn build_screen_orientation_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    screen: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let profile = &DEFAULT_WINDOW_SURFACE_PROFILE;
    let orientation = ScreenOrientationObjectDeclaration::new(
        profile.orientation_type,
        profile.orientation_angle,
    )
    .bind(scope)?;
    mark_simple_event_target_slot(scope, orientation, SCREEN_ORIENTATION_EVENT_LISTENERS_SLOT);
    set_private_value(scope, screen, SCREEN_ORIENTATION_SLOT, orientation.into());
    Ok(orientation)
}

fn screen_orientation_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_ORIENTATION_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        SCREEN_ORIENTATION_ATTRIBUTE_SLOTS,
        "ScreenOrientation slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

const SCREEN_ATTRIBUTE_SLOTS: &[&str] = &[
    SCREEN_AVAIL_WIDTH_SLOT,
    SCREEN_AVAIL_HEIGHT_SLOT,
    SCREEN_AVAIL_LEFT_SLOT,
    SCREEN_AVAIL_TOP_SLOT,
    SCREEN_WIDTH_SLOT,
    SCREEN_HEIGHT_SLOT,
    SCREEN_COLOR_DEPTH_SLOT,
    SCREEN_PIXEL_DEPTH_SLOT,
    SCREEN_ORIENTATION_SLOT,
];

const SCREEN_ORIENTATION_ATTRIBUTE_SLOTS: &[&str] =
    &[SCREEN_ORIENTATION_TYPE_SLOT, SCREEN_ORIENTATION_ANGLE_SLOT];

fn screen_orientation_onchange_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_ORIENTATION_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), SCREEN_ORIENTATION_ONCHANGE_SLOT)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn screen_orientation_onchange_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), SCREEN_ORIENTATION_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    set_private_value(
        scope,
        args.this(),
        SCREEN_ORIENTATION_ONCHANGE_SLOT,
        args.get(0),
    );
    rv.set_undefined();
}

fn receiver_has_brand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    get_private_value(scope, receiver, slot).is_some_and(|value| value.boolean_value(scope))
}
