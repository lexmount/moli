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
        PageMiscPlatformApiTargetEffect, RendererPageMiscPlatformApiTarget,
        RendererPageMiscPlatformApiTaskId, RendererPageMiscPlatformApiTaskKind,
    },
};

use super::window_document_tasks::{PendingWindowTask, WindowTaskLedger};
use super::{
    OwnerDispatchScope, PageScriptHost, WindowDocumentOwner, WindowDocumentTaskTarget,
    WindowExecutionContextBinding,
};

pub(super) type MiscPlatformApiTaskState = WindowTaskLedger<
    RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
    MiscPlatformApiTask,
    RendererPageMiscPlatformApiTarget,
>;

pub(crate) enum MiscPlatformApiTask {
    LegacyStorageQuota(LegacyStorageQuotaCallbackTask),
    MediaTrackConstraints(Box<MediaTrackConstraintsTask>),
    EncryptedMediaRequest(EncryptedMediaRequestTask),
    PerformanceObserver(WindowExecutionContextBinding),
    ContentSecurityPolicyViolation {
        report: bool,
        element: Option<DomHandle>,
        violation: Box<DocumentContentSecurityPolicyViolation>,
    },
}

impl PageScriptHost {
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
        self.report_and_queue_content_security_policy_violation(
            scope, owner, element, element, violation,
        )
    }

    /// Fetch violations target the requesting Document. Retain the initiating
    /// element independently for the Inspector issue's node attribution.
    pub(crate) fn queue_subresource_content_security_policy_violation_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: OwnerDispatchScope,
        element: DomHandle,
        violation: &DocumentContentSecurityPolicyViolation,
    ) -> bool {
        self.report_and_queue_content_security_policy_violation(
            scope,
            owner,
            Some(element),
            None,
            violation,
        )
    }

    fn report_and_queue_content_security_policy_violation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        owner: OwnerDispatchScope,
        diagnostic_element: Option<DomHandle>,
        event_element: Option<DomHandle>,
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
                unsafe { &mut *self.document_runtime }
                    .record_content_security_policy_inspector_issue(diagnostic_element, violation);
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
        self.queue_content_security_policy_violation_event_task(
            target,
            event_element,
            violation,
            false,
        )
    }

    /// All newly queued CSP violation events share this exact-owner source,
    /// including checks that have already recorded their inspector issue and
    /// sent their network reports.
    pub(crate) fn queue_content_security_policy_violation_event_task(
        &mut self,
        target: WindowDocumentTaskTarget,
        element: Option<DomHandle>,
        violation: &DocumentContentSecurityPolicyViolation,
        report: bool,
    ) -> bool {
        let task_id = self
            .misc_platform_api_tasks
            .allocate_task_id(RendererPageMiscPlatformApiTaskId::from_raw);
        self.queue_misc_platform_api_task(
            RendererPageMiscPlatformApiTarget::Document(target),
            task_id,
            RendererPageMiscPlatformApiTaskKind::ContentSecurityPolicyViolation,
            MiscPlatformApiTask::ContentSecurityPolicyViolation {
                report,
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
        self.queue_misc_platform_api_task(
            RendererPageMiscPlatformApiTarget::Document(target),
            task_id,
            kind,
            task,
        )
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
        if self.misc_platform_api_tasks.contains_target(target) {
            return;
        }
        let binding = WindowExecutionContextBinding::new(
            identity.owner(),
            identity.dispatch_scope(),
            identity.realm_token(),
            v8::Global::new(scope, context),
        );
        let task_id = self
            .misc_platform_api_tasks
            .allocate_task_id(RendererPageMiscPlatformApiTaskId::from_raw);
        self.queue_misc_platform_api_task(
            target,
            task_id,
            RendererPageMiscPlatformApiTaskKind::PerformanceObserver,
            MiscPlatformApiTask::PerformanceObserver(binding),
        );
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
        self.queue_misc_platform_api_task(
            RendererPageMiscPlatformApiTarget::Document(target),
            task_id,
            kind,
            task,
        )
    }

    fn queue_misc_platform_api_task(
        &mut self,
        target: RendererPageMiscPlatformApiTarget,
        task_id: RendererPageMiscPlatformApiTaskId,
        kind: RendererPageMiscPlatformApiTaskKind,
        task: MiscPlatformApiTask,
    ) -> bool {
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
            "retired native continuation after MiscPlatformApi route closure"
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
        let target = match target {
            RendererPageMiscPlatformApiTarget::Window(_) => {
                let invoked = match task {
                    MiscPlatformApiTask::PerformanceObserver(binding) => binding
                        .with_current_scope(scope, host_ptr, |scope, _| {
                            crate::context_bootstrap::notify_performance_observers(scope)
                        })
                        .unwrap_or(false),
                    _ => false,
                };
                return misc_platform_api_callback_effect(invoked);
            }
            RendererPageMiscPlatformApiTarget::Document(target) => target,
        };
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
            MiscPlatformApiTask::PerformanceObserver(_) => false,
            MiscPlatformApiTask::ContentSecurityPolicyViolation {
                report,
                element,
                violation,
            } => {
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
                let report_owner = target.owner();
                let target = EventTargetHandle::Node(event_target);
                let metadata = self.document_csp_violation_metadata(resolved.document_handle);
                let fields = metadata.event_fields(&violation);
                if report {
                    unsafe { &mut *self.document_runtime }
                        .record_content_security_policy_inspector_issue(element, &violation);
                    match report_owner {
                        WindowDocumentOwner::Frame(document_owner) => {
                            let child = match dispatch_scope {
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
                                scope, self, document_owner.popup_id(), document_owner, &fields,
                                &violation.report_uri_endpoints, &violation.report_to_endpoints);
                        }
                    }
                }

                crate::host::event_target_value(scope, host_ptr, target)
                    .ok()
                    .and_then(|value| {
                        create_content_security_policy_violation_event(scope, value, value, &fields)
                            .ok()
                    })
                    .is_some_and(|event| {
                        // SAFETY: the Host and its DocumentRuntime share one ScriptVm.
                        let _ = unsafe { &mut *self.document_runtime }
                            .dispatch_public_event_best_effort(
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
        dispatch_scope.restore(scope, previous_scope);
        misc_platform_api_callback_effect(invoked)
    }
}

fn misc_platform_api_callback_effect(invoked: bool) -> PageMiscPlatformApiTargetEffect {
    if invoked {
        PageMiscPlatformApiTargetEffect::CallbackInvokedForCurrentOwner
    } else {
        PageMiscPlatformApiTargetEffect::CurrentOwnerCallbackRetired
    }
}
