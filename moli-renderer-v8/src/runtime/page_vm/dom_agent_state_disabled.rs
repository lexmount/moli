//! Backend identity used by normal snapshots and waits without a DOM frontend.
use super::backend_node_registry::{
    RendererBackendNodeKey, SharedRendererBackendNodeRegistry,
    new_shared_renderer_backend_node_registry,
};
use crate::{document_runtime::DomHandle, dom::native::DomHost, frame_owner_model::DocumentId};
use moli_page_types::DocumentNodeInspectorIdentity;

#[derive(Clone)]
pub(crate) struct RendererDomAgentState {
    backend_nodes: SharedRendererBackendNodeRegistry,
}
impl Default for RendererDomAgentState {
    fn default() -> Self {
        Self::new(new_shared_renderer_backend_node_registry())
    }
}
impl RendererDomAgentState {
    pub(crate) fn new(backend_nodes: SharedRendererBackendNodeRegistry) -> Self {
        Self { backend_nodes }
    }
    pub(crate) fn discard_all_frontend_bindings(&self, _document: Option<DocumentId>) {}
    pub(crate) fn reset_for_document_replacement(&self, _document: DocumentId) {}
    pub(crate) fn sync_mutation_recording_interest(&self, dom_host: &DomHost) {
        dom_host.set_devtools_mutation_records_enabled(false);
    }
    pub(crate) fn has_node_stack_trace_capture_interest(&self) -> bool {
        false
    }
    pub(crate) fn record_node_creation_stack_trace(
        &self,
        _document: DocumentId,
        _handle: DomHandle,
        _trace: crate::RendererDomNodeCreationStackTrace,
    ) {
    }
    pub(crate) fn backend_node_id_for_node(
        &self,
        document_id: DocumentId,
        handle: DomHandle,
    ) -> u32 {
        let backend_nodes = self.backend_nodes.clone();
        backend_nodes.borrow_mut().id_for_node(document_id, handle)
    }
    pub(crate) fn backend_node_id_for_inspector_node(
        &self,
        document_id: DocumentId,
        host: DomHandle,
        inspector_identity: DocumentNodeInspectorIdentity,
    ) -> u32 {
        let backend_nodes = self.backend_nodes.clone();
        backend_nodes
            .borrow_mut()
            .id_for_inspector_node(document_id, host, inspector_identity)
    }
    pub(crate) fn backend_node_key_for_id(
        &self,
        backend_node_id: u32,
    ) -> Option<RendererBackendNodeKey> {
        let backend_nodes = self.backend_nodes.clone();
        backend_nodes.borrow().key_for_id(backend_node_id)
    }
    pub(crate) fn retain_detached_backend_node_resolution(&self, backend_node_id: u32) -> bool {
        let backend_nodes = self.backend_nodes.clone();
        backend_nodes
            .borrow_mut()
            .retain_detached_resolution(backend_node_id)
    }
    pub(crate) fn backend_node_resolves_while_detached(&self, backend_node_id: u32) -> bool {
        let backend_nodes = self.backend_nodes.clone();
        backend_nodes
            .borrow()
            .resolves_while_detached(backend_node_id)
    }
    pub(crate) fn remove_stale_backend_node_id(
        &self,
        backend_node_id: u32,
        key: RendererBackendNodeKey,
    ) {
        let backend_nodes = self.backend_nodes.clone();
        backend_nodes
            .borrow_mut()
            .remove_stale_id(backend_node_id, key);
    }
}
