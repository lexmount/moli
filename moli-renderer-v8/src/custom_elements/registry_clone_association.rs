use super::registry::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(super) fn registry_association_for_clone_source(
    host: &JsContextHost,
    source: DomHandle,
    target_document: DomHandle,
) -> CustomElementRegistryAssociation {
    match host.effective_custom_element_registry_association(source) {
        CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Scoped(
            owner,
            scoped_id,
        )) => CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Scoped(
            owner, scoped_id,
        )),
        CustomElementRegistryAssociation::Null => CustomElementRegistryAssociation::Null,
        CustomElementRegistryAssociation::Registry(
            CustomElementRegistryKey::Global(_) | CustomElementRegistryKey::Child(_, _),
        ) => host.default_custom_element_registry_association_for_document(target_document),
    }
}

pub(super) fn registry_association_for_import_clone_source(
    host: &JsContextHost,
    source: DomHandle,
    target_document: DomHandle,
    fallback_registry: Option<CustomElementRegistryAssociation>,
    preserve_null_shadow_registry: bool,
) -> Option<CustomElementRegistryAssociation> {
    if preserve_null_shadow_registry {
        return None;
    }

    let target_default =
        host.default_custom_element_registry_association_for_document(target_document);
    match host.effective_custom_element_registry_association(source) {
        CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Scoped(
            owner,
            scoped_id,
        )) => Some(CustomElementRegistryAssociation::Registry(
            CustomElementRegistryKey::Scoped(owner, scoped_id),
        )),
        CustomElementRegistryAssociation::Null => Some(fallback_registry.unwrap_or(target_default)),
        CustomElementRegistryAssociation::Registry(
            CustomElementRegistryKey::Global(_) | CustomElementRegistryKey::Child(_, _),
        ) => Some(target_default),
    }
}
