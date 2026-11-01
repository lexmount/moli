use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::document_runtime::DomHandle;
use crate::frame_owner_model::DocumentId;
use moli_page_types::DocumentNodeInspectorIdentity;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RendererBackendNodeKey {
    pub(crate) document_id: DocumentId,
    pub(crate) handle: DomHandle,
    pub(crate) inspector_identity: Option<DocumentNodeInspectorIdentity>,
}

#[derive(Clone, Copy, Debug)]
struct RendererBackendNodeRecord {
    key: RendererBackendNodeKey,
    resolves_while_detached: bool,
}

/// Where a registry draws new ids from.
///
/// Every Document gets its own registry. With a per-registry counter each new
/// Document restarts at `RENDERER_BACKEND_NODE_ID_START`, so an id handed out
/// for the previous Document silently names a different node after navigation
/// (Chromium's DOMNodeIds are unique for the renderer's lifetime). Production
/// registries therefore share one process-wide counter; `Local` keeps unit
/// tests deterministic.
#[derive(Debug)]
enum IdSource {
    Local(u32),
    ProcessWide,
}

static NEXT_PROCESS_WIDE_BACKEND_NODE_ID: AtomicU32 =
    AtomicU32::new(moli_page_types::RENDERER_BACKEND_NODE_ID_START);

pub(crate) struct RendererBackendNodeRegistry {
    node_ids: HashMap<RendererBackendNodeKey, u32>,
    nodes: HashMap<u32, RendererBackendNodeRecord>,
    ids: IdSource,
}

pub(crate) type SharedRendererBackendNodeRegistry = Rc<RefCell<RendererBackendNodeRegistry>>;

pub(crate) fn new_shared_renderer_backend_node_registry() -> SharedRendererBackendNodeRegistry {
    Rc::new(RefCell::new(RendererBackendNodeRegistry {
        ids: IdSource::ProcessWide,
        ..RendererBackendNodeRegistry::default()
    }))
}

impl Default for RendererBackendNodeRegistry {
    fn default() -> Self {
        Self {
            node_ids: HashMap::new(),
            nodes: HashMap::new(),
            ids: IdSource::Local(moli_page_types::RENDERER_BACKEND_NODE_ID_START),
        }
    }
}

impl RendererBackendNodeRegistry {
    pub(crate) fn id_for_node(&mut self, document_id: DocumentId, handle: DomHandle) -> u32 {
        let key = RendererBackendNodeKey {
            document_id,
            handle,
            inspector_identity: None,
        };
        self.id_for_key(key)
    }

    pub(crate) fn id_for_inspector_node(
        &mut self,
        document_id: DocumentId,
        host: DomHandle,
        inspector_identity: DocumentNodeInspectorIdentity,
    ) -> u32 {
        self.id_for_key(RendererBackendNodeKey {
            document_id,
            handle: host,
            inspector_identity: Some(inspector_identity),
        })
    }

    fn id_for_key(&mut self, key: RendererBackendNodeKey) -> u32 {
        if let Some(backend_node_id) = self.node_ids.get(&key).copied() {
            return backend_node_id;
        }

        let backend_node_id = match &mut self.ids {
            IdSource::Local(next) => {
                let id = *next;
                *next = next
                    .checked_add(1)
                    .expect("renderer backend node id namespace exhausted");
                id
            }
            IdSource::ProcessWide => {
                let id = NEXT_PROCESS_WIDE_BACKEND_NODE_ID.fetch_add(1, Ordering::Relaxed);
                assert!(
                    id != u32::MAX,
                    "renderer backend node id namespace exhausted"
                );
                id
            }
        };
        self.node_ids.insert(key, backend_node_id);
        self.nodes.insert(
            backend_node_id,
            RendererBackendNodeRecord {
                key,
                resolves_while_detached: false,
            },
        );
        backend_node_id
    }

    pub(crate) fn key_for_id(&self, backend_node_id: u32) -> Option<RendererBackendNodeKey> {
        Some(self.nodes.get(&backend_node_id)?.key)
    }

    /// Keeps an event-exposed node id resolvable after its Document detaches
    /// the node without destroying the underlying DOM object.
    ///
    /// Blink freezes the input node's DOMNodeId synchronously when
    /// `Page.fileChooserOpened` is emitted. If the same script then calls
    /// `document.open()`, the event id still resolves to that detached input.
    /// This bit is intentionally per-record: ordinary ids keep the stricter
    /// Document-generation validation used elsewhere in Lightmount.
    pub(crate) fn retain_detached_resolution(&mut self, backend_node_id: u32) -> bool {
        let Some(record) = self.nodes.get_mut(&backend_node_id) else {
            return false;
        };
        record.resolves_while_detached = true;
        true
    }

    pub(crate) fn resolves_while_detached(&self, backend_node_id: u32) -> bool {
        self.nodes
            .get(&backend_node_id)
            .is_some_and(|record| record.resolves_while_detached)
    }

    pub(crate) fn remove_stale_id(&mut self, backend_node_id: u32, key: RendererBackendNodeKey) {
        self.nodes.remove(&backend_node_id);
        if self.node_ids.get(&key).copied() == Some(backend_node_id) {
            self.node_ids.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document_id(value: u64) -> DocumentId {
        DocumentId(value)
    }

    fn dom_handle(value: usize) -> DomHandle {
        DomHandle::new(value)
    }

    #[test]
    fn production_registries_never_reuse_ids_across_documents() {
        // One registry per Document; if each restarted at the same base,
        // the previous Document's ids would name nodes of the next one.
        let first = new_shared_renderer_backend_node_registry();
        let second = new_shared_renderer_backend_node_registry();
        let a = first
            .borrow_mut()
            .id_for_node(document_id(1), dom_handle(7));
        let b = second
            .borrow_mut()
            .id_for_node(document_id(2), dom_handle(7));
        assert_ne!(a, b);
        assert!(moli_page_types::is_renderer_backend_node_id(a));
        assert!(moli_page_types::is_renderer_backend_node_id(b));
        assert!(second.borrow().key_for_id(a).is_none());
    }

    #[test]
    fn registry_reuses_id_for_same_document_node_key() {
        let mut registry = RendererBackendNodeRegistry::default();
        let first = registry.id_for_node(document_id(1), dom_handle(7));
        let second = registry.id_for_node(document_id(1), dom_handle(7));

        assert_eq!(first, moli_page_types::RENDERER_BACKEND_NODE_ID_START);
        assert_eq!(first, second);
        assert_eq!(
            registry.key_for_id(first),
            Some(RendererBackendNodeKey {
                document_id: document_id(1),
                handle: dom_handle(7),
                inspector_identity: None,
            })
        );
    }

    #[test]
    fn registry_keeps_pseudo_element_identity_distinct_from_originating_element() {
        let mut registry = RendererBackendNodeRegistry::default();
        let element = registry.id_for_node(document_id(1), dom_handle(7));
        let marker = registry.id_for_inspector_node(
            document_id(1),
            dom_handle(7),
            DocumentNodeInspectorIdentity::MarkerPseudoElement,
        );
        let repeated_marker = registry.id_for_inspector_node(
            document_id(1),
            dom_handle(7),
            DocumentNodeInspectorIdentity::MarkerPseudoElement,
        );

        assert_ne!(element, marker);
        assert_eq!(marker, repeated_marker);
        assert_eq!(
            registry.key_for_id(marker),
            Some(RendererBackendNodeKey {
                document_id: document_id(1),
                handle: dom_handle(7),
                inspector_identity: Some(DocumentNodeInspectorIdentity::MarkerPseudoElement),
            })
        );
    }

    #[test]
    fn registry_tracks_generated_shadow_structure_and_dynamic_state_separately() {
        let mut registry = RendererBackendNodeRegistry::default();
        let document_id = document_id(1);
        let host = dom_handle(7);
        let element = registry.id_for_node(document_id, host);
        let root_identity = DocumentNodeInspectorIdentity::UserAgentShadowTreeNode {
            tree_kind: 1,
            ordinal: 0,
            state: 0,
        };
        let initial_text_identity = DocumentNodeInspectorIdentity::UserAgentShadowTreeNode {
            tree_kind: 1,
            ordinal: 2,
            state: 11,
        };
        let updated_text_identity = DocumentNodeInspectorIdentity::UserAgentShadowTreeNode {
            tree_kind: 1,
            ordinal: 2,
            state: 12,
        };

        let root = registry.id_for_inspector_node(document_id, host, root_identity);
        let repeated_root = registry.id_for_inspector_node(document_id, host, root_identity);
        let initial_text = registry.id_for_inspector_node(document_id, host, initial_text_identity);
        let updated_text = registry.id_for_inspector_node(document_id, host, updated_text_identity);

        assert_eq!(root, repeated_root);
        assert_ne!(element, root);
        assert_ne!(root, initial_text);
        assert_ne!(initial_text, updated_text);
    }

    #[test]
    fn registry_keeps_document_instances_disjoint() {
        let mut registry = RendererBackendNodeRegistry::default();
        let first = registry.id_for_node(document_id(1), dom_handle(7));
        let second = registry.id_for_node(document_id(2), dom_handle(7));

        assert_ne!(first, second);
        assert_eq!(second, first + 1);
    }

    #[test]
    fn removing_stale_id_clears_reverse_mapping_only_for_matching_id() {
        let mut registry = RendererBackendNodeRegistry::default();
        let key = RendererBackendNodeKey {
            document_id: document_id(1),
            handle: dom_handle(7),
            inspector_identity: None,
        };
        let first = registry.id_for_node(key.document_id, key.handle);

        registry.remove_stale_id(first, key);

        assert_eq!(registry.key_for_id(first), None);
        let second = registry.id_for_node(key.document_id, key.handle);
        assert_ne!(first, second);
    }

    #[test]
    fn detached_resolution_is_opt_in_per_backend_node_record() {
        let mut registry = RendererBackendNodeRegistry::default();
        let retained = registry.id_for_node(document_id(1), dom_handle(7));
        let ordinary = registry.id_for_node(document_id(1), dom_handle(8));

        assert!(registry.retain_detached_resolution(retained));
        assert!(registry.resolves_while_detached(retained));
        assert!(!registry.resolves_while_detached(ordinary));
        assert!(!registry.retain_detached_resolution(u32::MAX));
    }
}
