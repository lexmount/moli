use super::{JsContextHost, PendingWindowMessageEndpoint};
use crate::document_runtime::DomHandle;

impl JsContextHost {
    pub(crate) fn document_has_focus(&self, document_handle: DomHandle) -> bool {
        if !self.top_level_page_is_focused()
            || !self
                .dom_host()
                .node(document_handle)
                .is_some_and(|node| node.is_document())
        {
            return false;
        }

        let mut focused_document = self.focused_document_handle();
        loop {
            if focused_document == document_handle {
                return true;
            }
            let Some(frame_owner) =
                self.child_browsing_context_host_for_document_handle(focused_document)
            else {
                return false;
            };
            let Some(parent_document) = self
                .dom_host()
                .node(frame_owner)
                .and_then(crate::dom::native::Node::owner_document)
            else {
                return false;
            };
            focused_document = parent_document;
        }
    }

    pub(crate) fn focused_window_endpoint(&self) -> PendingWindowMessageEndpoint {
        self.window_endpoint_for_document(self.focused_document_handle())
            .unwrap_or(PendingWindowMessageEndpoint::TopWindow)
    }

    pub(crate) fn window_endpoint_for_document(
        &self,
        document_handle: DomHandle,
    ) -> Option<PendingWindowMessageEndpoint> {
        if document_handle == self.document_handle() {
            return Some(PendingWindowMessageEndpoint::TopWindow);
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
