use super::{
    construct_original_event, dispatch_simple_event_target_event, initialize_simple_event_target,
    shared_event_targets::resolve_event_target, simple_object_event_set_handler,
};
use crate::util::{get_private_value, set_private_value};

const ONABORT_SLOT: &str = "__moliAbortSignalOnabort";

// Abort state and algorithms remain in their Window/worker owner. The signal's
// EventTarget state uses the same registry for inherited methods and native abort.
pub(crate) fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, signal: v8::Local<'s, v8::Object>) {
    initialize_simple_event_target(scope, signal);

    set_private_value(scope, signal, ONABORT_SLOT, v8::null(scope).into());
}

pub(crate) fn dispatch_abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: v8::Local<'s, v8::Object>,
) {
    let context = signal
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(event) = construct_original_event(scope, "abort") {
        super::mark_event_trusted(scope, event);
        dispatch_simple_event_target_event(scope, signal, "abort", event);
    }
}

pub(crate) fn onabort_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(signal) = resolve_event_target(scope, args.this()) else {
        return;
    };
    rv.set(
        get_private_value(scope, signal.handler_owner, ONABORT_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(crate) fn onabort_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(signal) = resolve_event_target(scope, args.this()) else {
        return;
    };
    simple_object_event_set_handler(
        scope,
        signal.handler_owner,
        "abort",
        ONABORT_SLOT,
        args.get(0),
    );
}
