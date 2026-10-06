use super::*;
use crate::context_bootstrap::{
    cached_cross_origin_window_surface, window_closed_getter, window_length_getter,
    window_opener_getter, window_parent_getter, window_top_getter,
};

const INDEXED_COUNT: &str = "__moliMainWindowCrossOriginIndexedCount";
const WINDOW_LOCATION_SLOT: &str = "__moliWindowLocation";

pub(super) fn window_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    property: CrossOriginWindowProperty,
) -> Option<v8::Local<'s, v8::Value>> {
    if matches!(property, CrossOriginWindowProperty::Location) {
        return location_for_window(scope, window).map(Into::into);
    }
    let context = window.get_creation_context(scope)?;
    let host = crate::util::context_host_ptr_from_context_slot(context);
    let closed = host.is_none_or(|host| unsafe { &*host }.browsing_context_is_closed());
    Some(match property {
        CrossOriginWindowProperty::Closed => v8::Boolean::new(scope, closed).into(),
        CrossOriginWindowProperty::Length => {
            let length = if closed {
                0
            } else {
                unsafe { &*host? }.child_browsing_context_count()
            };
            v8::Integer::new_from_unsigned(scope, length as u32).into()
        }
        CrossOriginWindowProperty::Opener => {
            if closed {
                v8::null(scope).into()
            } else {
                unsafe { &*host? }
                    .top_window_opener(scope)
                    .map_or_else(|| v8::null(scope).into(), Into::into)
            }
        }
        CrossOriginWindowProperty::Parent | CrossOriginWindowProperty::Top if closed => {
            v8::null(scope).into()
        }
        CrossOriginWindowProperty::Parent
        | CrossOriginWindowProperty::Top
        | CrossOriginWindowProperty::Window => window.into(),
        CrossOriginWindowProperty::Location => unreachable!(),
    })
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct MainWindowCrossOriginSurface {
    #[webapi(method, length = 1, callback = window_host::window_post_message_callback, readonly, dont_delete)]
    post_message: (),
    #[webapi(method, callback = crate::context_bootstrap::window_close_callback, readonly, dont_delete)]
    close: (),
    #[webapi(method, callback = crate::context_bootstrap::window_noop_callback, readonly, dont_delete)]
    focus: (),
    #[webapi(method, callback = crate::context_bootstrap::window_noop_callback, readonly, dont_delete)]
    blur: (),
    #[webapi(accessor_property, getter = window_closed_getter)]
    closed: (),
    #[webapi(accessor_property, getter = window_length_getter)]
    length: (),
    #[webapi(accessor_property, getter = window_opener_getter)]
    opener: (),
    #[webapi(accessor_property, getter = window_parent_getter)]
    parent: (),
    #[webapi(accessor_property, getter = window_top_getter)]
    top: (),
    #[webapi(accessor_property, getter = location_getter, setter = location_setter)]
    location: (),
}

pub(super) fn access_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target_context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Object> {
    let surface = cached_access_surface(scope, target_context);
    refresh_indexed_children(scope, target_context, surface);
    surface
}

fn cached_access_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target_context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Object> {
    let accessing_context = cross_origin_accessing_context(scope);
    cached_cross_origin_window_surface(scope, target_context, accessing_context, |scope| {
        let window = target_context.global(scope);
        let location = location_for_window(scope, window)
            .expect("main Window cross-origin Location should initialize");
        build_accessing_surface(scope, window, location)
    })
}

fn build_accessing_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    location: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let surface = new_null_prototype_object(scope);
    MainWindowCrossOriginSurface {
        post_message: (),
        close: (),
        focus: (),
        blur: (),
        closed: (),
        length: (),
        opener: (),
        parent: (),
        top: (),
        location: (),
    }
    .initialize(scope, surface)
    .expect("main Window cross-origin surface should initialize");
    for name in ["window", "self", "frames"] {
        set_cross_origin_object_slot(scope, surface, name, window.into());
    }
    let undefined = v8::undefined(scope).into();
    set_cross_origin_object_slot(scope, surface, "then", undefined);
    set_private_value(
        scope,
        surface,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    set_private_value(scope, surface, CROSS_ORIGIN_WINDOW_SELF_SLOT, window.into());
    install_cross_origin_symbol_slots(scope, surface);

    surface
}

pub(super) fn is_main_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    surface: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, surface, INDEXED_COUNT).is_some()
}

fn refresh_indexed_children<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target_context: v8::Local<'s, v8::Context>,
    surface: v8::Local<'s, v8::Object>,
) {
    let previous = get_private_value(scope, surface, INDEXED_COUNT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let scope = &mut v8::ContextScope::new(scope, target_context);
    let count = crate::util::context_host_ptr_from_context_slot(target_context)
        .filter(|host| !unsafe { &**host }.browsing_context_is_closed())
        .map_or(0, |host| {
            let host = unsafe { &mut *host };
            host.sync_child_browsing_context_subtree(scope, host.document_handle());
            let count = host.child_browsing_context_count() as u32;
            for index in 0..count {
                if let Some(handle) = host.child_browsing_context_handle_by_index_for_document(
                    host.document_handle(),
                    index as usize,
                ) && let Some(window) =
                    host.child_browsing_context_window_proxy_for_current_realm(scope, handle)
                {
                    let key = v8_string(scope, &index.to_string()).unwrap();
                    let _ = surface.define_own_property(
                        scope,
                        key.into(),
                        window.into(),
                        v8::PropertyAttribute::READ_ONLY,
                    );
                }
            }
            count
        });
    for index in count..previous {
        let _ = surface.delete_index(scope, index);
    }
    let count = v8::Integer::new_from_unsigned(scope, count).into();
    set_private_value(scope, surface, INDEXED_COUNT, count);
}

fn navigate_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    kind: crate::context_bootstrap::LocationNavigationKind,
    block_javascript: bool,
) -> bool {
    let raw = match webidl::convert::<webidl::UsvString>(
        scope,
        value,
        webidl::Context::member("Location", "href"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return false;
        }
    };
    if block_javascript && url::Url::parse(&raw).is_ok_and(|url| url.scheme() == "javascript") {
        throw_cross_origin_location_security_error(scope);
        return false;
    }
    let Some(context) = window.get_creation_context(scope) else {
        return false;
    };
    let accessing_context = cross_origin_accessing_context(scope);
    let raw = if context != accessing_context
        && kind != crate::context_bootstrap::LocationNavigationKind::Reload
    {
        crate::context_bootstrap::resolve_cross_window_location_target(scope, &raw).unwrap_or(raw)
    } else {
        raw
    };
    if let Ok(destination) = url::Url::parse(&raw)
        && crate::context_bootstrap::blocks_ancestor_location_navigation(
            scope,
            window,
            &destination,
        )
    {
        throw_cross_origin_location_security_error(scope);
        return false;
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(location) = crate::context_bootstrap::window_location_for_holder(scope, window) else {
        return false;
    };
    crate::context_bootstrap::navigate_location_object_with_child_navigate_event(
        scope,
        location,
        kind,
        Some(raw),
    );
    true
}

fn require_window_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    operation: &str,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    // The internal surface carries the cross-origin location slot, but it is
    // not a Window. Callers must pass the real WindowProxy as `this`.
    if !is_main_surface(scope, receiver)
        && (crate::context_bootstrap::is_window_receiver(scope, receiver)
            || is_cross_origin_window_proxy(scope, receiver))
    {
        return Some(receiver);
    }
    throw_type_error(
        scope,
        &format!("Failed to {operation} the '{name}' property on 'Window': Illegal invocation"),
    );
    None
}

pub(super) fn location_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = window.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    get_private_value(scope, window, WINDOW_LOCATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn location_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(window) = require_window_receiver(scope, &args, "get", "location") else {
        return;
    };
    if let Some(location) = location_for_window(scope, window) {
        rv.set(location.into());
    }
}

fn location_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(window) = require_window_receiver(scope, &args, "set", "location") else {
        return;
    };
    let block_javascript = !caller_can_access_window(scope, window);
    navigate_window(
        scope,
        window,
        args.get(0),
        crate::context_bootstrap::LocationNavigationKind::Assign,
        block_javascript,
    );
}
