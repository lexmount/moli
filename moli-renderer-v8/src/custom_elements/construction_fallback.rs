use super::CustomElementRegistryAssociation;
use super::construction_failure::{
    ConstructionFailure, report_custom_element_construction_failure,
};
use super::element_state::{
    create_element_with_owner_document, set_dom_custom_element_is_name,
    set_dom_custom_element_state, set_dom_element_prefix,
};

use super::super::{document_runtime::DomHandle, native_bridge::PageScriptHost};
use crate::dom::native::CustomElementState;

pub(super) fn failed_custom_element_construction_fallback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    original_handle: DomHandle,
    owner_document: DomHandle,
    explicit_registry_association: Option<CustomElementRegistryAssociation>,
    constructor: v8::Local<'s, v8::Function>,
    definition_name: &str,
    local_name: &str,
    post_construction_prefix: Option<&str>,
    failure: ConstructionFailure<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(original_handle)
        .discard_pending_construction(original_handle);
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(original_handle)
        .mark_failed_construction_handle(original_handle);
    unsafe { &mut *host_ptr }
        .custom_element_reactions_mut()
        .clear_reactions(original_handle);
    set_dom_custom_element_state(host_ptr, original_handle, CustomElementState::Failed);
    report_custom_element_construction_failure(scope, host_ptr, Some(constructor), failure);

    let fallback_handle = create_element_with_owner_document(host_ptr, owner_document, local_name)?;
    if let Some(registry_association) = explicit_registry_association {
        unsafe { &mut *host_ptr }
            .set_custom_element_registry_association(fallback_handle, registry_association);
    }
    if definition_name != local_name {
        set_dom_custom_element_is_name(host_ptr, fallback_handle, definition_name);
    }
    if let Some(prefix) = post_construction_prefix {
        set_dom_element_prefix(host_ptr, fallback_handle, Some(prefix.to_owned()));
    }
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(fallback_handle)
        .mark_failed_construction_handle(fallback_handle);
    set_dom_custom_element_state(host_ptr, fallback_handle, CustomElementState::Failed);
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, fallback_handle)
}
