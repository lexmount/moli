use super::{JsContextHost, WindowExecutionContextIdentity};
use crate::{
    context_bootstrap::{
        CanvasBlobCompletion, CanvasBlobEncodeJob, CanvasBlobPromise, CanvasBlobTask,
        CanvasBlobTaskEffect,
    },
    page_task_queue::{
        PageCanvasBlobSerializationTargetEffect, RendererPageCanvasBlobSerializationTaskId,
        RendererPageCanvasBlobSerializationTaskKind,
    },
};
use moli_webidl_callback::WebIdlCallbackFunction;
use std::collections::HashMap;

pub(super) struct PendingCanvasBlobTask {
    execution_context: WindowExecutionContextIdentity,
    task: CanvasBlobTask,
}
pub(super) struct CanvasBlobSerializationTaskState {
    pending: HashMap<RendererPageCanvasBlobSerializationTaskId, PendingCanvasBlobTask>,
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
        let completion = CanvasBlobCompletion::callback(scope, self, callback);
        self.queue_canvas_blob_completion(scope, canvas_context, completion, encode)
    }

    pub(crate) fn queue_canvas_blob_promise_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        canvas_context: v8::Local<'_, v8::Context>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        encode: CanvasBlobEncodeJob,
    ) -> bool {
        let completion =
            CanvasBlobCompletion::Promise(CanvasBlobPromise::new(scope, resolver, canvas_context));
        self.queue_canvas_blob_completion(scope, canvas_context, completion, encode)
    }

    fn queue_canvas_blob_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        canvas_context: v8::Local<'_, v8::Context>,
        completion: CanvasBlobCompletion,
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
        let task = CanvasBlobTask::new(scope, completion, canvas_context, encoded_rx);
        let replaced = self.canvas_blob_serialization_tasks.pending.insert(
            task_id,
            PendingCanvasBlobTask {
                execution_context,
                task,
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
        encode.spawn(encoded_tx, move || {
            let _ = sender.send(
                execution_context,
                task_id,
                RendererPageCanvasBlobSerializationTaskKind::Encoded,
            );
        });
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

    pub(crate) fn retire_canvas_blob_execution_context_owner(
        &mut self,
        owner: super::WindowExecutionContextOwner,
    ) -> usize {
        let before = self.canvas_blob_serialization_tasks.pending.len();
        self.canvas_blob_serialization_tasks
            .pending
            .retain(|_, pending| pending.execution_context.owner() != owner);
        before - self.canvas_blob_serialization_tasks.pending.len()
    }

    pub(crate) fn retire_canvas_blob_context_token(
        &mut self,
        realm_token: super::RuntimeObservableContextToken,
    ) -> usize {
        let before = self.canvas_blob_serialization_tasks.pending.len();
        self.canvas_blob_serialization_tasks
            .pending
            .retain(|_, pending| pending.execution_context.realm_token() != realm_token);
        before - self.canvas_blob_serialization_tasks.pending.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_canvas_blob_task_count_for_test(
        &self,
        realm_token: super::RuntimeObservableContextToken,
    ) -> usize {
        self.canvas_blob_serialization_tasks
            .pending
            .values()
            .filter(|pending| pending.execution_context.realm_token() == realm_token)
            .count()
    }

    pub(crate) fn take_pending_canvas_blob_serialization_task_for_exact_owner(
        &mut self,
        task_id: RendererPageCanvasBlobSerializationTaskId,
        execution_context: WindowExecutionContextIdentity,
    ) -> Option<CanvasBlobTask> {
        let pending = self.canvas_blob_serialization_tasks.pending.get(&task_id)?;
        if pending.execution_context != execution_context {
            return None;
        }
        self.canvas_blob_serialization_tasks
            .pending
            .remove(&task_id)
            .map(|pending| pending.task)
    }
    pub(crate) fn dispatch_authorized_canvas_blob_serialization_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        execution_context: WindowExecutionContextIdentity,
        task: CanvasBlobTask,
    ) -> PageCanvasBlobSerializationTargetEffect {
        let context = task.context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch_scope = execution_context.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let effect = match task.invoke(scope, host_ptr) {
            CanvasBlobTaskEffect::Completed => {
                PageCanvasBlobSerializationTargetEffect::CompletionAppliedForCurrentOwner
            }
            CanvasBlobTaskEffect::Retired => {
                PageCanvasBlobSerializationTargetEffect::CurrentOwnerCompletionRetired
            }
        };
        dispatch_scope.restore(scope, previous_scope);
        effect
    }
}
