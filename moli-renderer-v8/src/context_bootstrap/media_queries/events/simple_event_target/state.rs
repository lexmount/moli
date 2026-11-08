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

pub(crate) struct ResolvedSimpleEventTarget<'s> {
    pub(super) identity: ResolvedEventTarget<'s>,
    pub(super) registry: SimpleEventListenerRegistry<'s>,
}

impl<'s> ResolvedSimpleEventTarget<'s> {
    pub(crate) fn from_identity(
        scope: &mut v8::PinScope<'s, '_>,
        identity: ResolvedEventTarget<'s>,
    ) -> Option<Self> {
        let registry = SimpleEventListenerRegistry::from_owner(scope, identity.listener_owner)?;
        Some(Self { identity, registry })
    }

    pub(crate) fn add_event_listener(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
    ) {
        simple_object_event_target_add_listener_for_target(
            scope,
            args,
            self.identity.listener_owner,
            self.registry,
        );
    }

    pub(crate) fn remove_event_listener(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
    ) {
        simple_object_event_target_remove_listener_in_registry(scope, args, self.registry);
    }

    pub(crate) fn dispatch_event(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        rv: &mut v8::ReturnValue<'s, v8::Value>,
    ) {
        simple_object_event_target_dispatch(scope, args, self, rv);
    }
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

pub(super) fn resolve_simple_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<ResolvedSimpleEventTarget<'s>> {
    let identity = resolve_event_target(scope, target)?;
    ResolvedSimpleEventTarget::from_identity(scope, identity)
}

pub(super) fn simple_object_event_listener_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<SimpleEventListenerRegistry<'s>> {
    let target = shared_target_owner(scope, target);
    SimpleEventListenerRegistry::from_owner(scope, target)
}
