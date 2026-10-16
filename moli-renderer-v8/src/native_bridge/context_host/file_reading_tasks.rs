use crate::{
    context_bootstrap::{
        DirectoryReaderCallbackAdmission, DirectoryReaderCallbackTask,
        DirectoryReaderCallbackTaskEffect, FileReadingTask as FileReadTask,
    },
    page_task_queue::{
        PageFileReadingTargetEffect, RendererPageFileReadingTaskId, RendererPageFileReadingTaskKind,
    },
};
use moli_webidl_callback::WebIdlCallbackFunction;

use super::window_document_tasks::{PendingWindowTask, WindowTaskLedger};
use super::{JsContextHost, WindowExecutionContextIdentity};

pub(super) type FileReadingState = WindowTaskLedger<
    RendererPageFileReadingTaskId,
    RendererPageFileReadingTaskKind,
    FileReadingTask,
    WindowExecutionContextIdentity,
>;

enum FileReadingCallback {
    Directory(DirectoryReaderCallbackTask),
    Read(FileReadTask),
}

pub(crate) struct FileReadingTask {
    context: v8::Global<v8::Context>,
    callback: FileReadingCallback,
}

impl FileReadingTask {
    fn rollback_admission(&self, scope: &mut v8::PinScope<'_, '_>) {
        if let FileReadingCallback::Directory(task) = &self.callback {
            task.rollback_admission(scope);
        }
    }
}

impl JsContextHost {
    pub(crate) fn register_blob_read_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: crate::blob::read::BlobReadTask,
    ) -> Option<moli_file_api::BlobReadCompletion> {
        let target = self.current_runtime_window_execution_context_identity(scope)?;
        let task_id = self.file_reading_tasks.allocate_task_id(RendererPageFileReadingTaskId::from_raw);
        let sender = self.page_file_reading_sender();
        let complete = crate::blob::read::completion_slot(&task.result, move || {
            sender.send(target, task_id, RendererPageFileReadingTaskKind::BlobRead).is_ok()
        });
        let payload = FileReadingTask {
            context: v8::Global::new(scope, scope.get_current_context()),
            callback: FileReadingCallback::Read(FileReadTask::Blob(task)),
        };
        self.file_reading_tasks.push(PendingWindowTask::new(
            task_id, target, RendererPageFileReadingTaskKind::BlobRead, payload,
        ));
        Some(complete)
    }


    pub(crate) fn queue_file_reading_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: FileReadTask,
    ) -> bool {
        let Some(target) = self.current_runtime_window_execution_context_identity(scope) else {
            return false;
        };
        let kind = task.kind();
        let task_id = self
            .file_reading_tasks
            .allocate_task_id(RendererPageFileReadingTaskId::from_raw);
        let payload = FileReadingTask {
            context: v8::Global::new(scope, scope.get_current_context()),
            callback: FileReadingCallback::Read(task),
        };
        self.file_reading_tasks
            .push(PendingWindowTask::new(task_id, target, kind, payload));
        if self
            .page_file_reading_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }
        self.file_reading_tasks.remove_exact(task_id, target, kind);
        false
    }

    /// Admit one `readEntries()` operation on the exact calling
    /// Window realm and publish its immutable envelope to FileReading.
    pub(crate) fn queue_directory_reader_callback_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        reader: v8::Local<'s, v8::Object>,
        success_callback: WebIdlCallbackFunction,
        error_callback: Option<WebIdlCallbackFunction>,
    ) -> bool {
        let Some(target) = self.current_runtime_window_execution_context_identity(scope) else {
            return false;
        };
        let task_id = self
            .file_reading_tasks
            .allocate_task_id(RendererPageFileReadingTaskId::from_raw);
        let admission = DirectoryReaderCallbackTask::admit(
            scope,
            self,
            reader,
            success_callback,
            error_callback,
            task_id,
        );
        let DirectoryReaderCallbackAdmission::Task { kind, task } = admission else {
            return true;
        };
        self.file_reading_tasks.push(PendingWindowTask::new(
            task_id,
            target,
            kind,
            FileReadingTask {
                context: v8::Global::new(scope, scope.get_current_context()),
                callback: FileReadingCallback::Directory(task),
            },
        ));

        if self
            .page_file_reading_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }

        if let Some(pending) = self.file_reading_tasks.remove_exact(task_id, target, kind) {
            pending.payload().rollback_admission(scope);
        }
        tracing::debug!(
            ?target,
            ?task_id,
            ?kind,
            "retired directory-reader callback after FileReading route closure"
        );
        false
    }

    pub(crate) fn current_pending_file_reading_task(
        &self,
        task_id: RendererPageFileReadingTaskId,
    ) -> Option<(
        WindowExecutionContextIdentity,
        RendererPageFileReadingTaskKind,
    )> {
        let pending = self.file_reading_tasks.pending(task_id)?;
        self.window_execution_context_identity_is_current(pending.target())
            .then_some((pending.target(), pending.kind()))
    }

    pub(crate) fn take_pending_file_reading_task_for_exact_target(
        &mut self,
        task_id: RendererPageFileReadingTaskId,
        target: WindowExecutionContextIdentity,
        kind: RendererPageFileReadingTaskKind,
    ) -> Option<FileReadingTask> {
        self.file_reading_tasks
            .remove_exact(task_id, target, kind)
            .map(PendingWindowTask::into_payload)
    }

    /// Discard one stale scheduler envelope and roll back only the exact
    /// active reader request installed by that payload.
    pub(crate) fn discard_pending_file_reading_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task_id: RendererPageFileReadingTaskId,
    ) -> bool {
        let Some(pending) = self.file_reading_tasks.remove(task_id) else {
            return false;
        };
        pending.payload().rollback_admission(scope);
        true
    }

    pub(crate) fn dispatch_authorized_file_reading_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowExecutionContextIdentity,
        task: FileReadingTask,
    ) -> PageFileReadingTargetEffect {
        if !self.window_execution_context_identity_is_current(target) {
            return PageFileReadingTargetEffect::CurrentOwnerCallbackRetired;
        }
        let context = v8::Local::new(scope, &task.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let effect = match task.callback {
            FileReadingCallback::Directory(task) => match task.invoke(scope, host_ptr) {
                DirectoryReaderCallbackTaskEffect::CallbackInvoked => {
                    PageFileReadingTargetEffect::CallbackInvokedForCurrentOwner
                }
                DirectoryReaderCallbackTaskEffect::CallbackNotInvoked => {
                    PageFileReadingTargetEffect::CurrentOwnerCallbackRetired
                }
                DirectoryReaderCallbackTaskEffect::StaleReaderRequest => {
                    PageFileReadingTargetEffect::DiscardedStaleReaderRequest
                }
            },
            FileReadingCallback::Read(task) => task.invoke(scope),
        };
        if matches!(effect, PageFileReadingTargetEffect::BlobReadCompleted) {
            dispatch_scope.defer_restore(scope, previous_scope);
        } else {
            dispatch_scope.restore(scope, previous_scope);
        }
        effect
    }
}
