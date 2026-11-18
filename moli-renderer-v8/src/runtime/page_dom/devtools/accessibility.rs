use super::super::*;
use crate::runtime::page_surface::RendererAccessibilityPayloadsForObjectId;

impl PageVm {
    fn build_accessibility_payloads(
        &self,
        handle: DomHandle,
        request: AccessibilityRequest,
    ) -> Option<Vec<Value>> {
        let mut backend_node_id_for_node =
            |node_id| self.renderer_backend_node_id_for_live_handle(node_id);
        self.vm()
            .accessibility_payloads(handle, request, &mut backend_node_id_for_node)
    }

    pub(crate) fn accessibility_tree_payloads_for_live_handle(
        &mut self,
        root_handle: DomHandle,
        max_depth: Option<i32>,
    ) -> Option<Vec<Value>> {
        self.build_accessibility_payloads(root_handle, AccessibilityRequest::Tree { max_depth })
    }

    pub(crate) fn accessibility_tree_payloads_for_document(
        &mut self,
        max_depth: Option<i32>,
    ) -> Option<Vec<Value>> {
        let root = self.vm().document_runtime.dom_host().document_node_id();
        self.accessibility_tree_payloads_for_live_handle(root, max_depth)
    }

    pub(crate) fn accessibility_node_payload_for_live_handle(
        &mut self,
        handle: DomHandle,
    ) -> Option<Value> {
        self.build_accessibility_payloads(handle, AccessibilityRequest::Node)?
            .pop()
    }

    pub(crate) fn accessibility_node_payload_for_document(&mut self) -> Option<Value> {
        let root = self.vm().document_runtime.dom_host().document_node_id();
        self.accessibility_node_payload_for_live_handle(root)
    }

    pub(crate) fn accessibility_tree_payloads_for_node(
        &mut self,
        reference: RendererDomNodeReference,
        max_depth: Option<i32>,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_node(reference, AccessibilityRequest::Tree { max_depth })
    }

    pub(crate) fn accessibility_node_payload_for_node(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_node(reference, AccessibilityRequest::Node)
    }

    pub(crate) fn accessibility_node_and_ancestor_payloads_for_node(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_node(reference, AccessibilityRequest::Ancestors)
    }

    pub(crate) fn accessibility_child_node_payloads_for_node(
        &mut self,
        reference: RendererDomNodeReference,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_node(reference, AccessibilityRequest::Children)
    }

    pub(crate) fn accessibility_partial_tree_payloads_for_node(
        &mut self,
        reference: RendererDomNodeReference,
        fetch_relatives: bool,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_node(
            reference,
            AccessibilityRequest::Partial { fetch_relatives },
        )
    }

    fn accessibility_payloads_for_node(
        &mut self,
        reference: RendererDomNodeReference,
        request: AccessibilityRequest,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        let handle = self.live_handle_for_document_node_reference(reference)?;
        self.accessibility_payloads_for_live_handle(handle, request)
    }

    pub(crate) fn accessibility_tree_payloads_for_child_frame(
        &mut self,
        frame_id: &str,
        max_depth: Option<i32>,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_child_frame(
            frame_id,
            AccessibilityRequest::Tree { max_depth },
        )
    }

    pub(crate) fn accessibility_node_payload_for_child_frame(
        &mut self,
        frame_id: &str,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        self.accessibility_payloads_for_child_frame(frame_id, AccessibilityRequest::Node)
    }

    fn accessibility_payloads_for_child_frame(
        &self,
        frame_id: &str,
        request: AccessibilityRequest,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        let document = self
            .vm()
            .child_browsing_context_document_handle_by_frame_id(frame_id)?;
        Some(RendererAccessibilityPayloadsForObjectId {
            frame_id: Some(frame_id.to_owned()),
            payloads: self.build_accessibility_payloads(document, request),
        })
    }

    fn accessibility_payloads_for_live_handle(
        &mut self,
        handle: DomHandle,
        request: AccessibilityRequest,
    ) -> Option<RendererAccessibilityPayloadsForObjectId> {
        let frame_id = self.vm().child_frame_id_for_live_node_handle(handle);
        let payloads = self.build_accessibility_payloads(handle, request)?;
        Some(RendererAccessibilityPayloadsForObjectId {
            frame_id,
            payloads: Some(payloads),
        })
    }

    pub(crate) fn accessibility_tree_payloads_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        self.accessibility_payloads_for_object_id(
            inspector_session_id,
            object_id,
            AccessibilityRequest::Tree { max_depth: None },
        )
    }

    pub(crate) fn accessibility_node_and_ancestor_payloads_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        self.accessibility_payloads_for_object_id(
            inspector_session_id,
            object_id,
            AccessibilityRequest::Ancestors,
        )
    }

    pub(crate) fn accessibility_partial_tree_payloads_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        fetch_relatives: bool,
    ) -> Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        self.accessibility_payloads_for_object_id(
            inspector_session_id,
            object_id,
            AccessibilityRequest::Partial { fetch_relatives },
        )
    }

    fn accessibility_payloads_for_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
        request: AccessibilityRequest,
    ) -> Result<Option<RendererAccessibilityPayloadsForObjectId>> {
        let Some(handle) = self
            .vm_mut()
            .live_node_handle_for_runtime_object_id(inspector_session_id, object_id)?
        else {
            return Ok(None);
        };
        Ok(self.accessibility_payloads_for_live_handle(handle, request))
    }
}
