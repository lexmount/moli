use super::*;
use crate::context_bootstrap::{
    EventHandlerType, event_target_dispatch, events, invoke_simple_event_target_listeners,
};
use crate::util::get_private_object;

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap::indexed_db) struct IdbEventDispatchResult {
    pub(in crate::context_bootstrap::indexed_db) uncanceled: bool,
    pub(in crate::context_bootstrap::indexed_db) did_throw: bool,
}

impl Default for IdbEventDispatchResult {
    fn default() -> Self {
        Self {
            uncanceled: true,
            did_throw: false,
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_idb_named_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    extras: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>),
) -> IdbEventDispatchResult {
    let Some(event) = events::construct_original_event(scope, event_type) else {
        return IdbEventDispatchResult::default();
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
    dispatch_idb_event_object(scope, target, event, event_type)
}

pub(crate) fn dispatch_indexed_db_script_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((event, event_type)) =
        event_target_dispatch::prepare_script_dispatch(scope, args.this(), args.get(0))
    else {
        return;
    };
    // Script dispatch shares DOM propagation, but never performs a database
    // request's default abort action or activates a transaction.
    let result = dispatch_idb_event_object(scope, args.this(), event, &event_type);
    rv.set_bool(result.uncanceled);
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_idb_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> IdbEventDispatchResult {
    let mut result = IdbEventDispatchResult::default();
    let mut path = vec![target];
    while let Some(parent) = get_private_object(scope, *path.last().unwrap(), EVENT_PARENT_SLOT) {
        path.push(parent);
    }
    let owner_scope = indexed_db_typed_owner_scope(scope, target)
        .expect("IDB event target should have typed owner state");
    let previous_owner = owner_scope.enter(scope);
    event_target_dispatch::begin_dispatch(scope, target, event);
    let values: Vec<v8::Local<'s, v8::Value>> =
        path.iter().map(|target| (*target).into()).collect();
    let composed_path = v8::Array::new_with_elements(scope, &values);
    events::set_event_composed_path(scope, event, composed_path);
    let bubbles = object_bool_property(scope, event, "bubbles").unwrap_or(false);
    let arguments = [event.into()];
    for capture in [true, false] {
        let indices: Vec<usize> = if capture {
            (0..path.len()).rev().collect()
        } else {
            (0..path.len()).collect()
        };
        for index in indices {
            if events::event_internal_bool_flag(scope, event, events::EVENT_STOP_PROPAGATION_SLOT) {
                break;
            }
            if !capture && index != 0 && !bubbles {
                continue;
            }
            let current_target = path[index];
            let phase = if index == 0 {
                2
            } else if capture {
                1
            } else {
                3
            };
            let _ = event.set(
                scope,
                v8str(scope, "currentTarget").into(),
                current_target.into(),
            );
            let _ = event.set(
                scope,
                v8str(scope, "eventPhase").into(),
                v8::Integer::new(scope, phase).into(),
            );
            let invoked = invoke_simple_event_target_listeners(
                scope,
                current_target,
                INDEXED_DB_EVENT_LISTENERS_SLOT,
                event_type,
                event,
                capture,
                &arguments,
                EventHandlerType::EventHandler,
                None,
            );
            result.did_throw |= invoked.did_throw;
        }
    }
    result.uncanceled = !object_bool_property(scope, event, "defaultPrevented").unwrap_or(false);
    event_target_dispatch::finish_dispatch(scope, event);
    owner_scope.defer_restore(scope, previous_owner);
    result
}
