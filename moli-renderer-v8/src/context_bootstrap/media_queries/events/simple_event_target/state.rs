use super::*;
use crate::context_bootstrap::shared_event_targets::{
    ResolvedEventTarget, resolve_event_target, shared_target_owner,
};
use crate::util::{get_private_object, new_null_prototype_object, set_private_value};

// A call-local view, not an event-type array snapshot. Listener arrays can be
// replaced by removals or reentrant callbacks while the registry stays shared.
#[derive(Clone, Copy)]
pub(super) struct SimpleEventListenerRegistry<'s>(pub(super) v8::Local<'s, v8::Object>);

impl<'s> SimpleEventListenerRegistry<'s> {
    pub(super) fn from_owner(
        scope: &mut v8::PinScope<'s, '_>,
        owner: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        // Read only the owner's own state; prototypes cannot supply a registry.
        get_private_object(scope, owner, SIMPLE_EVENT_TARGET_LISTENERS_SLOT).map(Self)
    }
}

pub(super) struct ResolvedSimpleEventTarget<'s> {
    pub(super) identity: ResolvedEventTarget<'s>,
    pub(super) registry: SimpleEventListenerRegistry<'s>,
}

pub(crate) fn initialize_simple_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) {
    let target = shared_target_owner(scope, target);
    if get_private_object(scope, target, SIMPLE_EVENT_TARGET_LISTENERS_SLOT).is_none() {
        let registry = new_null_prototype_object(scope);
        set_private_value(
            scope,
            target,
            SIMPLE_EVENT_TARGET_LISTENERS_SLOT,
            registry.into(),
        );
    }
}

pub(crate) fn is_simple_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    resolve_simple_event_target(scope, target).is_some()
}

pub(super) fn resolve_simple_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<ResolvedSimpleEventTarget<'s>> {
    // Resolve registered native Proxies only for the simple-target branch.
    // DOM and Window targets keep their existing receiver resolution.
    let identity = resolve_event_target(scope, target)?;
    let registry = SimpleEventListenerRegistry::from_owner(scope, identity.listener_owner)?;
    Some(ResolvedSimpleEventTarget { identity, registry })
}

pub(super) fn simple_object_event_listener_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<SimpleEventListenerRegistry<'s>> {
    let target = shared_target_owner(scope, target);
    SimpleEventListenerRegistry::from_owner(scope, target)
}
