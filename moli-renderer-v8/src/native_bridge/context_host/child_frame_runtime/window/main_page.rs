use super::*;
use crate::context_bootstrap::{
    cached_cross_origin_window_surface, window_closed_getter, window_length_getter,
    window_opener_getter, window_parent_getter, window_top_getter,
};

const LOCATION: &str = "__moliMainWindowCrossOriginLocationObject";
const LOCATION_SURFACE: &str = "__moliMainWindowCrossOriginLocationSurface";
const MAIN_LOCATION: &str = "__moliMainWindowCrossOriginLocation";
const INDEXED_COUNT: &str = "__moliMainWindowCrossOriginIndexedCount";
const WINDOW_LOCATION_SLOT: &str = "__moliWindowLocation";

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
        let location = shared_cross_origin_location(scope, window)
            .expect("main Window cross-origin Location should initialize");
        build_accessing_surface(scope, window, location)
    })
}

fn ensure_shared_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    anchor: v8::Local<'s, v8::Object>,
    window: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(location) = get_private_value(scope, anchor, LOCATION)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return location;
    }
    let target = new_null_prototype_object(scope);
    let marker = v8::Boolean::new(scope, true).into();
    set_private_value(scope, target, CROSS_ORIGIN_LOCATION_PROXY_SLOT, marker);
    set_private_value(scope, target, MAIN_LOCATION, window.into());
    // Preserve the target's keys without exposing realm-specific
    // functions. Configurable placeholders allow each observer's
    // get/descriptor projection to obey Proxy invariants.
    let undefined = v8::undefined(scope).into();
    for name in CROSS_ORIGIN_LOCATION_DENIED_PROPERTIES
        .iter()
        .copied()
        .chain(["href", "replace", "then"])
    {
        let key = v8str(scope, name).into();
        let _ = target.define_own_property(scope, key, undefined, v8::PropertyAttribute::DONT_ENUM);
    }
    install_cross_origin_symbol_slots(scope, target, "Location");
    let location = wrap_cross_origin_location_proxy(scope, target)
        .expect("main Window cross-origin Location proxy should initialize");
    set_private_value(
        scope,
        target,
        CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT,
        location.into(),
    );
    set_private_value(scope, anchor, LOCATION, location.into());
    location
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
    install_cross_origin_symbol_slots(scope, surface, "Window");

    let location_surface = new_null_prototype_object(scope);
    install_cross_origin_symbol_slots(scope, location_surface, "Location");
    install_cross_origin_denied_accessors(
        scope,
        location_surface,
        CROSS_ORIGIN_LOCATION_DENIED_PROPERTIES,
    );
    let setter = v8::Function::builder(cross_origin_location_navigate_setter_callback)
        .length(1)
        .data(cross_origin_proxy_storage_object(scope, location).into())
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .expect("cross-origin Location href setter should initialize");
    setter.set_name(v8str(scope, "set href"));
    let href = v8str(scope, "href").into();
    define_get_set_property(
        scope,
        location_surface,
        href,
        undefined,
        setter.into(),
        cross_origin_property_attributes(),
        "href",
    )
    .expect("cross-origin Location href descriptor should initialize");
    install_cross_origin_location_methods(scope, location_surface);
    set_cross_origin_object_slot(scope, location_surface, "then", undefined);
    set_private_value(scope, surface, LOCATION_SURFACE, location_surface.into());
    surface
}

pub(super) fn location_access_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let window = main_location_window(scope, receiver)?;
    let context = window.get_creation_context(scope)?;
    let surface = cached_access_surface(scope, context);
    get_private_value(scope, surface, LOCATION_SURFACE)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
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
                if let Some(handle) = host.child_browsing_context_handle_by_index(index as usize)
                    && let Some(window) =
                        host.child_browsing_context_window_proxy_for_top(scope, handle)
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

fn main_location_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_cross_origin_proxy_private_value(scope, receiver, MAIN_LOCATION)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn is_main_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    main_location_window(scope, receiver).is_some()
}

pub(super) fn navigate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    kind: crate::context_bootstrap::LocationNavigationKind,
) -> bool {
    let Some(window) = main_location_window(scope, receiver) else {
        return false;
    };
    navigate_window(scope, window, value, kind, true)
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
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(location) = window
        .get(scope, v8str(scope, "location").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
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

fn caller_can_access_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(accessed) = window.get_creation_context(scope) else {
        return false;
    };
    let accessing = cross_origin_accessing_context(scope);
    contexts_can_script_access(scope, accessing, accessed)
}

fn shared_cross_origin_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = window.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let anchor = context.get_extras_binding_object(scope);
    let window = context.global(scope);
    Some(ensure_shared_location(scope, anchor, window))
}

fn location_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(window) = require_window_receiver(scope, &args, "get", "location") else {
        return;
    };
    if caller_can_access_window(scope, window) {
        if let Some(location) = get_private_value(scope, window, WINDOW_LOCATION_SLOT) {
            rv.set(location);
            return;
        }
        throw_type_error(
            scope,
            "Failed to get the 'location' property on 'Window': Illegal invocation",
        );
        return;
    }
    if let Some(location) = shared_cross_origin_location(scope, window) {
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
