use anyhow::Result;

use super::ScriptVm;
use crate::{
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
            assert!(
                unsafe { &mut *host_ptr }
                    .definitely_close_lightweight_popup_browsing_context(scope, popup_id),
                "authorized popup close task must retain its closing browsing context"
            );
            Ok(())
        })
    }
}
