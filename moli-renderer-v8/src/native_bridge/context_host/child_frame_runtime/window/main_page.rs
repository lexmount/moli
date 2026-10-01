use super::*;

const SURFACES: &str = "__moliMainWindowCrossOriginSurfaces";
const LOCATION: &str = "__moliMainWindowCrossOriginLocationObject";
const LOCATION_SURFACE: &str = "__moliMainWindowCrossOriginLocationSurface";
const MAIN_LOCATION: &str = "__moliMainWindowCrossOriginLocation";
const INDEXED_COUNT: &str = "__moliMainWindowCrossOriginIndexedCount";

#[derive(WebApiObject)]
#[webapi(plain)]
struct MainWindowCrossOriginSurface<'scope> {
    window: v8::Local<'scope, v8::Object>,
    #[webapi(method, length = 1, callback = window_host::window_post_message_callback, readonly, dont_delete)]
    post_message: (),
    #[webapi(method, callback = crate::context_bootstrap::window_close_callback, readonly, dont_delete)]
    close: (),
    #[webapi(method, callback = crate::context_bootstrap::window_noop_callback, readonly, dont_delete)]
    focus: (),
    #[webapi(method, callback = crate::context_bootstrap::window_noop_callback, readonly, dont_delete)]
    blur: (),
    #[webapi(accessor_property, getter = closed_getter, data = self.window)]
    closed: (),
    #[webapi(accessor_property, getter = length_getter, data = self.window)]
    length: (),
    #[webapi(accessor_property, getter = opener_getter, data = self.window)]
    opener: (),
    #[webapi(accessor_property, getter = parent_getter, data = self.window)]
    parent: (),
    #[webapi(accessor_property, getter = parent_getter, data = self.window)]
    top: (),
    #[webapi(accessor_property, getter = location_getter, setter = location_setter, data = self.window)]
    location: (),
}

pub(super) fn access_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target_context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Object> {
    let accessing_context = cross_origin_accessing_context(scope);
    let scope = &mut v8::ContextScope::new(scope, accessing_context);
    let anchor = target_context.get_extras_binding_object(scope);
    let accessing_anchor = accessing_context.get_extras_binding_object(scope);
    let surfaces = get_private_value(scope, anchor, SURFACES)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .unwrap_or_else(|| {
            let surfaces = v8::Map::new(scope);
            set_private_value(scope, anchor, SURFACES, surfaces.into());
            surfaces
        });
    if let Some(surface) = surfaces
        .get(scope, accessing_anchor.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        refresh_indexed_children(scope, target_context, surface);
        return surface;
    }
    // WindowProxy and Location keep their target identity. Only the functions
    // and accessor descriptors are specific to the accessing settings object.
    let window = target_context.global(scope);
    let location = get_private_value(scope, anchor, LOCATION)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or_else(|| {
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
                let _ = target.define_own_property(
                    scope,
                    key,
                    undefined,
                    v8::PropertyAttribute::DONT_ENUM,
                );
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
        });
    let surface = new_null_prototype_object(scope);
    MainWindowCrossOriginSurface {
        window,
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
    let _ = surfaces.set(scope, accessing_anchor.into(), surface.into());
    refresh_indexed_children(scope, target_context, surface);
    surface
}

pub(super) fn location_access_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let window = main_location_window(scope, receiver)?;
    let context = window.get_creation_context(scope)?;
    let surface = access_surface(scope, context);
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
    navigate_window(scope, window, value, kind)
}

fn navigate_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
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
    if url::Url::parse(&raw).is_ok_and(|url| url.scheme() == "javascript") {
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

fn target_window<'s>(args: &v8::FunctionCallbackArguments<'s>) -> v8::Local<'s, v8::Object> {
    v8::Local::<v8::Object>::try_from(args.data()).expect("main Window accessor target")
}

fn target_host<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<*mut JsContextHost> {
    target_window(args)
        .get_creation_context(scope)
        .and_then(crate::util::context_host_ptr_from_context_slot)
}

fn location_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(context) = target_window(&args).get_creation_context(scope) else {
        return;
    };
    let anchor = context.get_extras_binding_object(scope);
    if let Some(location) = get_private_value(scope, anchor, LOCATION) {
        rv.set(location);
    }
}

fn location_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    navigate_window(
        scope,
        target_window(&args),
        args.get(0),
        crate::context_bootstrap::LocationNavigationKind::Assign,
    );
}

fn closed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(
        target_host(scope, &args).is_none_or(|host| unsafe { &*host }.browsing_context_is_closed()),
    );
}

fn length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let length = target_host(scope, &args)
        .filter(|host| !unsafe { &**host }.browsing_context_is_closed())
        .map_or(0, |host| unsafe { &*host }.child_browsing_context_count());
    rv.set_uint32(length as u32);
}

fn parent_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_null();
    if target_host(scope, &args).is_some_and(|host| !unsafe { &*host }.browsing_context_is_closed())
    {
        rv.set(target_window(&args).into());
    }
}

fn opener_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_null();
    let Some(environment) =
        target_host(scope, &args).and_then(|host| unsafe { &*host }.page_script_environment())
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
