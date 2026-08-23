use std::{
    future::{Future, Ready, ready},
    pin::Pin,
};

use super::page_vm::ParseTimeLiveExecution;
use super::parser_blocking_owner::{
    MainParserBlockingBeginExecutionOwner, MainParserBlockingLifecycleOwner,
};
use super::parser_blocking_pending::PendingParsingBlockingClassicScriptRunner;
use super::parser_blocking_task::{
    MainParserBlockingClassicScriptCompletionAction, MainParserBlockingClassicScriptExecutionEntry,
    MainParserBlockingClassicScriptReadyAction, MainParserBlockingClassicScriptSourceFailureAction,
    MainParserBlockingNextAction,
};
use super::*;
use crate::document_runtime::ParserConnectedScriptBridge;
use crate::document_script_scheduler::{
    DocumentScriptExecutionOutcome, ParserClassicDocumentScriptCompletionPlan,
    ParserClassicDocumentScriptContinuation, ParserClassicDocumentScriptExecutionHooks,
    ParserClassicDocumentScriptExecutionOwner, ParserClassicDocumentScriptExecutionResult,
    ParserClassicDocumentScriptExecutionStartReport, ParserClassicDocumentScriptReadyOwner,
    ParserClassicDocumentScriptSourceFailureReport,
};
use crate::parser_script::action::{
    ParserClassicScriptExecutionStart, ParserClassicScriptScheduling,
};
use crate::script_vm::{
    ParserOwnedClassicScriptCompletion, ParserOwnedClassicScriptCompletionApplication,
    ParserOwnedClassicScriptExecutionContext,
};

pub(super) struct MainParserBlockingDocumentScriptOwner<'page, 'runner> {
    page_vm: &'page mut PageVm,
    pending_runner: &'runner mut PendingParsingBlockingClassicScriptRunner,
    parser_bridge: Option<ParserConnectedScriptBridge>,
    log_message: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MainParserBlockingPostExecution {
    navigation_triggered: bool,
    owner_replaced: bool,
}

impl MainParserBlockingPostExecution {
    fn stops_parser(self) -> bool {
        self.navigation_triggered || self.owner_replaced
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MainParserBlockingPrepareFollowup;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MainParserBlockingSourceFailureReportFollowup;

impl<'page, 'runner> MainParserBlockingDocumentScriptOwner<'page, 'runner> {
    pub(super) fn new(
        page_vm: &'page mut PageVm,
        pending_runner: &'runner mut PendingParsingBlockingClassicScriptRunner,
        parser_bridge: Option<ParserConnectedScriptBridge>,
        log_message: &'static str,
    ) -> Self {
        Self {
            page_vm,
            pending_runner,
            parser_bridge,
            log_message,
        }
    }

    fn claim_next_classic_script_action(
        &mut self,
        action: MainParserBlockingNextAction,
    ) -> Option<MainParserBlockingNextAction> {
        self.page_vm
            .claim_main_document_ready_action(action, "parser-blocking classic")
    }

    pub(super) async fn run_next_classic_script_action(
        &mut self,
        action: MainParserBlockingNextAction,
    ) -> Result<DocumentScriptExecutionOutcome> {
        let Some(action) = self.claim_next_classic_script_action(action) else {
            return Ok(DocumentScriptExecutionOutcome::NoProgress);
        };
        action.run_with(self).await
    }

    async fn execute_ready_classic_script(
        &mut self,
        ready_script: MainParserBlockingClassicScriptReadyAction,
    ) -> Result<DocumentScriptExecutionOutcome> {
        ParserClassicDocumentScriptExecutionOwner::new(self)
            .run_ready_work(ready_script)
            .await
    }
}

impl ParserClassicDocumentScriptExecutionHooks
    for &mut MainParserBlockingDocumentScriptOwner<'_, '_>
{
    type Ready = MainParserBlockingClassicScriptReadyAction;
    type SourceFailure = MainParserBlockingClassicScriptSourceFailureAction;
    type ExecutionAction = MainParserBlockingClassicScriptExecutionEntry;
    type Completion = MainParserBlockingClassicScriptCompletionAction;
    type CompletionAction = MainParserBlockingClassicScriptCompletionAction;
    type CompletionContinuationAction =
        crate::document_script_scheduler::MainDocumentClassicScriptTarget;
    type PrepareFollowup = MainParserBlockingPrepareFollowup;
    type CompletionEffectsFollowup = ParserOwnedClassicScriptCompletionApplication;
    type CompletionFollowup = ParserOwnedClassicScriptCompletionApplication;
    type SourceFailureReportFollowup = MainParserBlockingSourceFailureReportFollowup;
    type PostExecution = MainParserBlockingPostExecution;
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
                            MainParserBlockingPostExecution,
                        >,
                    >,
                > + 'owner,
        >,
    >
    where
        Self: 'owner;

    fn prepare_execution(
        &mut self,
        ready_script: MainParserBlockingClassicScriptReadyAction,
    ) -> ParserClassicDocumentScriptExecutionStartReport<
        Self::ExecutionAction,
        Self::Completion,
        Self::PrepareFollowup,
    > {
        let owner = &mut **self;
        let script_handle = ready_script.script_handle();
        let completion_target = *ready_script.target();
        let parser_bridge = owner.parser_bridge.clone();
        let mut begin_owner = MainParserBlockingBeginExecutionOwner {
            parser_bridge,
            completion_target,
        };
        let Some(execution_entry) = owner
            .pending_runner
            .take_current_parser_blocking_begin_execution_action_with_owner(
                script_handle,
                &mut begin_owner,
            )
        else {
            return ParserClassicDocumentScriptExecutionStartReport::new(
                ParserClassicScriptExecutionStart::Dropped,
                MainParserBlockingPrepareFollowup,
            );
        };
        ParserClassicDocumentScriptExecutionStartReport::new(
            ParserClassicScriptExecutionStart::Execute(Box::new(execution_entry)),
            MainParserBlockingPrepareFollowup,
        )
    }

    fn execute_action(&mut self, action: Self::ExecutionAction) -> Self::ExecuteFuture<'_> {
        let owner = &mut **self;
        Box::pin(async move {
            debug!(
                phase = "parse-time classic script",
                url = %action.script_url(),
                source_kind = ?action.source_kind(),
                "{}", owner.log_message
            );
            let (target, execution, executable_script) = action.into_parts();
            let script_handle = execution.metadata.script_handle();
            let completion_target = target.completion_target();
            let live_execution = ParseTimeLiveExecution::ParserOwnedClassicScript {
                execution_context: ParserOwnedClassicScriptExecutionContext::ParserBlocking {
                    parser_bridge: target.parser_bridge(),
                },
                script: Box::new(executable_script.into_prepared_script()),
            };
            let execution_outcome = owner
                .page_vm
                .execute_parse_time_on_existing_live_document_on_named_owner_local_task(
                    live_execution,
                )
                .await?;
            let navigation_triggered = execution_outcome.navigation_triggered();
            let completion_effects = execution_outcome
                .into_parser_owned_classic_script_completion()
                .expect("parser-connected live execution must return completion effects");
            let current_owner = owner.page_vm.vm().current_main_document_task_owner();
            let owner_replaced = current_owner != Some(completion_target.owner());
            let mut finish_owner = MainParserBlockingLifecycleOwner::new(
                completion_target,
                current_owner,
                completion_effects,
            );
            let completion = owner
                .pending_runner
                .take_current_parser_blocking_finished_execution_action_with_owner(
                    script_handle,
                    &mut finish_owner,
                );
            debug_assert!(
                completion.is_some() || owner_replaced,
                "main parser-blocking script execution must finish the current shared runner item unless its document owner was replaced"
            );
            Ok(ParserClassicDocumentScriptExecutionResult::new(
                completion,
                MainParserBlockingPostExecution {
                    navigation_triggered,
                    owner_replaced,
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
        let (target, failure, _event) = failure.into_parts();
        let script_handle = failure.script_handle();
        let script_url = failure.script_url().clone();
        let error = failure.error().to_owned();
        let event = owner
            .page_vm
            .vm()
            .document_runtime
            .plan_parser_owned_script_event_task(
                crate::host::ScriptEventKind::Error,
                script_handle,
            );
        if let Some((script, _, source_network_result)) = failure.into_execution_failure_parts()
            && let Some(network_result) = source_network_result.as_deref()
        {
            owner
                .page_vm
                .vm_mut()
                .record_script_subresource_network_result(
                    script.initiator_url,
                    script.url,
                    network_result,
                );
        }
        tracing::debug!(
            expected_owner = ?target.owner(),
            ?script_handle,
            script_url = %script_url,
            %error,
            event_planned = event.is_some(),
            "main parser-blocking classic source failure produced completion effects"
        );
        let completion = MainParserBlockingClassicScriptCompletionAction::new(
            target,
            ParserOwnedClassicScriptCompletion::parser_blocking_source_failure(
                owner.parser_bridge.clone(),
                event,
            ),
        );
        Ok(ParserClassicDocumentScriptSourceFailureReport::new(
            Some(completion),
            MainParserBlockingSourceFailureReportFollowup,
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
        let target = completion.target();
        Ok(ParserClassicDocumentScriptCompletionPlan::new(
            completion,
            ParserClassicScriptScheduling::ParserBlocking,
            target,
        ))
    }

    fn apply_completion_action(
        &mut self,
        action: Self::CompletionAction,
    ) -> Self::CompletionEffectsFuture<'_> {
        let owner = &mut **self;
        Box::pin(async move {
            let expected_owner = action.target().owner();
            let completion = action.into_completion();
            let application = owner
                .page_vm
                .apply_parser_owned_classic_script_completion_on_named_owner_local_task(
                    expected_owner,
                    completion,
                )
                .await?;
            tracing::debug!(
                ?expected_owner,
                completion_applied = application.completion_was_applied(),
                script_event_dispatched = application.script_event_was_dispatched(),
                evaluation = ?application.evaluation(),
                stale_owner = application.owner_was_stale(),
                "main parser-blocking classic completion effects applied"
            );
            Ok(application)
        })
    }

    fn apply_completion_continuation(
        &mut self,
        continuation: ParserClassicDocumentScriptContinuation<Self::CompletionContinuationAction>,
        mut effects: Self::CompletionEffectsFollowup,
    ) -> Self::CompletionFuture<'_> {
        let ParserClassicDocumentScriptContinuation::ResumeParser(target) = continuation else {
            return ready(Err(anyhow::anyhow!(
                "main parser-blocking completion cannot release a deferred continuation"
            )));
        };
        let current_owner = self.page_vm.vm().current_main_document_task_owner();
        if current_owner != Some(target.owner()) {
            tracing::debug!(
                expected_owner = ?target.owner(),
                ?current_owner,
                "canceling main parser continuation after completion effects replaced its owner"
            );
            effects.note_stale_owner();
        }
        ready(Ok(effects))
    }

    fn outcome_after_executed_completion(
        &mut self,
        post_execution: Self::PostExecution,
        completion_followup: Self::CompletionFollowup,
    ) -> Result<DocumentScriptExecutionOutcome> {
        let owner = &mut **self;
        if post_execution.stops_parser() || completion_followup.owner_was_stale() {
            return Ok(DocumentScriptExecutionOutcome::TriggeredNavigation);
        }
        debug_assert!(
            completion_followup.made_progress(),
            "current main parser-blocking completion should apply an event, finish work, or stale-owner drop"
        );
        if owner
            .page_vm
            .vm()
            .document_runtime
            .has_pending_document_write_external_script_load()
        {
            return Ok(DocumentScriptExecutionOutcome::BlockedOnDocumentWriteExternalLoad);
        }
        Ok(DocumentScriptExecutionOutcome::Progressed)
    }

    fn outcome_after_completion(
        &mut self,
        completion_followup: Self::CompletionFollowup,
    ) -> Result<DocumentScriptExecutionOutcome> {
        if completion_followup.owner_was_stale() {
            return Ok(DocumentScriptExecutionOutcome::TriggeredNavigation);
        }
        if self
            .page_vm
            .vm()
            .document_runtime
            .has_pending_document_write_external_script_load()
        {
            return Ok(DocumentScriptExecutionOutcome::BlockedOnDocumentWriteExternalLoad);
        }
        Ok(if completion_followup.made_progress() {
            DocumentScriptExecutionOutcome::Progressed
        } else {
            DocumentScriptExecutionOutcome::NoProgress
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
        if post_execution.stops_parser() {
            Ok(DocumentScriptExecutionOutcome::TriggeredNavigation)
        } else {
            tracing::warn!(
                "main parser-blocking execution lost its completion without replacing the document owner"
            );
            Ok(DocumentScriptExecutionOutcome::NoProgress)
        }
    }

    fn outcome_for_source_failure_without_completion(
        &mut self,
        _report_followup: Self::SourceFailureReportFollowup,
    ) -> Result<DocumentScriptExecutionOutcome> {
        Ok(DocumentScriptExecutionOutcome::Progressed)
    }
}

impl
    ParserClassicDocumentScriptReadyOwner<
        MainParserBlockingClassicScriptReadyAction,
        MainParserBlockingClassicScriptSourceFailureAction,
    > for MainParserBlockingDocumentScriptOwner<'_, '_>
{
    type Output<'dispatch>
        = Pin<Box<dyn Future<Output = Result<DocumentScriptExecutionOutcome>> + 'dispatch>>
    where
        Self: 'dispatch;

    fn run_parser_classic_source_failed<'dispatch>(
        &'dispatch mut self,
        failure: MainParserBlockingClassicScriptSourceFailureAction,
    ) -> Self::Output<'dispatch> {
        Box::pin(async move {
            ParserClassicDocumentScriptExecutionOwner::new(self)
                .run_source_failure(failure)
                .await
        })
    }

    fn run_parser_classic_ready<'dispatch>(
        &'dispatch mut self,
        ready_script: MainParserBlockingClassicScriptReadyAction,
    ) -> Self::Output<'dispatch> {
        Box::pin(async move { self.execute_ready_classic_script(ready_script).await })
    }
}
