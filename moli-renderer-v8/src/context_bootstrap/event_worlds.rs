//! World-local Event views over one native dispatch state.
//! Callback realms in the same wrapper world preserve the owner's identity;
//! isolated worlds receive their own wrappers without sharing page expandos.

use super::{events, platform_object_worlds, shared_event_targets, world_wrappers};
use crate::native_bridge::identity::contexts_share_wrapper_world;

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
    // Preserve owner identity within a wrapper world. A foreign Window without
    // a local view must not move the Event into that Window's different world.
    let context = target
        .get_creation_context(scope)
        .filter(|creation| contexts_share_wrapper_world(*creation, context))
        .unwrap_or(context);
    if let Some(wrapper) = world_wrappers::get(scope, backing, context) {
        return Some(wrapper);
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    events::new_event_wrapper(scope, backing)
}
