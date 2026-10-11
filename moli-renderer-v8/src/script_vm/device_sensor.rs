use anyhow::{Result, anyhow};

use super::ScriptVm;
use crate::{
    page_task_queue::{
        PageDeviceSensorTargetEffect, RendererPageDeviceSensorOwner,
        RendererPageDeviceSensorTaskId, RendererPageDeviceSensorTaskKind,
    },
    runtime::{AuthorizedCurrentPageDeviceSensorTask, RendererDocumentToken},
};

impl ScriptVm {
    pub(crate) fn current_pending_device_sensor_owner(
        &self,
        task_id: RendererPageDeviceSensorTaskId,
        root_document: RendererDocumentToken,
    ) -> Option<(
        RendererPageDeviceSensorOwner,
        RendererPageDeviceSensorTaskKind,
    )> {
        let (target, kind) = self
            ._context_host
            .borrow()
            .current_pending_device_sensor_task(task_id)?;
        Some((
            RendererPageDeviceSensorOwner::new(root_document, target),
            kind,
        ))
    }

    /// Execute one already-authorized device motion and orientation callback body.
    ///
    /// The selected Page-task dispatcher, not this body, owns checkpoint and
    /// runtime follow-up.
    pub(crate) fn apply_current_device_sensor_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageDeviceSensorTask,
    ) -> Result<PageDeviceSensorTargetEffect> {
        let task = authorization.into_task();
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let Some(callback) = self
            ._context_host
            .borrow_mut()
            .take_pending_device_sensor_task_for_exact_target(task_id, owner.target(), kind)
        else {
            return Err(anyhow!(
                "authorized device motion and orientation task lost its exact callback payload"
            ));
        };
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                unsafe { &mut *host_ptr }.dispatch_authorized_device_sensor_task(
                    scope,
                    owner.target(),
                    callback,
                ),
            )
        })
    }

    pub(crate) fn discard_stale_device_sensor_task(
        &mut self,
        task_id: RendererPageDeviceSensorTaskId,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .discard_pending_device_sensor_task(task_id)
    }
}
