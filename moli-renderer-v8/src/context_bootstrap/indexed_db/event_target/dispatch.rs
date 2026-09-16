use super::*;
use crate::context_bootstrap::events;
use crate::context_bootstrap::events::{clear_event_dispatch_fields, set_event_dispatch_fields};
use crate::exception_reporting::invoke_event_handler;

pub(in crate::context_bootstrap::indexed_db) fn dispatch_idb_named_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    extras: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>),
) -> bool {
    let Some(event) = events::construct_original_event(scope, event_type) else {
        return true;
    };
    events::initialize_event_object(
        scope,
        event,
        event_type,
        matches!(event_type, "error" | "abort"),
        event_type == "error",
    );
    events::mark_event_trusted(scope, event);
    extras(scope, event);
    dispatch_idb_event_object(scope, target, event)
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_idb_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(event_type) = object_string_property_defined(scope, event, "type") else {
        return true;
    };
    let listeners = event_listener_snapshot(scope, target, &event_type);
    set_event_dispatch_fields(scope, target, event);
    let owner_scope = idb_event_target_owner_scope(scope, target);
    let previous_owner = owner_scope.enter(scope);

    let handler_name = format!("on{event_type}");
    if let Some(handler_key) = v8_string(scope, &handler_name)
        && let Some(handler_value) = target.get(scope, handler_key.into())
        && let Ok(handler) = v8::Local::<v8::Function>::try_from(handler_value)
    {
        // IndexedDB requests/transactions are still EventTarget dispatch. Page exceptions should
        // be reported, but they must not abort delivery to later listeners or flip the operation
        // into a host/runtime failure.
        let _ = invoke_event_handler(
            scope,
            &handler_name,
            handler,
            target.into(),
            &[event.into()],
        );
    }

    for listener in listeners {
        let listener_name = format!("{event_type} listener");
        let _ = invoke_event_handler(
            scope,
            &listener_name,
            listener,
            target.into(),
            &[event.into()],
        );
    }

    clear_event_dispatch_fields(scope, event);
    owner_scope.defer_restore(scope, previous_owner);
    !object_bool_property(scope, event, "defaultPrevented").unwrap_or(false)
}

fn idb_event_target_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> crate::native_bridge::OwnerDispatchScope {
    indexed_db_typed_owner_scope(scope, target)
        .expect("IDB event target should have typed owner state")
}
