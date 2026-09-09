use crate::{
    page_task_queue::{RendererPagePromiseRejectionTaskId, RendererPagePromiseRejectionTaskKind},
    script_vm::PromiseRejectionTaskPayload,
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(super) type PromiseRejectionTaskState = ExactWindowDocumentTaskLedger<
    RendererPagePromiseRejectionTaskId,
    RendererPagePromiseRejectionTaskKind,
    PromiseRejectionTaskPayload,
>;

impl JsContextHost {
    pub(crate) fn queue_promise_rejection_task(
        &mut self,
        target: WindowDocumentTaskTarget,
        kind: RendererPagePromiseRejectionTaskKind,
        payload: PromiseRejectionTaskPayload,
    ) -> bool {
        let task_id = self
            .promise_rejection_tasks
            .allocate_task_id(RendererPagePromiseRejectionTaskId::from_raw);
        self.promise_rejection_tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, payload,
            ));
        if self
            .page_promise_rejection_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }
        self.promise_rejection_tasks
            .remove_exact(task_id, target, kind);
        false
    }

    pub(crate) fn current_pending_promise_rejection_target(
        &self,
        task_id: RendererPagePromiseRejectionTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPagePromiseRejectionTaskKind,
    )> {
        let pending = self.promise_rejection_tasks.pending(task_id)?;
        let target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((target, pending.kind()))
    }

    pub(crate) fn take_pending_promise_rejection_task(
        &mut self,
        task_id: RendererPagePromiseRejectionTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPagePromiseRejectionTaskKind,
    ) -> Option<PromiseRejectionTaskPayload> {
        self.promise_rejection_tasks
            .remove_exact(task_id, target, kind)
            .map(PendingExactWindowDocumentTask::into_payload)
    }

    pub(crate) fn discard_pending_promise_rejection_task(
        &mut self,
        task_id: RendererPagePromiseRejectionTaskId,
    ) -> bool {
        self.promise_rejection_tasks.remove(task_id).is_some()
    }
}
