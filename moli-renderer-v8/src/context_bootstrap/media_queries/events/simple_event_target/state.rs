use super::*;
use crate::context_bootstrap::shared_event_targets::shared_target_owner;
use crate::util::{get_private_object, new_null_prototype_object, set_private_value};

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
    simple_event_target_receiver(scope, target).is_some()
}

pub(super) fn simple_event_target_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    // Resolve registered native Proxies only for the simple-target branch.
    // DOM and Window targets keep their existing receiver resolution.
    let target = if target.is_proxy() {
        moli_webapi_declare::web_api_object_target(scope, target)?
    } else {
        target
    };
    simple_object_event_listener_registry(scope, target).map(|_| target)
}

pub(super) fn simple_object_event_listener_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = shared_target_owner(scope, target);
    // Read only the target's own private state. Looking through prototypes would
    // let instances inherit another target's listener list and receiver identity.
    get_private_object(scope, target, SIMPLE_EVENT_TARGET_LISTENERS_SLOT)
}
