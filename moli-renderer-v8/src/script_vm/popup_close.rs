use anyhow::Result;

use super::ScriptVm;
use crate::{
    context_bootstrap::inform_about_canceled_navigation_for_window,
    native_bridge::OwnerDispatchScope,
    page_task_queue::RendererPagePopupCloseOwner,
    runtime::{AuthorizedCurrentPagePopupClose, RendererDocumentToken},
};

impl ScriptVm {
    pub(crate) fn current_popup_close_owner(
        &self,
        popup_id: u64,
        root_document: RendererDocumentToken,
    ) -> Option<RendererPagePopupCloseOwner> {
        self._context_host
            .borrow()
            .lightweight_popup_is_closing(popup_id)
            .then(|| RendererPagePopupCloseOwner::new(root_document, popup_id))
    }

    pub(crate) fn apply_current_popup_close_body(
        &mut self,
        authorization: AuthorizedCurrentPagePopupClose,
    ) -> Result<()> {
        let popup_id = authorization.into_task().owner().popup_id();
        self.with_default_context_scope(|scope, host_ptr| {
            // A popup can outlive its opener's realm registration. Restore
            // only its scoped alias from the retained popup Window.
            unsafe { &mut *host_ptr }.ensure_lightweight_popup_execution_context(scope, popup_id);
            let host = unsafe { &*host_ptr };
            let dispatch_scope = OwnerDispatchScope::LightweightPopup(popup_id);
            let binding = host
                .current_window_execution_context_owner(dispatch_scope)
                .and_then(|owner| {
                    host.clone_window_execution_context_binding(scope, owner, dispatch_scope)
                });
            let window = host
                .lightweight_popup_window(scope, popup_id)
                .map(|window| v8::Global::new(scope, window));
            // Closing keeps the LocalWindow alive until this task retires it.
            // Abort in its exact owner scope before removing event callbacks:
            // cancellation listeners can reenter the host or open other popups.
            if let (Some(binding), Some(window)) = (binding, window) {
                binding.with_current_scope(scope, host_ptr, |scope, _| {
                    let window = v8::Local::new(scope, &window);
                    inform_about_canceled_navigation_for_window(scope, window);
                });
            }
            assert!(
                unsafe { &mut *host_ptr }
                    .definitely_close_lightweight_popup_browsing_context(scope, popup_id),
                "authorized popup close task must retain its closing browsing context"
            );
            Ok(())
        })
    }
}
