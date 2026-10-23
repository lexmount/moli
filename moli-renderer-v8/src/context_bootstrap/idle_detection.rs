use super::{
    SIMPLE_EVENT_TARGET_LISTENERS_SLOT, dispatch_simple_event_target_event,
    global_constructor_object, simple_event_target_handler_getter_callback,
    simple_event_target_handler_setter_callback,
};
use crate::web_api_interfaces;
use crate::{
    context_bootstrap::events::mark_event_trusted,
    util::{
        context_host_ptr_from_global_bridge, get_private_value, set_private_value,
        throw_type_error, v8_string, v8str,
    },
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const IDLE_DETECTOR_STARTED_SLOT: &str = "__lmIdleDetectorStarted";
const IDLE_DETECTOR_USER_ACTIVE_SLOT: &str = "__lmIdleDetectorUserActive";
const IDLE_DETECTOR_SCREEN_UNLOCKED_SLOT: &str = "__lmIdleDetectorScreenUnlocked";
const IDLE_DETECTOR_ABORT_SIGNAL_SLOT: &str = "__lmIdleDetectorAbortSignal";
const IDLE_DETECTOR_ABORT_ALGORITHM_SLOT: &str = "__lmIdleDetectorAbortAlgorithm";
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

    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    #[webapi(slot = IDLE_DETECTOR_ABORT_SIGNAL_SLOT, init = "null")]
    abort_signal: (),

    #[webapi(slot = IDLE_DETECTOR_ABORT_ALGORITHM_SLOT, init = "null")]
    abort_algorithm: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IdleDetector)]
struct IdleDetectorPrototypeDeclaration {
    #[webapi(
        accessor_property = "onchange",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onchange"),
        setter_data = v8str(scope, "onchange"),
        enumerable,
        receiver = web_api_interfaces::IdleDetector::is_instance
    )]
    onchange: (),

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

    #[webapi(method, length = 0, callback = idle_detector_start_callback,
        returns_promise, receiver = web_api_interfaces::IdleDetector::is_instance)]
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
    if web_api_interfaces::IdleDetector::is_instance(scope, receiver) {
        return Some(receiver);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

fn idle_detector_owner_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    detector: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Context>> {
    let context = detector.get_creation_context(scope)?;
    if crate::util::page_context_is_detached_document(context)
        || crate::util::page_context_is_disconnected(context)
    {
        return None;
    }
    let host_ptr = crate::util::context_host_ptr_from_context_slot(context)?;
    let host = unsafe { &*host_ptr };
    // DOM removal revokes execution authority before the deferred Context
    // cleanup runs. Public state and start must observe that revocation now.
    host.window_execution_context_identity_for_v8_context(scope, context)
        .filter(|identity| {
            host.window_execution_context_identity_is_current(*identity)
                && match identity.dispatch_scope() {
                    crate::native_bridge::OwnerDispatchScope::Child(handle) => {
                        host.child_browsing_context_is_live(handle)
                    }
                    _ => true,
                }
        })
        .map(|_| context)
}

fn idle_detector_started<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    detector: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, detector, IDLE_DETECTOR_STARTED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
        && idle_detector_owner_context(scope, detector).is_some()
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

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IdleDetector.start")]
struct IdleDetectorStartArgs<'s> {
    #[webidl(dictionary)]
    options: IdleOptions<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "IdleOptions")]
struct IdleOptions<'s> {
    #[webidl(converter = "raw")]
    signal: Option<crate::abort_signal_route::ResolvedAbortSignal<'s>>,
    #[webidl(default = MINIMUM_IDLE_THRESHOLD_MS, converter = "enforce_range_unsigned_long")]
    threshold: u32,
}

fn idle_detector_start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdleDetectorStartArgs>(scope, &args) else {
        return;
    };
    let options = parsed.options;
    let detector = args.this();
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    rv.set(resolver.get_promise(scope).into());

    let Some(owner_context) = idle_detector_owner_context(scope, detector) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "Execution context is detached.",
        );
        return;
    };
    let Some(host_ptr) = crate::util::context_host_ptr_from_context_slot(owner_context) else {
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "Execution context is detached.",
        );
        return;
    };
    // Copy native state before registering an algorithm, which mutably borrows
    // the signal's host. No host borrow crosses the registration or callbacks.
    let (permission_granted, state) = {
        let host = unsafe { &*host_ptr };
        let state = if context_is_top_level_window(scope, host, owner_context) {
            host.idle_override().unwrap_or(ACTUAL_IDLE_STATE)
        } else {
            ACTUAL_IDLE_STATE
        };
        (host.permission_state("idle-detection") == "granted", state)
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
    if options.threshold < MINIMUM_IDLE_THRESHOLD_MS {
        reject_type_error(scope, resolver, "Minimum threshold is 1 minute.");
        return;
    }
    let signal = options.signal;
    if let Some(signal) = signal
        && signal.is_aborted(scope)
    {
        let reason = signal.reason(scope);
        let _ = resolver.reject(scope, reason);
        return;
    }

    if !permission_granted {
        reject_dom_exception(
            scope,
            resolver,
            "NotAllowedError",
            "Idle detection permission denied",
        );
        return;
    }

    let algorithm = if let Some(signal) = signal {
        let algorithm = {
            let owner_scope = &mut v8::ContextScope::new(scope, owner_context);
            v8::Function::builder(idle_detector_abort_callback)
                .data(detector.into())
                .build(owner_scope)
        };
        let Some(algorithm) = algorithm else {
            return;
        };
        if !signal.register_algorithm(scope, algorithm) {
            reject_dom_exception(
                scope,
                resolver,
                "InvalidStateError",
                "AbortSignal execution context is detached.",
            );
            return;
        }
        Some((signal, algorithm))
    } else {
        None
    };
    let owner_global = owner_context.global(scope);
    if !register_idle_detector(scope, owner_global, detector) {
        if let Some((signal, algorithm)) = algorithm {
            signal.unregister_algorithm(scope, algorithm);
        }
        reject_dom_exception(
            scope,
            resolver,
            "InvalidStateError",
            "Idle detector observation could not be registered.",
        );
        return;
    }
    if let Some((signal, algorithm)) = algorithm {
        set_private_value(
            scope,
            detector,
            IDLE_DETECTOR_ABORT_SIGNAL_SLOT,
            signal.value().into(),
        );
        set_private_value(
            scope,
            detector,
            IDLE_DETECTOR_ABORT_ALGORITHM_SLOT,
            algorithm.into(),
        );
    }
    set_private_value(
        scope,
        detector,
        IDLE_DETECTOR_STARTED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
    update_idle_detector(scope, detector, state, true);
}

fn context_is_top_level_window(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
    context: v8::Local<'_, v8::Context>,
) -> bool {
    let current_global = context.global(scope);
    host.page_default_context(scope)
        .is_some_and(|context| context.global(scope).strict_equals(current_global.into()))
}

fn clear_idle_abort_registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    detector: v8::Local<'s, v8::Object>,
) {
    let signal = get_private_value(scope, detector, IDLE_DETECTOR_ABORT_SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let algorithm = get_private_value(scope, detector, IDLE_DETECTOR_ABORT_ALGORITHM_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    set_private_value(
        scope,
        detector,
        IDLE_DETECTOR_ABORT_SIGNAL_SLOT,
        v8::null(scope).into(),
    );
    set_private_value(
        scope,
        detector,
        IDLE_DETECTOR_ABORT_ALGORITHM_SLOT,
        v8::null(scope).into(),
    );
    if let (Some(signal), Some(algorithm)) = (signal, algorithm)
        && let Some(source_context) = signal.get_creation_context(scope)
    {
        // The observing realm can retire while the signal's realm remains
        // live. Remove only this registration from the source owner's store.
        let source_scope = &mut v8::ContextScope::new(scope, source_context);
        if let Some(signal) =
            crate::abort_signal_route::ResolvedAbortSignal::resolve(source_scope, signal)
        {
            signal.unregister_algorithm(source_scope, algorithm);
        }
    }
}

fn idle_detector_abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(detector) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    set_private_value(
        scope,
        detector,
        IDLE_DETECTOR_STARTED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    clear_idle_abort_registration(scope, detector);
    // Stop observing and release the registry's strong reference. The signal
    // owner drains this one-shot abort algorithm; realm teardown owns the rest.
    let global = scope.get_current_context().global(scope);
    if let Some(registry) = get_private_value(scope, global, IDLE_DETECTOR_REGISTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        let mut retained = Vec::new();
        for index in 0..registry.length() {
            if let Some(value) = registry.get_index(scope, index)
                && !value.strict_equals(detector.into())
            {
                retained.push(value);
            }
        }
        let retained = v8::Array::new_with_elements(scope, &retained);
        set_private_value(scope, global, IDLE_DETECTOR_REGISTRY_SLOT, retained.into());
    }
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
    global: v8::Local<'s, v8::Object>,
    detector: v8::Local<'s, v8::Object>,
) -> bool {
    let registry = get_private_value(scope, global, IDLE_DETECTOR_REGISTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .unwrap_or_else(|| {
            let registry = v8::Array::new(scope, 0);
            set_private_value(scope, global, IDLE_DETECTOR_REGISTRY_SLOT, registry.into());
            registry
        });
    let Some(index) = v8::Integer::new_from_unsigned(scope, registry.length()).to_string(scope)
    else {
        return false;
    };
    registry.create_data_property(scope, index.into(), detector.into()) == Some(true)
}

pub(crate) fn retire_idle_detectors_in_current_context(scope: &mut v8::PinScope<'_, '_>) {
    let global = scope.get_current_context().global(scope);
    let Some(registry) = get_private_value(scope, global, IDLE_DETECTOR_REGISTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return;
    };
    for index in 0..registry.length() {
        if let Some(detector) = registry
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            set_private_value(
                scope,
                detector,
                IDLE_DETECTOR_STARTED_SLOT,
                v8::Boolean::new(scope, false).into(),
            );
            clear_idle_abort_registration(scope, detector);
        }
    }
    set_private_value(
        scope,
        global,
        IDLE_DETECTOR_REGISTRY_SLOT,
        v8::Array::new(scope, 0).into(),
    );
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
    let _ = dispatch_simple_event_target_event(scope, detector, "change", event);
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
