use super::registry::{
    AdoptionCallbackTarget, CustomElementAdoptionPlan, CustomElementRegistryAssociation,
    RegistryAssociationRetarget,
};
use crate::{
    document_runtime::DomHandle,
    dom::native::{DomAdoptionTarget, Node},
    native_bridge::JsContextHost,
};

pub(crate) fn roots_need_registry_retargets(host: &JsContextHost, roots: &[DomHandle]) -> bool {
    // Insertion roots share a node document; removal does not adopt them.
    // Without root preservation, only explicit associations in these subtrees
    // can produce retargets. Unrelated clones must not force an adoption plan.
    if roots
        .iter()
        .any(|&root| should_preserve_same_document_registry_association(host, root))
    {
        return true;
    }
    host.has_explicit_custom_element_registry_associations()
        && roots.iter().any(|&root| {
            super::traversal::shadow_including_subtree_handles(host as *const _ as *mut _, root)
                .into_iter()
                .any(|handle| host.custom_element_registry_association(handle).is_some())
        })
}

pub(crate) fn adoption_plan_before_adoption(
    host_ptr: *mut JsContextHost,
    adoption_targets: &[DomAdoptionTarget],
) -> CustomElementAdoptionPlan {
    let host = unsafe { &*host_ptr };
    let mut plan = CustomElementAdoptionPlan::default();
    let mut descendant_policies: Vec<(bool, bool)> = Vec::with_capacity(adoption_targets.len());
    for target in adoption_targets {
        let DomAdoptionTarget {
            handle,
            old_document,
            new_document,
            parent,
        } = *target;
        let (preserve_same_document_inherited, preserve_null_shadow_registry) = parent
            .map(|parent| descendant_policies[parent])
            .unwrap_or_else(|| {
                (
                    should_preserve_same_document_registry_association(host, handle),
                    false,
                )
            });
        let crosses_document = old_document != new_document;
        if crosses_document {
            plan.targets.push(AdoptionCallbackTarget {
                handle,
                old_document,
                new_document,
            });
        }
        let current = host.effective_custom_element_registry_association(handle);
        let new_document_default =
            host.default_custom_element_registry_association_for_document(new_document);
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
            plan.registry_retargets.push(RegistryAssociationRetarget {
                handle,
                association,
            });
        }
        descendant_policies.push((
            preserve_same_document_inherited || has_explicit_association || should_retarget,
            preserves_null_shadow_registry,
        ));
    }
    plan
}

fn should_preserve_same_document_registry_association(
    runtime: &JsContextHost,
    root: DomHandle,
) -> bool {
    let host = runtime.dom_host();
    if host
        .owner_document_handle(root)
        .is_some_and(|document| runtime.document_uses_scoped_custom_element_registries(document))
    {
        return true;
    }
    host.node(root)
        .and_then(Node::parent_node)
        .is_some_and(|parent| {
            host.is_shadow_root(parent)
                || !host.node(parent).is_some_and(Node::is_document_fragment)
        })
}
