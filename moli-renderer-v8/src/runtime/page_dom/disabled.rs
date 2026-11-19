use super::*;

impl PageVm {
    pub(in crate::runtime) fn abandon_pending_runtime_command_lifecycle(
        &mut self,
        _scope: PageVmRuntimeCommandOutputScopeId,
    ) -> bool {
        false
    }
    pub(in crate::runtime) fn finish_pending_runtime_command_output(
        &mut self,
        _error: Option<String>,
        _cancel: bool,
    ) -> bool {
        false
    }

    pub(crate) fn backend_node_id_for_document_node_reference(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Option<u32> {
        match reference {
            RendererDomNodeReference::BackendNodeId(id) => Some(id),
            _ => None,
        }
    }
}
