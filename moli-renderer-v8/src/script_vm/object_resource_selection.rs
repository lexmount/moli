use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::{
    page_task_queue::{
        PageObjectResourceSelectionTargetEffect, RendererPageObjectResourceSelectionOwner,
        RendererPageObjectResourceSelectionTaskId,
    },
    runtime::AuthorizedCurrentPageObjectResourceSelection,
};

impl ScriptVm {
    pub(crate) fn current_pending_object_resource_selection_owner(
        &self,
        task_id: RendererPageObjectResourceSelectionTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(RendererPageObjectResourceSelectionOwner, ())> {
        let target = self
            ._context_host
            .borrow()
            .current_pending_object_resource_selection_target(task_id)?;
        Some((
            RendererPageObjectResourceSelectionOwner::new(root_document, target),
            (),
        ))
    }

    /// Apply an authorized object selection; the DOM source owns callback completion.
    pub(crate) fn apply_current_object_resource_selection_body(
        &mut self,
        authorization: AuthorizedCurrentPageObjectResourceSelection,
    ) -> Result<PageObjectResourceSelectionTargetEffect> {
        let task = authorization.into_task();
        let owner = task.owner();
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_authorized_object_resource_selection(
                    scope,
                    host_ptr,
                    task.task_id(),
                    owner.target(),
                )
                .ok_or_else(|| {
                    anyhow!("authorized object resource selection lost its exact payload")
                })
        })
    }

    pub(crate) fn discard_stale_object_resource_selection_task(
        &mut self,
        task_id: RendererPageObjectResourceSelectionTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_object_resource_selection_task(task_id)
    }
}
