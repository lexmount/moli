use crate::{
    context_bootstrap::{FileEntryFileCallbackTask, FileEntryFileCallbackTaskEffect},
    page_task_queue::{
        PageFileEntryFileCallbackTargetEffect, RendererPageFileEntryFileCallbackTaskId,
        RendererPageFileEntryFileCallbackTaskKind,
    },
};
use moli_webidl_callback::WebIdlCallbackFunction;

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{PageScriptHost, WindowDocumentTaskTarget};

pub(super) type FileEntryFileCallbackState = ExactWindowDocumentTaskLedger<
    RendererPageFileEntryFileCallbackTaskId,
    RendererPageFileEntryFileCallbackTaskKind,
    FileEntryFileCallbackTask,
>;

impl PageScriptHost {
    /// Queue one `FileSystemFileEntry.file()` success callback on the exact
    /// calling Window/Document and the shared DOM-manipulation source.
    pub(crate) fn queue_file_entry_file_callback_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        callback: WebIdlCallbackFunction,
        file: v8::Local<'_, v8::Object>,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        let kind = RendererPageFileEntryFileCallbackTaskKind::Success;
        let task_id = self
            .file_entry_file_callbacks
            .allocate_task_id(RendererPageFileEntryFileCallbackTaskId::from_raw);
        let callback = FileEntryFileCallbackTask::new(scope, self, callback, file);
        self.file_entry_file_callbacks
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, callback,
            ));

        if self
            .page_file_entry_file_callback_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }

        let _ = self
            .file_entry_file_callbacks
            .remove_exact(task_id, target, kind);
        tracing::debug!(
            ?target,
            ?task_id,
            ?kind,
            "retired FileSystemFileEntry.file callback after DOM-manipulation route closure"
        );
        false
    }

    pub(crate) fn current_pending_file_entry_file_callback_task(
        &self,
        task_id: RendererPageFileEntryFileCallbackTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageFileEntryFileCallbackTaskKind,
    )> {
        let pending = self.file_entry_file_callbacks.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    pub(crate) fn take_pending_file_entry_file_callback_for_exact_target(
        &mut self,
        task_id: RendererPageFileEntryFileCallbackTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageFileEntryFileCallbackTaskKind,
    ) -> Option<FileEntryFileCallbackTask> {
        self.file_entry_file_callbacks
            .remove_exact(task_id, target, kind)
            .map(PendingExactWindowDocumentTask::into_payload)
    }

    pub(crate) fn discard_pending_file_entry_file_callback_task(
        &mut self,
        task_id: RendererPageFileEntryFileCallbackTaskId,
    ) -> bool {
        self.file_entry_file_callbacks.remove(task_id).is_some()
    }

    /// Invoke one callback already authorized against its exact calling
    /// Window/Document. Callback-realm currentness remains independent.
    pub(crate) fn dispatch_authorized_file_entry_file_callback(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        target: WindowDocumentTaskTarget,
        callback: FileEntryFileCallbackTask,
    ) -> PageFileEntryFileCallbackTargetEffect {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return PageFileEntryFileCallbackTargetEffect::CurrentOwnerCallbackRetired;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let effect = match callback.invoke(scope, host_ptr) {
            FileEntryFileCallbackTaskEffect::CallbackInvoked => {
                PageFileEntryFileCallbackTargetEffect::CallbackInvokedForCurrentOwner
            }
            FileEntryFileCallbackTaskEffect::CallbackNotInvoked => {
                PageFileEntryFileCallbackTargetEffect::CurrentOwnerCallbackRetired
            }
        };
        dispatch_scope.restore(scope, previous_scope);
        effect
    }
}
