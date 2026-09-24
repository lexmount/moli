use super::{JsContextHost, PendingWindowMessageEndpoint};
use crate::document_runtime::DomHandle;

#[derive(Default)]
pub(super) struct DocumentFocusChangeState {
    focused_area: Option<DomHandle>,
    sequential_starting_point: Option<DomHandle>,
    epoch: u64,
}

impl JsContextHost {
    pub(crate) fn focus_change_epoch(&self, document: DomHandle) -> u64 {
        self.document_focus_changes
            .get(&document)
            .map_or(0, |state| state.epoch)
    }

    pub(crate) fn note_document_focused_area(
        &mut self,
        document: DomHandle,
        area: Option<DomHandle>,
    ) {
        let state = self.document_focus_changes.entry(document).or_default();
        if area.is_some() {
            state.sequential_starting_point = area;
        }
        if state.focused_area != area {
            state.focused_area = area;
            state.epoch = state.epoch.wrapping_add(1);
        }
    }

    pub(crate) fn sequential_focus_starting_point(&self, document: DomHandle) -> Option<DomHandle> {
        self.document_focus_changes
            .get(&document)
            .and_then(|state| state.sequential_starting_point)
            .filter(|handle| {
                self.dom_host()
                    .node(*handle)
                    .is_some_and(|node| node.is_connected())
                    && self.dom_host().owner_document_handle(*handle) == Some(document)
            })
    }

    pub(crate) fn set_sequential_focus_starting_point(
        &mut self,
        document: DomHandle,
        target: Option<DomHandle>,
    ) {
        self.document_focus_changes
            .entry(document)
            .or_default()
            .sequential_starting_point = target;
    }

    pub(crate) fn mark_focus_changed(
        &mut self,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
    ) {
        // Only Documents in the new focus chain change their focused area.
        // Focusing a parent does not change the focused area retained by an
        // inactive child, or cancel that child's pending navigation focus reset.
        if let Some(mut next) = next {
            while let Some(document) = self.dom_host().owner_document_handle(next) {
                self.note_document_focused_area(document, Some(next));
                let Some(container) =
                    self.child_browsing_context_host_for_document_handle(document)
                else {
                    break;
                };
                next = container;
            }
        } else if let Some(document) =
            previous.and_then(|handle| self.dom_host().owner_document_handle(handle))
        {
            self.note_document_focused_area(document, None);
        }
    }

    pub(crate) fn window_endpoint_for_document(
        &self,
        document_handle: DomHandle,
    ) -> Option<PendingWindowMessageEndpoint> {
        if document_handle == self.document_handle() {
            return Some(PendingWindowMessageEndpoint::TopWindow);
        }
        if let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document_handle) {
            return Some(PendingWindowMessageEndpoint::LightweightPopup(popup_id));
        }
        self.child_browsing_context_host_for_document_handle(document_handle)
            .map(PendingWindowMessageEndpoint::ChildWindow)
    }

    pub(crate) fn scroll_window_endpoint_to(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        endpoint: PendingWindowMessageEndpoint,
        x: f64,
        y: f64,
    ) {
        if matches!(endpoint, PendingWindowMessageEndpoint::LightweightPopup(_)) {
            return;
        }
        let dispatch_scope = endpoint.dispatch_scope();
        let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
            return;
        };
        let Some((_, context)) = self.window_execution_context(scope, owner, dispatch_scope) else {
            return;
        };
        let context = v8::Global::new(scope, context);
        let context = v8::Local::new(scope, &context);
        let target_scope = &mut v8::ContextScope::new(scope, context);
        crate::window_host::scroll_window_to(target_scope, self, x, y);
    }
}
