//! A single listener list behind world-local Navigation and entry wrappers.
//! The list retains callback realms using the ordinary EventTarget machinery;
//! dispatch still runs once, with one cancellation/propagation state.

use super::{history_runtime::native, media_queries};
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
        return native::entry_in_realm(scope, owner, context);
    }
    let global = context.global(scope);
    get_private_value(scope, global, WRAPPERS)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .and_then(|map| map.get(scope, owner.into()))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or(target)
}

pub(super) fn install_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    entry: bool,
) {
    media_queries::install_simple_event_target_ordered_handlers(scope, target);
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
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, target, slot, value);
    let Some(listeners) = media_queries::simple_event_target_slot_name(scope, target) else {
        return;
    };
    media_queries::simple_object_event_set_ordered_handler(
        scope, target, &listeners, event_type, slot, active,
    );
}
