use super::{
    registry::{CustomElementRegistryAssociation, RegistryAssociationRetarget},
    registry_clone_association::{
        registry_association_for_clone_source, registry_association_for_import_clone_source,
    },
    registry_retarget::should_record_import_clone_registry_association,
};
use crate::{
    document_runtime::DomHandle,
    dom::native::{Element, Node},
    native_bridge::JsContextHost,
};

pub(crate) fn registry_association_retargets_for_import_clone(
    host_ptr: *mut JsContextHost,
    source_root: DomHandle,
    clone_root: DomHandle,
    target_document: DomHandle,
    fallback_registry: Option<CustomElementRegistryAssociation>,
) -> Vec<RegistryAssociationRetarget> {
    let host = unsafe { &*host_ptr };
    let fallback_registry = fallback_registry.unwrap_or_else(|| {
        host.default_custom_element_registry_association_for_document(target_document)
    });
    let mut retargets = Vec::new();
    collect_registry_association_retargets_for_import_clone(
        host,
        source_root,
        clone_root,
        target_document,
        fallback_registry,
        &mut retargets,
    );
    retargets
}

pub(crate) fn registry_association_retargets_for_clone(
    host_ptr: *mut JsContextHost,
    source_root: DomHandle,
    clone_root: DomHandle,
) -> Vec<RegistryAssociationRetarget> {
    let host = unsafe { &*host_ptr };
    let target_document = host
        .dom_host()
        .node(clone_root)
        .filter(|node| node.is_document())
        .map(|_| clone_root)
        .or_else(|| {
            host.dom_host()
                .node(clone_root)
                .and_then(Node::owner_document)
        })
        .unwrap_or_else(|| host.dom_host().document_handle());
    let mut retargets = Vec::new();
    collect_registry_association_retargets_for_clone(
        host,
        source_root,
        clone_root,
        target_document,
        &mut retargets,
    );
    retargets
}

fn collect_registry_association_retargets_for_import_clone(
    host: &JsContextHost,
    source: DomHandle,
    clone: DomHandle,
    target_document: DomHandle,
    fallback_registry: CustomElementRegistryAssociation,
    retargets: &mut Vec<RegistryAssociationRetarget>,
) {
    let mut stack = vec![(source, clone, fallback_registry)];
    while let Some((source, clone, fallback_registry)) = stack.pop() {
        if should_record_import_clone_registry_association(host.dom_host(), clone) {
            // ShadowRoot's registry is copied independently of the fallback
            // used to clone the host and its light children.
            let association = if host.dom_host().is_shadow_root(source) {
                registry_association_for_clone_source(host, source, target_document)
            } else {
                registry_association_for_import_clone_source(
                    host,
                    source,
                    target_document,
                    fallback_registry,
                )
            };
            retargets.push(RegistryAssociationRetarget {
                handle: clone,
                association,
            });
        }

        let source_children = host.dom_host().child_handles(source).collect::<Vec<_>>();
        let clone_children = host.dom_host().child_handles(clone).collect::<Vec<_>>();
        for (source_child, clone_child) in source_children.into_iter().zip(clone_children).rev() {
            stack.push((source_child, clone_child, fallback_registry));
        }
        if let (Some(source_shadow), Some(clone_shadow)) = (
            host.dom_host().shadow_root_handle(source),
            host.dom_host().shadow_root_handle(clone),
        ) {
            // The DOM cloning algorithm intentionally omits fallbackRegistry
            // when recursively cloning shadow children. Explicit registries
            // on those children still retain their own identity.
            stack.push((
                source_shadow,
                clone_shadow,
                CustomElementRegistryAssociation::Null,
            ));
        }
    }
}

fn collect_registry_association_retargets_for_clone(
    host: &JsContextHost,
    source: DomHandle,
    clone: DomHandle,
    target_document: DomHandle,
    retargets: &mut Vec<RegistryAssociationRetarget>,
) {
    let mut stack = vec![(source, clone)];
    while let Some((source, clone)) = stack.pop() {
        if should_record_import_clone_registry_association(host.dom_host(), clone) {
            retargets.push(RegistryAssociationRetarget {
                handle: clone,
                association: registry_association_for_clone_source(host, source, target_document),
            });
        }

        let source_children = host.dom_host().child_handles(source).collect::<Vec<_>>();
        let clone_children = host.dom_host().child_handles(clone).collect::<Vec<_>>();
        for (source_child, clone_child) in source_children.into_iter().zip(clone_children).rev() {
            stack.push((source_child, clone_child));
        }
        if let (Some(source_template), Some(clone_template)) = (
            host.dom_host()
                .node(source)
                .and_then(Node::as_element)
                .and_then(Element::template_contents),
            host.dom_host()
                .node(clone)
                .and_then(Node::as_element)
                .and_then(Element::template_contents),
        ) {
            stack.push((source_template, clone_template));
        }
        if let (Some(source_shadow), Some(clone_shadow)) = (
            host.dom_host().shadow_root_handle(source),
            host.dom_host().shadow_root_handle(clone),
        ) {
            stack.push((source_shadow, clone_shadow));
        }
    }
}
