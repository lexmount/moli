use super::JsContextHost;
use crate::{
    document_runtime::DomHandle, dom::native::Node, navigation_document::NavigationImageDocument,
    types::SubresourceRequestInitiatorType,
};

impl JsContextHost {
    /// Bind the exact browser-created host to its navigation response. This is
    /// not an image subresource fetch: no second request or CSP exception for
    /// author-created images is introduced. Later DOM mutations use the normal
    /// image update algorithm and its current Document policy.
    pub(crate) fn initialize_navigation_image_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        document: DomHandle,
        source: &NavigationImageDocument,
    ) {
        let image = self
            .dom_host()
            .document_body_handle_for_document(document)
            .and_then(|body| self.dom_host().first_child(body))
            .filter(|image| {
                self.dom_host()
                    .node(*image)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("img"))
            })
            .expect("image document parser must create its native image host");
        let url = self.document_url_for_handle(document);
        self.dom_host_mut()
            .set_attribute(image, "src", url.as_str());
        let request = crate::native_bridge::element::image_selected_request_key(self, image)
            .expect("image document URL must select its native image source");
        let target = self
            .window_document_task_target_for_node(scope, image)
            .expect("image document must retain its committed Window owner");
        let pending = self
            .register_pending_image_load_event(
                image,
                target,
                SubresourceRequestInitiatorType::Other,
            )
            .expect("new image document must have no prior image load");
        assert!(self.prepare_pending_image_load_request_if_matches(
            image,
            pending.id(),
            request.clone()
        ));
        self.set_current_image_request(image, Some(request));
        let manager = self
            .owner_dispatch_scope_for_node(image)
            .and_then(|owner| self.document_resource_loader_for_dispatch_scope(owner))
            .map_or_else(
                moli_parkable_image::ParkableImageManager::default,
                |loader| {
                    let runner = loader.task_runner();
                    loader.request_client().parkable_image_manager(&runner)
                },
            );
        let descriptor = crate::network_host::image_response_descriptor_from_bytes(
            source.headers(),
            source.bytes(),
        );
        let completion = self.complete_pending_image_load_local_response_if_matches(
            image,
            pending.id(),
            descriptor,
            manager.from_frozen_bytes(source.bytes().to_vec()),
        );
        crate::native_bridge::element::queue_image_load_network_terminal_followup(
            self,
            image,
            pending.id(),
            completion.followup(),
        );
    }
}
