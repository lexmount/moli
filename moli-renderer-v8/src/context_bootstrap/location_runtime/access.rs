use super::*;
use crate::native_bridge::window_contexts_allow_access;
use crate::util::{get_private_value, new_null_prototype_object, set_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;
use std::{cell::RefCell, rc::Rc};

const SURFACE_TARGET_SLOT: &str = "__moliLocationCrossOriginTarget";
const REFLECT_SET_SLOT: &str = "__moliLocationReflectSet";

#[derive(WebApiObject)]
#[webapi(plain)]
struct LocationProxyHandler<'s> {
    reflect_set: v8::Local<'s, v8::Function>,
    #[webapi(method, callback = location_proxy_set, data = self.reflect_set, length = 4)]
    set: (),
    #[webapi(method, callback = location_proxy_set_prototype, length = 2)]
    set_prototype_of: (),
    #[webapi(method, callback = location_proxy_prevent_extensions, length = 1)]
    prevent_extensions: (),
}

pub(in crate::context_bootstrap) fn location_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object)
}

pub(in crate::context_bootstrap) fn wrap_location_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> anyhow::Result<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let reflect_set = if let Some(function) = get_private_value(scope, global, REFLECT_SET_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        function
    } else {
        // The first Location is installed before author script. Reuse this
        // intrinsic when navigation subsequently creates another Location.
        let reflect = global
            .get(scope, v8str(scope, "Reflect").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(|| anyhow!("missing Reflect during Location bootstrap"))?;
        let function = reflect
            .get(scope, v8str(scope, "set").into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .ok_or_else(|| anyhow!("missing Reflect.set during Location bootstrap"))?;
        set_private_value(scope, global, REFLECT_SET_SLOT, function.into());
        function
    };
    let handler = LocationProxyHandler {
        reflect_set,
        set: (),
        set_prototype_of: (),
        prevent_extensions: (),
    }
    .bind(scope)?;
    if handler.set_prototype(scope, v8::null(scope).into()) != Some(true) {
        return Err(anyhow!("failed to initialize Location proxy handler"));
    }
    // Property access, descriptors and own keys go directly to V8's checked
    // target. A shadow target cannot hide non-configurable author expandos
    // after an origin change without violating JavaScript Proxy invariants.
    let proxy = v8::Proxy::new(scope, target, handler)
        .ok_or_else(|| anyhow!("failed to create Location proxy"))?;
    moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
    Ok(proxy.into())
}

fn location_proxy_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let caller = scope
        .get_incumbent_context()
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, caller);
    if target
        .get_creation_context(scope)
        .is_some_and(|owner| window_contexts_allow_access(caller, owner))
    {
        let Ok(reflect_set) = v8::Local::<v8::Function>::try_from(args.data()) else {
            return;
        };
        if let Some(value) = reflect_set.call(
            scope,
            v8::undefined(scope).into(),
            &[target.into(), args.get(1), args.get(2), args.get(3)],
        ) {
            rv.set(value);
        }
        return;
    }
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        return;
    };
    if key != v8str(scope, "href") {
        crate::native_bridge::throw_cross_origin_location_security_error(scope);
        return;
    }
    let Some(surface) = surface_for(scope, target) else {
        return;
    };
    let Some(descriptor) = surface
        .get_own_property_descriptor(scope, key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(setter) = descriptor
        .get(scope, v8str(scope, "set").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    // Preserve Reflect.set's receiver. Interceptor callbacks only expose the
    // holder, and must not substitute it for a forged or author Proxy receiver.
    if setter.call(scope, args.get(3), &[args.get(2)]).is_some() {
        rv.set_bool(true);
    }
}

fn location_proxy_set_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let caller = scope
        .get_incumbent_context()
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, caller);
    if !target
        .get_creation_context(scope)
        .is_some_and(|owner| window_contexts_allow_access(caller, owner))
    {
        rv.set_bool(args.get(1).is_null());
        return;
    }
    if let Some(prototype) = target.get_prototype(scope) {
        rv.set_bool(prototype.strict_equals(args.get(1)));
    }
}

fn location_proxy_prevent_extensions<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(false);
}

// Weak entries do not retain removed frames. Each cached function keeps its
// surface alive through callback data, so keeping either function preserves
// both descriptor identities, as required by CrossOriginPropertyDescriptorMap.
#[derive(Default, Clone)]
struct CrossOriginSurfaces(Rc<RefCell<Vec<CrossOriginSurface>>>);

struct CrossOriginSurface {
    context: v8::Weak<v8::Context>,
    target: v8::Weak<v8::Object>,
    surface: v8::Weak<v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain, receiver = web_api_interfaces::Location::is_instance)]
struct CrossOriginSurfaceDeclaration<'s> {
    surface: v8::Local<'s, v8::Object>,
    #[webapi(method = "set href", callback = cross_origin_href_setter, data = self.surface, length = 1)]
    href: (),
    #[webapi(method, callback = super::methods::location_replace_callback, data = self.surface, length = 1, readonly)]
    replace: (),
}

pub(super) fn install_access_check(template: v8::Local<'_, v8::ObjectTemplate>) {
    template.set_security_token_access_check_and_handlers(
        location_access_check,
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(cross_origin_getter)
            .query(cross_origin_query)
            .descriptor(cross_origin_descriptor)
            .enumerator(cross_origin_enumerator),
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(cross_origin_indexed_getter)
            .descriptor(cross_origin_indexed_descriptor)
            .enumerator(cross_origin_indexed_enumerator),
    );
}

unsafe extern "C" fn location_access_check(
    accessing_context: v8::Local<'_, v8::Context>,
    object: v8::Local<'_, v8::Object>,
    _data: v8::Local<'_, v8::Value>,
) -> bool {
    let scope = std::pin::pin!(unsafe { v8::CallbackScope::new(accessing_context) });
    let scope = &mut scope.init();
    object
        .get_creation_context(scope)
        .is_some_and(|owner| window_contexts_allow_access(accessing_context, owner))
}

pub(super) fn require_same_origin(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    let current = scope.get_current_context();
    if object
        .get_creation_context(scope)
        .is_some_and(|owner| window_contexts_allow_access(current, owner))
    {
        return true;
    }
    crate::native_bridge::throw_dom_exception(
        scope,
        "SecurityError",
        18,
        "Blocked access to a cross-origin Location.",
    );
    false
}

pub(super) fn require_entry_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if !super::install::location_belongs_to_current_local_window(scope, object) {
        return true;
    }
    let entry = scope.get_entered_or_microtask_context();
    if object
        .get_creation_context(scope)
        .is_some_and(|owner| window_contexts_allow_access(entry, owner))
    {
        return true;
    }
    crate::native_bridge::throw_dom_exception(
        scope,
        "SecurityError",
        18,
        "Blocked access to a cross-origin Location.",
    );
    false
}

fn surface_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache = scope
        .get_slot::<CrossOriginSurfaces>()
        .cloned()
        .unwrap_or_else(|| {
            let cache = CrossOriginSurfaces::default();
            scope.set_slot(cache.clone());
            cache
        });
    let context = scope.get_current_context();
    {
        let mut entries = cache.0.borrow_mut();
        entries.retain(|entry| {
            !entry.context.is_empty() && !entry.target.is_empty() && !entry.surface.is_empty()
        });
        for entry in entries.iter() {
            if entry.context.to_local(scope) == Some(context)
                && entry.target.to_local(scope) == Some(target)
            {
                return entry.surface.to_local(scope);
            }
        }
    }
    let surface = new_null_prototype_object(scope);
    set_private_value(scope, surface, SURFACE_TARGET_SLOT, target.into());
    CrossOriginSurfaceDeclaration {
        surface,
        href: (),
        replace: (),
    }
    .initialize(scope, surface)
    .ok()?;
    let setter_name = v8str(scope, "set href");
    let setter = surface.get(scope, setter_name.into())?;
    surface.delete(scope, setter_name.into())?;
    crate::definitions::define_get_set_property(
        scope,
        surface,
        v8str(scope, "href").into(),
        v8::undefined(scope).into(),
        setter,
        v8::PropertyAttribute::DONT_ENUM,
        "href",
    )
    .ok()?;
    // initialize() binds the declaration prototype; the cache object must not
    // inherit page-defined getters or property descriptor fields.
    surface.set_prototype(scope, v8::null(scope).into())?;
    for key in fallback_keys(scope) {
        surface.define_own_property(
            scope,
            key,
            v8::undefined(scope).into(),
            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
        )?;
    }
    cache.0.borrow_mut().push(CrossOriginSurface {
        context: v8::Weak::new(scope, context),
        target: v8::Weak::new(scope, target),
        surface: v8::Weak::new(scope, surface),
    });
    Some(surface)
}

fn fallback_keys<'s>(scope: &mut v8::PinScope<'s, '_>) -> [v8::Local<'s, v8::Name>; 4] {
    [
        v8str(scope, "then").into(),
        v8::Symbol::get_to_string_tag(scope).into(),
        v8::Symbol::get_has_instance(scope).into(),
        v8::Symbol::get_is_concat_spreadable(scope).into(),
    ]
}

fn cross_origin_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    if key == v8str(scope, "href") {
        return v8::Intercepted::kNo;
    }
    let Some(surface) = surface_for(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if surface.has_own_property(scope, key) == Some(true)
        && let Some(value) = surface.get(scope, key.into())
    {
        rv.set(value);
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

fn cross_origin_href_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = match crate::webidl::convert::<crate::webidl::UsvString>(
        scope,
        args.get(0),
        crate::webidl::Context::member("Location", "href"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            crate::webidl::throw_error(scope, &error);
            return;
        }
    };
    navigate_location_object(
        scope,
        args.this(),
        LocationNavigationKind::Assign,
        Some(value),
    );
}

fn cross_origin_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Some(surface) = surface_for(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if surface.has_own_property(scope, key) == Some(true)
        && let Some(attributes) = surface.get_property_attributes(scope, key.into())
    {
        rv.set_int32(attributes.as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

fn cross_origin_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(surface) = surface_for(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if surface.has_own_property(scope, key) == Some(true)
        && let Some(descriptor) = surface.get_own_property_descriptor(scope, key)
    {
        rv.set(descriptor);
        return v8::Intercepted::kYes;
    }
    // Declining here lets V8 expose an ordinary own descriptor on the target.
    crate::native_bridge::throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn cross_origin_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let mut keys = vec![v8str(scope, "href").into(), v8str(scope, "replace").into()];
    keys.extend(fallback_keys(scope).map(v8::Local::<v8::Value>::from));
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn cross_origin_indexed_getter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    v8::Intercepted::kNo
}

fn cross_origin_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    rv.set(v8::Array::new(scope, 0));
}

fn cross_origin_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    crate::native_bridge::throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}
