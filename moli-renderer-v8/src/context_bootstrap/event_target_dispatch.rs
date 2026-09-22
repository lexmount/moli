use super::events::{clear_event_dispatch_fields, set_event_dispatch_fields_with_original_target};
use super::{
    EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, EVENT_STOP_PROPAGATION_SLOT,
    clear_event_composed_path, event_initialized, event_internal_bool_flag, event_is_dispatching,
    new_dom_exception_value, set_event_composed_path, set_event_internal_flag, set_event_trusted,
};
use crate::util::{throw_type_error, v8str};

pub(crate) fn prepare_script_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, String)> {
    let event = v8::Local::<v8::Object>::try_from(value).ok();
    let Some((event, initialized)) =
        event.and_then(|event| event_initialized(scope, event).map(|flag| (event, flag)))
    else {
        throw_type_error(scope, "EventTarget.dispatchEvent requires an Event.");
        return None;
    };
    if !initialized || event_is_dispatching(scope, event) {
        let error = new_dom_exception_value(
            scope,
            "The event is uninitialized or already being dispatched.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return None;
    }
    if !target_execution_context_is_live(scope, target) {
        return None;
    }
    set_event_trusted(scope, event, false);
    let event_type = event
        .get(scope, v8str(scope, "type").into())?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    Some((event, event_type))
}

pub(crate) fn target_execution_context_is_live<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(context) = target.get_creation_context(scope) else {
        return true;
    };
    if context
        .get_slot::<crate::native_bridge::RuntimeObservableContextToken>()
        .is_none()
    {
        return true;
    }
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return true;
    };
    // A borrowed method still dispatches on the receiver's relevant realm.
    // Retained objects must not resume execution after that realm is retired.
    let host = unsafe { &*host_ptr };
    host.window_execution_context_identity_for_v8_context(scope, context)
        .is_some_and(|identity| host.window_execution_context_identity_is_current(identity))
}

// Even a pre-stopped event needs finish_dispatch, but its listeners must not run.
pub(crate) fn begin_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    begin_dispatch_with_original_target(scope, target, target, event)
}

pub(crate) fn begin_dispatch_with_original_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    original_target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    // HTML's legacy target override changes Event.target, not the dispatch
    // path or the Window whose listeners are invoked.
    set_event_dispatch_fields_with_original_target(scope, target, original_target, event);
    let path = v8::Array::new_with_elements(scope, &[target.into()]);
    set_event_composed_path(scope, event, path);
    !event_internal_bool_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT)
}

pub(crate) fn finish_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    clear_event_dispatch_fields(scope, event);
    clear_event_composed_path(scope, event);
    for flag in [
        EVENT_STOP_PROPAGATION_SLOT,
        EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT,
        EVENT_PASSIVE_SLOT,
    ] {
        set_event_internal_flag(scope, event, flag, false);
    }
}
