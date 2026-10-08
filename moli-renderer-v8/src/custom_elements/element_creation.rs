use super::CustomElementRegistryAssociation;
use super::construction_runtime::create_custom_element_for_registry_key;
use super::element_state::{
    create_element_with_owner_document, set_dom_custom_element_is_name,
    set_dom_custom_element_state, set_dom_element_prefix,
};
use crate::dom::native::{CustomElementState, DomStringValue};
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(crate) fn create_element_for_document_local_name_is_and_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    owner_document: DomHandle,
    local_name: &str,
    is_name: Option<&[u16]>,
    explicit_registry_association: Option<CustomElementRegistryAssociation>,
    post_construction_prefix: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let is_name = is_name.map(DomStringValue::from_utf16);
    let lookup_registry_association = explicit_registry_association.unwrap_or_else(|| {
        unsafe { &*host_ptr }.effective_custom_element_registry_association(owner_document)
    });
    if let CustomElementRegistryAssociation::Registry(registry_key) = lookup_registry_association {
        match is_name.as_ref().and_then(DomStringValue::as_str) {
            Some(is_name)
                if unsafe { &*host_ptr }
                    .custom_elements_for_registry_key(registry_key)
                    .and_then(|store| store.definition_extends_local_name(is_name))
                    .is_some_and(|extends_local_name| extends_local_name == local_name) =>
            {
                return create_custom_element_for_registry_key(
                    scope,
                    host_ptr,
                    registry_key,
                    owner_document,
                    explicit_registry_association,
                    is_name,
                    local_name,
                    post_construction_prefix,
                );
            }
            _ if unsafe { &*host_ptr }
                .custom_elements_for_registry_key(registry_key)
                .is_some_and(|store| store.has_autonomous_definition(local_name)) =>
            {
                return create_custom_element_for_registry_key(
                    scope,
                    host_ptr,
                    registry_key,
                    owner_document,
                    explicit_registry_association,
                    local_name,
                    local_name,
                    post_construction_prefix,
                );
            }
            _ => {}
        }
    }
    if let Some(registry_association) = explicit_registry_association {
        let handle = create_element_with_owner_document(host_ptr, owner_document, local_name)?;
        unsafe { &mut *host_ptr }
            .set_custom_element_registry_association(handle, registry_association);
        if let Some(is_name) = &is_name {
            set_dom_custom_element_is_name(host_ptr, handle, is_name);
            set_dom_custom_element_state(host_ptr, handle, CustomElementState::Undefined);
        }
        if let Some(prefix) = post_construction_prefix {
            set_dom_element_prefix(host_ptr, handle, Some(prefix.to_owned()));
        }
        return unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, handle);
    }
    let handle = create_element_with_owner_document(host_ptr, owner_document, local_name)?;
    if let Some(is_name) = &is_name {
        set_dom_custom_element_is_name(host_ptr, handle, is_name);
        set_dom_custom_element_state(host_ptr, handle, CustomElementState::Undefined);
    }
    if let Some(prefix) = post_construction_prefix {
        set_dom_element_prefix(host_ptr, handle, Some(prefix.to_owned()));
    }
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
}
