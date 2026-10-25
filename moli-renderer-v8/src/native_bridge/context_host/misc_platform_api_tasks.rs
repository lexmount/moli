use moli_webidl_callback::WebIdlCallbackFunction;

use crate::{
    content_security_policy::ContentSecurityPolicyViolationEventFields,
    context_bootstrap::{
        EncryptedMediaRequestTask, LegacyStorageQuotaCallbackOutcome,
        LegacyStorageQuotaCallbackTask, LegacyStorageQuotaCallbackTaskEffect,
        MediaTrackConstraintsTask,
    },
    document_runtime::{
        DocumentContentSecurityPolicyViolation, DomHandle, EventTargetHandle,
        create_content_security_policy_violation_event,
    },
    page_task_queue::{
        PageMiscPlatformApiTargetEffect, RendererPageMiscPlatformApiTaskId,
        RendererPageMiscPlatformApiTaskKind,
    },
};

use super::window_document_tasks::{ExactWindowDocumentTaskLedger, PendingExactWindowDocumentTask};
use super::{JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget};

pub(super) type MiscPlatformApiTaskState = ExactWindowDocumentTaskLedger<
    RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
    MiscPlatformApiTask,
>;

pub(crate) enum MiscPlatformApiTask {
    LegacyStorageQuota(LegacyStorageQuotaCallbackTask),
    MediaTrackConstraints(Box<MediaTrackConstraintsTask>),
    EncryptedMediaRequest(EncryptedMediaRequestTask),
    ContentSecurityPolicyViolation {
        element: Option<DomHandle>,
        violation: Box<DocumentContentSecurityPolicyViolation>,
    },
}

impl JsContextHost {
    /// Report on the policy's exact Window/Document, then queue its event.
    /// An optional element is retained until dispatch, when disconnecting or
    /// adopting it must retarget the event to that Document.
    pub(crate) fn queue_content_security_policy_violation_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: OwnerDispatchScope,
        element: Option<DomHandle>,
        violation: &DocumentContentSecurityPolicyViolation,
    ) -> bool {
        let Some(target) = self.current_window_document_task_target_for_dispatch_scope(owner)
        else {
            return false;
        };
        let fields = ContentSecurityPolicyViolationEventFields::from(violation);
        match target.owner() {
            WindowDocumentOwner::Frame(document_owner) => {
                // SAFETY: the Host and its DocumentRuntime share one ScriptVm.
                unsafe { &mut *self.runtime }
                    .record_content_security_policy_inspector_issue(element, violation);
                let child = match owner {
                    OwnerDispatchScope::Child(handle) => Some(handle),
                    _ => None,
                };
                crate::network_host::send_content_security_policy_reports_for_window(
                    scope,
                    self,
                    document_owner,
                    child,
                    &fields,
                    &violation.report_uri_endpoints,
                    &violation.report_to_endpoints,
                );
            }
            WindowDocumentOwner::LightweightPopup(document_owner) => {
                crate::network_host::send_content_security_policy_reports_for_lightweight_popup(
                    scope,
                    self,
                    document_owner.popup_id(),
                    document_owner,
                    &fields,
                    &violation.report_uri_endpoints,
                    &violation.report_to_endpoints,
                );
            }
        }
        let task_id = self
            .misc_platform_api_tasks
            .allocate_task_id(RendererPageMiscPlatformApiTaskId::from_raw);
        self.queue_misc_platform_api_task(
            target,
            task_id,
            RendererPageMiscPlatformApiTaskKind::ContentSecurityPolicyViolation,
            MiscPlatformApiTask::ContentSecurityPolicyViolation {
                element,
                violation: Box::new(violation.clone()),
            },
        )
    }

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
            MiscPlatformApiTask::ContentSecurityPolicyViolation { element, violation } => {
                // CSP checks the shadow-including root at dispatch time, not
                // at the earlier policy check. A stale owner has
                // already been rejected by the Page task arbiter.
                let event_target = element
                    .filter(|element| {
                        self.dom_host().is_connected(*element)
                            && self.dom_host().owner_document_handle(*element)
                                == Some(resolved.document_handle)
                    })
                    .unwrap_or(resolved.document_handle);
                let target = EventTargetHandle::Node(event_target);
                let metadata = self.document_csp_violation_metadata(resolved.document_handle);
                let fields = metadata.event_fields(&violation);
                crate::host::event_target_value(scope, host_ptr, target)
                    .ok()
                    .and_then(|value| {
                        create_content_security_policy_violation_event(scope, value, value, &fields)
                            .ok()
                    })
                    .is_some_and(|event| {
                        // SAFETY: the Host and its DocumentRuntime share one ScriptVm.
                        let _ = unsafe { &mut *self.runtime }.dispatch_public_event_best_effort(
                            scope,
                            host_ptr,
                            target,
                            event,
                            "securitypolicyviolation",
                        );
                        true
                    })
            }
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
