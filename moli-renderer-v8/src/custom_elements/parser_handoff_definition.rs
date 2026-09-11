use super::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use crate::{
    document_runtime::DomHandle,
    dom::native::Attribute,
    native_bridge::{JsContextHost, document::XHTML_NS},
};

#[derive(Clone, Debug)]
pub(super) struct ParserCustomElementDefinitionMatch {
    pub(super) registry_association: CustomElementRegistryAssociation,
    pub(super) registry_key: CustomElementRegistryKey,
    pub(super) definition_name: String,
    pub(super) local_name: String,
}

pub(super) fn lookup_parser_custom_element_definition_for_token(
    host_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    local_name: &str,
    namespace: &str,
    token_attributes: &[Attribute],
    intended_parent: Option<DomHandle>,
) -> Option<ParserCustomElementDefinitionMatch> {
    if namespace != XHTML_NS {
        return None;
    }
    let host = unsafe { &*host_ptr };
    let document_default =
        host.default_custom_element_registry_association_for_document(document_handle);
    let registry_association = intended_parent
        .filter(|parent| host.dom_host().owner_document_handle(*parent) == Some(document_handle))
        .map(|parent| host.effective_custom_element_registry_association(parent))
        .unwrap_or(document_default);
    let CustomElementRegistryAssociation::Registry(registry_key) = registry_association else {
        return None;
    };
    let store = host.custom_elements_for_registry_key(registry_key)?;
    let is_name = parser_token_is_attribute(token_attributes);
    let definition_name = match is_name {
        Some(is_name) => store
            .definition_extends_local_name(is_name)
            .filter(|extends_local_name| *extends_local_name == local_name)
            .map(|_| is_name.to_owned())?,
        None if store.has_autonomous_definition(local_name) => local_name.to_owned(),
        None => return None,
    };
    Some(ParserCustomElementDefinitionMatch {
        registry_association,
        registry_key,
        definition_name,
        local_name: local_name.to_owned(),
    })
}

fn parser_token_is_attribute(token_attributes: &[Attribute]) -> Option<&str> {
    token_attributes
        .iter()
        .find(|attribute| {
            attribute.namespace().is_empty()
                && attribute.prefix().is_none()
                && attribute.local_name() == "is"
        })
        .map(Attribute::value)
}
