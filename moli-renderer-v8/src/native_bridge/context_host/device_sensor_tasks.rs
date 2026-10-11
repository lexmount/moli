use crate::{
    context_bootstrap::DevicePermissionTask,
    page_task_queue::{
        PageDeviceSensorTargetEffect, RendererPageDeviceSensorTaskId,
        RendererPageDeviceSensorTaskKind,
    },
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(super) type DeviceSensorTaskState = ExactWindowDocumentTaskLedger<
    RendererPageDeviceSensorTaskId,
    RendererPageDeviceSensorTaskKind,
    DevicePermissionTask,
>;

impl JsContextHost {
    pub(crate) fn queue_device_sensor_permission_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: DevicePermissionTask,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        let task_id = self
            .device_sensor_tasks
            .allocate_task_id(RendererPageDeviceSensorTaskId::from_raw);
        let kind = RendererPageDeviceSensorTaskKind::Permission;
        self.device_sensor_tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, task,
            ));
        if self
            .page_device_sensor_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }
        let _ = self.device_sensor_tasks.remove_exact(task_id, target, kind);
        tracing::debug!(
            ?target,
            ?task_id,
            "retired sensor permission after device task route closure"
        );
        false
    }

    pub(crate) fn current_pending_device_sensor_task(
        &self,
        task_id: RendererPageDeviceSensorTaskId,
    ) -> Option<(WindowDocumentTaskTarget, RendererPageDeviceSensorTaskKind)> {
        let pending = self.device_sensor_tasks.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    pub(crate) fn take_pending_device_sensor_task_for_exact_target(
        &mut self,
        task_id: RendererPageDeviceSensorTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageDeviceSensorTaskKind,
    ) -> Option<DevicePermissionTask> {
        self.device_sensor_tasks
            .remove_exact(task_id, target, kind)
            .map(PendingExactWindowDocumentTask::into_payload)
    }

    pub(crate) fn discard_pending_device_sensor_task(
        &mut self,
        task_id: RendererPageDeviceSensorTaskId,
    ) -> bool {
        self.device_sensor_tasks.remove(task_id).is_some()
    }

    pub(crate) fn dispatch_authorized_device_sensor_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        target: WindowDocumentTaskTarget,
        task: DevicePermissionTask,
    ) -> PageDeviceSensorTargetEffect {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return PageDeviceSensorTargetEffect::CurrentOwnerPromiseRetired;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let invoked = task.invoke(scope);
        dispatch_scope.restore(scope, previous_scope);
        if invoked {
            PageDeviceSensorTargetEffect::PromiseSettledForCurrentOwner
        } else {
            PageDeviceSensorTargetEffect::CurrentOwnerPromiseRetired
        }
    }
}
