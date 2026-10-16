use crate::{
    context_bootstrap::{
        DirectoryReaderCallbackAdmission, DirectoryReaderCallbackTask,
        DirectoryReaderCallbackTaskEffect, FileReaderTask, FileReaderTaskEffect,
    },
    page_task_queue::{
        PageFileReadingTargetEffect, RendererPageFileReaderPhase, RendererPageFileReadingTaskId,
        RendererPageFileReadingTaskKind,
    },
};
use moli_webidl_callback::WebIdlCallbackFunction;

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(super) type FileReadingState = ExactWindowDocumentTaskLedger<
    RendererPageFileReadingTaskId,
    RendererPageFileReadingTaskKind,
    FileReadingTask,
>;

pub(crate) enum FileReadingTask {
    Directory(DirectoryReaderCallbackTask),
    Reader(FileReaderTask),
    Blob {
        task: crate::blob::read::BlobReadTask,
        execution_context: super::WindowExecutionContextIdentity,
    },
}

impl FileReadingTask {
    fn rollback_admission(&self, scope: &mut v8::PinScope<'_, '_>) {
        if let Self::Directory(task) = self {
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
        let target = self.current_window_document_task_target(scope)?;
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        let task_id = self
            .file_reading_tasks
            .allocate_task_id(RendererPageFileReadingTaskId::from_raw);
        let sender = self.page_file_reading_sender();
        let complete = crate::blob::read::completion_slot(&task.result, move || {
            sender
                .send(target, task_id, RendererPageFileReadingTaskKind::BlobRead)
                .is_ok()
        });
        self.file_reading_tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id,
                target,
                RendererPageFileReadingTaskKind::BlobRead,
                FileReadingTask::Blob {
                    task,
                    execution_context,
                },
            ));
        Some(complete)
    }


    pub(crate) fn queue_file_reader_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        reader: v8::Local<'s, v8::Object>,
        read_id: f64,
        phase: RendererPageFileReaderPhase,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        let kind = RendererPageFileReadingTaskKind::FileReader(phase);
        let task_id = self
            .file_reading_tasks
            .allocate_task_id(RendererPageFileReadingTaskId::from_raw);
        let payload = FileReadingTask::Reader(FileReaderTask::new(scope, reader, read_id, phase));
        self.file_reading_tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, payload,
            ));
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
    /// Window/Document and publish its immutable envelope to FileReading.
    pub(crate) fn queue_directory_reader_callback_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        reader: v8::Local<'s, v8::Object>,
        success_callback: WebIdlCallbackFunction,
        error_callback: Option<WebIdlCallbackFunction>,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target(scope) else {
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
        self.file_reading_tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id,
                target,
                kind,
                FileReadingTask::Directory(task),
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
    ) -> Option<(WindowDocumentTaskTarget, RendererPageFileReadingTaskKind)> {
        let pending = self.file_reading_tasks.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    pub(crate) fn take_pending_file_reading_task_for_exact_target(
        &mut self,
        task_id: RendererPageFileReadingTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageFileReadingTaskKind,
    ) -> Option<FileReadingTask> {
        self.file_reading_tasks
            .remove_exact(task_id, target, kind)
            .map(PendingExactWindowDocumentTask::into_payload)
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
        target: WindowDocumentTaskTarget,
        task: FileReadingTask,
    ) -> PageFileReadingTargetEffect {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return PageFileReadingTargetEffect::CurrentOwnerCallbackRetired;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let settle_promise = matches!(&task, FileReadingTask::Blob { .. });
        let effect = match task {
            FileReadingTask::Directory(task) => match task.invoke(scope, host_ptr) {
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
            FileReadingTask::Blob { task, execution_context } => {
                if unsafe { &*host_ptr }.window_execution_context_identity_is_current(execution_context) {
                    task.settle(scope);
                    PageFileReadingTargetEffect::CallbackInvokedForCurrentOwner
                } else {
                    PageFileReadingTargetEffect::CurrentOwnerCallbackRetired
                }
            }
            FileReadingTask::Reader(task) => match task.invoke(scope) {
                FileReaderTaskEffect::Advanced => {
                    PageFileReadingTargetEffect::FileReaderPhaseAdvanced
                }
                FileReaderTaskEffect::Stale => {
                    PageFileReadingTargetEffect::DiscardedStaleReaderRequest
                }
            },
        };
        if settle_promise {
            dispatch_scope.defer_restore(scope, previous_scope);
        } else {
            dispatch_scope.restore(scope, previous_scope);
        }
        effect
    }
}
