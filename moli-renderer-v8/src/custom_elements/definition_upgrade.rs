use super::element_state::definition_name_for_handle;
use super::reactions::{CustomElementReaction, enqueue_custom_element_reaction};
use super::registry_roots::is_shadow_including_rooted_in_browsing_context_document;
use super::traversal::shadow_including_subtree_handles;
use super::with_custom_element_reaction_scope;
use super::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

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
    let host = unsafe { &*host_ptr };
    let Some(data) = host
        .custom_elements_for_registry_key(registry_key)
        .map(|store| store.registry_data())
    else {
        return;
    };
    let bindings = host.custom_element_registry_bindings.clone();
    let mut candidates = Vec::new();
    for associated in data.associated_documents() {
        let Some(owner) = bindings
            .as_ref()
            .and_then(|bindings| bindings.host(associated.owner))
        else {
            continue;
        };
        let target = owner.as_ptr();
        let target_host = unsafe { &*target };
        if target_host.browsing_context_is_closed()
            || target_host.context_host_lifecycle_handle().get()
                != crate::util::ContextHostLifecycle::Active
            || !is_shadow_including_rooted_in_browsing_context_document(
                target_host,
                associated.document,
            )
        {
            continue;
        }
        let handles = shadow_including_subtree_handles(target, associated.document)
            .into_iter()
            .filter(|handle| {
                target_host.effective_custom_element_registry_association(*handle)
                    == CustomElementRegistryAssociation::Registry(registry_key)
            })
            .filter(|handle| {
                definition_name_for_handle(target, *handle).as_deref() == Some(definition_name)
            })
            .collect::<Vec<_>>();
        candidates.push((owner, handles));
    }
    // Keep native owners pinned for this reaction turn. Their local handles
    // never enter another owner's store or reaction queue.
    for (owner, handles) in candidates {
        let target = owner.as_ptr();
        if unsafe { &*target }.browsing_context_is_closed() {
            continue;
        }
        with_custom_element_reaction_scope(scope, target, |scope| {
            for handle in handles {
                enqueue_custom_element_reaction(
                    scope,
                    target,
                    handle,
                    CustomElementReaction::Upgrade,
                );
            }
        });
    }
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
