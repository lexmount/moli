use crate::{
    document_script_scheduler::{
        DocumentScriptExecutionHooks, DocumentScriptExecutionOutcome,
        FrameDocumentScriptExecutionOwner, FrameDocumentScriptExecutionStartReport,
        FrameModuleScriptTaskActivity,
    },
    frame_owner_model::{
        FrameDocumentExternalClassicExecutionFollowup, FrameDocumentExternalClassicExecutionResult,
        FrameDocumentExternalClassicPrepareFollowup, FrameDocumentExternalClassicPrepareSkipReason,
        FrameDocumentExternalClassicScriptExecution, FrameDocumentJavascriptUrlCompletion,
        FrameDocumentJavascriptUrlExecutionFollowup, FrameDocumentJavascriptUrlExecutionResult,
        FrameDocumentJavascriptUrlPrepareFollowup, FrameDocumentJavascriptUrlPrepareSkipReason,
        FrameDocumentScriptExecutionFollowup, FrameDocumentScriptExecutionResult,
        FrameDocumentScriptExecutionWork, FrameDocumentScriptPrepareFollowup,
        PendingChildDocumentScriptExecutionWork, PendingChildExternalClassicDocumentScript,
        PendingChildJavascriptUrlDocumentScript,
    },
};
use std::{future::Future, pin::Pin};

use super::{
    ChildDocumentScriptActivity, ChildDocumentScriptReadyRunOutcome, ChildDocumentScriptRunOutcome,
    ScriptVm,
    child_document_script_owner_hooks::{
        ChildDocumentScriptOwnerHooks, ChildDocumentScriptRealmSelection,
    },
    frame_script_jobs::FrameScriptCompletionValue,
};

pub(in crate::script_vm) struct ChildDocumentScriptExecutionOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

impl<'vm> ChildDocumentScriptExecutionOwner<'vm> {
    pub(in crate::script_vm) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(in crate::script_vm) async fn run_ready_work(
        &mut self,
        work: PendingChildDocumentScriptExecutionWork,
    ) -> ChildDocumentScriptReadyRunOutcome {
        let hooks = ScriptVmChildDocumentScriptExecutionHooks::new(self.vm);
        FrameDocumentScriptExecutionOwner::new(hooks)
            .run_ready_work(work)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "child document script execution owner failed");
                ChildDocumentScriptReadyRunOutcome::Applied(ChildDocumentScriptRunOutcome::new(
                    DocumentScriptExecutionOutcome::Progressed,
                    ChildDocumentScriptActivity::NoScriptOrEvent,
                ))
            })
    }
}

struct ScriptVmChildDocumentScriptExecutionHooks<'vm> {
    hooks: ChildDocumentScriptOwnerHooks<'vm>,
}

impl<'vm> ScriptVmChildDocumentScriptExecutionHooks<'vm> {
    fn new(vm: &'vm mut ScriptVm) -> Self {
        Self {
            hooks: ChildDocumentScriptOwnerHooks::new(vm),
        }
    }
}

impl ScriptVmChildDocumentScriptExecutionHooks<'_> {
    fn dropped_external_classic(
        &mut self,
        work: &PendingChildExternalClassicDocumentScript,
        reason: FrameDocumentExternalClassicPrepareSkipReason,
    ) -> FrameDocumentScriptExecutionStartReport<FrameDocumentScriptPrepareFollowup> {
        let _ = self.hooks.settle_child_async_classic_script_load_delay(
            work.child_handle,
            work.owner,
            work.load_delay,
        );
        FrameDocumentScriptExecutionStartReport::dropped(
            FrameDocumentScriptPrepareFollowup::ExternalClassic(
                FrameDocumentExternalClassicPrepareFollowup::skipped(reason),
            ),
        )
    }

    fn dropped_javascript_url(
        &mut self,
        work: &PendingChildJavascriptUrlDocumentScript,
        reason: FrameDocumentJavascriptUrlPrepareSkipReason,
    ) -> FrameDocumentScriptExecutionStartReport<FrameDocumentScriptPrepareFollowup> {
        self.hooks.drop_child_javascript_url_document_script(work);
        FrameDocumentScriptExecutionStartReport::dropped(
            FrameDocumentScriptPrepareFollowup::JavascriptUrl(
                FrameDocumentJavascriptUrlPrepareFollowup::skipped(reason),
            ),
        )
    }

    fn prepare_external_classic(
        &mut self,
        work: PendingChildExternalClassicDocumentScript,
    ) -> FrameDocumentScriptExecutionStartReport<FrameDocumentScriptPrepareFollowup> {
        match self.hooks.select_current_realm(
            work.child_handle,
            work.realm_id,
            work.script_handle,
            "child_external_classic_ready",
        ) {
            ChildDocumentScriptRealmSelection::Current(realm_id) => {
                let action = self
                    .hooks
                    .child_external_classic_script_execution_action_for_owner(&work, realm_id);
                let Some(action) = action else {
                    tracing::debug!(
                        child_handle = ?work.child_handle,
                        script_handle = ?work.script_handle,
                        owner = ?work.owner,
                        realm_id = ?realm_id,
                        "child external classic script task is stale"
                    );
                    return self.dropped_external_classic(
                        &work,
                        FrameDocumentExternalClassicPrepareSkipReason::ExecutionActionUnavailable,
                    );
                };
                FrameDocumentScriptExecutionStartReport::execute(
                    FrameDocumentScriptExecutionWork::external_classic(action),
                    FrameDocumentScriptPrepareFollowup::ExternalClassic(
                        FrameDocumentExternalClassicPrepareFollowup::prepared_execution_action(),
                    ),
                )
            }
            ChildDocumentScriptRealmSelection::RealmMaterializationFailed => self
                .dropped_external_classic(
                    &work,
                    FrameDocumentExternalClassicPrepareSkipReason::RealmMaterializationFailed,
                ),
            ChildDocumentScriptRealmSelection::MissingCurrentRealm => self
                .dropped_external_classic(
                    &work,
                    FrameDocumentExternalClassicPrepareSkipReason::MissingCurrentRealm,
                ),
            ChildDocumentScriptRealmSelection::StaleRealm { expected, current } => self
                .dropped_external_classic(
                    &work,
                    FrameDocumentExternalClassicPrepareSkipReason::StaleRealm { expected, current },
                ),
        }
    }

    fn prepare_javascript_url(
        &mut self,
        work: PendingChildJavascriptUrlDocumentScript,
    ) -> FrameDocumentScriptExecutionStartReport<FrameDocumentScriptPrepareFollowup> {
        match self.hooks.select_current_realm(
            work.child_handle,
            work.realm_id,
            work.child_handle,
            "child_javascript_url_ready",
        ) {
            ChildDocumentScriptRealmSelection::Current(realm_id) => {
                let action = self
                    .hooks
                    .child_javascript_url_script_execution_action_for_owner(&work, realm_id);
                let Some(action) = action else {
                    tracing::debug!(
                        child_handle = ?work.child_handle,
                        owner = ?work.owner,
                        realm_id = ?realm_id,
                        url = %work.url,
                        "child javascript URL script task is stale"
                    );
                    return self.dropped_javascript_url(
                        &work,
                        FrameDocumentJavascriptUrlPrepareSkipReason::ExecutionActionUnavailable,
                    );
                };
                FrameDocumentScriptExecutionStartReport::execute(
                    FrameDocumentScriptExecutionWork::javascript_url(action),
                    FrameDocumentScriptPrepareFollowup::JavascriptUrl(
                        FrameDocumentJavascriptUrlPrepareFollowup::prepared_execution_action(),
                    ),
                )
            }
            ChildDocumentScriptRealmSelection::RealmMaterializationFailed => self
                .dropped_javascript_url(
                    &work,
                    FrameDocumentJavascriptUrlPrepareSkipReason::RealmMaterializationFailed,
                ),
            ChildDocumentScriptRealmSelection::MissingCurrentRealm => self.dropped_javascript_url(
                &work,
                FrameDocumentJavascriptUrlPrepareSkipReason::MissingCurrentRealm,
            ),
            ChildDocumentScriptRealmSelection::StaleRealm { expected, current } => self
                .dropped_javascript_url(
                    &work,
                    FrameDocumentJavascriptUrlPrepareSkipReason::StaleRealm { expected, current },
                ),
        }
    }

    fn prepare_module_script(
        &mut self,
        work: crate::document_script_scheduler::FrameDocumentModuleScriptReadyWork,
    ) -> FrameDocumentScriptExecutionStartReport<FrameDocumentScriptPrepareFollowup> {
        FrameDocumentScriptExecutionStartReport::execute(
            FrameDocumentScriptExecutionWork::module_script(work),
            FrameDocumentScriptPrepareFollowup::ModuleScript(
                DocumentScriptExecutionOutcome::Progressed,
            ),
        )
    }

    fn execute_external_classic(
        &mut self,
        action: crate::frame_owner_model::FrameDocumentExternalClassicScriptExecutionAction,
    ) -> FrameDocumentScriptExecutionResult {
        let target = action.target();
        let mut attempted_script_job = false;
        let mut failed_script_job = false;
        let mut source_failed = false;
        match action.into_execution() {
            FrameDocumentExternalClassicScriptExecution::ScriptJob(job) => {
                attempted_script_job = true;
                if let Err(error) = self.hooks.execute_frame_script_job_selected_task_body(*job) {
                    tracing::warn!(
                        error = %error,
                        child_handle = ?target.child_handle(),
                        script_handle = ?target.script_handle(),
                        owner = ?target.owner(),
                        realm_id = ?target.realm_id(),
                        "child external classic script execution failed"
                    );
                    failed_script_job = true;
                }
            }
            FrameDocumentExternalClassicScriptExecution::SourceFailure { message } => {
                source_failed = true;
                tracing::debug!(
                    error = %message,
                    child_handle = ?target.child_handle(),
                    script_handle = ?target.script_handle(),
                    owner = ?target.owner(),
                    realm_id = ?target.realm_id(),
                    "child external classic source failed before execution"
                );
            }
        }
        FrameDocumentScriptExecutionResult::ExternalClassic(
            FrameDocumentExternalClassicExecutionResult::new(
                target,
                attempted_script_job,
                failed_script_job,
                source_failed,
            ),
        )
    }

    fn execute_javascript_url(
        &mut self,
        action: crate::frame_owner_model::FrameDocumentJavascriptUrlScriptExecutionAction,
    ) -> FrameDocumentScriptExecutionResult {
        let (target, job, url, preserve_window_event_state, dispatch_load_on_no_string_completion) =
            action.into_parts();
        let completion = match self
            .hooks
            .execute_frame_script_job_value_type_completion_selected_task_body(job)
        {
            Ok(FrameScriptCompletionValue::String(completion)) => {
                FrameDocumentJavascriptUrlCompletion::String(completion)
            }
            Ok(FrameScriptCompletionValue::NonString) => {
                FrameDocumentJavascriptUrlCompletion::NonString
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    child_handle = ?target.child_handle(),
                    owner = ?target.owner(),
                    realm_id = ?target.realm_id(),
                    url = %url,
                    "child javascript URL execution failed"
                );
                FrameDocumentJavascriptUrlCompletion::FailedScriptJob
            }
        };
        FrameDocumentScriptExecutionResult::JavascriptUrl(
            FrameDocumentJavascriptUrlExecutionResult::new(
                target,
                url,
                true,
                completion,
                preserve_window_event_state,
                dispatch_load_on_no_string_completion,
            ),
        )
    }

    fn apply_external_classic_post_execution(
        &mut self,
        action: crate::frame_owner_model::FrameDocumentExternalClassicPostExecutionAction,
    ) -> FrameDocumentExternalClassicExecutionFollowup {
        let target = action.target();
        let script_event_dispatched = self
            .hooks
            .dispatch_script_element_event_for_parts_selected_task_body(
                target.task_owner(),
                target.realm_id(),
                target.script_handle(),
                action.event_kind(),
            )
            .is_ok();
        let lifecycle_followup_queued = self.hooks.settle_child_async_classic_script_load_delay(
            target.child_handle(),
            target.task_owner(),
            target.load_delay(),
        );
        FrameDocumentExternalClassicExecutionFollowup::new(
            action.attempted_script_job(),
            action.failed_script_job(),
            action.source_failed(),
            script_event_dispatched,
            lifecycle_followup_queued,
        )
    }

    fn apply_javascript_url_post_execution(
        &mut self,
        action: crate::frame_owner_model::FrameDocumentJavascriptUrlPostExecutionAction,
    ) -> FrameDocumentJavascriptUrlExecutionFollowup {
        let application = self.hooks.apply_child_javascript_url_post_execution(action);
        if let Some(work) = application.initial_classic_ready_work {
            self.hooks.notify_parser_classic_next_owner_action(work);
        }
        FrameDocumentJavascriptUrlExecutionFollowup::new(
            application.attempted_script_job,
            application.failed_script_job,
            application.string_completion_committed,
            application.lifecycle_followup_queued,
        )
    }
}

impl DocumentScriptExecutionHooks for ScriptVmChildDocumentScriptExecutionHooks<'_> {
    type Ready = PendingChildDocumentScriptExecutionWork;
    type PreparedWork = FrameDocumentScriptExecutionWork;
    type PrepareFollowup = FrameDocumentScriptPrepareFollowup;
    type ExecutionResult = FrameDocumentScriptExecutionResult;
    type PostExecutionFollowup = FrameDocumentScriptExecutionFollowup;
    type Output = ChildDocumentScriptReadyRunOutcome;
    type ExecuteFuture<'owner>
        = Pin<Box<dyn Future<Output = anyhow::Result<FrameDocumentScriptExecutionResult>> + 'owner>>
    where
        Self: 'owner;

    fn prepare_execution(
        &mut self,
        work: PendingChildDocumentScriptExecutionWork,
    ) -> FrameDocumentScriptExecutionStartReport<FrameDocumentScriptPrepareFollowup> {
        match work {
            PendingChildDocumentScriptExecutionWork::ExternalClassic(work) => {
                self.prepare_external_classic(work)
            }
            PendingChildDocumentScriptExecutionWork::JavascriptUrl(work) => {
                self.prepare_javascript_url(work)
            }
            PendingChildDocumentScriptExecutionWork::ModuleScript(work) => {
                self.prepare_module_script(*work)
            }
        }
    }

    fn execute_work(&mut self, work: FrameDocumentScriptExecutionWork) -> Self::ExecuteFuture<'_> {
        Box::pin(async move {
            Ok(match work {
                FrameDocumentScriptExecutionWork::ExternalClassic(action) => {
                    self.execute_external_classic(action)
                }
                FrameDocumentScriptExecutionWork::JavascriptUrl(action) => {
                    self.execute_javascript_url(*action)
                }
                FrameDocumentScriptExecutionWork::ModuleScript(work) => {
                    FrameDocumentScriptExecutionResult::ModuleScript(
                        self.hooks.run_child_module_script_ready_work(*work).await,
                    )
                }
            })
        })
    }

    fn prepare_post_execution_followup(
        &mut self,
        execution_result: FrameDocumentScriptExecutionResult,
    ) -> anyhow::Result<FrameDocumentScriptExecutionFollowup> {
        Ok(match execution_result {
            FrameDocumentScriptExecutionResult::ExternalClassic(result) => {
                FrameDocumentScriptExecutionFollowup::ExternalClassic(
                    result.into_post_execution_action(),
                )
            }
            FrameDocumentScriptExecutionResult::JavascriptUrl(result) => {
                FrameDocumentScriptExecutionFollowup::JavascriptUrl(
                    result.into_post_execution_action(),
                )
            }
            FrameDocumentScriptExecutionResult::ModuleScript(outcome) => {
                FrameDocumentScriptExecutionFollowup::ModuleScript(outcome)
            }
        })
    }

    fn apply_post_execution_followup(
        &mut self,
        followup: FrameDocumentScriptExecutionFollowup,
    ) -> anyhow::Result<ChildDocumentScriptReadyRunOutcome> {
        let (made_progress, activity) = match followup {
            FrameDocumentScriptExecutionFollowup::ExternalClassic(action) => {
                let followup = self.apply_external_classic_post_execution(action);
                (
                    followup.made_progress(),
                    if followup.script_or_event_was_dispatched() {
                        ChildDocumentScriptActivity::ScriptOrEvent
                    } else {
                        ChildDocumentScriptActivity::NoScriptOrEvent
                    },
                )
            }
            FrameDocumentScriptExecutionFollowup::JavascriptUrl(action) => {
                let followup = self.apply_javascript_url_post_execution(action);
                (
                    followup.made_progress(),
                    if followup.script_was_attempted() {
                        ChildDocumentScriptActivity::ScriptOrEvent
                    } else {
                        ChildDocumentScriptActivity::NoScriptOrEvent
                    },
                )
            }
            FrameDocumentScriptExecutionFollowup::ModuleScript(outcome) => {
                let activity = match outcome.activity() {
                    FrameModuleScriptTaskActivity::NoScriptOrEvent => {
                        ChildDocumentScriptActivity::NoScriptOrEvent
                    }
                    FrameModuleScriptTaskActivity::ScriptOrEvent => {
                        ChildDocumentScriptActivity::ScriptOrEvent
                    }
                };
                return Ok(ChildDocumentScriptReadyRunOutcome::Applied(
                    ChildDocumentScriptRunOutcome::new(outcome.into_output(), activity),
                ));
            }
        };
        let execution = if made_progress {
            DocumentScriptExecutionOutcome::Progressed
        } else {
            DocumentScriptExecutionOutcome::NoProgress
        };
        Ok(ChildDocumentScriptReadyRunOutcome::Applied(
            ChildDocumentScriptRunOutcome::new(execution, activity),
        ))
    }

    fn outcome_for_dropped_ready(
        &mut self,
        prepare_followup: FrameDocumentScriptPrepareFollowup,
    ) -> anyhow::Result<ChildDocumentScriptReadyRunOutcome> {
        let execution = if prepare_followup.made_progress() {
            DocumentScriptExecutionOutcome::Progressed
        } else {
            DocumentScriptExecutionOutcome::NoProgress
        };
        Ok(ChildDocumentScriptReadyRunOutcome::Applied(
            ChildDocumentScriptRunOutcome::new(
                execution,
                ChildDocumentScriptActivity::NoScriptOrEvent,
            ),
        ))
    }
}
