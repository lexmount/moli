use super::super::*;
use crate::context_bootstrap::{
    dispatch_simple_event_target_event, simple_object_event_listeners_snapshot,
};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct XhrReadyStateChangeEventDeclaration<'scope> {
    r#type: &'static str,
    target: v8::Local<'scope, v8::Object>,
    current_target: v8::Local<'scope, v8::Object>,
}

pub(in crate::network_host::xhr) fn xhr_fire_readystatechange(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    ready_state: XhrReadyState,
) {
    // Many XHR entrypoints receive `xhr` through different callback/value paths, so its
    // `Local` lifetime is not always tied to the current scope. Re-root it here before we
    // hand it to the shared callback-reporting helper, which expects scope-owned locals.
    let xhr = local_object_in_scope(scope, xhr);
    set_xhr_ready_state(scope, xhr, ready_state);
    if !xhr_has_event_observers(scope, xhr, "readystatechange")
        || super::super::lifecycle::xhr_context_is_destroyed(scope, xhr)
    {
        return;
    }

    let event = XhrReadyStateChangeEventDeclaration::new("readystatechange", xhr, xhr)
        .bind(scope)
        .expect("XHR readystatechange event declaration should bind");
    let _ = dispatch_simple_event_target_event(scope, xhr, "readystatechange", event);
}

fn xhr_dispatch_event(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    event_type: &str,
    event: v8::Local<'_, v8::Object>,
) {
    let xhr = local_object_in_scope(scope, xhr);
    let event = local_object_in_scope(scope, event);
    if super::super::lifecycle::xhr_context_is_destroyed(scope, xhr) {
        return;
    }
    let _ = dispatch_simple_event_target_event(scope, xhr, event_type, event);
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
    let xhr = local_object_in_scope(scope, xhr);
    if !xhr_has_event_observers(scope, xhr, event_type)
        || super::super::lifecycle::xhr_context_is_destroyed(scope, xhr)
    {
        return;
    }
    let event = super::progress::make_progress_event(
        scope,
        event_type,
        xhr,
        length_computable,
        loaded,
        total,
    );
    xhr_dispatch_event(scope, xhr, event_type, event);
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
    for event_type in event_types {
        if scope.is_execution_terminating() {
            return;
        }
        if !xhr_has_event_observers(scope, upload, event_type) {
            continue;
        }
        let event = super::progress::make_progress_event(
            scope,
            event_type,
            upload,
            total > 0.0,
            loaded,
            total,
        );
        xhr_dispatch_event(scope, upload, event_type, event);
    }
}

fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    // XHR helpers are reached from a mix of ordinary Rust entrypoints and V8 callback trampolines.
    // Those paths do not guarantee that the incoming `Local<Object>` already carries the exact same
    // lifetime as the current scope. Most V8 operations are fine with that, but our structured
    // callback-reporting helper intentionally works with one coherent scope lifetime so it can build
    // `TryCatch`, inspect message/stack state, and keep the call contract explicit.
    //
    // Re-rooting through a short-lived `Global` is the least invasive way to normalize the value
    // back into the current scope without changing all caller signatures or tightening V8 callback
    // template types.
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

fn xhr_has_event_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> bool {
    !simple_object_event_listeners_snapshot(scope, xhr, event_type).is_empty()
}
