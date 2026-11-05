use std::collections::{HashMap, HashSet};

use super::{BridgeIdentityStore, JsContextHost};
use crate::{
    custom_elements::{CustomElementRegistryAssociation, CustomElementRegistryKey},
    document_runtime::DomHandle,
};

fn registry_for_node(host: &JsContextHost, node: DomHandle) -> Option<CustomElementRegistryKey> {
    let CustomElementRegistryAssociation::Registry(key) =
        host.effective_custom_element_registry_association(node)
    else {
        return None;
    };
    host.custom_elements_for_registry_key(key)
        .is_some_and(|store| store.registry_data().has_shared_native_owners())
        .then_some(key)
}

fn parent(host: &JsContextHost, node: DomHandle) -> Option<DomHandle> {
    host.dom_host()
        .node(node)
        .and_then(|node| node.parent_node())
        .or_else(|| host.dom_host().shadow_root_host(node))
}

pub(super) fn registry_values_for_subtree<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: *mut JsContextHost,
    root: DomHandle,
) -> Vec<v8::Local<'s, v8::Value>> {
    let host = unsafe { &*host };
    let Some(bindings) = &host.custom_element_registry_bindings else {
        return Vec::new();
    };
    let mut keys = HashSet::new();
    if let Some(key) = registry_for_node(host, root) {
        keys.insert(key);
    }
    for (handle, _) in host.custom_element_registry_associations_in_order() {
        let Some(key) = registry_for_node(host, handle) else {
            continue;
        };
        let mut current = Some(handle);
        while let Some(handle) = current {
            if handle == root {
                keys.insert(key);
                break;
            }
            current = parent(host, handle);
        }
    }
    keys.into_iter()
        .filter_map(|key| bindings.wrapper(scope, key).map(Into::into))
        .collect()
}

impl BridgeIdentityStore {
    /// Mirrors only registry references into the V8 graph. Connected trees
    /// trace through their Document anchor; detached subtrees through retained
    /// ancestor/node anchors. The native index and ownership records stay weak.
    pub(in crate::native_bridge) fn synchronize_registry_references(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) {
        let host = unsafe { &*host_ptr };
        if !host.custom_element_registry_references_enabled.get() {
            return;
        }
        let version = host.dom_host().query_version();
        if host.custom_element_registry_references_version.get() == Some(version) {
            return;
        }
        host.custom_element_registry_references_version
            .set(Some(version));
        let Some(bindings) = host.custom_element_registry_bindings.clone() else {
            return;
        };
        let associations = host
            .custom_element_registry_associations_in_order()
            .collect::<Vec<_>>();
        let mut nodes = self
            .node_ownership
            .iter()
            .map(|(&handle, ownership)| (handle, ownership.clone()))
            .collect::<Vec<_>>();
        let mut roots: HashMap<DomHandle, HashSet<CustomElementRegistryKey>> = HashMap::new();
        // A retained non-element also traces its owner Document and the
        // registry of its containing shadow tree.
        for (handle, ownership) in &nodes {
            if ownership.existing_anchor(scope).is_none() {
                continue;
            }
            if let Some(key) = registry_for_node(host, *handle) {
                roots.entry(*handle).or_default().insert(key);
            }
            if let Some(document) = host.dom_host().owner_document_handle(*handle)
                && document != *handle
            {
                let document = self.node_ownership(scope, host_ptr, document);
                ownership.trace_document(scope, &document);
            }
        }
        // Include Document records first materialized by the loop above.
        nodes = self
            .node_ownership
            .iter()
            .map(|(&handle, ownership)| (handle, ownership.clone()))
            .collect();
        for (handle, _) in associations {
            let Some(key) = registry_for_node(host, handle) else {
                continue;
            };
            let mut current = Some(handle);
            while let Some(handle) = current {
                if self.node_ownership.contains_key(&handle) {
                    roots.entry(handle).or_default().insert(key);
                }
                current = parent(host, handle);
            }
        }
        for (handle, ownership) in nodes {
            let values = roots
                .remove(&handle)
                .into_iter()
                .flatten()
                .filter_map(|key| bindings.wrapper(scope, key).map(Into::into))
                .collect::<Vec<_>>();
            ownership.trace_registries(scope, &values);
        }
        host.custom_element_registry_references_version
            .set(Some(version));
    }
}
