use crate::page_task_queue::{
    PageRenderingUpdateTargetEffect, PageRenderingUpdateTurnAction, PageRenderingUpdateTurnOutcome,
    RendererPageRenderingUpdateOwner, RendererPageRenderingUpdateTask,
    RendererPageRenderingUpdateTaskId, RendererPageRenderingUpdateTaskKind,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageRenderingUpdateTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageRenderingUpdateTargetEffect::DispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageRenderingUpdateTargetEffect::CurrentOwnerHadNoEventTarget => {
                PageTaskCompletion::CheckpointOnly
            }
            PageRenderingUpdateTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

/// Proof that the Page arbiter matched the PageVm namespace, exact Document,
/// Host-local payload id, and rendering operation kind before V8 application.
pub(crate) type AuthorizedCurrentPageRenderingUpdate =
    AuthorizedCurrentWindowDocumentTask<RendererPageRenderingUpdateTask>;

impl PageVm {
    pub(in crate::runtime) fn current_realtime_rendering_opportunity(
        &self,
    ) -> Option<RendererPageRenderingUpdateTask> {
        self.vm()
            .current_realtime_rendering_opportunity(self.document_lifecycle.identity().document)
    }

    fn current_page_rendering_update_owner(
        &self,
        task_id: RendererPageRenderingUpdateTaskId,
    ) -> Option<(
        RendererPageRenderingUpdateOwner,
        RendererPageRenderingUpdateTaskKind,
    )> {
        self.vm().current_pending_rendering_update_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_rendering_update_turn(
        &mut self,
        task: RendererPageRenderingUpdateTask,
    ) -> anyhow::Result<PageRenderingUpdateTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_rendering_update_owner(task_id);
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => {
                    if self
                        .vm_mut()
                        .apply_current_rendering_update_body(authorization)?
                    {
                        PageRenderingUpdateTargetEffect::DispatchedToCurrentOwner
                    } else {
                        PageRenderingUpdateTargetEffect::CurrentOwnerHadNoEventTarget
                    }
                }
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut().discard_stale_rendering_update_task(task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale exact-Document rendering update"
                    );
                    PageRenderingUpdateTargetEffect::DiscardedStaleOwner { current_owner }
                }
            };

        let action = PageRenderingUpdateTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageRenderingUpdateTurnOutcome::new(action))
    }
}
