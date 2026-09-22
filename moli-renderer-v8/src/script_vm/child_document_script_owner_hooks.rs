use std::{cell::RefCell, rc::Rc};

use crate::{
    document_runtime::DomHandle,
    document_script_scheduler::{
        DocumentScriptExecutionOutcome, FrameDocumentClassicReadyWork,
        FrameDocumentClassicScriptSchedulerWork, FrameDocumentClassicSourceFailureWork,
        FrameDocumentModuleScriptReadyWork, FrameModuleScriptRunOutcome,
    },
    frame_owner_model::{
        FrameDocumentClassicParserResumeApplication,
        FrameDocumentClassicParserResumeCompletionAction,
        FrameDocumentClassicScriptCompletionAction, FrameDocumentClassicScriptExecutionFinish,
        FrameDocumentClassicScriptReadyTarget, FrameDocumentClassicScriptSourceFailureTarget,
        FrameDocumentClassicSourceFailureReportApplication, FrameDocumentJavascriptUrlCompletion,
        FrameDocumentJavascriptUrlPostExecutionApplication, FrameDocumentScriptElementEventKind,
        FrameDocumentTaskOwner, FrameRealmId, FrameScriptJob,
        PendingChildExternalClassicDocumentScript, PendingChildJavascriptUrlDocumentScript,
    },
    native_bridge::JsContextHost,
};

use super::{
    ScriptVm, child_document_event::ChildDocumentEventOwner,
    child_document_script_scheduler::ChildDocumentScriptSchedulerOwner,
    frame_script_jobs::FrameScriptCompletionValue,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::script_vm) enum ChildDocumentScriptRealmSelection {
    Current(FrameRealmId),
    RealmMaterializationFailed,
    MissingCurrentRealm,
    StaleRealm {
        expected: FrameRealmId,
        current: FrameRealmId,
    },
}

pub(in crate::script_vm) struct ChildDocumentScriptOwnerHooks<'vm> {
    vm: &'vm mut ScriptVm,
}

pub(in crate::script_vm) struct ChildParserScriptNestingScope {
    context_host: Rc<RefCell<JsContextHost>>,
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
}

impl Drop for ChildParserScriptNestingScope {
    fn drop(&mut self) {
        self.context_host
            .borrow_mut()
            .exit_child_parser_script_nesting(self.child_handle, self.owner);
    }
}

impl<'vm> ChildDocumentScriptOwnerHooks<'vm> {
    pub(in crate::script_vm) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(in crate::script_vm) fn enter_parser_script_nesting(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ChildParserScriptNestingScope> {
        let context_host = self.vm._context_host.clone();
        let entered = context_host
            .borrow_mut()
            .enter_child_parser_script_nesting(child_handle, owner);
        entered.then_some(ChildParserScriptNestingScope {
            context_host,
            child_handle,
            owner,
        })
    }

    pub(in crate::script_vm) fn resume_parser_for_classic_execution(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        script_handle: DomHandle,
    ) -> bool {
        self.vm
            ._context_host
            .borrow_mut()
            .resume_live_child_document_parser_for_classic_execution(
                child_handle,
                owner,
                script_handle,
            )
    }

    pub(in crate::script_vm) fn select_current_realm(
        &mut self,
        child_handle: DomHandle,
        expected_realm_id: Option<FrameRealmId>,
        script_handle: DomHandle,
        operation: &'static str,
    ) -> ChildDocumentScriptRealmSelection {
        let Some(owner) = self
            .vm
            ._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
        else {
            return ChildDocumentScriptRealmSelection::MissingCurrentRealm;
        };
        let current_realm_id = self
            .vm
            .current_child_frame_realm_id_for_owner(child_handle, owner);
        let Ok(current_realm_id) = current_realm_id else {
            tracing::warn!(
                error = %current_realm_id.expect_err("matched realm lookup error"),
                ?child_handle,
                ?script_handle,
                operation,
                "child document script work has no authorized materialized FrameRealm"
            );
            return ChildDocumentScriptRealmSelection::RealmMaterializationFailed;
        };

        if let Some(expected) = expected_realm_id
            && expected != current_realm_id
        {
            tracing::debug!(
                ?child_handle,
                ?script_handle,
                expected_realm_id = ?expected,
                current_realm_id = ?current_realm_id,
                operation,
                "child document script work has a stale FrameRealm"
            );
            return ChildDocumentScriptRealmSelection::StaleRealm {
                expected,
                current: current_realm_id,
            };
        }

        ChildDocumentScriptRealmSelection::Current(current_realm_id)
    }

    pub(in crate::script_vm) fn prepare_child_classic_script_execution(
        &mut self,
        ready: FrameDocumentClassicReadyWork,
    ) -> crate::frame_owner_model::FrameDocumentClassicPrepareApplication {
        self.vm
            ._context_host
            .borrow_mut()
            .prepare_child_classic_script_execution(ready)
    }

    pub(in crate::script_vm) fn report_child_classic_script_source_failure(
        &mut self,
        failed: FrameDocumentClassicSourceFailureWork,
    ) -> FrameDocumentClassicSourceFailureReportApplication {
        self.vm
            ._context_host
            .borrow_mut()
            .report_child_classic_script_source_failure(failed)
    }

    pub(in crate::script_vm) fn cancel_child_deferred_classic_ready_work(
        &mut self,
        target: FrameDocumentClassicScriptReadyTarget,
        script_handle: DomHandle,
    ) -> crate::frame_owner_model::FrameDocumentClassicDeferredCompletionApplication {
        self.vm
            ._context_host
            .borrow_mut()
            .cancel_child_deferred_classic_ready_work(target, script_handle)
    }

    pub(in crate::script_vm) fn complete_child_deferred_classic_terminal_without_event(
        &mut self,
        target: FrameDocumentClassicScriptSourceFailureTarget,
        script_handle: DomHandle,
    ) -> crate::frame_owner_model::FrameDocumentClassicDeferredCompletionApplication {
        self.vm
            ._context_host
            .borrow_mut()
            .complete_child_deferred_classic_terminal_without_event(target, script_handle)
    }

    pub(in crate::script_vm) fn execute_frame_script_job_selected_task_body(
        &mut self,
        job: FrameScriptJob,
    ) -> anyhow::Result<()> {
        self.vm.execute_frame_script_job_selected_task_body(job)
    }

    pub(in crate::script_vm) fn execute_frame_script_job_value_type_completion_selected_task_body(
        &mut self,
        job: FrameScriptJob,
    ) -> anyhow::Result<FrameScriptCompletionValue> {
        self.vm
            .execute_frame_script_job_value_type_completion_selected_task_body(job)
    }

    pub(in crate::script_vm) fn finish_child_classic_script_execution(
        &mut self,
        finish: FrameDocumentClassicScriptExecutionFinish,
    ) -> Option<FrameDocumentClassicScriptCompletionAction> {
        self.vm
            ._context_host
            .borrow_mut()
            .finish_executing_child_classic_script(finish)
    }

    pub(in crate::script_vm) fn dispatch_script_element_event_for_parts_selected_task_body(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        script_handle: DomHandle,
        event_kind: FrameDocumentScriptElementEventKind,
    ) -> anyhow::Result<()> {
        ChildDocumentEventOwner::new(self.vm)
            .dispatch_script_element_event_for_parts_selected_task_body(
                task_owner,
                realm_id,
                script_handle,
                event_kind,
            )
    }

    pub(in crate::script_vm) fn resume_child_classic_parser_after_completion(
        &mut self,
        action: FrameDocumentClassicParserResumeCompletionAction,
    ) -> FrameDocumentClassicParserResumeApplication {
        let realm_id = action.target().realm_id();
        let context_host = self.vm._context_host.clone();
        self.vm
            .with_frame_realm_scope(
                realm_id,
                move |scope, _host_ptr| {
                Ok(context_host
                    .borrow_mut()
                    .resume_child_classic_parser_after_completion(scope, action))
                },
            )
            .unwrap_or_else(|error| {
                tracing::warn!(
                    %error,
                    ?realm_id,
                    "failed to enter child parser-resume FrameRealm"
                );
                FrameDocumentClassicParserResumeApplication::skipped(
                    crate::frame_owner_model::FrameDocumentClassicParserResumeSkipReason::StaleRealm,
                )
            })
    }

    pub(in crate::script_vm) fn notify_parser_classic_next_owner_action(
        &mut self,
        work: FrameDocumentClassicScriptSchedulerWork,
    ) {
        ChildDocumentScriptSchedulerOwner::new(self.vm)
            .notify_parser_classic_next_owner_action(work)
    }

    pub(in crate::script_vm) fn child_external_classic_script_execution_action_for_owner(
        &mut self,
        work: &PendingChildExternalClassicDocumentScript,
        realm_id: FrameRealmId,
    ) -> Option<crate::frame_owner_model::FrameDocumentExternalClassicScriptExecutionAction> {
        self.vm
            ._context_host
            .borrow()
            .child_external_classic_script_execution_action_for_owner(work, realm_id)
    }

    pub(in crate::script_vm) fn child_javascript_url_script_execution_action_for_owner(
        &mut self,
        work: &PendingChildJavascriptUrlDocumentScript,
        realm_id: FrameRealmId,
    ) -> Option<crate::frame_owner_model::FrameDocumentJavascriptUrlScriptExecutionAction> {
        self.vm
            ._context_host
            .borrow()
            .child_javascript_url_script_execution_action_for_owner(work, realm_id)
    }

    pub(in crate::script_vm) fn drop_child_javascript_url_document_script(
        &mut self,
        work: &PendingChildJavascriptUrlDocumentScript,
    ) {
        let _ = self
            .vm
            ._context_host
            .borrow_mut()
            .drop_child_javascript_url_document_script(work);
    }

    pub(in crate::script_vm) fn apply_child_javascript_url_post_execution(
        &mut self,
        action: crate::frame_owner_model::FrameDocumentJavascriptUrlPostExecutionAction,
    ) -> FrameDocumentJavascriptUrlPostExecutionApplication {
        let attempted_script_job = action.attempted_script_job();
        let failed_script_job = action.failed_script_job();
        match action.completion() {
            FrameDocumentJavascriptUrlCompletion::FailedScriptJob => {
                let target = action.target();
                self.vm
                    ._context_host
                    .borrow_mut()
                    .finish_child_javascript_url_without_string_completion(target, false);
                return FrameDocumentJavascriptUrlPostExecutionApplication {
                    attempted_script_job,
                    failed_script_job,
                    string_completion_committed: false,
                    lifecycle_followup_queued: false,
                    initial_classic_ready_work: None,
                    owner_transition: None,
                };
            }
            FrameDocumentJavascriptUrlCompletion::String(markup) => {
                let target = action.target();
                let url = action.url().clone();
                let markup = markup.to_owned();
                let preserve_window_event_state = action.preserve_window_event_state();
                let context_host = self.vm._context_host.clone();
                let result = self.vm.with_default_context_scope(move |scope, _host_ptr| {
                    Ok(context_host
                        .borrow_mut()
                        .commit_child_javascript_url_string_completion(
                            scope,
                            target,
                            url,
                            markup,
                            preserve_window_event_state,
                        ))
                });
                let application = result.unwrap_or_else(|error| {
                    tracing::warn!(
                        error = %error,
                        "child javascript URL string-completion commit failed"
                    );
                    FrameDocumentJavascriptUrlPostExecutionApplication {
                        attempted_script_job,
                        failed_script_job: true,
                        string_completion_committed: false,
                        lifecycle_followup_queued: false,
                        initial_classic_ready_work: None,
                        owner_transition: None,
                    }
                });
                if let Some(transition) = application.owner_transition {
                    self.vm.apply_child_document_owner_transition(transition);
                }
                return application;
            }
            FrameDocumentJavascriptUrlCompletion::NonString => {}
        }
        let lifecycle_followup_queued = self
            .vm
            ._context_host
            .borrow_mut()
            .finish_child_javascript_url_without_string_completion(
                action.target(),
                action.dispatch_load_on_no_string_completion(),
            );
        FrameDocumentJavascriptUrlPostExecutionApplication {
            attempted_script_job,
            failed_script_job,
            string_completion_committed: false,
            lifecycle_followup_queued,
            initial_classic_ready_work: None,
            owner_transition: None,
        }
    }

    pub(in crate::script_vm) async fn run_child_module_script_ready_work(
        &mut self,
        work: FrameDocumentModuleScriptReadyWork,
    ) -> FrameModuleScriptRunOutcome<DocumentScriptExecutionOutcome> {
        self.vm.run_child_module_script_ready_work(work).await
    }

    pub(in crate::script_vm) fn settle_child_async_classic_script_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        load_delay: crate::frame_owner_model::ChildDocumentAsyncClassicScriptLoadDelay,
    ) -> bool {
        self.vm
            ._context_host
            .borrow_mut()
            .settle_child_async_classic_script_load_delay(child_handle, owner, load_delay)
    }

    pub(in crate::script_vm) fn complete_child_deferred_classic_script(
        &mut self,
        target: crate::frame_owner_model::FrameDocumentClassicScriptCompletionTarget,
    ) -> crate::frame_owner_model::FrameDocumentClassicDeferredCompletionApplication {
        self.vm
            ._context_host
            .borrow_mut()
            .complete_child_deferred_classic_script(target)
    }
}
