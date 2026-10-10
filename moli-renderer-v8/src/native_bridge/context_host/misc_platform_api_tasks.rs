use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    context_bootstrap::{
        EncryptedMediaRequestTask, LegacyStorageQuotaCallbackOutcome,
        LegacyStorageQuotaCallbackTask, LegacyStorageQuotaCallbackTaskEffect,
        MediaTrackConstraintsTask,
    },
    page_task_queue::{
        PageMiscPlatformApiTargetEffect, RendererPageMiscPlatformApiTaskId,
        RendererPageMiscPlatformApiTaskKind,
    },
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, WindowDocumentTaskTarget};

pub(super) type MiscPlatformApiTaskState = ExactWindowDocumentTaskLedger<
    RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
    MiscPlatformApiTask,
>;

pub(crate) enum MiscPlatformApiTask {
    LegacyStorageQuota(LegacyStorageQuotaCallbackTask),
    MediaTrackConstraints(Box<MediaTrackConstraintsTask>),
    EncryptedMediaRequest(EncryptedMediaRequestTask),
}

impl JsContextHost {
    /// Publish one deprecated-quota callback to the exact calling
    /// Window/Document's miscellaneous-platform task source.
    pub(crate) fn queue_legacy_storage_quota_callback_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        callback: WebIdlCallbackFunction,
        outcome: LegacyStorageQuotaCallbackOutcome,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        let task_id = self
            .misc_platform_api_tasks
            .allocate_task_id(RendererPageMiscPlatformApiTaskId::from_raw);
        let kind = outcome.kind();
        let task = MiscPlatformApiTask::LegacyStorageQuota(LegacyStorageQuotaCallbackTask::new(
            scope, self, callback, outcome,
        ));
        self.queue_misc_platform_api_task(target, task_id, kind, task)
    }

    pub(crate) fn queue_media_track_constraints_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        track: v8::Local<'s, v8::Object>,
        task: MediaTrackConstraintsTask,
    ) -> bool {
        self.queue_receiver_misc_platform_api_task(
            scope,
            track,
            RendererPageMiscPlatformApiTaskKind::MediaTrackConstraints,
            MiscPlatformApiTask::MediaTrackConstraints(Box::new(task)),
        )
    }

    pub(crate) fn queue_encrypted_media_request_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        navigator: v8::Local<'s, v8::Object>,
        task: EncryptedMediaRequestTask,
    ) -> bool {
        self.queue_receiver_misc_platform_api_task(
            scope,
            navigator,
            RendererPageMiscPlatformApiTaskKind::EncryptedMediaRequest,
            MiscPlatformApiTask::EncryptedMediaRequest(task),
        )
    }

    fn queue_receiver_misc_platform_api_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        receiver: v8::Local<'s, v8::Object>,
        kind: RendererPageMiscPlatformApiTaskKind,
        task: MiscPlatformApiTask,
    ) -> bool {
        // The native receiver owns admission. Borrowed bindings may allocate
        // Promises in another realm without changing the task's document owner.
        let Some(context) = receiver.get_creation_context(scope) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let Some(target) = self.current_window_document_task_target(scope) else {
            return false;
        };
        let task_id = self
            .misc_platform_api_tasks
            .allocate_task_id(RendererPageMiscPlatformApiTaskId::from_raw);
        self.queue_misc_platform_api_task(target, task_id, kind, task)
    }

    fn queue_misc_platform_api_task(
        &mut self,
        target: WindowDocumentTaskTarget,
        task_id: RendererPageMiscPlatformApiTaskId,
        kind: RendererPageMiscPlatformApiTaskKind,
        task: MiscPlatformApiTask,
    ) -> bool {
        self.misc_platform_api_tasks
            .push(PendingExactWindowDocumentTask::new(
                task_id, target, kind, task,
            ));

        if self
            .page_misc_platform_api_sender()
            .send(target, task_id, kind)
            .is_ok()
        {
            return true;
        }

        let _ = self
            .misc_platform_api_tasks
            .remove_exact(task_id, target, kind);
        tracing::debug!(
            ?target,
            ?task_id,
            ?kind,
            "retired native continuation after MiscPlatformApi route closure"
        );
        false
    }

    pub(crate) fn current_pending_misc_platform_api_task(
        &self,
        task_id: RendererPageMiscPlatformApiTaskId,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererPageMiscPlatformApiTaskKind,
    )> {
        let pending = self.misc_platform_api_tasks.pending(task_id)?;
        let current_target = self.current_window_document_task_target_for_dispatch_scope(
            pending.target().dispatch_scope(),
        )?;
        Some((current_target, pending.kind()))
    }

    pub(crate) fn take_pending_misc_platform_api_task_for_exact_target(
        &mut self,
        task_id: RendererPageMiscPlatformApiTaskId,
        target: WindowDocumentTaskTarget,
        kind: RendererPageMiscPlatformApiTaskKind,
    ) -> Option<MiscPlatformApiTask> {
        self.misc_platform_api_tasks
            .remove_exact(task_id, target, kind)
            .map(PendingExactWindowDocumentTask::into_payload)
    }

    pub(crate) fn discard_pending_misc_platform_api_task(
        &mut self,
        task_id: RendererPageMiscPlatformApiTaskId,
    ) -> bool {
        self.misc_platform_api_tasks.remove(task_id).is_some()
    }

    pub(crate) fn dispatch_authorized_misc_platform_api_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        task: MiscPlatformApiTask,
    ) -> PageMiscPlatformApiTargetEffect {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return PageMiscPlatformApiTargetEffect::CurrentOwnerCallbackRetired;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let invoked = match task {
            MiscPlatformApiTask::LegacyStorageQuota(task) => {
                task.invoke(scope, host_ptr)
                    == LegacyStorageQuotaCallbackTaskEffect::CallbackInvoked
            }
            MiscPlatformApiTask::MediaTrackConstraints(task) => task.invoke(scope),
            MiscPlatformApiTask::EncryptedMediaRequest(task) => task.invoke(scope),
        };
        let effect = if invoked {
            PageMiscPlatformApiTargetEffect::CallbackInvokedForCurrentOwner
        } else {
            PageMiscPlatformApiTargetEffect::CurrentOwnerCallbackRetired
        };
        dispatch_scope.restore(scope, previous_scope);
        effect
    }
}
