use super::element_state::definition_name_for_handle;
use super::reactions::{CustomElementReaction, enqueue_custom_element_reaction};
use super::registry_roots::{
    is_shadow_including_rooted_in_browsing_context_document, shadow_including_root_document_handle,
};
use super::traversal::shadow_including_subtree_handles;
use super::with_custom_element_reaction_scope;
use super::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};
use std::collections::HashSet;

pub(crate) fn upgrade_existing_definition_for_child(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    child_handle: Option<DomHandle>,
    definition_name: &str,
) {
    let document_handle = match child_handle {
        Some(handle) => {
            let Some(document_handle) =
                unsafe { &*host_ptr }.child_browsing_context_document_handle(handle)
            else {
                return;
            };
            document_handle
        }
        None => unsafe { &*host_ptr }.dom_host().document_handle(),
    };
    let mut handles = Vec::new();
    collect_matching_definition_handles(host_ptr, document_handle, definition_name, &mut handles);
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        for handle in handles {
            enqueue_custom_element_reaction(
                scope,
                host_ptr,
                handle,
                CustomElementReaction::Upgrade,
            );
        }
    });
}

pub(crate) fn upgrade_existing_definition_for_registry(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    registry_key: CustomElementRegistryKey,
    definition_name: &str,
) {
    let handles = matching_definition_handles_for_registry_in_upgrade_order(
        host_ptr,
        registry_key,
        definition_name,
    );
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        for handle in handles {
            enqueue_custom_element_reaction(
                scope,
                host_ptr,
                handle,
                CustomElementReaction::Upgrade,
            );
        }
    });
}

fn matching_definition_handles_for_registry_in_upgrade_order(
    host_ptr: *mut JsContextHost,
    registry_key: CustomElementRegistryKey,
    definition_name: &str,
) -> Vec<DomHandle> {
    let host = unsafe { &*host_ptr };
    let target_association = CustomElementRegistryAssociation::Registry(registry_key);
    let mut documents = Vec::new();
    let mut seen_documents = HashSet::new();
    for (handle, association) in host.custom_element_registry_associations_in_order() {
        if association != target_association {
            continue;
        }
        let Some(document) = shadow_including_root_document_handle(host.dom_host(), handle) else {
            continue;
        };
        if !is_shadow_including_rooted_in_browsing_context_document(host, document) {
            continue;
        }
        if seen_documents.insert(document) {
            documents.push(document);
        }
    }

    let mut handles = Vec::new();
    let mut seen_handles = HashSet::new();
    for document in documents {
        for handle in shadow_including_subtree_handles(host_ptr, document) {
            if !seen_handles.insert(handle) {
                continue;
            }
            if unsafe { &*host_ptr }.effective_custom_element_registry_association(handle)
                != target_association
            {
                continue;
            }
            if definition_name_for_handle(host_ptr, handle).as_deref() == Some(definition_name) {
                handles.push(handle);
            }
        }
    }
    handles
}

fn collect_matching_definition_handles(
    host_ptr: *mut JsContextHost,
    root: DomHandle,
    definition_name: &str,
    handles: &mut Vec<DomHandle>,
) {
    for handle in shadow_including_subtree_handles(host_ptr, root) {
        if definition_name_for_handle(host_ptr, handle).as_deref() == Some(definition_name) {
            handles.push(handle);
        }
    }
}
