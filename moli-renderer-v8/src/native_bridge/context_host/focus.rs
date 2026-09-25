use super::{JsContextHost, PendingWindowMessageEndpoint};
use crate::document_runtime::DomHandle;

#[derive(Default)]
pub(super) struct DocumentFocusChangeState {
    focused_area: Option<DomHandle>,
    sequential_starting_point: Option<DomHandle>,
    epoch: u64,
}

impl JsContextHost {
    pub(crate) fn clear_disconnected_document_focus(&mut self) {
        let documents = self
            .document_focus_changes
            .iter()
            .filter_map(|(document, state)| {
                state
                    .focused_area
                    .filter(|area| {
                        !self.dom_host().is_connected(*area)
                            || self.dom_host().owner_document_handle(*area) != Some(*document)
                    })
                    .map(|_| *document)
            })
            .collect::<Vec<_>>();
        for document in documents {
            // A retained iframe handle must not revive its former parent's
            // focus when script inserts the iframe again after removal.
            self.note_document_focused_area(document, None);
        }
    }

    pub(crate) fn note_inserted_autofocus_candidates(&mut self, roots: &[DomHandle]) {
        for &root in roots {
            let Some(document) = self.dom_host().owner_document_handle(root) else {
                continue;
            };
            let Some(top_document) = self.top_level_document_for_document(document) else {
                continue;
            };
            if self.autofocus_processed(top_document)
                || !self.document_allows_autofocus(document)
                || !self.queue_autofocus_candidates_in_subtree(top_document, root)
            {
                continue;
            }
            self.queue_top_document_post_parse_autofocus(top_document);
        }
    }

    pub(super) fn queue_top_document_post_parse_autofocus(&mut self, document: DomHandle) {
        // Parsing admits after DOMContentLoaded and its checkpoint. Later
        // insertion or child stylesheet completion also publishes rendering work.
        if document == self.document_handle() {
            if self.dom_content_loaded_dispatched()
                && let Some(owner) = self.current_main_document_task_owner()
            {
                let _ = self.queue_main_document_post_parse_autofocus(owner);
            }
        } else if let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document) {
            self.queue_lightweight_popup_post_parse_autofocus(popup_id);
        }
    }

    pub(crate) fn autofocus_document_has_blocking_stylesheets(&self, document: DomHandle) -> bool {
        if let Some(child) = self.child_browsing_context_host_for_document_handle(document) {
            return self
                .current_child_document_task_owner(child)
                .is_some_and(|owner| {
                    self.frame_document_blocking_stylesheets
                        .has_pending(owner.document_owner())
                });
        }
        self.has_blocking_stylesheets_for_document(document)
    }

    pub(crate) fn autofocus_ancestor_has_target(&self, mut document: DomHandle) -> bool {
        loop {
            if self.dom_host().document_target_element(document).is_some() {
                return true;
            }
            let Some(PendingWindowMessageEndpoint::ChildWindow(container)) =
                self.window_endpoint_for_document(document)
            else {
                return false;
            };
            let Some(parent) = self.dom_host().owner_document_handle(container) else {
                return false;
            };
            document = parent;
        }
    }

    pub(crate) fn document_focused_area(&self, document: DomHandle) -> Option<DomHandle> {
        self.document_focus_changes
            .get(&document)
            .and_then(|state| state.focused_area)
            .filter(|handle| {
                self.dom_host().is_connected(*handle)
                    && self.dom_host().owner_document_handle(*handle) == Some(document)
            })
    }

    pub(crate) fn top_level_document_for_document(
        &self,
        mut document: DomHandle,
    ) -> Option<DomHandle> {
        loop {
            match self.window_endpoint_for_document(document)? {
                PendingWindowMessageEndpoint::TopWindow
                | PendingWindowMessageEndpoint::LightweightPopup(_) => return Some(document),
                PendingWindowMessageEndpoint::ChildWindow(container) => {
                    if !self.dom_host().is_connected(container) {
                        return None;
                    }
                    document = self.dom_host().owner_document_handle(container)?;
                }
            }
        }
    }

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
