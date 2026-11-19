use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    context_bootstrap::{
        LegacyStorageQuotaCallbackOutcome, LegacyStorageQuotaCallbackTask,
        LegacyStorageQuotaCallbackTaskEffect,
    },
    page_task_queue::{
        PageMiscPlatformApiTargetEffect, RendererPageMiscPlatformApiTarget,
        RendererPageMiscPlatformApiTaskId, RendererPageMiscPlatformApiTaskKind,
    },
};

use super::window_document_tasks::{PendingWindowTask, WindowTaskLedger};
use super::{PageScriptHost, WindowExecutionContextBinding};

pub(super) type MiscPlatformApiTaskState = WindowTaskLedger<
    RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
    MiscPlatformApiTask,
    RendererPageMiscPlatformApiTarget,
>;

pub(crate) enum MiscPlatformApiTask {
    LegacyStorageQuota(LegacyStorageQuotaCallbackTask),
    PerformanceObserver(WindowExecutionContextBinding),
}

impl PageScriptHost {
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
        let kind = outcome.kind();
        let callback = LegacyStorageQuotaCallbackTask::new(scope, self, callback, outcome);
        self.queue_misc_platform_api_task(
            RendererPageMiscPlatformApiTarget::Document(target),
            kind,
            MiscPlatformApiTask::LegacyStorageQuota(callback),
        )
    }

    pub(crate) fn queue_performance_observer_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
    ) {
        let Some(identity) = self.window_execution_context_identity_for_v8_context(scope, context)
        else {
            return;
        };
        let target = RendererPageMiscPlatformApiTarget::Window(identity);
        if self
            .misc_platform_api_tasks
            .targets()
            .any(|queued| queued == target)
        {
            return;
        }
        let binding = WindowExecutionContextBinding::new(
            identity.owner(),
            identity.dispatch_scope(),
            identity.realm_token(),
            v8::Global::new(scope, context),
        );
        self.queue_misc_platform_api_task(
            target,
            RendererPageMiscPlatformApiTaskKind::PerformanceObserver,
            MiscPlatformApiTask::PerformanceObserver(binding),
        );
    }

    fn queue_misc_platform_api_task(
        &mut self,
        target: RendererPageMiscPlatformApiTarget,
        kind: RendererPageMiscPlatformApiTaskKind,
        task: MiscPlatformApiTask,
    ) -> bool {
        let task_id = self
            .misc_platform_api_tasks
            .allocate_task_id(RendererPageMiscPlatformApiTaskId::from_raw);
        self.misc_platform_api_tasks
            .push(PendingWindowTask::new(task_id, target, kind, task));

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
            "retired callback after MiscPlatformApi route closure"
        );
        false
    }

    pub(crate) fn current_pending_misc_platform_api_task(
        &self,
        task_id: RendererPageMiscPlatformApiTaskId,
    ) -> Option<(
        RendererPageMiscPlatformApiTarget,
        RendererPageMiscPlatformApiTaskKind,
    )> {
        let pending = self.misc_platform_api_tasks.pending(task_id)?;
        let current_target = match pending.target() {
            RendererPageMiscPlatformApiTarget::Document(target) => {
                RendererPageMiscPlatformApiTarget::Document(
                    self.current_window_document_task_target_for_dispatch_scope(
                        target.dispatch_scope(),
                    )?,
                )
            }
            RendererPageMiscPlatformApiTarget::Window(identity) => self
                .window_execution_context_identity_is_current(identity)
                .then_some(pending.target())?,
        };
        Some((current_target, pending.kind()))
    }

    pub(crate) fn take_pending_misc_platform_api_task_for_exact_target(
        &mut self,
        task_id: RendererPageMiscPlatformApiTaskId,
        target: RendererPageMiscPlatformApiTarget,
        kind: RendererPageMiscPlatformApiTaskKind,
    ) -> Option<MiscPlatformApiTask> {
        self.misc_platform_api_tasks
            .remove_exact(task_id, target, kind)
            .map(PendingWindowTask::into_payload)
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
        host_ptr: *mut PageScriptHost,
        target: RendererPageMiscPlatformApiTarget,
        task: MiscPlatformApiTask,
    ) -> PageMiscPlatformApiTargetEffect {
        let (target, task) = match (target, task) {
            (
                RendererPageMiscPlatformApiTarget::Window(_),
                MiscPlatformApiTask::PerformanceObserver(binding),
            ) => {
                let invoked = binding
                    .with_current_scope(scope, host_ptr, |scope, _| {
                        crate::context_bootstrap::notify_performance_observers(scope)
                    })
                    .unwrap_or(false);
                return if invoked {
                    PageMiscPlatformApiTargetEffect::CallbackInvokedForCurrentOwner
                } else {
                    PageMiscPlatformApiTargetEffect::CurrentOwnerCallbackRetired
                };
            }
            (
                RendererPageMiscPlatformApiTarget::Document(target),
                MiscPlatformApiTask::LegacyStorageQuota(task),
            ) => (target, task),
            _ => return PageMiscPlatformApiTargetEffect::CurrentOwnerCallbackRetired,
        };
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return PageMiscPlatformApiTargetEffect::CurrentOwnerCallbackRetired;
        };
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let effect = match task.invoke(scope, host_ptr) {
            LegacyStorageQuotaCallbackTaskEffect::CallbackInvoked => {
                PageMiscPlatformApiTargetEffect::CallbackInvokedForCurrentOwner
            }
            LegacyStorageQuotaCallbackTaskEffect::CallbackNotInvoked => {
                PageMiscPlatformApiTargetEffect::CurrentOwnerCallbackRetired
            }
        };
        dispatch_scope.restore(scope, previous_scope);
        effect
    }
}
