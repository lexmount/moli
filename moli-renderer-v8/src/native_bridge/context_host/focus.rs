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
        // Main retains one active element for this native document tree.
        // Walk its embedding chain instead of reading author-visible getters.
        let Some(mut focused) = self.active_element_handle() else {
            return false;
        };
        loop {
            if focused == container {
                return true;
            }
            let Some(owner) = self.dom_host().owner_document_handle(focused) else {
                return false;
            };
            let Some(parent) = self.child_browsing_context_host_for_document_handle(owner) else {
                return false;
            };
            focused = parent;
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
