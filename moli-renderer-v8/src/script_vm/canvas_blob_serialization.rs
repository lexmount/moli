use super::ScriptVm;
use crate::{
    native_bridge::WindowExecutionContextIdentity,
    page_task_queue::{
        PageCanvasBlobSerializationTargetEffect, RendererPageCanvasBlobSerializationOwner,
        RendererPageCanvasBlobSerializationTaskId,
    },
    runtime::{AuthorizedCurrentPageCanvasBlobSerializationTask, RendererDocumentToken},
};
use anyhow::{Result, anyhow};
impl ScriptVm {
    pub(crate) fn current_pending_canvas_blob_serialization_owner(
        &self,
        task_id: RendererPageCanvasBlobSerializationTaskId,
        root_document: RendererDocumentToken,
    ) -> Option<RendererPageCanvasBlobSerializationOwner> {
        let execution_context = self
            ._context_host
            .borrow()
            .current_pending_canvas_blob_serialization_task(task_id)?;
        Some(RendererPageCanvasBlobSerializationOwner::new(
            root_document,
            execution_context,
            task_id,
        ))
    }
    pub(crate) fn apply_current_canvas_blob_serialization_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageCanvasBlobSerializationTask,
    ) -> Result<PageCanvasBlobSerializationTargetEffect> {
        let task = authorization.into_task();
        let owner = task.owner();
        let Some(callback) = self
            ._context_host
            .borrow_mut()
            .take_pending_canvas_blob_serialization_task_for_exact_owner(
                owner.task(),
                owner.execution_context(),
            )
        else {
            return Err(anyhow!(
                "authorized canvas blob task lost its exact completion payload"
            ));
        };
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                unsafe { &mut *host_ptr }.dispatch_authorized_canvas_blob_serialization_task(
                    scope,
                    host_ptr,
                    owner.execution_context(),
                    callback,
                ),
            )
        })
    }
    pub(crate) fn discard_stale_canvas_blob_serialization_task(
        &mut self,
        task_id: RendererPageCanvasBlobSerializationTaskId,
        execution_context: WindowExecutionContextIdentity,
    ) {
        let _ = self
            ._context_host
            .borrow_mut()
            .take_pending_canvas_blob_serialization_task_for_exact_owner(
                task_id,
                execution_context,
            );
    }
}
