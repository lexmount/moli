use crate::page_task_queue::{
    PageWebTransportTargetEffect, PageWebTransportTurnAction, PageWebTransportTurnOutcome,
    RendererPageWebTransportOwner, RendererPageWebTransportTask, RendererPageWebTransportTaskId,
    RendererPageWebTransportTaskKind,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageWebTransportTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageWebTransportTargetEffect::DispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageWebTransportTargetEffect::CurrentOwnerHadNoEventTarget => {
                PageTaskCompletion::CheckpointOnly
            }
            PageWebTransportTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

/// Proof that the Page arbiter matched the PageVm namespace, exact Window realm,
/// Host-local payload id, and WebTransport operation before V8 application.
pub(crate) type AuthorizedCurrentPageWebTransport =
    AuthorizedCurrentWindowDocumentTask<RendererPageWebTransportTask>;

impl PageVm {
    fn current_page_webtransport_owner(
        &self,
        task_id: RendererPageWebTransportTaskId,
    ) -> Option<(
        RendererPageWebTransportOwner,
        RendererPageWebTransportTaskKind,
    )> {
        self.vm().current_pending_webtransport_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_webtransport_turn(
        &mut self,
        task: RendererPageWebTransportTask,
    ) -> anyhow::Result<PageWebTransportTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_webtransport_owner(task_id);
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => {
                    if self
                        .vm_mut()
                        .apply_current_webtransport_body(authorization)?
                    {
                        PageWebTransportTargetEffect::DispatchedToCurrentOwner
                    } else {
                        PageWebTransportTargetEffect::CurrentOwnerHadNoEventTarget
                    }
                }
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut().discard_stale_webtransport_task(task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale exact-Window WebTransport task"
                    );
                    PageWebTransportTargetEffect::DiscardedStaleOwner { current_owner }
                }
            };
        let action = PageWebTransportTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageWebTransportTurnOutcome::new(action))
    }
}
