use super::super::PageScriptHost;
use crate::{document_runtime::DomHandle, dom::NodeId};

impl PageScriptHost {
    pub(in crate::native_bridge::context_host) fn allocate_child_document_loader_id(
        &self,
    ) -> String {
        self.browser_context_runtime
            .allocate_child_document_loader_id()
    }

    pub(crate) fn child_browsing_context_owner_node_id_by_frame_id(
        &self,
        frame_id: &str,
    ) -> Option<NodeId> {
        self.child_browsing_contexts
            .iter()
            .find_map(|(handle, entry)| (entry.frame_id() == frame_id).then_some(*handle))
    }

    pub(crate) fn child_browsing_context_handle_by_frame_id(
        &self,
        frame_id: &str,
    ) -> Option<DomHandle> {
        self.child_browsing_context_owner_node_id_by_frame_id(frame_id)
    }

    pub(crate) fn child_browsing_context_frame_id_by_owner_node_id(
        &self,
        owner_node_id: NodeId,
    ) -> Option<String> {
        self.child_browsing_contexts
            .get(&owner_node_id)
            .map(|entry| entry.frame_id().to_owned())
    }
}
