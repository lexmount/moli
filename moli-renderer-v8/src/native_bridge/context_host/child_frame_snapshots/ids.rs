use super::super::PageScriptHost;
use crate::{document_runtime::DomHandle, dom::NodeId};

impl PageScriptHost {
    pub(in crate::native_bridge::context_host) fn allocate_child_document_loader_id(
        &self,
    ) -> String {
        self.browser_context_runtime
            .allocate_child_document_loader_id()
    }

    pub(in crate::native_bridge::context_host) fn ensure_child_navigation_loader_id(
        &mut self,
        handle: DomHandle,
        navigation: crate::frame_owner_model::FrameDocumentNavigationLoadBinding,
    ) -> String {
        if let Some((owner, observation)) = self
            .child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.navigation_loader_id.as_ref())
            && *owner == navigation
        {
            return observation.loader_id().to_owned();
        }
        let loader = self.allocate_child_document_loader_id();
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            if let Some((_, previous)) = entry.navigation_loader_id.take() {
                previous.abort();
            }
            let observation = crate::runtime::RendererChildFrameNavigation::new(
                entry.frame_id().to_owned(),
                loader.clone(),
            );
            entry.navigation_loader_id = Some((navigation, observation));
        }
        loader
    }

    pub(crate) fn child_navigation_observation(
        &self,
        handle: DomHandle,
    ) -> Option<crate::runtime::RendererChildFrameNavigation> {
        self.child_browsing_contexts
            .get(&handle)?
            .navigation_loader_id
            .as_ref()
            .map(|(_, observation)| observation.clone())
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
