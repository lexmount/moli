use super::registry_runtime::CUSTOM_ELEMENTS_REGISTRY_CHILD_HANDLE_SLOT;
use crate::web_api_interfaces;
use crate::{
    context_bootstrap::{WindowLazySurface, rematerialize_window_lazy_surface_if_cached},
    document_runtime::DomHandle,
    util::{get_private_value, set_private_value},
};
use anyhow::Result;
use moli_webapi_declare::WebApiObject;
use std::rc::Rc;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CustomElementRegistry, prototype = "Object")]
struct CustomElementsRegistryDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

const CUSTOM_ELEMENTS_WINDOW_OWNER_CHILD_SLOT: &str = "__moliCustomElementsWindowOwnerChild";
const WINDOW_REGISTRY_CACHE_SLOT: &str = "__moliWindowCustomElementsRegistries";

struct WindowRegistryCache(v8::Weak<v8::Map>);

fn window_registry_cache_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    child_handle: Option<DomHandle>,
) -> v8::Local<'s, v8::Value> {
    child_handle
        .map(|handle| v8::BigInt::new_from_u64(scope, handle.index() as u64).into())
        .unwrap_or_else(|| v8::undefined(scope).into())
}

pub(crate) fn custom_elements_registry_for_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    child_handle: Option<DomHandle>,
) -> Result<v8::Local<'s, v8::Object>> {
    let context = scope.get_current_context();
    let cached = context
        .get_slot::<WindowRegistryCache>()
        .and_then(|cache| cache.0.to_local(scope));
    let cache = if let Some(cache) = cached {
        cache
    } else {
        let prototype = crate::context_bootstrap::ensure_intrinsic_interface_prototype(
            scope,
            "CustomElementRegistry",
        )?;
        let cache = v8::Map::new(scope);
        // The realm traces the cache through its intrinsic prototype. The
        // native slot is weak so it does not add a root retaining the realm.
        // WindowProxy slots cannot serve this role after detach_global().
        set_private_value(scope, prototype, WINDOW_REGISTRY_CACHE_SLOT, cache.into());
        let _ = context.set_slot(Rc::new(WindowRegistryCache(v8::Weak::new(scope, cache))));
        cache
    };
    let key = window_registry_cache_key(scope, child_handle);
    if let Some(registry) = cache
        .get(scope, key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Ok(registry);
    }
    let registry = new_custom_elements_registry_for_owner(scope, child_handle)?;
    cache
        .set(scope, key, registry.into())
        .ok_or_else(|| anyhow::anyhow!("failed to cache the Window custom element registry"))?;
    Ok(registry)
}

pub(crate) fn build_custom_elements_registry_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let child_handle = custom_elements_owner_child(scope, window).or_else(|| {
        crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(scope)
    });
    custom_elements_registry_for_current_realm(scope, child_handle)
}

pub(crate) fn rebind_materialized_child_custom_elements_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    child_handle: DomHandle,
) -> Result<()> {
    let relevant_context = window
        .get_creation_context(scope)
        .ok_or_else(|| anyhow::anyhow!("CustomElementRegistry target has no creation context"))?;
    if relevant_context != scope.get_current_context() {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let target_window = relevant_context.global(target_scope);
        return rebind_materialized_child_custom_elements_registry_in_current_realm(
            target_scope,
            target_window,
            child_handle,
        );
    }
    rebind_materialized_child_custom_elements_registry_in_current_realm(scope, window, child_handle)
}

fn rebind_materialized_child_custom_elements_registry_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    child_handle: DomHandle,
) -> Result<()> {
    let handle_value = v8::BigInt::new_from_u64(scope, child_handle.index() as u64);
    set_private_value(
        scope,
        window,
        CUSTOM_ELEMENTS_WINDOW_OWNER_CHILD_SLOT,
        handle_value.into(),
    );
    if let Some(cache) = scope
        .get_current_context()
        .get_slot::<WindowRegistryCache>()
        .and_then(|cache| cache.0.to_local(scope))
    {
        let key = window_registry_cache_key(scope, Some(child_handle));
        let _ = cache.delete(scope, key);
    }
    rematerialize_window_lazy_surface_if_cached(scope, window, WindowLazySurface::CustomElements)?;
    Ok(())
}

fn custom_elements_owner_child<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_private_value(scope, window, CUSTOM_ELEMENTS_WINDOW_OWNER_CHILD_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (index, lossless) = value.u64_value();
            lossless.then(|| DomHandle::new(index as usize))
        })
}

fn new_custom_elements_registry_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    child_handle: Option<DomHandle>,
) -> Result<v8::Local<'s, v8::Object>> {
    let registry = new_custom_elements_registry(scope)?;
    let Some(child_handle) = child_handle else {
        return Ok(registry);
    };
    let handle_value = v8::BigInt::new_from_u64(scope, child_handle.index() as u64);
    set_private_value(
        scope,
        registry,
        CUSTOM_ELEMENTS_REGISTRY_CHILD_HANDLE_SLOT,
        handle_value.into(),
    );
    Ok(registry)
}

fn new_custom_elements_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    let prototype = crate::context_bootstrap::ensure_intrinsic_interface_prototype(
        scope,
        "CustomElementRegistry",
    )?;
    CustomElementsRegistryDeclaration::new(prototype)
        .bind(scope)
        .map_err(anyhow::Error::from)
}
