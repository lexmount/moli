use super::registry::{CustomElementRegistryAssociation, RegistryAssociationRetarget};
use crate::{document_runtime::DomHandle, dom::native::DomHost, native_bridge::PageScriptHost};

pub(crate) fn registry_association_retargets_before_removal(
    host_ptr: *mut PageScriptHost,
    root: DomHandle,
) -> Vec<RegistryAssociationRetarget> {
    let host = unsafe { &*host_ptr };
    let has_shadow_roots = host.dom_host().has_shadow_roots();
    if !has_shadow_roots && !host.has_explicit_custom_element_registry_associations() {
        return Vec::new();
    }
    let mut retargets = Vec::new();
    for handle in host.dom_host().shadow_including_subtree(root) {
        if !should_record_import_clone_registry_association(host.dom_host(), handle) {
            continue;
        }
        let has_explicit_registry = host.custom_element_registry_association(handle).is_some();
        if !has_shadow_roots && !has_explicit_registry {
            continue;
        }
        let current = host.effective_custom_element_registry_association(handle);
        let Some(owner_document) = host.dom_host().owner_document_handle(handle) else {
            continue;
        };
        let document_default =
            host.default_custom_element_registry_association_for_document(owner_document);
        if current != document_default || has_explicit_registry {
            retargets.push(RegistryAssociationRetarget {
                handle,
                association: current,
            });
        }
    }
    retargets
}

pub(super) fn should_record_import_clone_registry_association(
    host: &DomHost,
    handle: DomHandle,
) -> bool {
    host.node(handle)
        .is_some_and(|node| node.is_document() || node.is_document_fragment() || node.is_element())
}

pub(crate) fn apply_registry_association_retargets(
    host_ptr: *mut PageScriptHost,
    retargets: &[RegistryAssociationRetarget],
) {
    unsafe { &mut *host_ptr }.apply_custom_element_registry_association_retargets(retargets);
}

pub(crate) fn apply_parser_created_null_registry_associations(
    host_ptr: *mut PageScriptHost,
    handles: &[DomHandle],
) {
    let host = unsafe { &mut *host_ptr };
    for &handle in handles {
        if should_record_import_clone_registry_association(host.dom_host(), handle) {
            host.set_custom_element_registry_association(
                handle,
                CustomElementRegistryAssociation::Null,
            );
        }
    }
    for root in host
        .dom_host()
        .declarative_shadow_roots_with_document_registry()
    {
        if host.custom_element_registry_association(root).is_none()
            && let Some(document) = host.dom_host().owner_document_handle(root)
        {
            let association = host.effective_custom_element_registry_association(document);
            host.set_custom_element_registry_association(root, association);
        }
    }
}
