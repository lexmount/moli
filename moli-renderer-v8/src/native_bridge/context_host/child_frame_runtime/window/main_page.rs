use super::*;

const SURFACE: &str = "__moliMainWindowCrossOriginSurface";
const MAIN_LOCATION: &str = "__moliMainWindowCrossOriginLocation";
const INDEXED_COUNT: &str = "__moliMainWindowCrossOriginIndexedCount";

#[derive(Default, WebApiObject)]
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
    #[webapi(accessor_property, getter = closed_getter)]
    closed: (),
    #[webapi(accessor_property, getter = length_getter)]
    length: (),
    #[webapi(accessor_property, getter = opener_getter)]
    opener: (),
    #[webapi(accessor_property, getter = parent_getter)]
    parent: (),
    #[webapi(accessor_property, getter = parent_getter)]
    top: (),
    #[webapi(accessor_property, getter = cross_origin_window_location_getter_callback, setter = location_setter)]
    location: (),
}

pub(super) fn access_surface<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let context = scope.get_current_context();
    let anchor = context.get_extras_binding_object(scope);
    if let Some(surface) = get_private_value(scope, anchor, SURFACE)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        refresh_indexed_children(scope, surface);
        return surface;
    }
    // Keep the allowlisted access surface in the target realm's traced graph.
    // Its functions must enter that realm, while event.source still captures
    // the incumbent sender before any message crosses the Page task queue.
    let surface = new_null_prototype_object(scope);
    MainWindowCrossOriginSurface::default()
        .initialize(scope, surface)
        .expect("main Window cross-origin surface should initialize");
    let window = context.global(scope);
    for name in ["window", "self", "frames"] {
        set_cross_origin_object_slot(scope, surface, name, window.into());
    }
    set_cross_origin_object_slot(scope, surface, "then", v8::undefined(scope).into());
    let location = build_detached_cross_origin_location_proxy(scope);
    let target = cross_origin_proxy_storage_object(scope, location);
    set_private_value(
        scope,
        target,
        MAIN_LOCATION,
        v8::Boolean::new(scope, true).into(),
    );
    set_private_value(
        scope,
        surface,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    install_cross_origin_symbol_slots(scope, surface, "Window");
    set_private_value(scope, anchor, SURFACE, surface.into());
    refresh_indexed_children(scope, surface);
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
    surface: v8::Local<'s, v8::Object>,
) {
    let previous = get_private_value(scope, surface, INDEXED_COUNT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let host_ptr = context_host_ptr_from_global_bridge(scope);
    let count = host_ptr
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
    set_private_value(
        scope,
        surface,
        INDEXED_COUNT,
        v8::Integer::new_from_unsigned(scope, count).into(),
    );
}

pub(super) fn is_main_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_cross_origin_proxy_private_value(scope, receiver, MAIN_LOCATION)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(super) fn navigate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    kind: crate::context_bootstrap::LocationNavigationKind,
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
    // Cross-origin Location permits navigation, but cannot run source code in
    // the target realm. URL parsing also handles leading whitespace and the
    // case-insensitive scheme before this navigation reaches the renderer.
    if url::Url::parse(&raw).is_ok_and(|url| url.scheme() == "javascript") {
        throw_cross_origin_location_security_error(scope);
        return false;
    }
    let global = scope.get_current_context().global(scope);
    // Resolve the native Window's own Location after entering the target
    // context, rather than treating another Page's local endpoint id as ours.
    let Some(location) = global
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

fn location_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    navigate(
        scope,
        args.get(0),
        crate::context_bootstrap::LocationNavigationKind::Assign,
    );
}

fn closed_getter(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(
        context_host_ptr_from_global_bridge(scope)
            .is_none_or(|host| unsafe { &*host }.browsing_context_is_closed()),
    );
}

fn length_getter(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let length = context_host_ptr_from_global_bridge(scope)
        .filter(|host| !unsafe { &**host }.browsing_context_is_closed())
        .map_or(0, |host| unsafe { &*host }.child_browsing_context_count());
    rv.set_uint32(length as u32);
}

fn parent_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_null();
    if context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host| !unsafe { &*host }.browsing_context_is_closed())
    {
        rv.set(scope.get_current_context().global(scope).into());
    }
}

fn opener_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_null();
    let Some(environment) = context_host_ptr_from_global_bridge(scope)
        .and_then(|host| unsafe { &*host }.page_script_environment())
    else {
        return;
    };
    let Some(opener) = environment.opener_in_scope(scope) else {
        return;
    };
    let Some(host) = opener
        .get_creation_context(scope)
        .and_then(crate::util::context_host_ptr_from_context_slot)
    else {
        return;
    };
    if !unsafe { &*host }.browsing_context_is_closed() {
        rv.set(opener.into());
    }
}
