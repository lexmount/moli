use super::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use crate::web_api_interfaces;
use crate::{
    document_runtime::DomHandle,
    native_bridge::JsContextHost,
    util::{
        context_host_ptr_from_context_slot, get_private_object, get_private_value,
        new_null_prototype_object, set_private_value,
    },
};

pub(super) const CUSTOM_ELEMENTS_REGISTRY_CHILD_HANDLE_SLOT: &str =
    "__moliCustomElementsRegistryChildHandle";
const CUSTOM_ELEMENTS_REGISTRY_SCOPED_ID_SLOT: &str = "__moliCustomElementsRegistryScopedId";
const CUSTOM_ELEMENTS_REGISTRY_OWNER_SLOT: &str = "__moliCustomElementsRegistryOwner";

pub(crate) fn mark_custom_elements_registry_owner(
    scope: &mut v8::PinScope<'_, '_>,
    registry: v8::Local<'_, v8::Object>,
) {
    // The allocation context of a receiver can come from an unrelated
    // NewTarget. Anchor the native owner in the constructing binding's realm
    // instead, using the same retained-Document lifetime as native nodes.
    let owner = new_null_prototype_object(scope);
    set_private_value(
        scope,
        registry,
        CUSTOM_ELEMENTS_REGISTRY_OWNER_SLOT,
        owner.into(),
    );
}

pub(crate) fn registry_owner_host<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
) -> Option<*mut JsContextHost> {
    let registry = moli_webapi_declare::web_api_object_target(scope, registry)?;
    let owner = get_private_object(scope, registry, CUSTOM_ELEMENTS_REGISTRY_OWNER_SLOT)?;
    context_host_ptr_from_context_slot(owner.get_creation_context(scope)?)
}

pub(crate) fn mark_scoped_custom_elements_registry(
    scope: &mut v8::PinScope<'_, '_>,
    registry: v8::Local<'_, v8::Object>,
    scoped_id: u32,
) {
    let id_value = v8::BigInt::new_from_u64(scope, u64::from(scoped_id));
    set_private_value(
        scope,
        registry,
        CUSTOM_ELEMENTS_REGISTRY_SCOPED_ID_SLOT,
        id_value.into(),
    );
}

pub(crate) fn registry_store_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
) -> Option<CustomElementRegistryKey> {
    let owner = registry_owner_host(scope, registry)?;
    let owner = unsafe { &*owner }.custom_element_registry_owner_id();
    if let Some(id) =
        registry_u64_private_slot(scope, registry, CUSTOM_ELEMENTS_REGISTRY_SCOPED_ID_SLOT)
    {
        return Some(CustomElementRegistryKey::scoped(
            owner,
            u32::try_from(id).ok()?,
        ));
    }
    if let Some(child_handle) = registry_child_window_handle(scope, registry) {
        return Some(CustomElementRegistryKey::child(owner, child_handle));
    }
    Some(CustomElementRegistryKey::global(owner))
}

pub(crate) fn registry_association_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<CustomElementRegistryAssociation> {
    if value.is_undefined() {
        return None;
    }
    if value.is_null() {
        return Some(CustomElementRegistryAssociation::Null);
    }
    let registry = v8::Local::<v8::Object>::try_from(value).ok()?;
    if !web_api_interfaces::CustomElementRegistry::is_instance(scope, registry) {
        return None;
    }
    registry_store_key(scope, registry).map(CustomElementRegistryAssociation::Registry)
}

pub(crate) fn registry_association_matches_document(
    host: &JsContextHost,
    document_handle: DomHandle,
    association: CustomElementRegistryAssociation,
) -> bool {
    if association.is_document_default_backed_registry() {
        // A Document keeps its registry after its browsing context is removed.
        // Validate against that association, not a default inferred from live frames.
        return host.effective_custom_element_registry_association(document_handle) == association;
    }
    true
}

pub(crate) fn registry_child_window_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    registry_u64_private_slot(scope, registry, CUSTOM_ELEMENTS_REGISTRY_CHILD_HANDLE_SLOT)
        .map(|index| DomHandle::new(index as usize))
}

fn registry_u64_private_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<u64> {
    let value = get_private_value(scope, registry, slot)?;
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless.then_some(index);
    }
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| value as u64)
}
