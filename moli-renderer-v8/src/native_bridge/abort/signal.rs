use super::AbortStore;
use super::event::{invoke_abort_event_callbacks, local_object_in_scope};
use crate::util::{context_host_ptr_from_global_bridge, v8str};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.addEventListener")]
struct AbortSignalAddEventListenerArgs {
    #[webidl(required)]
    event_type: String,
    #[webidl(required, converter = "callback_interface", nullable)]
    listener: Option<webidl::WebIdlCallbackInterface>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.removeEventListener")]
struct AbortSignalRemoveEventListenerArgs {
    #[webidl(required)]
    event_type: String,
    #[webidl(required, converter = "callback_interface", nullable)]
    listener: Option<webidl::WebIdlCallbackInterface>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AbortSignal.dispatchEvent")]
struct AbortSignalDispatchEventArgs<'s> {
    #[webidl(required)]
    event: v8::Local<'s, v8::Value>,
}

pub(crate) fn abort_signal_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_undefined();
        return;
    }
    let Some(parsed) = webidl::parse_args::<AbortSignalAddEventListenerArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let Some(listener) = parsed.listener else {
        rv.set_undefined();
        return;
    };
    let options = webidl::event_listener_options(scope, &args, 2, true);
    unsafe { &mut *host_ptr }.register_abort_signal_event_listener(
        scope,
        signal,
        &parsed.event_type,
        listener,
        options,
    );
    rv.set_undefined();
}

pub(crate) fn abort_signal_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_undefined();
        return;
    }
    let Some(parsed) = webidl::parse_args::<AbortSignalRemoveEventListenerArgs>(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let Some(listener) = parsed.listener else {
        rv.set_undefined();
        return;
    };
    let capture = webidl::event_listener_options(scope, &args, 2, true).capture;
    unsafe { &mut *host_ptr }.unregister_abort_signal_event_listener(
        scope,
        signal,
        &parsed.event_type,
        &listener,
        capture,
    );
    rv.set_undefined();
}

pub(crate) fn abort_signal_dispatch_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_bool(false);
        return;
    };
    let signal = args.this();
    let Some(parsed) = webidl::parse_args::<AbortSignalDispatchEventArgs<'s>>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    let Ok(event) = v8::Local::<v8::Object>::try_from(parsed.event) else {
        rv.set_bool(false);
        return;
    };
    let Some(signal_id) = AbortStore::signal_id_from_object(scope, signal) else {
        rv.set_bool(false);
        return;
    };
    let Some(event_type) = event
        .get(scope, v8str(scope, "type").into())
        .and_then(|value| value.to_string(scope))
        .map(|s| s.to_rust_string_lossy(scope))
    else {
        rv.set_bool(false);
        return;
    };
    let default_prevented_key = v8str(scope, "defaultPrevented");
    // `dispatchEvent(...)` receives both `this` and the event object from the V8 callback frame.
    // Re-root them into the current scope before invoking user handlers so this path can share the
    // same reporting helper as the Rust-driven `dispatch_abort(...)` path without narrowing the
    // callback signature that `FunctionTemplate::builder(...)` expects.
    let signal = local_object_in_scope(scope, signal);
    let event = local_object_in_scope(scope, event);
    AbortStore::define_hidden_value(scope, event, "target", signal.into());
    AbortStore::define_hidden_value(scope, event, "currentTarget", signal.into());
    let Some(dispatch_snapshot) = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .abort
        .signal_state_mut(signal_id)
        .map(|state| state.take_dispatch_snapshot(&event_type))
    else {
        rv.set_bool(false);
        return;
    };
    invoke_abort_event_callbacks(
        scope,
        host_ptr,
        signal,
        signal_id,
        dispatch_snapshot,
        &event_type,
        event,
    );
    let result = event
        .get(scope, default_prevented_key.into())
        .is_none_or(|value| !value.boolean_value(scope));
    rv.set_bool(result);
}

pub(crate) fn abort_signal_aborted_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_bool(false);
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_bool(false);
        return;
    }
    let aborted = AbortStore::signal_aborted_from_object(scope, signal);
    rv.set_bool(aborted);
}

pub(crate) fn abort_signal_reason_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_undefined();
        return;
    }
    let Some(reason) = AbortStore::signal_reason_from_object(scope, signal) else {
        rv.set_undefined();
        return;
    };
    rv.set(reason);
}

pub(crate) fn abort_signal_onabort_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_null();
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_null();
        return;
    }
    let Some(onabort) = AbortStore::signal_id_from_object(scope, signal)
        .and_then(|id| {
            unsafe { &mut *host_ptr }
                .native_bridge_mut()
                .abort
                .signal_state(id)
        })
        .filter(|state| !state.detached)
        .and_then(|state| state.onabort.as_ref())
        .map(|onabort| v8::Local::new(scope, &onabort.callback))
    else {
        rv.set_null();
        return;
    };
    rv.set(onabort.into());
}

pub(crate) fn abort_signal_onabort_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_undefined();
        return;
    }
    let Some(signal_id) = AbortStore::signal_id_from_object(scope, signal) else {
        rv.set_undefined();
        return;
    };
    if let Some(state) = unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .abort
        .signal_state_mut(signal_id)
        .filter(|state| !state.detached)
    {
        state.onabort = v8::Local::<v8::Function>::try_from(args.get(0))
            .ok()
            .map(|function| super::AbortEventHandler {
                callback: v8::Global::new(scope, function),
                owner_realm: AbortStore::function_owner_realm(scope, function),
            });
    }
    rv.set_undefined();
}

pub(crate) fn abort_signal_throw_if_aborted_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let signal = args.this();
    if AbortStore::signal_id_from_object(scope, signal).is_none() {
        rv.set_undefined();
        return;
    }
    let Some(reason) = AbortStore::signal_aborted_from_object(scope, signal)
        .then(|| AbortStore::signal_reason_from_object(scope, signal))
        .flatten()
    else {
        rv.set_undefined();
        return;
    };
    // DOM requires throwing the stored abort reason itself. In particular, a
    // string reason remains a string rather than being wrapped in `Error`.
    scope.throw_exception(reason);
}
