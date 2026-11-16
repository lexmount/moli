use super::super::PageScriptHost;
use super::discovery::ChildFrameOwnerElementKind;
use crate::document_runtime::DomHandle;
use crate::types::SubresourceRequestInitiatorType;
use moli_fetch::NavigationRequestDestination;
use url::Url;

impl PageScriptHost {
    /// Bitmap owners use an image resource, without allocating a Window.
    /// Unclassified responses retain the ordinary image-Document navigation path.
    pub(crate) fn embedded_image_resource(
        &self,
        handle: DomHandle,
    ) -> Option<(Url, NavigationRequestDestination)> {
        let kind = self.child_frame_owner_element_kind(handle)?;
        if !matches!(
            kind,
            ChildFrameOwnerElementKind::Object | ChildFrameOwnerElementKind::Embed
        ) || !self.child_browsing_context_host_is_active(handle)
            || self.embedded_content_is_suppressed(handle)
        {
            return None;
        }
        let raw = self
            .dom_host()
            .get_attribute(handle, kind.navigation_attribute())?;
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let url = self.parse_child_frame_owner_attribute_url(handle, raw)?;
        let mime = self.embedded_content_mime_hint(handle, raw, &url)?;
        if !moli_web_mime::is_image_mime(&mime)
            || moli_web_mime::is_xml_document_mime(&mime)
            || !crate::native_bridge::element::element_is_being_rendered(self, handle)
        {
            return None;
        }
        Some((url, kind.navigation_destination()))
    }

    pub(super) fn refresh_embedded_image_resource(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        if self.embedded_image_resource(handle).is_none() {
            if matches!(
                self.child_frame_owner_element_kind(handle),
                Some(ChildFrameOwnerElementKind::Object | ChildFrameOwnerElementKind::Embed)
            ) {
                self.cancel_embedded_image_resource(handle);
            }
            return;
        }
        let Some(admission) = self.embedded_content_csp_admissions.get_mut(&handle) else {
            return;
        };
        if admission.image_started {
            return;
        }
        admission.image_started = true;
        let initiator = if crate::resource_initiator::anchor(scope).is_some() {
            SubresourceRequestInitiatorType::Script
        } else {
            SubresourceRequestInitiatorType::Parser
        };
        crate::native_bridge::element::queue_image_load_event_if_needed_with_initiator(
            scope, self, handle, initiator,
        );
    }

    pub(super) fn cancel_embedded_image_resource(&mut self, handle: DomHandle) {
        let _ = self.cancel_pending_image_load_event(handle);
        self.set_current_image_request(handle, None);
        let _ = self.retire_image_resource_for_element(handle);
        if let Some(admission) = self.embedded_content_csp_admissions.get_mut(&handle) {
            admission.image_started = false;
        }
        if let Some(element) = self
            .dom_host_mut()
            .node_mut(handle)
            .and_then(|node| node.data_mut().as_element_mut())
        {
            let _ = element.set_image_load_dispatched(false);
        }
    }

    pub(crate) fn embedded_image_network_enabled(&self, handle: DomHandle) -> bool {
        self.owner_dispatch_scope_for_node(handle)
            .and_then(|owner| self.document_resource_loader_for_dispatch_scope(owner))
            .is_some_and(|loader| {
                loader
                    .request_client()
                    .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Image)
            })
    }
}
