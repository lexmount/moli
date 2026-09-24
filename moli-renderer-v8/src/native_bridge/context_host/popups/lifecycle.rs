use super::{JsContextHost, LightweightPopupNavigationTaskToken};
use crate::context_bootstrap::{
    dispatch_beforeunload_for_runtime_owner, dispatch_pagehide_for_runtime_owner,
    dispatch_unload_for_runtime_owner, navigation_unload_event_active,
    replace_navigation_unload_event_active,
};

impl JsContextHost {
    pub(crate) fn dispatch_lightweight_popup_tree_beforeunload(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
    ) -> bool {
        let Some(owner) = self.current_lightweight_popup_document_owner(popup_id) else {
            return false;
        };
        let navigation_id = self.lightweight_popup_navigation_id(popup_id);
        let closing = self.lightweight_popup_is_closing(popup_id);
        let is_current = |host: &Self| {
            host.lightweight_popup_document_owner_is_current(owner)
                && host.lightweight_popup_navigation_id(popup_id) == navigation_id
                && host.lightweight_popup_is_closing(popup_id) == closing
        };
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return false;
        };
        let Some(context) = window.get_creation_context(scope) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        // close() owns a distinct beforeunload check, including when called
        // from pagehide. begin_lightweight_popup_close prevents close reentry.
        if !closing && navigation_unload_event_active(scope, window) {
            return false;
        }
        let descendants = self
            .lightweight_popup_document_handle(popup_id)
            .map(|document| self.child_document_descendants_unload_snapshot(document))
            .unwrap_or_default();
        // Keep the navigation guard through descendant callbacks, but scope
        // destructive-write suppression to each document's own beforeunload.
        let previous = replace_navigation_unload_event_active(scope, window, true);
        dispatch_beforeunload_for_runtime_owner(scope, window);
        self.dispatch_child_documents_beforeunload(scope, descendants, is_current);
        replace_navigation_unload_event_active(scope, window, previous);
        is_current(self)
    }

    pub(super) fn check_lightweight_popup_navigation_beforeunload(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
    ) -> bool {
        self.dispatch_lightweight_popup_tree_beforeunload(scope, task.popup_id())
            && self.lightweight_popup_navigation_attempt_is_current(task)
    }

    pub(crate) fn dispatch_lightweight_popup_document_unload(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
    ) -> bool {
        let Some(document) = self.lightweight_popup_document_record_mut(popup_id) else {
            return false;
        };
        if document.unload_events_dispatched {
            return false;
        }
        let Some(handle) = document.handle else {
            return false;
        };
        // Claim this exact document before callbacks can close the popup.
        document.unload_events_dispatched = true;
        let owner = document.owner;
        let descendants = self.child_document_descendants_unload_snapshot(handle);
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return false;
        };
        let Some(context) = window.get_creation_context(scope) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        // Descendant unload handlers must not erase or navigate the ancestor
        // document while its replacement is being committed.
        let _unload = self.enter_document_unload(handle);
        dispatch_pagehide_for_runtime_owner(scope, window);
        if self.lightweight_popup_document_owner_is_current(owner)
            && self
                .dom_host_mut()
                .set_document_visibility_hidden_for_handle(handle, true)
        {
            self.dispatch_lightweight_popup_document_event(scope, popup_id, "visibilitychange");
        }
        if self.lightweight_popup_document_owner_is_current(owner) {
            dispatch_unload_for_runtime_owner(scope, window);
            Self::dispatch_child_documents_unload_without_beforeunload(scope, self, descendants, false);
        }
        true
    }
}
