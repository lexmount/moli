use std::collections::HashMap;

use super::{JsContextHost, PendingImageLoadEventOwner, WindowDocumentTaskTarget};
use crate::{
    document_runtime::DomHandle,
    dom::native::DomStringValue,
    page_task_queue::{PageUserInteractionBodyEffect, RendererPageImageLoadEventKind},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InputImageRequestId(u64);

#[derive(Default)]
pub(super) struct InputImageStateStore {
    next_id: u64,
    states: HashMap<DomHandle, InputImageState>,
}

struct InputImageState {
    document: DomHandle,
    target: WindowDocumentTaskTarget,
    last_image_src: Option<DomStringValue>,
    id: InputImageRequestId,
    network_id: Option<u64>,
    load_delay: Option<PendingImageLoadEventOwner>,
}

pub(crate) struct InputImageUpdate {
    pub(crate) id: InputImageRequestId,
    pub(crate) target: WindowDocumentTaskTarget,
    pub(crate) source: Option<DomStringValue>,
    pub(crate) previous_network_id: Option<u64>,
}

impl JsContextHost {
    /// Image buttons remember the last src used in their Image state. Merely
    /// inserting a detached control, or switching back with unchanged src,
    /// does not create another fetch. Setting src in Image state does.
    pub(crate) fn begin_input_image_update(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        element: DomHandle,
        source_was_set: bool,
    ) -> Option<InputImageUpdate> {
        let node = self.dom_host().node(element)?;
        let input = node.as_element()?;
        if !input.is_html_input() || input.input_type().as_ref() != "image" {
            return None;
        }
        let source = input.attribute_ns_dom_string("", "src");
        let document = node.owner_document()?;
        let target = self.window_document_task_target_for_node(scope, element)?;
        if !source_was_set
            && self.input_images.states.get(&element).is_some_and(|state| {
                state.document == document
                    && state.target == target
                    && state.last_image_src == source
            })
        {
            return None;
        }
        self.input_images.next_id = self.input_images.next_id.checked_add(1)?;
        let id = InputImageRequestId(self.input_images.next_id);
        let previous = self.input_images.states.insert(
            element,
            InputImageState {
                document,
                target,
                last_image_src: source.clone(),
                id,
                network_id: None,
                load_delay: None,
            },
        );
        if let Some(owner) = previous.as_ref().and_then(|state| state.load_delay) {
            self.settle_pending_image_load_owner(owner, true);
        }
        Some(InputImageUpdate {
            id,
            target,
            source,
            previous_network_id: previous.and_then(|state| state.network_id),
        })
    }

    pub(crate) fn input_image_request_is_current(
        &self,
        element: DomHandle,
        id: InputImageRequestId,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        self.input_images.states.get(&element).is_some_and(|state| {
            state.id == id
                && state.target == target
                && self.dom_host().owner_document_handle(element) == Some(state.document)
                && self.window_document_owner_is_current_for_dispatch_scope(
                    target.owner(),
                    target.dispatch_scope(),
                )
        })
    }

    pub(crate) fn bind_input_image_network_request(
        &mut self,
        element: DomHandle,
        id: InputImageRequestId,
        target: WindowDocumentTaskTarget,
        internal_id: u64,
    ) -> bool {
        if !self.input_image_request_is_current(element, id, target) {
            return false;
        }
        let Some(load_delay) = self.accept_image_load_delay(element) else {
            return false;
        };
        let state = self
            .input_images
            .states
            .get_mut(&element)
            .expect("the current input image state must remain present");
        state.network_id = Some(internal_id);
        state.load_delay = Some(load_delay);
        true
    }

    pub(crate) fn finish_input_image_request(
        &mut self,
        element: DomHandle,
        id: InputImageRequestId,
        target: WindowDocumentTaskTarget,
        successful: bool,
    ) -> bool {
        let Some(state) = self
            .input_images
            .states
            .get_mut(&element)
            .filter(|state| state.id == id && state.target == target)
        else {
            return false;
        };
        state.network_id = None;
        if let Some(owner) = state.load_delay.take() {
            self.settle_pending_image_load_owner(owner, true);
        }
        // Adoption can invalidate event delivery while the original Document
        // still owns the networking delay. Always settle that delay first.
        if !self.input_image_request_is_current(element, id, target) {
            return false;
        }
        self.queue_input_image_event_task(
            target,
            element,
            id,
            if successful {
                RendererPageImageLoadEventKind::Load
            } else {
                RendererPageImageLoadEventKind::Error
            },
        )
    }

    pub(crate) fn dispatch_input_image_event_if_current(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        element: DomHandle,
        id: InputImageRequestId,
        kind: RendererPageImageLoadEventKind,
    ) -> PageUserInteractionBodyEffect {
        if !self.input_image_request_is_current(element, id, target) {
            return PageUserInteractionBodyEffect::NotApplied;
        }
        let event_type = match kind {
            RendererPageImageLoadEventKind::Load => "load",
            RendererPageImageLoadEventKind::Error => "error",
        };
        let Some(event) =
            super::super::element::construct_simple_event(scope, event_type, false, false, false)
        else {
            return PageUserInteractionBodyEffect::NotApplied;
        };
        let _ = super::super::element::dispatch_public_event(scope, host_ptr, element, event);
        PageUserInteractionBodyEffect::Applied
    }

    pub(super) fn retire_input_images_for_document(&mut self, document: DomHandle) {
        let mut delays = Vec::new();
        self.input_images.states.retain(|_, state| {
            if state.document != document {
                return true;
            }
            delays.extend(state.load_delay);
            false
        });
        for owner in delays {
            self.settle_pending_image_load_owner(owner, false);
        }
    }
}
