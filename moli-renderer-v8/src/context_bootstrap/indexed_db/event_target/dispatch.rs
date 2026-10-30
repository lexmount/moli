use super::*;
use crate::context_bootstrap::{dispatch_simple_event_target_event_with_ancestors, events};

pub(in crate::context_bootstrap::indexed_db) struct IdbEventDispatchResult {
    pub(in crate::context_bootstrap::indexed_db) uncanceled: bool,
    pub(in crate::context_bootstrap::indexed_db) did_listeners_throw: bool,
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_idb_named_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    extras: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>),
) -> bool {
    dispatch_idb_named_event_with_result(scope, target, event_type, extras).uncanceled
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_idb_named_event_with_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    extras: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>),
) -> IdbEventDispatchResult {
    let Some(event) = events::construct_original_event(scope, event_type) else {
        return IdbEventDispatchResult {
            uncanceled: true,
            did_listeners_throw: false,
        };
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
    dispatch_idb_event_object_with_result(scope, target, event)
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_idb_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    dispatch_idb_event_object_with_result(scope, target, event).uncanceled
}

pub(super) fn dispatch_idb_event_object_with_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> IdbEventDispatchResult {
    let Some(event_type) = crate::context_bootstrap::event_attribute(scope, event, "type")
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return IdbEventDispatchResult {
            uncanceled: true,
            did_listeners_throw: false,
        };
    };
    let mut parents = Vec::new();
    let transaction = if indexed_db_typed_wrapper_is(scope, target, IndexedDbWrapperKind::Request) {
        indexed_db_request_transaction_object(scope, target)
    } else if indexed_db_typed_wrapper_is(scope, target, IndexedDbWrapperKind::Transaction) {
        Some(target)
    } else {
        // Open requests always have a null event parent, even during upgrade.
        None
    };
    if let Some(transaction) = transaction {
        if transaction != target {
            parents.push(transaction);
        }
        if let Some(database) = indexed_db_transaction_database(scope, transaction) {
            parents.push(database);
        }
    }
    let owner_scope = idb_event_target_owner_scope(scope, target);
    let previous_owner = owner_scope.enter(scope);
    let outcome = dispatch_simple_event_target_event_with_ancestors(
        scope,
        target,
        &event_type,
        event,
        &parents,
        Some(idb_event_callback_checkpoint),
    );
    owner_scope.defer_restore(scope, previous_owner);
    IdbEventDispatchResult {
        uncanceled: outcome.uncanceled,
        did_listeners_throw: outcome.did_listeners_throw,
    }
}

fn idb_event_target_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> crate::native_bridge::OwnerDispatchScope {
    indexed_db_typed_owner_scope(scope, target)
        .expect("IDB event target should have typed owner state")
}

// Native IndexedDB callbacks finish their microtasks before the next listener
// and before request default actions. Script dispatch keeps its outer boundary.
fn idb_event_callback_checkpoint(scope: &mut v8::PinScope<'_, '_>) {
    if !scope.is_execution_terminating()
        && scope
            .get_current_context()
            .get_microtask_queue()
            .is_some_and(|queue| {
                queue.get_microtasks_scope_depth() == 0 && !queue.is_running_microtasks()
            })
    {
        crate::script_vm::perform_callback_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
}
