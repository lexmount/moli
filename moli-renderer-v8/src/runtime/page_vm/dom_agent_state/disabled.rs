use super::RendererDomAgentState;
use crate::{document_runtime::DomHandle, dom::native::DomHost, frame_owner_model::DocumentId};

impl RendererDomAgentState {
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
}
