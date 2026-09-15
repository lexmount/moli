use super::PendingInitialAttribute;
use super::element_state::set_dom_custom_element_state;
use super::reactions::{CustomElementReaction, enqueue_custom_element_reaction};
use super::registry_roots::is_shadow_including_rooted_in_document;
use crate::{
    document_runtime::DomHandle,
    dom::native::{CustomElementState, Node},
    native_bridge::JsContextHost,
};

pub(super) fn complete_existing_custom_element_upgrade<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    definition_name: &str,
) {
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(handle)
        .mark_upgraded_handle(handle, definition_name);
    set_dom_custom_element_state(host_ptr, handle, CustomElementState::Custom);
    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(handle)
        .finish_construction(handle);
}

pub(super) fn enqueue_existing_upgrade_callbacks(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    initial_attributes: Vec<PendingInitialAttribute>,
) {
    for PendingInitialAttribute {
        name,
        namespace,
        value,
    } in initial_attributes
    {
        enqueue_custom_element_reaction(
            scope,
            host_ptr,
            handle,
            CustomElementReaction::AttributeChanged {
                name,
                namespace,
                old_value: None,
                new_value: Some(value),
            },
        );
    }
    if is_shadow_including_rooted_in_document(unsafe { &*host_ptr }.dom_host(), handle) {
        enqueue_custom_element_reaction(scope, host_ptr, handle, CustomElementReaction::Connected);
    }
}

pub(super) fn observed_attributes_with_current_values(
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    definition_name: &str,
) -> Vec<PendingInitialAttribute> {
    let Some(observed_attributes) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.observed_attributes_for_definition(definition_name))
    else {
        return Vec::new();
    };
    let Some(element) = unsafe { &*host_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
    else {
        return Vec::new();
    };
    element
        .attributes()
        .iter()
        .filter(|attribute| {
            observed_attributes
                .iter()
                .any(|observed| observed == attribute.local_name())
        })
        .map(|attribute| PendingInitialAttribute {
            name: attribute.local_name().to_owned(),
            namespace: (!attribute.namespace().is_empty())
                .then(|| attribute.namespace().to_owned()),
            value: attribute.value().to_owned(),
        })
        .collect()
}
