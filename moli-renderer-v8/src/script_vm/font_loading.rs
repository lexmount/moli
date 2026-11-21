use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::{
    page_task_queue::{
        PageFontLoadingTargetEffect, RendererPageFontLoadingOwner, RendererPageFontLoadingTaskId,
        RendererPageFontLoadingTaskKind,
    },
    runtime::{AuthorizedCurrentPageFontLoadingTask, RendererDocumentToken},
    v8_execution_watchdog::V8ExecutionWatchdog,
};

impl ScriptVm {
    pub(crate) fn current_pending_font_loading_owner(
        &self,
        id: RendererPageFontLoadingTaskId,
        root: RendererDocumentToken,
    ) -> Option<(
        RendererPageFontLoadingOwner,
        RendererPageFontLoadingTaskKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_font_loading_task(id)?;
        Some((RendererPageFontLoadingOwner::new(root, target), kind))
    }

    pub(crate) fn apply_current_font_loading_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageFontLoadingTask,
    ) -> Result<PageFontLoadingTargetEffect> {
        let task = authorization.into_task();
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
        id: RendererPageFontLoadingTaskId,
    ) -> bool {
        self._context_host
            .borrow()
            .discard_pending_font_loading_task(id)
    }
}
