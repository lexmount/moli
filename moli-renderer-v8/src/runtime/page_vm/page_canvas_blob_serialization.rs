use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};
use crate::page_task_queue::{
    PageCanvasBlobSerializationTargetEffect, PageCanvasBlobSerializationTurnAction,
    PageCanvasBlobSerializationTurnOutcome, RendererPageCanvasBlobSerializationOwner,
    RendererPageCanvasBlobSerializationTask, RendererPageCanvasBlobSerializationTaskKind,
};
impl IntoPageTaskCompletion for PageCanvasBlobSerializationTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageCanvasBlobSerializationTargetEffect::CallbackInvokedForCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageCanvasBlobSerializationTargetEffect::CurrentOwnerCallbackRetired => {
                PageTaskCompletion::CheckpointOnly
            }
            PageCanvasBlobSerializationTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}
/// The Page arbiter alone may authorize the captured root Page and Window realm.
pub(crate) struct AuthorizedCurrentPageCanvasBlobSerializationTask(
    RendererPageCanvasBlobSerializationTask,
);
impl AuthorizedCurrentPageCanvasBlobSerializationTask {
    pub(crate) fn into_task(self) -> RendererPageCanvasBlobSerializationTask {
        self.0
    }
}
impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_canvas_blob_serialization_turn(
        &mut self,
        task: RendererPageCanvasBlobSerializationTask,
    ) -> anyhow::Result<PageCanvasBlobSerializationTurnOutcome> {
        let owner = task.owner();
        debug_assert_eq!(
            task.kind(),
            RendererPageCanvasBlobSerializationTaskKind::Encoded
        );
        debug_assert_eq!(task.task_id(), owner.task());
        let current_owner: Option<RendererPageCanvasBlobSerializationOwner> =
            self.vm().current_pending_canvas_blob_serialization_owner(
                owner.task(),
                self.document_lifecycle.identity().document,
            );
        let target_effect = if current_owner == Some(owner) {
            self.vm_mut()
                .apply_current_canvas_blob_serialization_task_body(
                    AuthorizedCurrentPageCanvasBlobSerializationTask(task),
                )?
        } else {
            if owner.root_document() == self.document_lifecycle.identity().document {
                self.vm_mut().discard_stale_canvas_blob_serialization_task(
                    owner.task(),
                    owner.execution_context(),
                );
            }
            tracing::debug!(
                ?owner,
                ?current_owner,
                "discarded stale canvas blob Window/realm task"
            );
            PageCanvasBlobSerializationTargetEffect::DiscardedStaleOwner { current_owner }
        };
        Ok(PageCanvasBlobSerializationTurnOutcome::new(
            PageCanvasBlobSerializationTurnAction {
                owner,
                target_effect,
            },
        ))
    }
}
