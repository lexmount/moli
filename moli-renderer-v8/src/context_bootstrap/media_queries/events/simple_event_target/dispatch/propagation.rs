use super::*;
use crate::context_bootstrap::events::{
    EVENT_STOP_PROPAGATION_SLOT, event_backing, event_bool_attribute, set_event_composed_path,
};

/// Dispatch a non-DOM target's explicit parent chain using the same listener
/// snapshots, callback invocation and exception reporting as standalone targets.
pub(crate) fn dispatch_simple_event_target_event_with_ancestors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &(impl EventTypeKey + ?Sized),
    event: v8::Local<'s, v8::Object>,
    ancestors: &[v8::Local<'s, v8::Object>],
) -> SimpleEventDispatchResult {
    let can_invoke =
        crate::context_bootstrap::event_target_dispatch::begin_dispatch(scope, target, event);
    let path = std::iter::once(target)
        .chain(ancestors.iter().copied())
        .collect::<Vec<_>>();
    let values = path
        .iter()
        .map(|target| (*target).into())
        .collect::<Vec<_>>();
    let composed_path = v8::Array::new_with_elements(scope, &values);
    set_event_composed_path(scope, event, composed_path);
    let mut result = SimpleEventDispatchResult::default();
    if can_invoke {
        for capture in [true, false] {
            for position in 0..path.len() {
                let index = if capture {
                    path.len() - 1 - position
                } else {
                    position
                };
                if !capture && index != 0 && !event_bool_attribute(scope, event, "bubbles") {
                    continue;
                }
                if event_internal_bool_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT) {
                    break;
                }
                let current_target = path[index];
                let backing = event_backing(scope, event);
                let current_target_in_realm = backing
                    .get_creation_context(scope)
                    .map(|context| {
                        crate::context_bootstrap::shared_event_targets::target_in_realm(
                            scope,
                            current_target,
                            context,
                        )
                    })
                    .unwrap_or(current_target);
                let _ = backing.set(
                    scope,
                    v8str(scope, "currentTarget").into(),
                    current_target_in_realm.into(),
                );
                let phase = if index == 0 {
                    2
                } else if capture {
                    1
                } else {
                    3
                };
                let _ = backing.set(
                    scope,
                    v8str(scope, "eventPhase").into(),
                    v8::Integer::new(scope, phase).into(),
                );
                let Some(target) = resolve_event_target(scope, current_target) else {
                    continue;
                };
                let Some(registry) =
                    SimpleEventListenerRegistry::from_owner(scope, target.listener_owner)
                else {
                    continue;
                };
                let invoked = invoke_simple_event_listeners(
                    scope,
                    target.dispatch_target,
                    registry,
                    event_type,
                    event,
                    Some(capture),
                    None,
                    None,
                );
                result.dispatched |= invoked.dispatched;
                result.did_listeners_throw |= invoked.did_listeners_throw;
            }
        }
    }
    result.uncanceled = finish_event_dispatch(scope, event, None);
    result
}
