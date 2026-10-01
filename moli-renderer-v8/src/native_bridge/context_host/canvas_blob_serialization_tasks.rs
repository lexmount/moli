use super::{JsContextHost, WindowExecutionContextIdentity};
use crate::{
    context_bootstrap::{
        CanvasBlobCallbackTask, CanvasBlobCallbackTaskEffect, CanvasBlobEncodeJob,
    },
    page_task_queue::{
        PageCanvasBlobSerializationTargetEffect, RendererPageCanvasBlobSerializationTaskId,
        RendererPageCanvasBlobSerializationTaskKind,
    },
};
use moli_webidl_callback::WebIdlCallbackFunction;
use std::collections::HashMap;

pub(super) struct PendingCanvasBlobCallback {
    execution_context: WindowExecutionContextIdentity,
    callback: CanvasBlobCallbackTask,
}
pub(super) struct CanvasBlobSerializationTaskState {
    pending: HashMap<RendererPageCanvasBlobSerializationTaskId, PendingCanvasBlobCallback>,
    next_id: RendererPageCanvasBlobSerializationTaskId,
}
impl Default for CanvasBlobSerializationTaskState {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
            next_id: RendererPageCanvasBlobSerializationTaskId::first(),
        }
    }
}
impl JsContextHost {
    /// Bind the element's relevant Window/realm, independently of parser
    /// epochs. document.open() preserves this Window-owned callback.
    pub(crate) fn queue_canvas_blob_serialization_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        canvas_context: v8::Local<'_, v8::Context>,
        callback: WebIdlCallbackFunction,
        encode: CanvasBlobEncodeJob,
    ) -> bool {
        let Some(execution_context) =
            self.window_execution_context_identity_for_v8_context(scope, canvas_context)
        else {
            return false;
        };
        if !self.window_execution_context_identity_is_current(execution_context) {
            return false;
        }
        let task_id = self.canvas_blob_serialization_tasks.next_id;
        self.canvas_blob_serialization_tasks.next_id = task_id
            .checked_next()
            .expect("canvas blob task id overflow");
        let (encoded_tx, encoded_rx) = tokio::sync::oneshot::channel();
        let callback =
            CanvasBlobCallbackTask::new(scope, self, callback, canvas_context, encoded_rx);
        let replaced = self.canvas_blob_serialization_tasks.pending.insert(
            task_id,
            PendingCanvasBlobCallback {
                execution_context,
                callback,
            },
        );
        assert!(
            replaced.is_none(),
            "canvas blob task ids must never be reused"
        );
        tracing::debug!(
            task_id = task_id.task_id(),
            ?execution_context,
            "registered canvas blob Window/realm task"
        );
        let sender = self.page_canvas_blob_serialization_sender();
        let encode = move || {
            if encoded_tx.send(encode.encode()).is_ok() {
                let _ = sender.send(
                    execution_context,
                    task_id,
                    RendererPageCanvasBlobSerializationTaskKind::Encoded,
                );
            }
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(encode);
        } else {
            std::thread::spawn(encode);
        }
        true
    }
    pub(crate) fn current_pending_canvas_blob_serialization_task(
        &self,
        task_id: RendererPageCanvasBlobSerializationTaskId,
    ) -> Option<WindowExecutionContextIdentity> {
        let pending = self.canvas_blob_serialization_tasks.pending.get(&task_id)?;
        self.window_execution_context_identity_is_current(pending.execution_context)
            .then_some(pending.execution_context)
    }
    pub(crate) fn take_pending_canvas_blob_serialization_task_for_exact_owner(
        &mut self,
        task_id: RendererPageCanvasBlobSerializationTaskId,
        execution_context: WindowExecutionContextIdentity,
    ) -> Option<CanvasBlobCallbackTask> {
        let pending = self.canvas_blob_serialization_tasks.pending.get(&task_id)?;
        if pending.execution_context != execution_context {
            return None;
        }
        self.canvas_blob_serialization_tasks
            .pending
            .remove(&task_id)
            .map(|pending| pending.callback)
    }
    pub(crate) fn dispatch_authorized_canvas_blob_serialization_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        execution_context: WindowExecutionContextIdentity,
        task: CanvasBlobCallbackTask,
    ) -> PageCanvasBlobSerializationTargetEffect {
        let context = task.context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch_scope = execution_context.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let effect = match task.invoke(scope, host_ptr) {
            CanvasBlobCallbackTaskEffect::CallbackInvoked => {
                PageCanvasBlobSerializationTargetEffect::CallbackInvokedForCurrentOwner
            }
            CanvasBlobCallbackTaskEffect::CallbackNotInvoked => {
                PageCanvasBlobSerializationTargetEffect::CurrentOwnerCallbackRetired
            }
        };
        dispatch_scope.restore(scope, previous_scope);
        effect
    }
}
