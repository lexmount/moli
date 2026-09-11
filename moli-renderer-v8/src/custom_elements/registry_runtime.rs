use super::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use crate::web_api_interfaces;
use crate::{
    document_runtime::DomHandle,
    native_bridge::JsContextHost,
    util::{get_private_value, set_private_value, v8str},
};

pub(super) const CUSTOM_ELEMENTS_REGISTRY_CHILD_HANDLE_SLOT: &str =
    "__moliCustomElementsRegistryChildHandle";
const CUSTOM_ELEMENTS_REGISTRY_SCOPED_ID_SLOT: &str = "__moliCustomElementsRegistryScopedId";

pub(crate) fn mark_scoped_custom_elements_registry(
    scope: &mut v8::PinScope<'_, '_>,
    registry: v8::Local<'_, v8::Object>,
    scoped_id: u64,
) {
    let id_value = v8::BigInt::new_from_u64(scope, scoped_id);
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
) -> CustomElementRegistryKey {
    if let Some(id) =
        registry_u64_private_slot(scope, registry, CUSTOM_ELEMENTS_REGISTRY_SCOPED_ID_SLOT)
    {
        return CustomElementRegistryKey::Scoped(id);
    }
    if let Some(child_handle) = registry_child_window_handle(scope, registry) {
        return CustomElementRegistryKey::Child(child_handle);
    }
    CustomElementRegistryKey::Global
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
    Some(CustomElementRegistryAssociation::Registry(
        registry_store_key(scope, registry),
    ))
}

pub(crate) fn registry_association_from_create_options_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<CustomElementRegistryAssociation> {
    if value.is_null_or_undefined() || value.is_string() {
        return None;
    }
    let options = value.to_object(scope)?;
    let registry = options.get(scope, v8str(scope, "customElementRegistry").into())?;
    registry_association_from_value(scope, registry)
}

pub(crate) fn registry_association_matches_document_default(
    host: &JsContextHost,
    document_handle: DomHandle,
    association: CustomElementRegistryAssociation,
) -> bool {
    if association.is_document_default_backed_registry() {
        return host.default_custom_element_registry_association_for_document(document_handle)
            == association;
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
