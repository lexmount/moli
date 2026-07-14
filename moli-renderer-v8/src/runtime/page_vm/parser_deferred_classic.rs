use std::{
    future::{Future, Ready, ready},
    pin::Pin,
};

use anyhow::Result;

use crate::{
    document_runtime::DomHandle,
    document_script_scheduler::{
        DocumentScriptExecutionOutcome, ParserClassicDocumentScriptCompletionPlan,
        ParserClassicDocumentScriptContinuation, ParserClassicDocumentScriptExecutionHooks,
        ParserClassicDocumentScriptExecutionOwner, ParserClassicDocumentScriptExecutionResult,
        ParserClassicDocumentScriptExecutionStartReport,
        ParserClassicDocumentScriptSourceFailureReport, ParserDeferredClassicReady,
    },
    frame_owner_model::{DocumentLoadDelayTokenId, FrameDocumentTaskOwner},
    host::ScriptEventKind,
    network::ResourceRequestClient,
    parser_script::action::{ParserClassicScriptExecutionStart, ParserClassicScriptScheduling},
    planning::PreparedScript,
    script_vm::{
        ParserOwnedClassicScriptCompletion, ParserOwnedClassicScriptCompletionApplication,
        PreparedScriptBodyActivity, ScriptTerminalBodyActivity,
    },
    types::{ScriptRun, SharedNavigationResponseResult},
};

use super::{
    PageVm,
    parser_task_completion::{
        MainParserContinuationBodyActivity, MainParserContinuationTaskEffect,
    },
};

pub(super) struct MainParserDeferredClassicExecution {
    run: Option<ScriptRun>,
    task_effect: MainParserContinuationTaskEffect,
}

impl MainParserDeferredClassicExecution {
    pub(super) fn into_parts(self) -> (Option<ScriptRun>, MainParserContinuationTaskEffect) {
        (self.run, self.task_effect)
    }
}

/// Body result for one parser-owned classic script before its owner-specific
/// terminal and selected-task completion are applied.
pub(super) struct MainParserDeferredClassicBodyExecution {
    run: ScriptRun,
    navigation_triggered: bool,
    completion: ParserOwnedClassicScriptCompletion,
    body_activity: PreparedScriptBodyActivity,
}

impl MainParserDeferredClassicBodyExecution {
    pub(super) fn new(
        run: ScriptRun,
        navigation_triggered: bool,
        completion: ParserOwnedClassicScriptCompletion,
        body_activity: PreparedScriptBodyActivity,
    ) -> Self {
        Self {
            run,
            navigation_triggered,
            completion,
            body_activity,
        }
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        ScriptRun,
        bool,
        ParserOwnedClassicScriptCompletion,
        PreparedScriptBodyActivity,
    ) {
        (
            self.run,
            self.navigation_triggered,
            self.completion,
            self.body_activity,
        )
    }
}

pub(super) struct MainParserDeferredClassicReady {
    owner: FrameDocumentTaskOwner,
    script: Box<PreparedScript>,
    source_network_result: Option<SharedNavigationResponseResult>,
}

pub(super) struct MainParserDeferredClassicSourceFailure {
    owner: FrameDocumentTaskOwner,
    script: Box<PreparedScript>,
    error: String,
    source_network_result: Option<SharedNavigationResponseResult>,
}

pub(super) struct MainParserDeferredClassicCompletion {
    owner: FrameDocumentTaskOwner,
    completion: ParserOwnedClassicScriptCompletion,
    run: Option<ScriptRun>,
    prepared_activity: PreparedScriptBodyActivity,
}

pub(super) struct MainParserDeferredClassicPostExecution {
    owner: FrameDocumentTaskOwner,
    navigation_triggered: bool,
    owner_replaced: bool,
    run: ScriptRun,
    prepared_activity: PreparedScriptBodyActivity,
}

pub(super) struct MainParserDeferredClassicCompletionFollowup {
    application: ParserOwnedClassicScriptCompletionApplication,
    run: Option<ScriptRun>,
    owner: FrameDocumentTaskOwner,
    prepared_activity: PreparedScriptBodyActivity,
    terminal_activity: ScriptTerminalBodyActivity,
}

pub(super) struct MainParserDeferredClassicSourceFailureFollowup {
    owner_replaced: bool,
}

pub(super) struct MainParserDeferredClassicDocumentScriptOwner<'page, 'loader> {
    page_vm: &'page mut PageVm,
    loader: &'loader ResourceRequestClient,
    completed_run: Option<ScriptRun>,
    task_effect: MainParserContinuationTaskEffect,
}

impl<'page, 'loader> MainParserDeferredClassicDocumentScriptOwner<'page, 'loader> {
    pub(super) fn new(page_vm: &'page mut PageVm, loader: &'loader ResourceRequestClient) -> Self {
        Self {
            page_vm,
            loader,
            completed_run: None,
            task_effect: MainParserContinuationTaskEffect::NotApplied,
        }
    }

    pub(super) async fn run_work(
        &mut self,
        owner: FrameDocumentTaskOwner,
        work: ParserDeferredClassicReady,
    ) -> Result<MainParserDeferredClassicExecution> {
        let (outcome, load_delay_token): (
            Result<DocumentScriptExecutionOutcome>,
            DocumentLoadDelayTokenId,
        ) = match work {
            ParserDeferredClassicReady::Execute {
                script,
                source_network_result,
                load_delay_token,
            } => (
                ParserClassicDocumentScriptExecutionOwner::new(&mut *self)
                    .run_ready_work(MainParserDeferredClassicReady {
                        owner,
                        script,
                        source_network_result,
                    })
                    .await,
                load_delay_token,
            ),
            ParserDeferredClassicReady::SourceFailure {
                script,
                error,
                source_network_result,
                load_delay_token,
            } => (
                ParserClassicDocumentScriptExecutionOwner::new(&mut *self)
                    .run_source_failure(MainParserDeferredClassicSourceFailure {
                        owner,
                        script,
                        error,
                        source_network_result,
                    })
                    .await,
                load_delay_token,
            ),
        };
        let released = self
            .page_vm
            .vm_mut()
            .release_main_parser_deferred_script_load_delay(owner, load_delay_token);
        let outcome = outcome?;
        tracing::debug!(
            ?owner,
            ?load_delay_token,
            lifecycle_delay_released = released,
            ?outcome,
            produced_run = self.completed_run.is_some(),
            "completed main parser classic-defer owner turn"
        );
        Ok(MainParserDeferredClassicExecution {
            run: self.completed_run.take(),
            task_effect: self.task_effect,
        })
    }

    fn owner_is_current(&self, owner: FrameDocumentTaskOwner) -> bool {
        self.page_vm.vm().current_main_document_task_owner() == Some(owner)
    }

    fn record_network_result(
        &mut self,
        script: &PreparedScript,
        source_network_result: Option<&SharedNavigationResponseResult>,
    ) {
        if let Some(network_result) = source_network_result.map(AsRef::as_ref) {
            self.page_vm
                .vm_mut()
                .record_script_subresource_network_result(
                    script.initiator_url.clone(),
                    script.url.clone(),
                    network_result,
                );
        }
    }

    fn note_applied_task(
        &mut self,
        owner: FrameDocumentTaskOwner,
        prepared_activity: PreparedScriptBodyActivity,
        terminal_activity: ScriptTerminalBodyActivity,
    ) {
        let activity = MainParserContinuationBodyActivity::from_prepared_script(prepared_activity)
            .note_terminal(terminal_activity);
        self.task_effect = MainParserContinuationTaskEffect::applied(owner, activity);
    }
}

impl ParserClassicDocumentScriptExecutionHooks
    for &mut MainParserDeferredClassicDocumentScriptOwner<'_, '_>
{
    type Ready = MainParserDeferredClassicReady;
    type SourceFailure = MainParserDeferredClassicSourceFailure;
    type ExecutionAction = MainParserDeferredClassicReady;
    type Completion = MainParserDeferredClassicCompletion;
    type CompletionAction = MainParserDeferredClassicCompletion;
    type CompletionContinuationAction = FrameDocumentTaskOwner;
    type PrepareFollowup = ();
    type CompletionEffectsFollowup = MainParserDeferredClassicCompletionFollowup;
    type CompletionFollowup = MainParserDeferredClassicCompletionFollowup;
    type SourceFailureReportFollowup = MainParserDeferredClassicSourceFailureFollowup;
    type PostExecution = MainParserDeferredClassicPostExecution;
    type Output = DocumentScriptExecutionOutcome;
    type CompletionEffectsFuture<'owner>
        = Pin<Box<dyn Future<Output = Result<Self::CompletionFollowup>> + 'owner>>
    where
        Self: 'owner;
    type CompletionFuture<'owner>
        = Ready<Result<Self::CompletionFollowup>>
    where
        Self: 'owner;
    type ExecuteFuture<'owner>
        = Pin<
        Box<
            dyn Future<
                    Output = Result<
                        ParserClassicDocumentScriptExecutionResult<
                            Self::Completion,
                            Self::PostExecution,
                        >,
                    >,
                > + 'owner,
        >,
    >
    where
        Self: 'owner;

    fn prepare_execution(
        &mut self,
        ready: Self::Ready,
    ) -> ParserClassicDocumentScriptExecutionStartReport<
        Self::ExecutionAction,
        Self::Completion,
        Self::PrepareFollowup,
    > {
        let owner = &mut **self;
        if !owner.owner_is_current(ready.owner) {
            tracing::debug!(
                expected_owner = ?ready.owner,
                current_owner = ?owner.page_vm.vm().current_main_document_task_owner(),
                script_url = %ready.script.url,
                "dropping stale main parser classic-defer execution start"
            );
            return ParserClassicDocumentScriptExecutionStartReport::new(
                ParserClassicScriptExecutionStart::Dropped,
                (),
            );
        }
        ParserClassicDocumentScriptExecutionStartReport::new(
            ParserClassicScriptExecutionStart::Execute(Box::new(ready)),
            (),
        )
    }

    fn execute_action(&mut self, action: Self::ExecutionAction) -> Self::ExecuteFuture<'_> {
        let owner = &mut **self;
        Box::pin(async move {
            owner.record_network_result(&action.script, action.source_network_result.as_ref());
            let expected_owner = action.owner;
            owner
                .page_vm
                .vm_mut()
                .document_runtime
                .begin_classic_defer_timer_schedule_range();
            let execution = owner
                .page_vm
                .execute_main_parser_deferred_classic_script_body_on_current_lane(
                    owner.loader,
                    action.script,
                )
                .await;
            owner
                .page_vm
                .vm_mut()
                .document_runtime
                .finish_classic_defer_timer_schedule_range();
            let (run, navigation_triggered, completion, prepared_activity) = execution.into_parts();
            let owner_replaced = !owner.owner_is_current(expected_owner);
            Ok(ParserClassicDocumentScriptExecutionResult::new(
                Some(MainParserDeferredClassicCompletion {
                    owner: expected_owner,
                    completion,
                    run: None,
                    prepared_activity,
                }),
                MainParserDeferredClassicPostExecution {
                    owner: expected_owner,
                    navigation_triggered,
                    owner_replaced,
                    run,
                    prepared_activity,
                },
            ))
        })
    }

    fn report_source_failure(
        &mut self,
        failure: Self::SourceFailure,
    ) -> Result<
        ParserClassicDocumentScriptSourceFailureReport<
            Self::Completion,
            Self::SourceFailureReportFollowup,
        >,
    > {
        let owner = &mut **self;
        if !owner.owner_is_current(failure.owner) {
            tracing::debug!(
                expected_owner = ?failure.owner,
                current_owner = ?owner.page_vm.vm().current_main_document_task_owner(),
                script_url = %failure.script.url,
                "dropping stale main parser classic-defer source failure"
            );
            return Ok(ParserClassicDocumentScriptSourceFailureReport::new(
                None,
                MainParserDeferredClassicSourceFailureFollowup {
                    owner_replaced: true,
                },
            ));
        }
        owner.record_network_result(&failure.script, failure.source_network_result.as_ref());
        let _ = owner
            .page_vm
            .vm_mut()
            .document_runtime
            .mark_script_already_started_by_node_id(failure.script.node_id);
        let script_handle = DomHandle::new(failure.script.node_id.index());
        let event = owner
            .page_vm
            .vm()
            .document_runtime
            .plan_parser_owned_script_event_task(ScriptEventKind::Error, script_handle);
        tracing::debug!(
            expected_owner = ?failure.owner,
            script_node_id = ?failure.script.node_id,
            script_url = %failure.script.url,
            error = failure.error,
            event_planned = event.is_some(),
            "main parser classic-defer source failure produced completion effects"
        );
        let run = ScriptRun::failed(
            failure.script.node_id,
            failure.script.kind,
            failure.script.mode,
            failure.script.source_kind,
            failure.script.url.clone(),
            failure.error,
        );
        Ok(ParserClassicDocumentScriptSourceFailureReport::new(
            Some(MainParserDeferredClassicCompletion {
                owner: failure.owner,
                completion: ParserOwnedClassicScriptCompletion::deferred_source_failure(event),
                run: Some(run),
                prepared_activity: PreparedScriptBodyActivity::NotEntered,
            }),
            MainParserDeferredClassicSourceFailureFollowup {
                owner_replaced: false,
            },
        ))
    }

    fn prepare_completion_plan(
        &mut self,
        completion: Self::Completion,
    ) -> Result<
        ParserClassicDocumentScriptCompletionPlan<
            Self::CompletionAction,
            Self::CompletionContinuationAction,
        >,
    > {
        let owner = completion.owner;
        Ok(ParserClassicDocumentScriptCompletionPlan::new(
            completion,
            ParserClassicScriptScheduling::Deferred,
            owner,
        ))
    }

    fn apply_completion_action(
        &mut self,
        action: Self::CompletionAction,
    ) -> Self::CompletionEffectsFuture<'_> {
        let owner = &mut **self;
        Box::pin(async move {
            let owner_token = action.owner;
            let prepared_activity = action.prepared_activity;
            let body = owner
                .page_vm
                .vm_mut()
                .apply_main_parser_deferred_classic_completion_body(action.owner, action.completion)
                .map_err(anyhow::Error::msg)?;
            let (application, terminal_activity) = body.into_parts();
            Ok(MainParserDeferredClassicCompletionFollowup {
                application,
                run: action.run,
                owner: owner_token,
                prepared_activity,
                terminal_activity,
            })
        })
    }

    fn apply_completion_continuation(
        &mut self,
        continuation: ParserClassicDocumentScriptContinuation<Self::CompletionContinuationAction>,
        mut effects: Self::CompletionEffectsFollowup,
    ) -> Self::CompletionFuture<'_> {
        let ParserClassicDocumentScriptContinuation::ReleaseDeferred(owner) = continuation else {
            return ready(Err(anyhow::anyhow!(
                "main parser-deferred completion cannot resume a parser-blocking continuation"
            )));
        };
        if !self.owner_is_current(owner) {
            tracing::debug!(
                expected_owner = ?owner,
                current_owner = ?self.page_vm.vm().current_main_document_task_owner(),
                "canceling main parser-deferred continuation after completion effects replaced its owner"
            );
            effects.application.note_stale_owner();
        }
        ready(Ok(effects))
    }

    fn outcome_after_executed_completion(
        &mut self,
        post_execution: Self::PostExecution,
        completion_followup: Self::CompletionFollowup,
    ) -> Result<DocumentScriptExecutionOutcome> {
        let owner = &mut **self;
        anyhow::ensure!(
            completion_followup.run.is_none(),
            "executed classic-defer run must remain owned by post-execution"
        );
        owner.note_applied_task(
            completion_followup.owner,
            post_execution.prepared_activity,
            completion_followup.terminal_activity,
        );
        owner.completed_run = Some(post_execution.run);
        if post_execution.navigation_triggered
            || post_execution.owner_replaced
            || completion_followup.application.owner_was_stale()
        {
            return Ok(DocumentScriptExecutionOutcome::TriggeredNavigation);
        }
        anyhow::ensure!(
            completion_followup.application.made_progress(),
            "current main parser classic-defer completion must apply or stale-drop"
        );
        Ok(DocumentScriptExecutionOutcome::Progressed)
    }

    fn outcome_after_completion(
        &mut self,
        completion_followup: Self::CompletionFollowup,
    ) -> Result<DocumentScriptExecutionOutcome> {
        let owner = &mut **self;
        owner.note_applied_task(
            completion_followup.owner,
            completion_followup.prepared_activity,
            completion_followup.terminal_activity,
        );
        owner.completed_run = completion_followup.run;
        Ok(if completion_followup.application.owner_was_stale() {
            DocumentScriptExecutionOutcome::TriggeredNavigation
        } else {
            DocumentScriptExecutionOutcome::Progressed
        })
    }

    fn outcome_for_dropped_ready(
        &mut self,
        _prepare_followup: Self::PrepareFollowup,
    ) -> Result<DocumentScriptExecutionOutcome> {
        Ok(DocumentScriptExecutionOutcome::NoProgress)
    }

    fn outcome_for_missing_execution_completion(
        &mut self,
        post_execution: Self::PostExecution,
    ) -> Result<DocumentScriptExecutionOutcome> {
        let owner = &mut **self;
        owner.note_applied_task(
            post_execution.owner,
            post_execution.prepared_activity,
            ScriptTerminalBodyActivity::NoEventDispatch,
        );
        owner.completed_run = Some(post_execution.run);
        if post_execution.navigation_triggered || post_execution.owner_replaced {
            Ok(DocumentScriptExecutionOutcome::TriggeredNavigation)
        } else {
            tracing::warn!("main parser classic-defer execution lost its completion effects");
            Ok(DocumentScriptExecutionOutcome::NoProgress)
        }
    }

    fn outcome_for_source_failure_without_completion(
        &mut self,
        report_followup: Self::SourceFailureReportFollowup,
    ) -> Result<DocumentScriptExecutionOutcome> {
        Ok(if report_followup.owner_replaced {
            DocumentScriptExecutionOutcome::TriggeredNavigation
        } else {
            DocumentScriptExecutionOutcome::NoProgress
        })
    }
}
