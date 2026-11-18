//! Node identities shared by runtime snapshots, waits and the DOM frontend.

use super::backend_node_registry::{
    RendererBackendNodeKey, SharedRendererBackendNodeRegistry,
    new_shared_renderer_backend_node_registry,
};
use crate::{document_runtime::DomHandle, frame_owner_model::DocumentId};
use moli_page_types::DocumentNodeInspectorIdentity;

#[cfg(not(feature = "devtools"))]
mod disabled;
#[cfg(feature = "devtools")]
mod frontend;

#[derive(Clone)]
pub(crate) struct RendererDomAgentState {
    backend_nodes: SharedRendererBackendNodeRegistry,
    #[cfg(feature = "devtools")]
    frontend: frontend::FrontendSessions,
}

impl Default for RendererDomAgentState {
    fn default() -> Self {
        Self::new(new_shared_renderer_backend_node_registry())
    }
}

impl RendererDomAgentState {
    pub(crate) fn new(backend_nodes: SharedRendererBackendNodeRegistry) -> Self {
        Self {
            backend_nodes,
            #[cfg(feature = "devtools")]
            frontend: frontend::FrontendSessions::default(),
        }
    }

    pub(crate) fn backend_node_id_for_node(
        &self,
        document_id: DocumentId,
        handle: DomHandle,
    ) -> u32 {
        self.backend_nodes
            .borrow_mut()
            .id_for_node(document_id, handle)
    }

    pub(crate) fn backend_node_id_for_inspector_node(
        &self,
        document_id: DocumentId,
        host: DomHandle,
        inspector_identity: DocumentNodeInspectorIdentity,
    ) -> u32 {
        self.backend_nodes
            .borrow_mut()
            .id_for_inspector_node(document_id, host, inspector_identity)
    }

    pub(crate) fn backend_node_key_for_id(
        &self,
        backend_node_id: u32,
    ) -> Option<RendererBackendNodeKey> {
        self.backend_nodes.borrow().key_for_id(backend_node_id)
    }

    pub(crate) fn retain_detached_backend_node_resolution(&self, backend_node_id: u32) -> bool {
        self.backend_nodes
            .borrow_mut()
            .retain_detached_resolution(backend_node_id)
    }

    pub(crate) fn backend_node_resolves_while_detached(&self, backend_node_id: u32) -> bool {
        self.backend_nodes
            .borrow()
            .resolves_while_detached(backend_node_id)
    }

    pub(crate) fn remove_stale_backend_node_id(
        &self,
        backend_node_id: u32,
        key: RendererBackendNodeKey,
    ) {
        self.backend_nodes
            .borrow_mut()
            .remove_stale_id(backend_node_id, key);
    }
}
