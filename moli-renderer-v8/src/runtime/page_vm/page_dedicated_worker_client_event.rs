use crate::{
    page_task_queue::{
        PageDedicatedWorkerClientEventTargetEffect, PageDedicatedWorkerClientEventTurnAction,
        PageDedicatedWorkerClientEventTurnOutcome, RendererPageDedicatedWorkerClientEventOwner,
        RendererPageDedicatedWorkerClientEventTask,
    },
    script_vm::DedicatedWorkerClientEventBodyEffect,
};

use super::PageVm;
use super::{IntoPageTaskCompletion, PageTaskCompletion};

impl From<DedicatedWorkerClientEventBodyEffect> for PageDedicatedWorkerClientEventTargetEffect {
    fn from(effect: DedicatedWorkerClientEventBodyEffect) -> Self {
        match effect {
            DedicatedWorkerClientEventBodyEffect::StateTransitionApplied => {
                Self::StateAppliedToCurrentOwner
            }
            DedicatedWorkerClientEventBodyEffect::CallbackDispatched => {
                Self::CallbackDispatchedToCurrentOwner
            }
            DedicatedWorkerClientEventBodyEffect::CurrentTargetHadNoCallback => {
                Self::CurrentOwnerHadNoCallback
            }
            DedicatedWorkerClientEventBodyEffect::CurrentTargetDisappeared => {
                Self::CurrentOwnerLostDuringExecution
            }
        }
    }
}

impl IntoPageTaskCompletion for PageDedicatedWorkerClientEventTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageDedicatedWorkerClientEventTargetEffect::CallbackDispatchedToCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageDedicatedWorkerClientEventTargetEffect::StateAppliedToCurrentOwner
            | PageDedicatedWorkerClientEventTargetEffect::CurrentOwnerHadNoCallback
            | PageDedicatedWorkerClientEventTargetEffect::CurrentOwnerLostDuringExecution => {
                PageTaskCompletion::CheckpointOnly
            }
            PageDedicatedWorkerClientEventTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

/// Proof that the Page arbiter matched a selected event against the exact
/// Worker wrapper or its surviving owner Window.
pub(crate) struct AuthorizedCurrentPageDedicatedWorkerClientEvent(
    RendererPageDedicatedWorkerClientEventTask,
);

impl AuthorizedCurrentPageDedicatedWorkerClientEvent {
    fn new(task: RendererPageDedicatedWorkerClientEventTask) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> RendererPageDedicatedWorkerClientEventTask {
        self.0
    }
}

impl PageVm {
    fn current_page_dedicated_worker_client_event_owner(
        &self,
        worker_id: crate::types::DedicatedWorkerId,
    ) -> Option<RendererPageDedicatedWorkerClientEventOwner> {
        let execution_context = self
            .vm()
            .current_dedicated_worker_client_event_identity(worker_id)?;
        Some(RendererPageDedicatedWorkerClientEventOwner::new(
            self.document_lifecycle.identity().document,
            execution_context,
            worker_id,
        ))
    }

    pub(in crate::runtime) fn apply_selected_page_dedicated_worker_client_event_turn(
        &mut self,
        task: RendererPageDedicatedWorkerClientEventTask,
    ) -> anyhow::Result<PageDedicatedWorkerClientEventTurnOutcome> {
        let owner = task.owner();
        let event_kind = task.event_kind();
        let current_owner =
            self.current_page_dedicated_worker_client_event_owner(owner.worker_id());
        let can_report_to_window = current_owner.is_none()
            && task.propagates_to_window()
            && self.document_lifecycle.identity().document == owner.root_document()
            && self
                .vm()
                .dedicated_worker_owner_window_is_current(owner.execution_context());
        let target_effect = if current_owner == Some(owner) || can_report_to_window {
            self.vm_mut()
                .apply_current_dedicated_worker_client_event_body(
                    AuthorizedCurrentPageDedicatedWorkerClientEvent::new(task),
                )?
                .into()
        } else {
            tracing::debug!(
                ?owner,
                ?current_owner,
                "ignored stale exact-owner DedicatedWorker client event"
            );
            PageDedicatedWorkerClientEventTargetEffect::DiscardedStaleOwner { current_owner }
        };
        let action = PageDedicatedWorkerClientEventTurnAction {
            owner,
            event_kind,
            target_effect,
        };
        Ok(PageDedicatedWorkerClientEventTurnOutcome::new(action))
    }
}
