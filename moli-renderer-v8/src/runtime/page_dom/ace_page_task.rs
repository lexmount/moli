use super::*;

impl PageVm {
    pub(in crate::runtime) fn renderer_backend_node_id_map_for_document_handle(
        &mut self,
        document_handle: DomHandle,
    ) -> HashMap<DomHandle, u32> {
        let handles = {
            let dom_host = self.vm().document_runtime.dom_host();
            child_frame_nodes::collect_shadow_including_document_handles(dom_host, document_handle)
        };
        handles
            .into_iter()
            .filter_map(|handle| {
                self.renderer_backend_node_id_for_live_handle(handle)
                    .map(|backend_node_id| (handle, backend_node_id))
            })
            .collect()
    }
}
