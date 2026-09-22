use super::{
    construct_original_event, dispatch_simple_event_target_event,
    install_simple_event_target_ordered_handlers, mark_simple_event_target_slot,
    simple_object_event_set_ordered_handler,
};
use crate::util::{get_private_value, set_private_value};

const LISTENERS_SLOT: &str = "__moliAbortSignalListeners";
const ONABORT_SLOT: &str = "__moliAbortSignalOnabort";

// Abort state and algorithms remain in their Window/worker owner. The signal's
// EventTarget state uses the same registry for inherited methods and native abort.
pub(crate) fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, signal: v8::Local<'s, v8::Object>) {
    mark_simple_event_target_slot(scope, signal, LISTENERS_SLOT);
    install_simple_event_target_ordered_handlers(scope, signal);
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
        dispatch_simple_event_target_event(scope, signal, LISTENERS_SLOT, "abort", event);
    }
}

pub(crate) fn onabort_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(
        get_private_value(scope, args.this(), ONABORT_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(crate) fn onabort_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    let signal = args.this();
    set_private_value(scope, signal, ONABORT_SLOT, value);
    simple_object_event_set_ordered_handler(
        scope,
        signal,
        LISTENERS_SLOT,
        "abort",
        ONABORT_SLOT,
        active,
    );
}
