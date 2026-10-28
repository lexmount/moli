use super::{
    registry::{CustomElementRegistryAssociation, RegistryAssociationRetarget},
    traversal::shadow_including_subtree_handles,
};
use crate::{document_runtime::DomHandle, dom::native::DomHost, native_bridge::JsContextHost};

pub(crate) fn registry_association_retargets_before_removal(
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> Vec<RegistryAssociationRetarget> {
    let host = unsafe { &*host_ptr };
    let mut retargets = Vec::new();
    for handle in shadow_including_subtree_handles(host_ptr, root) {
        if !should_record_import_clone_registry_association(host.dom_host(), handle) {
            continue;
        }
        let current = host.effective_custom_element_registry_association(handle);
        let Some(owner_document) = host.dom_host().owner_document_handle(handle) else {
            continue;
        };
        let document_default =
            host.default_custom_element_registry_association_for_document(owner_document);
        if current != document_default || host.custom_element_registry_association(handle).is_some()
        {
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
    host_ptr: *mut JsContextHost,
    retargets: &[RegistryAssociationRetarget],
) {
    unsafe { &mut *host_ptr }.apply_custom_element_registry_association_retargets(retargets);
}

pub(crate) fn apply_parser_created_registry_associations(
    host_ptr: *mut JsContextHost,
    associations: &[crate::parser::ParserCustomElementRegistryAssociation],
) {
    use crate::parser::ParserCustomElementRegistryAssociation;

    let host = unsafe { &mut *host_ptr };
    for &association in associations {
        let (handle, registry) = match association {
            ParserCustomElementRegistryAssociation::Null(handle) => {
                (handle, CustomElementRegistryAssociation::Null)
            }
            ParserCustomElementRegistryAssociation::DocumentDefault(root) => {
                if host.custom_element_registry_association(root).is_some() {
                    continue;
                }
                let Some(document) = host.dom_host().owner_document_handle(root) else {
                    continue;
                };
                (
                    root,
                    host.effective_custom_element_registry_association(document),
                )
            }
        };
        if should_record_import_clone_registry_association(host.dom_host(), handle) {
            host.set_custom_element_registry_association(handle, registry);
        }
    }
}
