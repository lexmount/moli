use super::{
    registry::{
        CustomElementAdoptionPlan, CustomElementRegistryAssociation, RegistryAssociationRetarget,
    },
    registry_adoption_callbacks::adoption_callback_targets,
    traversal::shadow_including_child_handles,
};
use crate::{document_runtime::DomHandle, dom::native::Node, native_bridge::JsContextHost};

pub(crate) fn adoption_plan_for_roots_before_adoption(
    host_ptr: *mut JsContextHost,
    roots: &[DomHandle],
    new_document: DomHandle,
    crosses_documents: bool,
) -> CustomElementAdoptionPlan {
    let host = unsafe { &*host_ptr };
    if !crosses_documents && !host.has_explicit_custom_element_registry_associations() {
        return CustomElementAdoptionPlan::default();
    }

    let mut targets = Vec::new();
    let mut registry_retargets = Vec::new();
    for &root in roots {
        if crosses_documents {
            targets.extend(adoption_callback_targets(
                host.dom_host(),
                root,
                new_document,
            ));
        }
        registry_retargets.extend(registry_association_retargets_before_adoption(
            host_ptr,
            root,
            new_document,
        ));
    }
    CustomElementAdoptionPlan {
        targets,
        registry_retargets,
    }
}

fn registry_association_retargets_before_adoption(
    host_ptr: *mut JsContextHost,
    root: DomHandle,
    new_document: DomHandle,
) -> Vec<RegistryAssociationRetarget> {
    let mut retargets = Vec::new();
    let preserve_same_document_inherited =
        should_preserve_same_document_registry_association(host_ptr, root);
    collect_registry_association_retargets_before_adoption(
        host_ptr,
        root,
        new_document,
        preserve_same_document_inherited,
        false,
        &mut retargets,
    );
    retargets
}

fn should_preserve_same_document_registry_association(
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> bool {
    let host = unsafe { &*host_ptr }.dom_host();
    host.node(root)
        .and_then(Node::parent_node)
        .is_some_and(|parent| {
            host.is_shadow_root(parent)
                || !host.node(parent).is_some_and(Node::is_document_fragment)
        })
}

fn collect_registry_association_retargets_before_adoption(
    host_ptr: *mut JsContextHost,
    root: DomHandle,
    new_document: DomHandle,
    preserve_same_document_inherited: bool,
    preserve_null_shadow_registry: bool,
    retargets: &mut Vec<RegistryAssociationRetarget>,
) {
    let host = unsafe { &*host_ptr };
    let mut stack = vec![(
        root,
        preserve_same_document_inherited,
        preserve_null_shadow_registry,
    )];
    while let Some((handle, preserve_same_document_inherited, preserve_null_shadow_registry)) =
        stack.pop()
    {
        let old_document = host.dom_host().owner_document_handle(handle);
        let current = host.effective_custom_element_registry_association(handle);
        let new_document_default =
            host.default_custom_element_registry_association_for_document(new_document);
        let crosses_document = old_document != Some(new_document);
        let preserves_null_shadow_registry = preserve_null_shadow_registry
            || host
                .dom_host()
                .shadow_root_uses_null_custom_element_registry(handle)
                .unwrap_or(false);
        let association = match current {
            CustomElementRegistryAssociation::Registry(key) if key.is_scoped() => current,
            CustomElementRegistryAssociation::Null if preserves_null_shadow_registry => {
                CustomElementRegistryAssociation::Null
            }
            _ if crosses_document => new_document_default,
            CustomElementRegistryAssociation::Registry(key)
                if key.is_document_default_backed() && current != new_document_default =>
            {
                new_document_default
            }
            _ => current,
        };
        let has_explicit_association = host.custom_element_registry_association(handle).is_some();
        let should_retarget =
            crosses_document || preserve_same_document_inherited || has_explicit_association;
        if should_retarget {
            retargets.push(RegistryAssociationRetarget {
                handle,
                association,
            });
        }
        let children = shadow_including_child_handles(host_ptr, handle);
        let preserve_descendants =
            preserve_same_document_inherited || has_explicit_association || should_retarget;
        stack.extend(
            children
                .into_iter()
                .rev()
                .map(|child| (child, preserve_descendants, preserves_null_shadow_registry)),
        );
    }
}
