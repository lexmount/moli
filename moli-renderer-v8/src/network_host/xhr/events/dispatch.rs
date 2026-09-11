use super::super::*;
use crate::context_bootstrap::{
    dispatch_simple_event_target_event, simple_object_event_listeners_snapshot,
};
use crate::util::get_private_value;
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
    ready_state: u32,
) {
    // Many XHR entrypoints receive `xhr` through different callback/value paths, so its
    // `Local` lifetime is not always tied to the current scope. Re-root it here before we
    // hand it to the shared callback-reporting helper, which expects scope-owned locals.
    let xhr = local_object_in_scope(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, ready_state as f64);
    if !xhr_has_event_observers(scope, xhr, "readystatechange") {
        return;
    }

    let event = XhrReadyStateChangeEventDeclaration::new("readystatechange", xhr, xhr)
        .bind(scope)
        .expect("XHR readystatechange event declaration should bind");
    if xhr_uses_simple_event_target(scope, xhr) {
        let _ = dispatch_simple_event_target_event(
            scope,
            xhr,
            XHR_SIMPLE_EVENT_TARGET_LISTENERS_SLOT,
            "readystatechange",
            event,
        );
        return;
    }

    let handler_val = match xhr.get(scope, v8str(scope, "onreadystatechange").into()) {
        Some(v) => v,
        None => return,
    };
    let Ok(handler) = v8::Local::<v8::Function>::try_from(handler_val) else {
        return;
    };
    let _ = invoke_callback(
        scope,
        "XMLHttpRequest.onreadystatechange",
        handler,
        xhr.into(),
        &[event.into()],
    );
}

pub(crate) fn xhr_invoke_handler(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    handler_name: &str,
    event: v8::Local<'_, v8::Object>,
) {
    // `event` objects may be created in this scope or passed through other helpers. Re-root
    // both sides before reporting so the structured logging path can use one coherent scope.
    let xhr = local_object_in_scope(scope, xhr);
    let event = local_object_in_scope(scope, event);
    if xhr_uses_simple_event_target(scope, xhr) {
        let event_type = handler_name.strip_prefix("on").unwrap_or(handler_name);
        let _ = dispatch_simple_event_target_event(
            scope,
            xhr,
            XHR_SIMPLE_EVENT_TARGET_LISTENERS_SLOT,
            event_type,
            event,
        );
        return;
    }
    let Some(key) = v8_string(scope, handler_name) else {
        return;
    };
    let handler_val = match xhr.get(scope, key.into()) {
        Some(v) => v,
        None => return,
    };
    let Ok(handler) = v8::Local::<v8::Function>::try_from(handler_val) else {
        return;
    };
    let callback_name = format!("XMLHttpRequest.{handler_name}");
    let _ = invoke_callback(scope, &callback_name, handler, xhr.into(), &[event.into()]);
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
    if !xhr_has_event_observers(scope, xhr, event_type) {
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
    let handler_name = format!("on{event_type}");
    xhr_invoke_handler(scope, xhr, &handler_name, event);
}

pub(crate) fn xhr_dispatch_upload_progress_event(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    event_type: &str,
    loaded: f64,
    total: f64,
) {
    if !xhr_state_bool_property(scope, xhr, XHR_UPLOAD_LISTENER_SLOT).unwrap_or(false) {
        return;
    }
    let Some(upload) = xhr_upload_object(scope, xhr) else {
        return;
    };
    if !xhr_has_event_observers(scope, upload, event_type) {
        return;
    }
    let event =
        super::progress::make_progress_event(scope, event_type, upload, total > 0.0, loaded, total);
    let handler_name = format!("on{event_type}");
    xhr_invoke_handler(scope, upload, &handler_name, event);
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

fn xhr_uses_simple_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> bool {
    let xhr = local_object_in_scope(scope, xhr);
    let mut current: Option<v8::Local<'s, v8::Value>> = Some(xhr.into());
    while let Some(value) = current {
        if value.is_null_or_undefined() {
            return false;
        }
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            return false;
        };
        if get_private_value(scope, object, XHR_SIMPLE_EVENT_TARGET_MARKER_SLOT)
            .is_some_and(|marker| marker.is_true())
        {
            return true;
        }
        current = object.get_prototype(scope);
    }
    false
}

fn xhr_has_event_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> bool {
    let handler_name = format!("on{event_type}");
    let has_handler = v8_string(scope, &handler_name)
        .and_then(|key| xhr.get(scope, key.into()))
        .is_some_and(|value| value.is_function());
    if has_handler {
        return true;
    }
    if !xhr_uses_simple_event_target(scope, xhr) {
        return false;
    }
    !simple_object_event_listeners_snapshot(
        scope,
        xhr,
        XHR_SIMPLE_EVENT_TARGET_LISTENERS_SLOT,
        event_type,
    )
    .is_empty()
}
