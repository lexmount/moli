use std::{convert::Infallible, future::Future, pin::Pin};

use anyhow::Result;

use crate::document_script_scheduler::{
    DocumentModuleScriptReadyWork, DocumentScriptExecutionHooks, DocumentScriptExecutionOutcome,
    DocumentScriptExecutionRunner, DocumentScriptExecutionStartReport,
    DocumentScriptReadyActionRoute, DocumentScriptReadyWorkOwner, MainDocumentReadyActionRoute,
    ParserModuleGraphTerminalWork,
};
use crate::frame_owner_model::{DocumentLoadDelayTokenId, FrameDocumentTaskOwner};
use crate::module_script_continuation::{
    MainDocumentModuleGraphReadyTarget, MainDocumentModuleGraphReadyWork, MainParserDocumentOwner,
    MainParserOwnedDocumentScriptWork, MainParserOwnedModuleScriptEvaluation,
    MainParserOwnedModuleScriptFailure, ModuleScriptContinuation,
};
use crate::parser_script::action::ParserClassicScriptNextOwnerAction;
use crate::script_vm::{
    ParserModuleTerminalDisposition, ParserOwnedModuleSuccessTerminal, PreparedScriptBodyActivity,
};

use super::super::main_document_ready_gate::{
    claim_main_document_ready_dispatch, report_main_document_ready_owner_mismatch,
};
use super::{
    PageOwnedScriptFailureClassification, PageVm,
    parser_task_completion::MainParserContinuationTaskEffect,
};

pub(super) struct MainParserOwnedDocumentScriptOwner<'page> {
    page_vm: &'page mut PageVm,
}

type MainParserOwnedModuleReadyInput = DocumentModuleScriptReadyWork<
    MainDocumentModuleGraphReadyWork,
    MainParserOwnedModuleScriptFailure,
    MainParserOwnedModuleScriptEvaluation,
>;

struct MainParserOwnedModuleExecutionHooks<'page> {
    page_vm: &'page mut PageVm,
    terminal_disposition: ParserModuleTerminalDisposition,
}

#[derive(Debug)]
pub(super) enum MainParserModuleExecution {
    Settled {
        outcome: DocumentScriptExecutionOutcome,
        task_effect: MainParserContinuationTaskEffect,
    },
    TerminalForSelectedTask {
        outcome: DocumentScriptExecutionOutcome,
        terminal: ParserOwnedModuleSuccessTerminal,
    },
}

impl MainParserModuleExecution {
    pub(super) fn settled(outcome: DocumentScriptExecutionOutcome) -> Self {
        Self::Settled {
            outcome,
            task_effect: MainParserContinuationTaskEffect::NotApplied,
        }
    }

    pub(super) fn settled_with_task_effect(
        outcome: DocumentScriptExecutionOutcome,
        task_effect: MainParserContinuationTaskEffect,
    ) -> Self {
        Self::Settled {
            outcome,
            task_effect,
        }
    }
}

impl<'page> MainParserOwnedDocumentScriptOwner<'page> {
    pub(super) fn new(page_vm: &'page mut PageVm) -> Self {
        Self { page_vm }
    }

    /// Consume at most one exact ready parser-owned module action.
    ///
    /// The stable source reservation represents one scheduler task. Stale
    /// queue entries may be discarded while claiming it, but once one current
    /// action is claimed this function never scans forward to execute another
    /// action in the same selected turn.
    pub(super) async fn run_next_ready_work(&mut self) -> Result<MainParserContinuationTaskEffect> {
        let Some(work) = self.take_next_claimed_ready_work() else {
            return Ok(MainParserContinuationTaskEffect::NotApplied);
        };
        let task_owner = work.payload_document_owner().task_owner();
        match work.run_with_ready_owner(self).await? {
            MainParserModuleExecution::Settled {
                outcome,
                task_effect,
            } => {
                tracing::debug!(
                    ?task_owner,
                    ?outcome,
                    ?task_effect,
                    "settled one selected parser-owned module continuation"
                );
                Ok(task_effect)
            }
            MainParserModuleExecution::TerminalForSelectedTask { outcome, terminal } => {
                tracing::debug!(
                    ?task_owner,
                    ?outcome,
                    "returned parser-owned module terminal to its selected task"
                );
                Ok(self
                    .page_vm
                    .dispatch_parser_module_terminal(task_owner, terminal))
            }
        }
    }

    fn take_next_claimed_ready_work(&mut self) -> Option<MainParserOwnedDocumentScriptWork> {
        let current_owner = self
            .page_vm
            .vm()
            .current_main_document_task_owner()
            .map(MainParserDocumentOwner::new);
        self.page_vm
            .vm_mut()
            .document_runtime
            .parser_module_document_scripts_mut()
            .take_next_claimed_ready_dispatch::<MainDocumentReadyActionRoute, _, _, _>(
                |dispatch| {
                    claim_main_document_ready_dispatch(dispatch, current_owner, "parser-owned")
                },
                |mismatch| {
                    report_main_document_ready_owner_mismatch(mismatch, "parser-owned");
                },
            )
    }

    async fn run_parser_module_ready_with_shared_runner(
        &mut self,
        work: MainParserOwnedModuleReadyInput,
        terminal_disposition: ParserModuleTerminalDisposition,
    ) -> Result<MainParserModuleExecution> {
        let hooks = MainParserOwnedModuleExecutionHooks {
            page_vm: self.page_vm,
            terminal_disposition,
        };
        let mut runner = DocumentScriptExecutionRunner::new(hooks);
        runner.run_ready_work(work).await
    }

    pub(super) async fn run_parser_deferred_module_terminal(
        &mut self,
        owner: FrameDocumentTaskOwner,
        terminal: ParserModuleGraphTerminalWork<
            MainDocumentModuleGraphReadyTarget,
            MainParserOwnedModuleScriptFailure,
        >,
        load_delay_token: DocumentLoadDelayTokenId,
    ) -> Result<MainParserModuleExecution> {
        let ready = match terminal {
            ParserModuleGraphTerminalWork::Ready(work) => {
                DocumentModuleScriptReadyWork::GraphReady(*work)
            }
            ParserModuleGraphTerminalWork::Failed(failure) => {
                DocumentModuleScriptReadyWork::GraphFailed(failure)
            }
        };
        let result = self
            .run_parser_module_ready_with_shared_runner(
                ready,
                ParserModuleTerminalDisposition::ReturnToSelectedParserTask,
            )
            .await;
        let released = self
            .page_vm
            .vm_mut()
            .release_main_parser_deferred_script_load_delay(owner, load_delay_token);
        tracing::debug!(
            ?owner,
            ?load_delay_token,
            released,
            outcome = ?result.as_ref().ok(),
            "settled main parser module-defer lifecycle ownership after terminal execution"
        );
        result
    }
}

impl MainParserOwnedModuleExecutionHooks<'_> {
    fn complete_graph_failure(
        &mut self,
        owned_failure: MainParserOwnedModuleScriptFailure,
    ) -> MainParserModuleExecution {
        let failure = owned_failure.into_action();
        if self.terminal_disposition
            == ParserModuleTerminalDisposition::CompleteWithinModuleSettlement
        {
            let _ = self
                .page_vm
                .complete_notified_module_script_graph_failures(std::iter::once((
                    failure.continuation,
                    failure.error,
                )));
            return MainParserModuleExecution::settled(DocumentScriptExecutionOutcome::Progressed);
        }

        let failure_classification = PageOwnedScriptFailureClassification::from_module_load_error(
            &failure.continuation.script,
            &failure.error,
        );
        let outcome = self.page_vm.complete_module_script_failure_for_runner(
            failure.continuation,
            failure.error.message().to_owned(),
            failure_classification,
            PreparedScriptBodyActivity::NotEntered,
            self.terminal_disposition,
        );
        let (run, execution) = outcome;
        self.page_vm.report.runs.push(run);
        execution
    }
}

impl DocumentScriptExecutionHooks for MainParserOwnedModuleExecutionHooks<'_> {
    type Ready = MainParserOwnedModuleReadyInput;
    type PreparedWork = MainParserOwnedModuleReadyInput;
    type PrepareFollowup = DocumentScriptExecutionOutcome;
    type ExecutionResult = MainParserModuleExecution;
    type PostExecutionFollowup = MainParserModuleExecution;
    type Output = MainParserModuleExecution;
    type ExecuteFuture<'owner>
        = Pin<Box<dyn Future<Output = Result<MainParserModuleExecution>> + 'owner>>
    where
        Self: 'owner;

    fn prepare_execution(
        &mut self,
        ready: Self::Ready,
    ) -> DocumentScriptExecutionStartReport<Self::PreparedWork, Self::PrepareFollowup> {
        DocumentScriptExecutionStartReport::execute(
            ready,
            DocumentScriptExecutionOutcome::Progressed,
        )
    }

    fn execute_work(&mut self, work: Self::PreparedWork) -> Self::ExecuteFuture<'_> {
        Box::pin(async move {
            match work {
                DocumentModuleScriptReadyWork::GraphReady(graph_ready) => {
                    self.page_vm
                        .finish_ready_completed_module_script(
                            ModuleScriptContinuation::from_main_document_graph_ready_work(
                                graph_ready,
                            ),
                            self.terminal_disposition,
                        )
                        .await
                }
                DocumentModuleScriptReadyWork::GraphFailed(owned_failure) => {
                    Ok(self.complete_graph_failure(owned_failure))
                }
                DocumentModuleScriptReadyWork::EvaluationCompleted(owned_evaluation) => {
                    self.page_vm
                        .run_ready_module_evaluation_completion(
                            Some(owned_evaluation.into_action()),
                            self.terminal_disposition,
                        )
                        .await
                }
            }
        })
    }

    fn prepare_post_execution_followup(
        &mut self,
        execution_result: Self::ExecutionResult,
    ) -> Result<Self::PostExecutionFollowup> {
        Ok(execution_result)
    }

    fn apply_post_execution_followup(
        &mut self,
        followup: Self::PostExecutionFollowup,
    ) -> Result<Self::Output> {
        Ok(followup)
    }

    fn outcome_for_dropped_ready(
        &mut self,
        prepare_followup: Self::PrepareFollowup,
    ) -> Result<Self::Output> {
        Ok(MainParserModuleExecution::settled(prepare_followup))
    }
}

impl<'page>
    DocumentScriptReadyWorkOwner<
        MainDocumentModuleGraphReadyTarget,
        MainParserOwnedModuleScriptEvaluation,
        MainParserOwnedModuleScriptFailure,
        Infallible,
        Infallible,
    > for MainParserOwnedDocumentScriptOwner<'page>
{
    type Output<'owner>
        = Pin<Box<dyn Future<Output = Result<MainParserModuleExecution>> + 'owner>>
    where
        Self: 'owner;

    fn run_module_script_ready_work<'owner>(
        &'owner mut self,
        work: MainParserOwnedModuleReadyInput,
    ) -> Self::Output<'owner> {
        Box::pin(async move {
            self.run_parser_module_ready_with_shared_runner(
                work,
                ParserModuleTerminalDisposition::ReturnToSelectedParserTask,
            )
            .await
        })
    }

    fn run_parser_classic_ready_work<'owner>(
        &'owner mut self,
        work: ParserClassicScriptNextOwnerAction<Infallible, Infallible>,
    ) -> Self::Output<'owner> {
        Box::pin(async move {
            match work {
                ParserClassicScriptNextOwnerAction::Ready(ready) => match ready {},
                ParserClassicScriptNextOwnerAction::SourceFailed(failure) => match failure {},
            }
        })
    }
}
