//! World-local Event views over one native dispatch state.
//! Callback realms in the same wrapper world preserve the owner's identity;
//! isolated worlds receive their own wrappers without sharing page expandos.

use super::{events, platform_object_worlds, shared_event_targets, world_wrappers};
use crate::native_bridge::identity::contexts_share_wrapper_world;

fn construction_context_in_world<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    world: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Context>> {
    let creation = event.get_creation_context(scope)?;
    if contexts_share_wrapper_world(creation, world) {
        return Some(creation);
    }
    let host_ptr = crate::util::context_host_ptr_from_context_slot(creation)?;
    let host = unsafe { &*host_ptr };
    let identity = host.window_execution_context_identity_for_access_check(creation)?;
    let (_, default) =
        host.window_execution_context(scope, identity.owner(), identity.dispatch_scope())?;
    contexts_share_wrapper_world(default, world).then_some(default)
}

pub(crate) fn target_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = shared_event_targets::target_in_realm(scope, target, context);
    platform_object_worlds::in_realm(scope, target.into(), context)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(crate) fn event_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !shared_event_targets::is_shared_target(scope, target)
        && !crate::web_api_interfaces::Node::is_instance(scope, target)
        && !super::is_window_receiver(scope, target)
    {
        return Some(event);
    }
    let backing = events::event_backing(scope, event);
    let target = target_in_realm(scope, target, context)?;
    if let Some(wrapper) = world_wrappers::get(scope, backing, context) {
        return Some(wrapper);
    }
    // Project into the construction Window's counterpart when it belongs to
    // the listener's world. Adoption retains the target's prototype realm,
    // which cannot determine an independently constructed Event's realm.
    let context = construction_context_in_world(scope, backing, context)
        .or_else(|| {
            target
                .get_creation_context(scope)
                .filter(|creation| contexts_share_wrapper_world(*creation, context))
        })
        .unwrap_or(context);
    let scope = &mut v8::ContextScope::new(scope, context);
    events::new_event_wrapper(scope, backing)
}
