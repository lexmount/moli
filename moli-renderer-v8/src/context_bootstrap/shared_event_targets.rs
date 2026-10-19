//! Resolve object-backed EventTarget identities independently of realm projection.
//! Shared wrappers retain one listener list and one dispatch state, while each
//! wrapper keeps its own EventHandler properties and callback realms.
//! A fixed dispatch target belongs to the native receiver, independently of
//! its listener owner. Sharing a registry never inherits another receiver's
//! fixed target; registered native Proxies use their backing receiver's policy.

use super::{history_runtime::native, media_queries};
use crate::context_bootstrap::navigation_entry::wrappers as entry_wrappers;
use crate::util::{get_private_object, get_private_value, set_private_value, v8str};

const OWNER: &str = "__moliSharedEventTargetOwner";
const WRAPPERS: &str = "__moliSharedEventTargetWrappers";
const DISPATCH_TARGET: &str = "__moliEventTargetDispatchTarget";
const HANDLERS: [(&str, &str); 5] = [
    ("onnavigate", "navigate"),
    ("onnavigatesuccess", "navigatesuccess"),
    ("onnavigateerror", "navigateerror"),
    ("oncurrententrychange", "currententrychange"),
    ("ondispose", "dispose"),
];

pub(super) struct ResolvedEventTarget<'s> {
    pub(super) listener_owner: v8::Local<'s, v8::Object>,
    // EventHandler values belong to the native receiver's wrapper, even when
    // its listener registry is shared with wrappers from other worlds.
    pub(super) handler_owner: v8::Local<'s, v8::Object>,
    // A logical target: realm projection and DOM retargeting remain separate.
    pub(super) dispatch_target: v8::Local<'s, v8::Object>,
}

fn native_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if target.is_proxy() {
        moli_webapi_declare::web_api_object_target(scope, target)
    } else {
        Some(target)
    }
}

pub(super) fn resolve_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<ResolvedEventTarget<'s>> {
    let handler_owner = native_target(scope, receiver)?;
    Some(ResolvedEventTarget {
        listener_owner: get_private_object(scope, handler_owner, OWNER).unwrap_or(handler_owner),
        handler_owner,
        dispatch_target: get_private_object(scope, handler_owner, DISPATCH_TARGET)
            .unwrap_or(receiver),
    })
}

/// Set this native receiver's logical dispatch target. Shared wrappers keep
/// their own policy, regardless of the order of this and `bind_shared_target`.
/// Bind each receiver explicitly if several wrappers need the same fixed target.
pub(crate) fn bind_dispatch_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    dispatch_target: v8::Local<'s, v8::Object>,
) {
    let target = native_target(scope, target).expect("native event target binding");
    set_private_value(scope, target, DISPATCH_TARGET, dispatch_target.into());
}

pub(super) fn is_shared_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(target) = native_target(scope, target) else {
        return false;
    };
    get_private_object(scope, target, OWNER).is_some()
}

pub(super) fn shared_target_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    // Native aliases share the backing registry while dispatch keeps the
    // original wrapper for callback this and Event.target.
    let target = native_target(scope, target).unwrap_or(target);
    get_private_object(scope, target, OWNER).unwrap_or(target)
}

/// Share listener storage and register the wrapper for realm projection.
/// This does not copy or inherit the owner's fixed dispatch target or handlers.
pub(crate) fn bind_shared_target<'s>(
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
    if let Some(node) = node_target_in_realm(scope, target, context) {
        return node;
    }
    let receiver = target;
    let target = native_target(scope, target).unwrap_or(target);
    let Some(owner) = get_private_object(scope, target, OWNER) else {
        // Ordinary native aliases retain their own public identity. Shared
        // platform objects instead project the native receiver's world wrapper.
        return receiver;
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

/// Only another Realm of this exact live Window can replace a real global.
/// Cross-Document aliases keep their existing behavior. In particular, the
/// current parent/top alias path has no target-Window isolated-world resolver;
/// this helper must not manufacture one by returning the callback global.
pub(super) fn window_target_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    let target_context = target.get_creation_context(scope)?;
    if !target.strict_equals(target_context.global(scope).into()) {
        return None;
    }
    let host_ptr = crate::util::context_host_ptr_from_context_slot(target_context)?;
    if crate::util::context_host_ptr_from_context_slot(context) != Some(host_ptr) {
        return None;
    }
    let host = unsafe { &*host_ptr };
    let target_identity =
        host.window_execution_context_identity_for_access_check(target_context)?;
    let destination_identity = host.window_execution_context_identity_for_access_check(context)?;
    if target_identity.owner() != destination_identity.owner()
        || target_identity.dispatch_scope() != destination_identity.dispatch_scope()
        || !host.window_execution_context_identity_is_current(target_identity)
        || !host.window_execution_context_identity_is_current(destination_identity)
    {
        return None;
    }
    Some(context.global(scope))
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
        let target = resolve_event_target(scope, args.this())
            .map_or(args.this(), |target| target.handler_owner);
        rv.set(get_private_value(scope, target, slot).unwrap_or_else(|| v8::null(scope).into()));
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
    let target =
        resolve_event_target(scope, args.this()).map_or(args.this(), |target| target.handler_owner);
    media_queries::simple_object_event_set_handler(scope, target, *event_type, slot, args.get(0));
}
