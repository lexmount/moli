//! A single listener list behind world-local Navigation and entry wrappers.
//! The list retains callback realms using the ordinary EventTarget machinery;
//! dispatch still runs once, with one cancellation/propagation state.

use super::{history_runtime::native, media_queries};
use crate::context_bootstrap::navigation_entry::wrappers as entry_wrappers;
use crate::util::{get_private_object, get_private_value, set_private_value, v8str};

const OWNER: &str = "__moliSharedEventTargetOwner";
const WRAPPERS: &str = "__moliSharedEventTargetWrappers";
const HANDLERS: [(&str, &str); 5] = [
    ("onnavigate", "navigate"),
    ("onnavigatesuccess", "navigatesuccess"),
    ("onnavigateerror", "navigateerror"),
    ("oncurrententrychange", "currententrychange"),
    ("ondispose", "dispose"),
];

pub(super) fn is_shared_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_object(scope, target, OWNER).is_some()
}

pub(super) fn shared_target_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_object(scope, target, OWNER).unwrap_or(target)
}

pub(super) fn bind_shared_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    let owner = shared_target_owner(scope, owner);
    set_private_value(scope, wrapper, OWNER, owner.into());
    if native::entry(scope, wrapper).is_none()
        && !crate::web_api_interfaces::AbortSignal::is_instance(scope, wrapper)
    {
        let context = wrapper
            .get_creation_context(scope)
            .expect("Navigation realm");
        let global = context.global(scope);
        let wrappers = get_private_value(scope, global, WRAPPERS)
            .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
            .unwrap_or_else(|| {
                let map = v8::Map::new(scope);
                set_private_value(scope, global, WRAPPERS, map.into());
                map
            });
        let _ = wrappers.set(scope, owner.into(), wrapper.into());
    }
}

pub(super) fn target_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Object> {
    if let Some(window) = window_target_in_realm(scope, target, context) {
        return window;
    }
    if crate::web_api_interfaces::Node::is_instance(scope, target) {
        return super::platform_object_worlds::in_realm(scope, target.into(), context)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .unwrap_or(target);
    }
    let Some(owner) = get_private_object(scope, target, OWNER) else {
        return target;
    };
    if crate::web_api_interfaces::AbortSignal::is_instance(scope, owner) {
        return super::platform_object_worlds::in_realm(scope, owner.into(), context)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .unwrap_or(target);
    }
    if native::entry(scope, target).is_some() {
        let window = super::navigation_window::runtime_window_owner(scope, owner);
        let navigation = super::navigation_window::window_navigation_for_holder(scope, window);
        let context = navigation
            .map(|navigation| target_in_realm(scope, navigation, context))
            .and_then(|navigation| navigation.get_creation_context(scope))
            .unwrap_or(context);
        return entry_wrappers::in_realm(scope, owner, context);
    }
    let global = context.global(scope);
    get_private_value(scope, global, WRAPPERS)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .and_then(|map| map.get(scope, owner.into()))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or(target)
}

/// A Window's view belongs to the target Window in the accessing world. A
/// borrowed child callback must never replace a parent with the child global.
pub(super) fn window_target_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    match crate::native_bridge::window_in_wrapper_world(scope, target, context) {
        Ok(window) => window,
        Err(error) => {
            super::throw_error(scope, &format!("Window world projection failed: {error}"));
            None
        }
    }
}

/// Select the listener world's wrapper only for a Node owned by this live
/// Window. A wrapper's creation Context is not its native ownerDocument:
/// adoption and borrowed callbacks can make those belong to different Windows.
pub(super) fn node_target_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !crate::web_api_interfaces::Node::is_instance(scope, target) {
        return None;
    }
    let (host_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, target)
            .ok()?;
    if crate::util::context_host_ptr_from_context_slot(context) != Some(host_ptr) {
        return None;
    }
    let host = unsafe { &*host_ptr };
    let identity = host.window_execution_context_identity_for_access_check(context)?;
    let owner_scope = host.owner_dispatch_scope_for_node(handle)?;
    if identity.dispatch_scope() != owner_scope
        || !host.window_execution_context_identity_is_current(identity)
    {
        return None;
    }
    super::platform_object_worlds::in_realm(scope, target.into(), context)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn install_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    entry: bool,
) {
    bind_shared_target(scope, target, target);
    for (index, (property, _)) in HANDLERS.iter().enumerate() {
        if (*property == "ondispose") != entry {
            continue;
        }
        let data = v8::Integer::new(scope, index as i32);
        let getter = v8::Function::builder(handler_getter)
            .data(data.into())
            .build(scope)
            .unwrap();
        let setter = v8::Function::builder(handler_setter)
            .data(data.into())
            .build(scope)
            .unwrap();
        crate::definitions::define_get_set_property(
            scope,
            target,
            v8str(scope, property).into(),
            getter.into(),
            setter.into(),
            v8::PropertyAttribute::NONE,
            "Navigation event handler",
        )
        .expect("Navigation handler accessor");
    }
}

fn handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let index = args.data().int32_value(scope).unwrap_or(-1) as usize;
    if let Some((slot, _)) = HANDLERS.get(index) {
        rv.set(
            get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::null(scope).into()),
        );
    }
}

fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let index = args.data().int32_value(scope).unwrap_or(-1) as usize;
    let Some((slot, event_type)) = HANDLERS.get(index) else {
        return;
    };
    let target = args.this();
    media_queries::simple_object_event_set_handler(scope, target, event_type, slot, args.get(0));
}
