use crate::page_task_queue::{
    PageMediaElementEventTargetEffect, PageMediaElementEventTurnAction,
    PageMediaElementEventTurnOutcome, RendererPageMediaElementEventOwner,
    RendererPageMediaElementEventTask, RendererPageMediaElementEventTaskId,
    RendererPageMediaElementEventTaskKind,
};

use super::{IntoPageTaskCompletion, PageTaskCompletion, PageVm};

impl IntoPageTaskCompletion for PageMediaElementEventTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageMediaElementEventTargetEffect::DispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageMediaElementEventTargetEffect::CurrentOwnerHadNoEventTarget => {
                PageTaskCompletion::CheckpointOnly
            }
            PageMediaElementEventTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

/// Proof that the Page arbiter matched the PageVm namespace, LocalWindow,
/// Host-local payload id, and media-event operation before V8 application.
pub(crate) struct AuthorizedCurrentPageMediaElementEvent(RendererPageMediaElementEventTask);
impl AuthorizedCurrentPageMediaElementEvent {
    pub(crate) fn into_task(self) -> RendererPageMediaElementEventTask {
        self.0
    }
}

impl PageVm {
    fn current_page_media_element_event_owner(
        &self,
        task_id: RendererPageMediaElementEventTaskId,
    ) -> Option<(
        RendererPageMediaElementEventOwner,
        RendererPageMediaElementEventTaskKind,
    )> {
        self.vm().current_pending_media_element_event_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_media_element_event_turn(
        &mut self,
        task: RendererPageMediaElementEventTask,
    ) -> anyhow::Result<PageMediaElementEventTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_media_element_event_owner(task_id);
        let target_effect = if current == Some((owner, kind)) {
            let authorization = AuthorizedCurrentPageMediaElementEvent(task);
            if self
                .vm_mut()
                .apply_current_media_element_event_body(authorization)?
            {
                PageMediaElementEventTargetEffect::DispatchedToCurrentOwner
            } else {
                PageMediaElementEventTargetEffect::CurrentOwnerHadNoEventTarget
            }
        } else {
            if owner.root_document() == self.document_lifecycle.identity().document {
                self.vm_mut()
                    .discard_stale_media_element_event_task(task_id);
            }
            PageMediaElementEventTargetEffect::DiscardedStaleOwner {
                current_owner: current.map(|(owner, _)| owner),
            }
        };
        let action = PageMediaElementEventTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageMediaElementEventTurnOutcome::new(action))
    }
}
