use anyhow::{Result, anyhow, ensure};

use super::ScriptVm;
use crate::{
    document_runtime::DomHandle,
    page_task_queue::{
        PageScriptPreparationErrorTargetEffect, RendererPageScriptPreparationErrorOwner,
        RendererPageScriptPreparationErrorTaskId,
    },
    runtime::AuthorizedCurrentPageScriptPreparationError,
};

impl ScriptVm {
    pub(crate) fn queue_script_preparation_error(&mut self, element: DomHandle) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            ensure!(
                unsafe { &mut *host_ptr }.queue_script_preparation_error(scope, element),
                "parser script preparation error requires a live Document task route"
            );
            Ok(())
        })
    }

    pub(crate) fn current_pending_script_preparation_error_owner(
        &self,
        task_id: RendererPageScriptPreparationErrorTaskId,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<(RendererPageScriptPreparationErrorOwner, ())> {
        let target = self
            ._context_host
            .borrow()
            .current_pending_script_preparation_error_target(task_id)?;
        Some((
            RendererPageScriptPreparationErrorOwner::new(root_document, target),
            (),
        ))
    }

    /// Execute only the authorized event body. The DOM-task coordinator owns
    /// the checkpoint, including when an event listener throws or reenters.
    pub(crate) fn apply_current_script_preparation_error_body(
        &mut self,
        authorization: AuthorizedCurrentPageScriptPreparationError,
    ) -> Result<PageScriptPreparationErrorTargetEffect> {
        let task = authorization.into_task();
        let owner = task.owner();
        let Some(element) = self
            ._context_host
            .borrow_mut()
            .take_pending_script_preparation_error(task.task_id(), owner.target())
        else {
            return Err(anyhow!(
                "authorized script preparation error lost its exact element"
            ));
        };
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                unsafe { &mut *host_ptr }.dispatch_authorized_script_preparation_error(
                    scope,
                    host_ptr,
                    owner.target(),
                    element,
                ),
            )
        })
    }

    pub(crate) fn discard_stale_script_preparation_error_task(
        &mut self,
        task_id: RendererPageScriptPreparationErrorTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_script_preparation_error_task(task_id)
    }
}
