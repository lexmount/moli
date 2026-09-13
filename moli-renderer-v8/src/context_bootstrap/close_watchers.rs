use super::{
    SIMPLE_EVENT_TARGET_SLOT, dispatch_simple_event_target_event,
    ensure_intrinsic_interface_prototype, events::initialize_event_object,
    install_simple_event_target_ordered_handlers, mark_event_trusted,
    simple_object_event_set_ordered_handler, throw_dom_exception_value,
};
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    native_bridge::OwnerDispatchScope,
    util::{
        context_host_ptr_from_context_slot, get_private_value, set_private_value, throw_type_error,
    },
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const WINDOW_SLOT: &str = "__moliCloseWatcherWindow";
const REALM_ANCHOR_SLOT: &str = "__moliCloseWatcherRealmAnchor";
const ACTIVE_SLOT: &str = "__moliCloseWatcherActive";
const RUNNING_CANCEL_SLOT: &str = "__moliCloseWatcherRunningCancel";
const LISTENERS_SLOT: &str = "__moliCloseWatcherListeners";
const ONCANCEL_SLOT: &str = "__moliCloseWatcherOncancel";
const ONCLOSE_SLOT: &str = "__moliCloseWatcherOnclose";
const SIGNAL_SLOT: &str = "__moliCloseWatcherSignal";
const ABORT_ALGORITHM_SLOT: &str = "__moliCloseWatcherAbortAlgorithm";
const WATCHERS_SLOT: &str = "__moliWindowCloseWatchers";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CloseWatcher)]
struct CloseWatcherObjectDeclaration<'scope> {
    #[webapi(slot = WINDOW_SLOT)]
    window: v8::Local<'scope, v8::Object>,
    #[webapi(slot = REALM_ANCHOR_SLOT)]
    realm_anchor: v8::Local<'scope, v8::Object>,
    #[webapi(slot = ACTIVE_SLOT, init = true)]
    active: (),
    #[webapi(slot = RUNNING_CANCEL_SLOT, init = false)]
    running_cancel: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS_SLOT)]
    event_target_slot: (),
    #[webapi(slot = ONCANCEL_SLOT, init = "null")]
    oncancel: (),
    #[webapi(slot = ONCLOSE_SLOT, init = "null")]
    onclose: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CloseWatcher, enumerable)]
struct CloseWatcherPrototypeDeclaration {
    #[webapi(method, length = 0, callback = request_close_callback)]
    request_close: (),
    #[webapi(method, length = 0, callback = close_callback)]
    close: (),
    #[webapi(method, length = 0, callback = destroy_callback)]
    destroy: (),
    #[webapi(accessor_property, getter = oncancel_getter, setter = oncancel_setter)]
    oncancel: (),
    #[webapi(accessor_property, getter = onclose_getter, setter = onclose_setter)]
    onclose: (),
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "CloseWatcherOptions")]
struct CloseWatcherOptions<'s> {
    #[webidl(converter = "raw")]
    signal: Option<v8::Local<'s, v8::Value>>,
}

pub(in crate::context_bootstrap) fn install_close_watcher_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    CloseWatcherPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn close_watcher_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'CloseWatcher': Please use the 'new' operator.",
        );
        return;
    }
    let options = match webidl::parse_dictionary::<CloseWatcherOptions<'s>>(
        scope,
        args.get(0),
        webidl::Context::argument("CloseWatcher", 1),
    ) {
        Ok(options) => options.unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let signal = if let Some(value) = options.signal {
        let Some(signal) = v8::Local::<v8::Object>::try_from(value)
            .ok()
            .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        else {
            throw_type_error(scope, "CloseWatcherOptions.signal must be an AbortSignal.");
            return;
        };
        Some(signal)
    } else {
        None
    };
    let context = scope.get_current_context();
    if !context_is_fully_active(context) {
        throw_dom_exception_value(
            scope,
            "The associated Document is not fully active.",
            "InvalidStateError",
        );
        return;
    }
    let window = context.global(scope);
    // Unlike a WindowProxy, this object's creation context cannot change when
    // navigation reuses the browsing context's public Window identity.
    let realm_anchor = v8::Object::new(scope);
    let watcher = args.this();
    if CloseWatcherObjectDeclaration::new(window, realm_anchor)
        .initialize(scope, watcher)
        .is_err()
    {
        return;
    }
    install_simple_event_target_ordered_handlers(scope, watcher);
    let mut watchers = window_watchers(scope, window);
    watchers.push(watcher.into());
    set_window_watchers(scope, window, &watchers);
    if let Some(signal) = signal {
        if signal.is_aborted(scope) {
            destroy(scope, watcher);
        } else if let Some(algorithm) = v8::Function::builder(abort_callback)
            .data(watcher.into())
            .build(scope)
        {
            set_private_value(scope, watcher, SIGNAL_SLOT, signal.value().into());
            set_private_value(scope, watcher, ABORT_ALGORITHM_SLOT, algorithm.into());
            signal.register_algorithm(scope, algorithm);
        }
    }
    rv.set(watcher.into());
}

fn context_is_fully_active(context: v8::Local<'_, v8::Context>) -> bool {
    let Some(host_ptr) = context_host_ptr_from_context_slot(context) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    let Some(identity) = host.window_execution_context_identity_for_access_check(context) else {
        return false;
    };
    if !host.window_execution_context_identity_is_current(identity) {
        return false;
    }
    match identity.dispatch_scope() {
        OwnerDispatchScope::Top => true,
        OwnerDispatchScope::Child(handle) => host.child_browsing_context_is_live(handle),
        OwnerDispatchScope::LightweightPopup(id) => host.lightweight_popup_is_open(id),
    }
}

// Retain active watchers in their Window. These arrays never escape to script,
// and replacement avoids invoking mutable Array.prototype methods or setters.
fn window_watchers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Value>> {
    let Some(watchers) = get_private_value(scope, window, WATCHERS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return Vec::new();
    };
    (0..watchers.length())
        .filter_map(|i| watchers.get_index(scope, i))
        .collect()
}

fn set_window_watchers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    watchers: &[v8::Local<'s, v8::Value>],
) {
    let array = v8::Array::new_with_elements(scope, watchers);
    set_private_value(scope, window, WATCHERS_SLOT, array.into());
}

fn receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if web_api_interfaces::CloseWatcher::is_instance(scope, object) {
        Some(object)
    } else {
        throw_type_error(scope, "Illegal invocation");
        None
    }
}

fn flag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    watcher: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    get_private_value(scope, watcher, slot).is_some_and(|value| value.is_true())
}

fn watcher_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    watcher: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Context>> {
    get_private_value(scope, watcher, REALM_ANCHOR_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|anchor| anchor.get_creation_context(scope))
}

fn is_active<'s>(scope: &mut v8::PinScope<'s, '_>, watcher: v8::Local<'s, v8::Object>) -> bool {
    flag(scope, watcher, ACTIVE_SLOT)
        && watcher_context(scope, watcher).is_some_and(context_is_fully_active)
}

fn destroy<'s>(scope: &mut v8::PinScope<'s, '_>, watcher: v8::Local<'s, v8::Object>) {
    if !flag(scope, watcher, ACTIVE_SLOT) {
        return;
    }
    set_private_value(
        scope,
        watcher,
        ACTIVE_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    if let Some(window) = get_private_value(scope, watcher, WINDOW_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        let mut watchers = window_watchers(scope, window);
        watchers.retain(|candidate| !candidate.strict_equals(watcher.into()));
        set_window_watchers(scope, window, &watchers);
    }
    let signal = get_private_value(scope, watcher, SIGNAL_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal));
    let algorithm = get_private_value(scope, watcher, ABORT_ALGORITHM_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    set_private_value(scope, watcher, SIGNAL_SLOT, v8::undefined(scope).into());
    set_private_value(
        scope,
        watcher,
        ABORT_ALGORITHM_SLOT,
        v8::undefined(scope).into(),
    );
    if let (Some(signal), Some(algorithm)) = (signal, algorithm) {
        signal.unregister_algorithm(scope, algorithm);
    }
}

fn fire_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    watcher: v8::Local<'s, v8::Object>,
    event_type: &str,
    cancelable: bool,
) -> bool {
    let Some(context) = watcher_context(scope, watcher) else {
        return true;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Ok(prototype) = ensure_intrinsic_interface_prototype(scope, "Event") else {
        return true;
    };
    let event = v8::Object::new(scope);
    let _ = event.set_prototype(scope, prototype.into());
    initialize_event_object(scope, event, event_type, false, cancelable);
    mark_event_trusted(scope, event);
    dispatch_simple_event_target_event(scope, watcher, LISTENERS_SLOT, event_type, event)
}

fn close<'s>(scope: &mut v8::PinScope<'s, '_>, watcher: v8::Local<'s, v8::Object>) {
    if !is_active(scope, watcher) {
        return;
    }
    // Deactivate before firing close: a close handler may invoke any method.
    destroy(scope, watcher);
    fire_event(scope, watcher, "close", false);
}

fn request_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(watcher) = receiver(scope, args.this()) else {
        return;
    };
    if !is_active(scope, watcher) || flag(scope, watcher, RUNNING_CANCEL_SLOT) {
        return;
    }
    set_private_value(
        scope,
        watcher,
        RUNNING_CANCEL_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    // Programmatic requestClose never requires history-action activation.
    let should_close = fire_event(scope, watcher, "cancel", true);
    set_private_value(
        scope,
        watcher,
        RUNNING_CANCEL_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    if should_close {
        close(scope, watcher);
    }
}

fn close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(watcher) = receiver(scope, args.this()) {
        close(scope, watcher);
    }
}

fn destroy_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(watcher) = receiver(scope, args.this()) {
        destroy(scope, watcher);
    }
}

fn abort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Ok(watcher) = v8::Local::<v8::Object>::try_from(args.data()) {
        destroy(scope, watcher);
    }
}

fn get_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    slot: &str,
) {
    if let Some(watcher) = receiver(scope, args.this()) {
        rv.set(get_private_value(scope, watcher, slot).unwrap_or_else(|| v8::null(scope).into()));
    }
}

fn set_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    slot: &'static str,
    event_type: &str,
) {
    let Some(watcher) = receiver(scope, args.this()) else {
        return;
    };
    let value = args.get(0);
    let active = value.is_object()
        && v8::Local::<v8::Object>::try_from(value).is_ok_and(|value| value.is_callable());
    set_private_value(
        scope,
        watcher,
        slot,
        if active {
            value
        } else {
            v8::null(scope).into()
        },
    );
    simple_object_event_set_ordered_handler(
        scope,
        watcher,
        LISTENERS_SLOT,
        event_type,
        slot,
        active,
    );
}

fn oncancel_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    get_handler(scope, args, rv, ONCANCEL_SLOT);
}
fn oncancel_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_handler(scope, args, ONCANCEL_SLOT, "cancel");
}
fn onclose_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    get_handler(scope, args, rv, ONCLOSE_SLOT);
}
fn onclose_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_handler(scope, args, ONCLOSE_SLOT, "close");
}
