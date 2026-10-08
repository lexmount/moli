use crate::dom::{
    custom_elements::is_valid_custom_element_name as is_valid_dom_custom_element_name,
    native::{CustomElementState, DomStringValue, Node},
};
use dom::ElementState as StyloElementState;

use super::super::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, document::XHTML_NS},
};

pub(super) fn create_element_with_owner_document(
    host_ptr: *mut JsContextHost,
    owner_document: DomHandle,
    local_name: &str,
) -> Option<DomHandle> {
    let runtime = unsafe { &mut *host_ptr };
    let handle = runtime.create_element(local_name);
    if runtime.dom_host().owner_document_handle(handle) != Some(owner_document) {
        runtime.initialize_new_native_node_owner_document(owner_document, handle)?;
    }
    Some(handle)
}

pub(super) fn set_dom_custom_element_state(
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    state: CustomElementState,
) {
    let old_style_state = unsafe { &*host_ptr }.retained_current_element_state(handle);
    let was_defined = unsafe { &*host_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(custom_element_state_matches_defined_pseudo);
    let changed = unsafe { &mut *host_ptr }
        .dom_host_mut()
        .set_custom_element_state(handle, state);
    if changed {
        unsafe { &mut *host_ptr }.note_style_subtree_context_change(handle);
        let is_defined = unsafe { &*host_ptr }
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(custom_element_state_matches_defined_pseudo);
        if was_defined != is_defined {
            unsafe { &mut *host_ptr }.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::DEFINED,
                old_style_state,
            );
        }
    }
}

fn custom_element_state_matches_defined_pseudo(element: &crate::dom::native::Element) -> bool {
    if element.namespace() != XHTML_NS {
        return true;
    }
    match element.custom_element_state() {
        CustomElementState::Custom => true,
        CustomElementState::Undefined | CustomElementState::Failed => false,
        CustomElementState::Uncustomized => !is_valid_dom_custom_element_name(element.local_name()),
    }
}

pub(super) fn set_dom_custom_element_is_name(
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    is_name: impl Into<DomStringValue>,
) {
    let changed = unsafe { &mut *host_ptr }
        .dom_host_mut()
        .set_custom_element_is_name(handle, Some(is_name.into()));
    if changed {
        unsafe { &mut *host_ptr }.note_style_subtree_context_change(handle);
    }
}

pub(super) fn set_dom_element_prefix(
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    prefix: Option<String>,
) {
    let changed = unsafe { &mut *host_ptr }
        .dom_host_mut()
        .set_element_prefix(handle, prefix);
    if changed {
        unsafe { &mut *host_ptr }.note_style_subtree_context_change(handle);
    }
}

pub(super) fn definition_name_for_handle(
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    if unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_failed_construction_handle(handle))
    {
        return None;
    }
    let node = unsafe { &*host_ptr }.dom_host().node(handle)?;
    if node.namespace() != Some(XHTML_NS) {
        return None;
    }
    let local_name = node.local_name()?;
    let custom_elements = unsafe { &*host_ptr }.custom_elements_for_node_handle(handle)?;
    if custom_elements.has_autonomous_definition(local_name) {
        return Some(local_name.to_owned());
    }
    let element = node.as_element()?;
    let is_name = match element.custom_element_is_name() {
        Some(value) => value.as_str(),
        None => element.attribute("is"),
    }?;
    let extends_local_name = custom_elements.definition_extends_local_name(is_name)?;
    (extends_local_name == local_name).then(|| is_name.to_owned())
}

pub(crate) fn is_form_associated_custom_element_handle(
    host: &JsContextHost,
    handle: DomHandle,
) -> bool {
    let Some(custom_elements) = host.custom_elements_for_node_handle(handle) else {
        return false;
    };
    if custom_elements.is_failed_construction_handle(handle) {
        return false;
    }
    let Some(node) = host.dom_host().node(handle) else {
        return false;
    };
    let Some(local_name) = node.local_name() else {
        return false;
    };
    let is_name = node
        .as_element()
        .and_then(|element| match element.custom_element_is_name() {
            Some(value) => value.as_str(),
            None => element.attribute("is"),
        });
    custom_elements.is_form_associated_definition_for_element(local_name, is_name)
}

pub(crate) fn preserves_custom_element_identity(
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| {
            store.is_upgraded_handle(handle) || store.is_pending_construction_handle(handle)
        })
}
