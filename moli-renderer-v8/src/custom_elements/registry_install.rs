use super::registry_runtime::CUSTOM_ELEMENTS_REGISTRY_CHILD_HANDLE_SLOT;
use crate::web_api_interfaces;
use crate::{
    context_bootstrap::{WindowLazySurface, rematerialize_window_lazy_surface_if_cached},
    document_runtime::DomHandle,
    util::{get_private_value, set_private_value, v8str},
};
use anyhow::Result;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CustomElementRegistry, prototype = "Object")]
struct CustomElementsRegistryDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

const CUSTOM_ELEMENTS_WINDOW_OWNER_CHILD_SLOT: &str = "__moliCustomElementsWindowOwnerChild";

pub(crate) fn build_custom_elements_registry_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let child_handle = custom_elements_owner_child(scope, window).or_else(|| {
        crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(scope)
    });
    new_custom_elements_registry_for_owner(scope, child_handle)
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
    let global = scope.get_current_context().global(scope);
    let registry = new_custom_elements_registry(scope, global)?;
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
    global: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let ctor = global
        .get(scope, v8str(scope, "CustomElementRegistry").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("failed to load CustomElementRegistry constructor"))?;
    let prototype = ctor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("failed to load CustomElementRegistry prototype"))?;
    CustomElementsRegistryDeclaration::new(prototype)
        .bind(scope)
        .map_err(anyhow::Error::from)
}
