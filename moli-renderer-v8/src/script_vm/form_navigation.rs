use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::{
    page_task_queue::{
        RendererPageFormNavigationOwner, RendererPageFormNavigationTaskId,
        RendererPageFormNavigationTaskKind,
    },
    runtime::AuthorizedCurrentPageFormNavigation,
};

impl ScriptVm {
    pub(crate) fn current_pending_form_navigation_owner(
        &self,
        task_id: RendererPageFormNavigationTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(
        RendererPageFormNavigationOwner,
        RendererPageFormNavigationTaskKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_form_navigation_task(task_id)?;
        Some((
            RendererPageFormNavigationOwner::new(root_document, target),
            kind,
        ))
    }

    /// Dispatch the navigation only after its DOM-manipulation task is selected.
    pub(crate) fn apply_current_form_navigation_body(
        &mut self,
        authorization: AuthorizedCurrentPageFormNavigation,
    ) -> Result<bool> {
        let task = authorization.into_task();
        let owner = task.owner();
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_authorized_form_navigation(
                    scope,
                    host_ptr,
                    task.task_id(),
                    owner.target(),
                    task.kind(),
                )
                .ok_or_else(|| {
                    anyhow!("authorized planned form navigation task lost its exact payload")
                })
        })
    }

    pub(crate) fn discard_stale_form_navigation_task(
        &mut self,
        task_id: RendererPageFormNavigationTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_form_navigation_task(task_id)
    }
}
