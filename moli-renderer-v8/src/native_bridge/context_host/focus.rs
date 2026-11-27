use super::{PageScriptHost, PendingWindowMessageEndpoint};
use crate::document_runtime::DomHandle;

impl PageScriptHost {
    pub(crate) fn document_has_focus(&self, document: DomHandle) -> bool {
        if !self.document_activity().focused || !self.document_is_fully_active(document) {
            return false;
        }
        let container = match self.window_endpoint_for_document(document) {
            Some(PendingWindowMessageEndpoint::TopWindow)
            | Some(PendingWindowMessageEndpoint::LightweightPopup(_)) => return true,
            Some(PendingWindowMessageEndpoint::ChildWindow(container)) => container,
            None => return false,
        };
        let Some(focused) = self.focused_frame_owner() else {
            return false;
        };
        focused == container
            || self
                .dom_host()
                .owner_document_handle(focused)
                .is_some_and(|owner| {
                    self.child_browsing_context_host_is_ancestor_of_document(container, owner)
                })
    }

    pub(crate) fn focused_frame_owner(&self) -> Option<DomHandle> {
        let PendingWindowMessageEndpoint::ChildWindow(handle) = self.focused_window else {
            return None;
        };
        (self.dom_host().is_connected(handle)
            && self
                .dom_host()
                .owner_document_handle(handle)
                .is_some_and(|document| self.document_is_fully_active(document)))
        .then_some(handle)
    }

    pub(crate) fn focus_window_for_element(
        &mut self,
        handle: DomHandle,
    ) -> Option<(PendingWindowMessageEndpoint, PendingWindowMessageEndpoint)> {
        let dom = self.dom_host();
        let next = if dom.is_html_element_named(handle, "iframe")
            || dom.is_html_element_named(handle, "frame")
        {
            PendingWindowMessageEndpoint::ChildWindow(handle)
        } else {
            self.window_endpoint_for_document(dom.owner_document_handle(handle)?)?
        };
        let previous = self.focused_window;
        if previous == next {
            return None;
        }
        self.focused_window = next;
        self.window_focus_pending = true;
        Some((previous, next))
    }

    /// Consume the notification once, even if element callbacks leave and
    /// reenter the same Window before the outer transition resumes.
    pub(crate) fn take_pending_window_focus(
        &mut self,
        window: PendingWindowMessageEndpoint,
    ) -> bool {
        self.focused_window == window && std::mem::take(&mut self.window_focus_pending)
    }

    pub(crate) fn unfocus_frame_owner(&mut self, handle: DomHandle) {
        if self.focused_window == PendingWindowMessageEndpoint::ChildWindow(handle) {
            self.focused_window = self
                .dom_host()
                .owner_document_handle(handle)
                .and_then(|document| self.window_endpoint_for_document(document))
                .unwrap_or(PendingWindowMessageEndpoint::TopWindow);
            self.window_focus_pending = false;
        }
    }

    pub(super) fn clear_focused_window(&mut self, endpoint: PendingWindowMessageEndpoint) {
        if self.focused_window == endpoint {
            self.focused_window = PendingWindowMessageEndpoint::TopWindow;
            self.window_focus_pending = false;
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
