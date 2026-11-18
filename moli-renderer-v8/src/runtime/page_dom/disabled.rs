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

    pub(super) fn project_renderer_dom_agent_ids_to_snapshot(
        &mut self,
        _session: Option<&str>,
        root: &mut DocumentNodeSnapshot,
        _whitespace: bool,
        _bind: bool,
    ) {
        let mut stack = vec![root];
        while let Some(snapshot) = stack.pop() {
            snapshot.backend_node_id = if let Some(identity) = snapshot.inspector_identity {
                self.renderer_backend_node_id_for_inspector_node(snapshot.node_id, identity)
            } else {
                self.renderer_backend_node_id_for_live_handle(snapshot.node_id)
            };
            snapshot.frame_id = self
                .vm()
                .child_browsing_context_frame_id_by_owner_node_id(snapshot.node_id);
            for node in snapshot.pseudo_elements.iter_mut().rev() {
                stack.push(node);
            }
            for node in snapshot.shadow_roots.iter_mut().rev() {
                stack.push(node);
            }
            if let Some(node) = snapshot.associated.as_deref_mut() {
                stack.push(node.node_mut());
            }
            for node in snapshot.children.iter_mut().rev() {
                stack.push(node);
            }
        }
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
