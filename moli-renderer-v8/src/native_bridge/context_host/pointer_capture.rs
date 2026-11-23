use super::*;

#[derive(Default)]
pub(super) struct PointerState {
    buttons_active: bool,
    active_document: Option<WindowDocumentTaskTarget>,
    pending: Option<PointerCaptureTarget>,
    #[cfg(feature = "devtools")]
    active: Option<PointerCaptureTarget>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct PointerCaptureTarget {
    pub(crate) element: DomHandle,
    document: WindowDocumentTaskTarget,
}

impl PageScriptHost {
    #[cfg(feature = "molio")]
    pub(crate) fn pointer_buttons_active(&self, pointer_id: i32) -> bool {
        self.pointers
            .get(&pointer_id)
            .is_some_and(|state| state.buttons_active)
    }

    pub(crate) fn pointer_is_known(&self, pointer_id: i32) -> bool {
        self.pointers.contains_key(&pointer_id)
    }

    pub(crate) fn set_pending_pointer_capture_target(
        &mut self,
        pointer_id: i32,
        element: DomHandle,
    ) {
        let Some(state) = self.pointers.get(&pointer_id) else {
            return;
        };
        let Some(document) = state.active_document.filter(|_| state.buttons_active) else {
            return;
        };
        let capture = PointerCaptureTarget { element, document };
        if self.pointer_capture_target_is_current(capture)
            && let Some(state) = self.pointers.get_mut(&pointer_id)
        {
            state.pending = Some(capture);
        }
    }

    pub(crate) fn release_pending_pointer_capture_target(&mut self, pointer_id: i32) {
        if let Some(state) = self.pointers.get_mut(&pointer_id) {
            state.pending = None;
        }
    }

    pub(crate) fn has_pending_pointer_capture_target(
        &self,
        pointer_id: i32,
        target: DomHandle,
    ) -> bool {
        self.pointers
            .get(&pointer_id)
            .and_then(|state| state.pending)
            .is_some_and(|capture| {
                capture.element == target && self.pointer_capture_target_is_current(capture)
            })
    }

    pub(crate) fn pointer_capture_target_is_current(&self, capture: PointerCaptureTarget) -> bool {
        let Some(document) = self.input_document_for_owner(capture.document) else {
            return false;
        };
        let dom = self.dom_host();
        dom.is_connected(capture.element)
            && dom.owner_document_handle(capture.element) == Some(document)
    }

    pub(crate) fn clear_pending_pointer_capture_targets_in_disconnected_subtree(
        &mut self,
        root: DomHandle,
    ) {
        let removed: Vec<_> = self
            .pointers
            .iter()
            .filter_map(|(&pointer_id, state)| {
                let target = state.pending?.element;
                self.dom_host()
                    .shadow_including_subtree(root)
                    .any(|node| node == target)
                    .then_some(pointer_id)
            })
            .collect();
        for pointer_id in removed {
            self.release_pending_pointer_capture_target(pointer_id);
        }
    }

    pub(crate) fn clear_pointer_capture_state(&mut self) {
        self.pointers.clear();
    }
}

#[cfg(feature = "devtools")]
impl PageScriptHost {
    pub(crate) fn set_pointer_buttons_active(&mut self, pointer_id: i32, active: bool) {
        self.pointers.entry(pointer_id).or_default().buttons_active = active;
    }

    pub(crate) fn set_pointer_active_document(
        &mut self,
        pointer_id: i32,
        document: WindowDocumentTaskTarget,
    ) {
        if let Some(state) = self.pointers.get_mut(&pointer_id) {
            state.active_document = Some(document);
        }
    }

    pub(crate) fn remove_touch_pointer_input(&mut self, pointer_id: i32) {
        self.pointers.remove(&pointer_id);
    }

    pub(crate) fn pending_pointer_capture_target(&self, pointer_id: i32) -> Option<DomHandle> {
        self.pointers
            .get(&pointer_id)?
            .pending
            .filter(|capture| self.pointer_capture_target_is_current(*capture))
            .map(|capture| capture.element)
    }

    pub(crate) fn active_pointer_capture_target(&self, pointer_id: i32) -> Option<DomHandle> {
        self.pointers
            .get(&pointer_id)?
            .active
            .filter(|capture| self.pointer_capture_target_is_current(*capture))
            .map(|capture| capture.element)
    }

    pub(crate) fn pointer_capture_target_for_click(&self, pointer_id: i32) -> Option<DomHandle> {
        // A release listener can detach the active capture target before
        // click targeting. Keep the override while its Document still owns it.
        let capture = self.pointers.get(&pointer_id)?.active?;
        let document = self.input_document_for_owner(capture.document)?;
        (self.dom_host().owner_document_handle(capture.element) == Some(document))
            .then_some(capture.element)
    }

    pub(crate) fn begin_pointer_capture_transition(
        &mut self,
        pointer_id: i32,
    ) -> (Option<DomHandle>, Option<PointerCaptureTarget>) {
        let pending = self
            .pointers
            .get(&pointer_id)
            .and_then(|state| state.pending)
            .filter(|capture| self.pointer_capture_target_is_current(*capture));
        let Some(state) = self.pointers.get_mut(&pointer_id) else {
            return (None, None);
        };
        state.pending = pending;
        if state.active == pending {
            return (None, None);
        }
        let previous = state.active.take();
        let lost_target = previous.and_then(|capture| {
            let document = self.input_document_for_owner(capture.document)?;
            Some(if self.pointer_capture_target_is_current(capture) {
                capture.element
            } else {
                document
            })
        });
        (lost_target, pending)
    }

    pub(crate) fn finish_pointer_capture_transition(
        &mut self,
        pointer_id: i32,
        capture: PointerCaptureTarget,
    ) -> Option<DomHandle> {
        if self.pointer_capture_target_is_current(capture) {
            if let Some(state) = self.pointers.get_mut(&pointer_id) {
                state.active = Some(capture);
            }
            return None;
        }
        if let Some(state) = self.pointers.get_mut(&pointer_id)
            && state.pending == Some(capture)
        {
            state.pending = None;
        }
        self.input_document_for_owner(capture.document)
    }
}
