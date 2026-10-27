use super::super::*;
use crate::context_bootstrap::{
    construct_original_event, dispatch_simple_event_target_event,
    simple_object_event_listeners_snapshot,
};

pub(in crate::network_host::xhr) fn xhr_fire_readystatechange(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    ready_state: XhrReadyState,
) {
    let xhr = v8::Local::new(scope, xhr);
    set_xhr_ready_state(scope, xhr, ready_state);
    if !xhr_has_event_observers(scope, xhr, "readystatechange")
        || super::super::lifecycle::xhr_context_is_destroyed(scope, xhr)
    {
        return;
    }

    let Some(context) = xhr.get_creation_context(scope) else {
        return;
    };
    // Borrowed open/send methods must not change the event's creation realm.
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(event) = construct_original_event(scope, "readystatechange") else {
        return;
    };
    let _ = dispatch_simple_event_target_event(scope, xhr, "readystatechange", event);
}

pub(crate) fn xhr_dispatch_progress_event(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    event_type: &str,
    loaded: f64,
    total: f64,
) {
    xhr_dispatch_progress_event_with_length_computable(
        scope,
        xhr,
        event_type,
        loaded > 0.0 || total > 0.0,
        loaded,
        total,
    );
}

pub(crate) fn xhr_dispatch_progress_event_with_length_computable(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    event_type: &str,
    length_computable: bool,
    loaded: f64,
    total: f64,
) {
    let xhr = v8::Local::new(scope, xhr);
    if !xhr_has_event_observers(scope, xhr, event_type)
        || super::super::lifecycle::xhr_context_is_destroyed(scope, xhr)
    {
        return;
    }
    let Some(context) = xhr.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(event) = super::super::bindings::construct_native_progress_event(
        scope,
        event_type,
        length_computable,
        loaded,
        total,
    ) else {
        return;
    };
    let _ = dispatch_simple_event_target_event(scope, xhr, event_type, event);
}

pub(crate) fn xhr_dispatch_upload_progress_event(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    event_type: &str,
    loaded: f64,
    total: f64,
) {
    xhr_dispatch_upload_progress_events(scope, xhr, &[event_type], loaded, total);
}

pub(crate) fn xhr_dispatch_upload_progress_events(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    event_types: &[&str],
    loaded: f64,
    total: f64,
) {
    if !xhr_state_bool_property(scope, xhr, XHR_UPLOAD_LISTENER_SLOT).unwrap_or(false)
        || super::super::lifecycle::xhr_context_is_destroyed(scope, xhr)
    {
        return;
    }
    let Some(upload) = xhr_upload_object(scope, xhr) else {
        return;
    };
    // The listener flag is tested once on entry to the event algorithm, even
    // when an earlier event reopens the XHR and resets its request state.
    for &event_type in event_types {
        if scope.is_execution_terminating() {
            return;
        }
        xhr_dispatch_progress_event_with_length_computable(
            scope,
            upload,
            event_type,
            event_type == "loadstart" || total > 0.0,
            loaded,
            total,
        );
    }
}

fn xhr_has_event_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> bool {
    !simple_object_event_listeners_snapshot(scope, xhr, event_type).is_empty()
}
