use super::*;
use crate::context_bootstrap::dispatch_simple_event_target_event;
use crate::context_bootstrap::events;

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
    let Some(event_type) = crate::context_bootstrap::event_attribute(scope, event, "type")
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return true;
    };
    let owner_scope = idb_event_target_owner_scope(scope, target);
    let previous_owner = owner_scope.enter(scope);
    let uncanceled = dispatch_simple_event_target_event(scope, target, &event_type, event);

    owner_scope.defer_restore(scope, previous_owner);
    uncanceled
}

fn idb_event_target_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> crate::native_bridge::OwnerDispatchScope {
    indexed_db_typed_owner_scope(scope, target)
        .expect("IDB event target should have typed owner state")
}
