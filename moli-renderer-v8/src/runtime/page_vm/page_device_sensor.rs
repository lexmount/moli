use crate::page_task_queue::{
    PageDeviceSensorTargetEffect, PageDeviceSensorTurnAction, PageDeviceSensorTurnOutcome,
    RendererPageDeviceSensorOwner, RendererPageDeviceSensorTask, RendererPageDeviceSensorTaskId,
    RendererPageDeviceSensorTaskKind,
};

use super::{
    AuthorizedCurrentWindowDocumentTask, IntoPageTaskCompletion, PageTaskCompletion, PageVm,
};

impl IntoPageTaskCompletion for PageDeviceSensorTurnAction {
    fn into_page_task_completion(self) -> PageTaskCompletion {
        match self.target_effect {
            PageDeviceSensorTargetEffect::PromiseSettledForCurrentOwner => {
                PageTaskCompletion::CallbackCompletion
            }
            PageDeviceSensorTargetEffect::CurrentOwnerPromiseRetired => {
                PageTaskCompletion::CheckpointOnly
            }
            PageDeviceSensorTargetEffect::DiscardedStaleOwner { .. } => {
                PageTaskCompletion::NoCompletion
            }
        }
    }
}

pub(crate) type AuthorizedCurrentPageDeviceSensorTask =
    AuthorizedCurrentWindowDocumentTask<RendererPageDeviceSensorTask>;

impl PageVm {
    fn current_page_device_sensor_owner(
        &self,
        task_id: RendererPageDeviceSensorTaskId,
    ) -> Option<(
        RendererPageDeviceSensorOwner,
        RendererPageDeviceSensorTaskKind,
    )> {
        self.vm().current_pending_device_sensor_owner(
            task_id,
            self.document_lifecycle.identity().document,
        )
    }

    pub(in crate::runtime) fn apply_selected_page_device_sensor_turn(
        &mut self,
        task: RendererPageDeviceSensorTask,
    ) -> anyhow::Result<PageDeviceSensorTurnOutcome> {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = self.current_page_device_sensor_owner(task_id);
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => self
                    .vm_mut()
                    .apply_current_device_sensor_task_body(authorization)?,
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        self.vm_mut().discard_stale_device_sensor_task(task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale exact-Document device motion and orientation task"
                    );
                    PageDeviceSensorTargetEffect::DiscardedStaleOwner { current_owner }
                }
            };
        let action = PageDeviceSensorTurnAction {
            owner,
            task_id,
            kind,
            target_effect,
        };
        Ok(PageDeviceSensorTurnOutcome::new(action))
    }
}
