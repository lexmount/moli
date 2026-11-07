use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::{
    page_task_queue::{
        PageFontLoadingTargetEffect, RendererPageFontLoadingOwner, RendererPageFontLoadingTask,
        RendererPageFontLoadingTaskId, RendererPageFontLoadingTaskKind,
    },
    runtime::{AuthorizedCurrentPageFontLoadingTask, RendererDocumentToken},
    v8_execution_watchdog::V8ExecutionWatchdog,
};

impl ScriptVm {
    pub(crate) fn current_pending_font_loading_owner(
        &self,
        task_id: RendererPageFontLoadingTaskId,
        root_document: RendererDocumentToken,
    ) -> Option<(
        RendererPageFontLoadingOwner,
        RendererPageFontLoadingTaskKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_font_loading_task(task_id)?;
        Some((
            RendererPageFontLoadingOwner::new(root_document, target),
            kind,
        ))
    }

    /// Execute one already-authorized font-loading callback body.
    ///
    /// The selected Page-task dispatcher, not this body, owns checkpoint and
    /// runtime follow-up.
    pub(crate) fn apply_current_font_loading_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageFontLoadingTask,
    ) -> Result<PageFontLoadingTargetEffect> {
        let task = authorization.into_task();
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        if kind == RendererPageFontLoadingTaskKind::CheckReadiness {
            return self.apply_font_readiness_task_body(task);
        }
        let Some(callback) = self
            ._context_host
            .borrow_mut()
            .take_pending_font_loading_task_for_exact_target(task_id, owner.target(), kind)
        else {
            return Err(anyhow!(
                "authorized font-loading task lost its exact callback payload"
            ));
        };
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                unsafe { &mut *host_ptr }.dispatch_authorized_font_loading_task(
                    scope,
                    host_ptr,
                    owner.target(),
                    callback,
                ),
            )
        })
    }

    fn apply_font_readiness_task_body(
        &mut self,
        task: RendererPageFontLoadingTask,
    ) -> Result<PageFontLoadingTargetEffect> {
        // Release the queued flag before reconciliation can admit new resources.
        let document = self
            ._context_host
            .borrow()
            .take_pending_font_loading_task(task.task_id(), task.owner().target(), task.kind())
            .ok_or_else(|| anyhow!("authorized font readiness task lost its exact payload"))?;
        let result = self.with_default_context_scope(|scope, host_ptr| {
            // Resolving with the public FontFaceSet can invoke an author-defined
            // `then` getter. Keep Host/layout borrows out of that operation and
            // apply the same JS budget as other lifecycle tasks.
            V8ExecutionWatchdog::with_lifecycle_budget(scope, "font readiness", |scope| {
                crate::context_bootstrap::check_realtime_document_font_readiness(
                    scope, host_ptr, document,
                )
            })?;
            Ok(PageFontLoadingTargetEffect::ReadinessCheckedForCurrentOwner)
        });
        let host = self._context_host.borrow();
        host.finish_font_readiness_check();
        if result.is_ok()
            && host.font_readiness_environment_ready(document)
            && !host.document_web_fonts_have_pending_requests(document)
        {
            // Promise resolution can synchronously start another loading
            // cycle. Only that unfinished cycle needs a subsequent task.
            host.queue_realtime_font_readiness(document);
        }
        result
    }

    pub(crate) fn discard_stale_font_loading_task(
        &mut self,
        task_id: RendererPageFontLoadingTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_font_loading_task(task_id)
    }
}

// Standalone domain fixtures apply bodies and checkpoints separately. They do
// not provide Page arbitration or authorize foreign/replaced Page namespaces.
#[cfg(test)]
impl ScriptVm {
    pub(crate) fn apply_next_font_loading_body_for_test(&mut self) -> Result<bool> {
        use crate::page_task_queue::{RendererPageReadyDescriptor, RendererPageSchedulerTask};
        let residence = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("font body fixture must retain its production Page source");
        let root_document = residence.root_document();
        let Some(task) = residence
            .task_sources()
            .take_scheduler_task_for_executor_test(|descriptor| {
                matches!(descriptor, RendererPageReadyDescriptor::FontLoading { .. })
            })
        else {
            return Ok(false);
        };
        let RendererPageSchedulerTask::FontLoading(task) = task else {
            unreachable!("font descriptor must dequeue a font task");
        };
        let owner = task.owner();
        anyhow::ensure!(
            owner.root_document() == root_document
                && self.current_pending_font_loading_owner(task.task_id(), root_document)
                    == Some((owner, task.kind())),
            "body-only font fixture cannot authorize stale or foreign-Page tasks"
        );
        self.apply_current_font_loading_task_body(
            AuthorizedCurrentPageFontLoadingTask::new_for_executor_test(task),
        )?;
        Ok(true)
    }
}
