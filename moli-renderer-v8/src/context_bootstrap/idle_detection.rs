use super::{
    SIMPLE_EVENT_TARGET_SLOT, dispatch_simple_event_target_event, global_constructor_object,
};
use crate::web_api_interfaces;
use crate::{
    context_bootstrap::events::mark_event_trusted,
    util::{
        array_push_value, context_host_ptr_from_global_bridge, get_private_value,
        set_private_value, throw_type_error, v8_string, v8str,
    },
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const IDLE_DETECTOR_STARTED_SLOT: &str = "__lmIdleDetectorStarted";
const IDLE_DETECTOR_USER_ACTIVE_SLOT: &str = "__lmIdleDetectorUserActive";
const IDLE_DETECTOR_SCREEN_UNLOCKED_SLOT: &str = "__lmIdleDetectorScreenUnlocked";
const IDLE_DETECTOR_LISTENERS_SLOT: &str = "__lmIdleDetectorListeners";
const IDLE_DETECTOR_REGISTRY_SLOT: &str = "__lmIdleDetectorRegistry";
const MINIMUM_IDLE_THRESHOLD_MS: u32 = 60_000;
const ACTUAL_IDLE_STATE: crate::protocol_types::EmulatedIdleOverride =
    crate::protocol_types::EmulatedIdleOverride {
        is_user_active: true,
        is_screen_unlocked: true,
    };

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IdleDetector)]
struct IdleDetectorObjectDeclaration {
    #[webapi(slot = IDLE_DETECTOR_STARTED_SLOT)]
    started: bool,

    #[webapi(slot = IDLE_DETECTOR_USER_ACTIVE_SLOT)]
    is_user_active: bool,

    #[webapi(slot = IDLE_DETECTOR_SCREEN_UNLOCKED_SLOT)]
    is_screen_unlocked: bool,

    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = IDLE_DETECTOR_LISTENERS_SLOT)]
    event_target_slot: (),

    #[webapi(data_property = "onchange", init = "null")]
    onchange: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IdleDetector)]
struct IdleDetectorPrototypeDeclaration {
    #[webapi(
        accessor_property = "userState",
        getter = idle_detector_user_state_getter,
        enumerable
    )]
    user_state: (),

    #[webapi(
        accessor_property = "screenState",
        getter = idle_detector_screen_state_getter,
        enumerable
    )]
    screen_state: (),

    #[webapi(method, length = 0, callback = idle_detector_start_callback)]
    start: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IdleDetector, enumerable)]
struct IdleDetectorConstructorDeclaration {
    #[webapi(
        static_method = "requestPermission",
        length = 0,
        callback = idle_detector_request_permission_callback
    )]
    request_permission: (),
}

pub(in crate::context_bootstrap) fn idle_detector_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'IdleDetector': Please use the 'new' operator.",
        );
        return;
    }

    let detector = args.this();
    if IdleDetectorObjectDeclaration::new(false, true, true)
        .initialize(scope, detector)
        .is_err()
    {
        rv.set_undefined();
        return;
    }
    rv.set(detector.into());
}

pub(in crate::context_bootstrap) fn install_idle_detector_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    IdleDetectorConstructorDeclaration::initialize_template(scope, template);
    let prototype = template.prototype_template(scope);
    IdleDetectorPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn idle_detector_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if moli_webapi_declare::implements_interface(scope, receiver, "IdleDetector") {
        return Some(receiver);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

fn idle_detector_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    detector: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, detector, IDLE_DETECTOR_STARTED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn idle_detector_user_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(detector) = idle_detector_receiver(scope, args.this()) else {
        return;
    };
    if !idle_detector_started(scope, detector) {
        rv.set_null();
        return;
    }
    let active = get_private_value(scope, detector, IDLE_DETECTOR_USER_ACTIVE_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    rv.set(v8str(scope, if active { "active" } else { "idle" }).into());
}

fn idle_detector_screen_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(detector) = idle_detector_receiver(scope, args.this()) else {
        return;
    };
    if !idle_detector_started(scope, detector) {
        rv.set_null();
        return;
    }
    let unlocked = get_private_value(scope, detector, IDLE_DETECTOR_SCREEN_UNLOCKED_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    rv.set(v8str(scope, if unlocked { "unlocked" } else { "locked" }).into());
}

fn idle_detector_start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    rv.set(resolver.get_promise(scope).into());

    let Some(detector) = idle_detector_receiver(scope, args.this()) else {
        return;
    };
    if idle_detector_started(scope, detector) {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "Idle detector is already started.",
        );
        return;
    }
    if let Err(message) = validate_idle_options(scope, args.get(0)) {
        reject_type_error(scope, resolver, &message);
        return;
    }

    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "Execution context is detached.",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    if host.permission_state("idle-detection") != "granted" {
        reject_dom_exception(
            scope,
            resolver,
            "NotAllowedError",
            "Idle detection permission denied",
        );
        return;
    }

    set_private_value(
        scope,
        detector,
        IDLE_DETECTOR_STARTED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    register_idle_detector(scope, detector);
    let state = if current_context_is_top_level_window(scope, host) {
        host.idle_override().unwrap_or(ACTUAL_IDLE_STATE)
    } else {
        ACTUAL_IDLE_STATE
    };
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
    update_idle_detector(scope, detector, state, true);
}

fn current_context_is_top_level_window(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
) -> bool {
    let current_global = scope.get_current_context().global(scope);
    host.page_default_context(scope)
        .is_some_and(|context| context.global(scope).strict_equals(current_global.into()))
}

fn validate_idle_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), String> {
    let options =
        webidl::dictionary_value(value, webidl::Context::argument("IdleDetector.start", 1))
            .map_err(|error| error.to_string())?;
    let Some(options) = options else {
        return Ok(());
    };
    let threshold = webidl::property_result(
        scope,
        options,
        "threshold",
        webidl::Context::member("IdleOptions", "threshold"),
    )
    .map_err(|error| error.to_string())?;
    let Some(threshold) = threshold.filter(|value| !value.is_undefined()) else {
        return Ok(());
    };
    let threshold = webidl::convert::<webidl::EnforceRangeUnsignedLong>(
        scope,
        threshold,
        webidl::Context::member("IdleOptions", "threshold"),
    )
    .map_err(|error| error.to_string())?
    .0;
    if threshold < MINIMUM_IDLE_THRESHOLD_MS {
        return Err("Minimum threshold is 1 minute.".to_owned());
    }
    Ok(())
}

fn idle_detector_request_permission_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let state = context_host_ptr_from_global_bridge(scope)
        .map(|host_ptr| unsafe { &*host_ptr }.permission_state("idle-detection"))
        .unwrap_or("denied");
    let value = v8_string(scope, state)
        .map(|value| value.into())
        .unwrap_or_else(|| v8str(scope, "denied").into());
    let _ = resolver.resolve(scope, value);
    rv.set(resolver.get_promise(scope).into());
}

fn register_idle_detector<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    detector: v8::Local<'s, v8::Object>,
) {
    let global = scope.get_current_context().global(scope);
    let registry = get_private_value(scope, global, IDLE_DETECTOR_REGISTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .unwrap_or_else(|| {
            let registry = v8::Array::new(scope, 0);
            set_private_value(scope, global, IDLE_DETECTOR_REGISTRY_SLOT, registry.into());
            registry
        });
    array_push_value(scope, registry, detector.into());
}

pub(crate) fn apply_idle_override_to_current_context(
    scope: &mut v8::PinScope<'_, '_>,
    idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
) {
    let global = scope.get_current_context().global(scope);
    let Some(registry) = get_private_value(scope, global, IDLE_DETECTOR_REGISTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return;
    };
    let state = idle_override.unwrap_or(ACTUAL_IDLE_STATE);
    for index in 0..registry.length() {
        let Some(detector) = registry
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if idle_detector_started(scope, detector) {
            update_idle_detector(scope, detector, state, false);
        }
    }
}

fn update_idle_detector<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    detector: v8::Local<'s, v8::Object>,
    state: crate::protocol_types::EmulatedIdleOverride,
    force_event: bool,
) {
    let previous_user_active = get_private_value(scope, detector, IDLE_DETECTOR_USER_ACTIVE_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    let previous_screen_unlocked =
        get_private_value(scope, detector, IDLE_DETECTOR_SCREEN_UNLOCKED_SLOT)
            .is_some_and(|value| value.boolean_value(scope));
    if !force_event
        && previous_user_active == state.is_user_active
        && previous_screen_unlocked == state.is_screen_unlocked
    {
        return;
    }
    set_private_value(
        scope,
        detector,
        IDLE_DETECTOR_USER_ACTIVE_SLOT,
        v8::Boolean::new(scope, state.is_user_active).into(),
    );
    set_private_value(
        scope,
        detector,
        IDLE_DETECTOR_SCREEN_UNLOCKED_SLOT,
        v8::Boolean::new(scope, state.is_screen_unlocked).into(),
    );
    dispatch_idle_detector_change_event(scope, detector);
}

fn dispatch_idle_detector_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    detector: v8::Local<'s, v8::Object>,
) {
    let Some(event_constructor) = global_constructor_object(scope, "Event") else {
        return;
    };
    let Ok(event_constructor) = v8::Local::<v8::Function>::try_from(event_constructor) else {
        return;
    };
    let Some(event) = event_constructor.new_instance(scope, &[v8str(scope, "change").into()])
    else {
        return;
    };
    mark_event_trusted(scope, event);
    let _ = dispatch_simple_event_target_event(
        scope,
        detector,
        IDLE_DETECTOR_LISTENERS_SLOT,
        "change",
        event,
    );
}

fn reject_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
) {
    let error = v8_string(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, error);
}

fn reject_dom_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    name: &str,
    message: &str,
) {
    let error = crate::context_bootstrap::new_dom_exception_value(scope, message, name);
    let _ = resolver.reject(scope, error);
}
