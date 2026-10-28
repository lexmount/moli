use super::registry::CustomElementRegistryAssociation;
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(super) fn registry_association_for_clone_source(
    host: &JsContextHost,
    source: DomHandle,
    target_document: DomHandle,
) -> CustomElementRegistryAssociation {
    match host.effective_custom_element_registry_association(source) {
        CustomElementRegistryAssociation::Registry(key) if key.is_scoped() => {
            CustomElementRegistryAssociation::Registry(key)
        }
        CustomElementRegistryAssociation::Null => CustomElementRegistryAssociation::Null,
        CustomElementRegistryAssociation::Registry(_) => {
            host.default_custom_element_registry_association_for_document(target_document)
        }
    }
}

pub(super) fn registry_association_for_import_clone_source(
    host: &JsContextHost,
    source: DomHandle,
    target_document: DomHandle,
    fallback_registry: CustomElementRegistryAssociation,
) -> CustomElementRegistryAssociation {
    let target_default =
        host.default_custom_element_registry_association_for_document(target_document);
    match host.effective_custom_element_registry_association(source) {
        CustomElementRegistryAssociation::Registry(key) if key.is_scoped() => {
            CustomElementRegistryAssociation::Registry(key)
        }
        CustomElementRegistryAssociation::Null => fallback_registry,
        CustomElementRegistryAssociation::Registry(_) => target_default,
    }
}
