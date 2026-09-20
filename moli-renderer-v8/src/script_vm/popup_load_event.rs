use anyhow::Result;

use super::ScriptVm;
use crate::{
    native_bridge::LightweightPopupNavigationTaskToken,
    page_task_queue::RendererPagePopupLoadEventOwner, runtime::AuthorizedCurrentPagePopupLoadEvent,
};

impl ScriptVm {
    pub(crate) fn current_popup_load_event_owner(
        &self,
        expected: LightweightPopupNavigationTaskToken,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<RendererPagePopupLoadEventOwner> {
        self._context_host
            .borrow()
            .current_lightweight_popup_load_event_task(expected)
            .map(|target| RendererPagePopupLoadEventOwner::new(root_document, target))
    }

    /// Dispatch one popup `load` only after the Page arbiter has matched its
    /// exact Document navigation and all direct child-frame load blockers have
    /// settled.
    pub(crate) fn apply_current_popup_load_event_body(
        &mut self,
        authorization: AuthorizedCurrentPagePopupLoadEvent,
    ) -> Result<()> {
        let task = authorization.into_task();
        let target = task.owner().target();
        self._context_host
            .borrow_mut()
            .take_current_lightweight_popup_load_event_task(target);
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .dispatch_lightweight_popup_load_event(scope, target.popup_id());
            unsafe { &mut *host_ptr }
                .note_lightweight_popup_load_event_finished(target.document_owner());
            Ok(())
        })
    }

    pub(crate) fn discard_stale_popup_load_event_task(
        &mut self,
        stale: LightweightPopupNavigationTaskToken,
    ) {
        self._context_host
            .borrow_mut()
            .discard_stale_lightweight_popup_load_event_task(stale);
    }
}
