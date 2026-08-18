//! Module graph/evaluation terminal settlement for main-document scripts.
//!
//! The sibling owner module claims typed parser work and decides which runner
//! owns it. This module alone translates graph/evaluation outcomes into script
//! terminal bodies, load-delay settlement, parser task effects, and late TLA
//! reaction results. Keeping these responsibilities separate prevents the
//! owner/claim path from becoming a second completion coordinator.

use anyhow::Result;

use crate::document_script_scheduler::DocumentScriptExecutionOutcome;
use crate::module_script_continuation::{
    MainParserDocumentOwner, ModuleScriptContinuation, ModuleScriptContinuationGraphAdvance,
    ModuleScriptEvaluationContinuation, ModuleScriptEvaluationReactionState,
};
use crate::network::ResourceRequestClient;
use crate::script_vm::{
    ParserModuleEvaluationSettlement, ParserModuleTerminalDisposition,
    ParserOwnedModuleSuccessTerminal, PreparedModuleSuccessSettlement, PreparedScriptBodyActivity,
};
use crate::types::ScriptRun;

use super::parser_owned_document_script::MainParserModuleExecution;
use super::parser_task_completion::{
    MainParserContinuationBodyActivity, MainParserContinuationTaskEffect,
};
use super::{
    ModuleScriptEvaluationStart, PageOwnedScriptFailureClassification, PageVm,
    complete_page_owned_prepared_script_execution_failure_body,
    complete_prepared_script_execution_failure,
};

enum ModuleScriptCompletionOutcome {
    Completed {
        run: ScriptRun,
        task_effect: MainParserContinuationTaskEffect,
    },
    TerminalForSelectedTask {
        run: ScriptRun,
        terminal: ParserOwnedModuleSuccessTerminal,
    },
}

impl ModuleScriptCompletionOutcome {
    fn into_parser_execution(self) -> (ScriptRun, MainParserModuleExecution) {
        match self {
            Self::Completed { run, task_effect } => (
                run,
                MainParserModuleExecution::settled_with_task_effect(
                    DocumentScriptExecutionOutcome::Progressed,
                    task_effect,
                ),
            ),
            Self::TerminalForSelectedTask { run, terminal } => (
                run,
                MainParserModuleExecution::TerminalForSelectedTask {
                    outcome: DocumentScriptExecutionOutcome::Progressed,
                    terminal,
                },
            ),
        }
    }
}
impl PageVm {
    pub(super) fn complete_module_script_failure_for_runner(
        &mut self,
        script_continuation: ModuleScriptContinuation,
        error: String,
        failure_classification: PageOwnedScriptFailureClassification,
        prepared_activity: PreparedScriptBodyActivity,
        terminal_disposition: ParserModuleTerminalDisposition,
    ) -> (ScriptRun, MainParserModuleExecution) {
        self.complete_module_script_failure_for_terminal_disposition(
            script_continuation,
            error,
            failure_classification,
            prepared_activity,
            terminal_disposition,
        )
        .into_parser_execution()
    }

    fn complete_module_script_failure_for_terminal_disposition(
        &mut self,
        mut script_continuation: ModuleScriptContinuation,
        error: String,
        failure_classification: PageOwnedScriptFailureClassification,
        prepared_activity: PreparedScriptBodyActivity,
        terminal_disposition: ParserModuleTerminalDisposition,
    ) -> ModuleScriptCompletionOutcome {
        let parser_owner = script_continuation
            .parser_document_owner()
            .map(MainParserDocumentOwner::task_owner);
        let completion_owner = script_continuation.completion_owner();
        let dynamic_script_owner_id = script_continuation.dynamic_script_owner_id();
        let load_delay_binding = script_continuation.take_main_document_load_delay_binding();
        let settlement_script = script_continuation.script.clone();

        let (run, task_effect) = if terminal_disposition
            == ParserModuleTerminalDisposition::ReturnToSelectedParserTask
        {
            let outcome = complete_page_owned_prepared_script_execution_failure_body(
                self.vm_mut(),
                script_continuation.script,
                completion_owner,
                dynamic_script_owner_id,
                error,
                failure_classification,
                prepared_activity,
            );
            let (run, activity) = outcome.into_parts();
            let task_effect =
                parser_owner.map_or(MainParserContinuationTaskEffect::NotApplied, |owner| {
                    MainParserContinuationTaskEffect::applied(
                        owner,
                        MainParserContinuationBodyActivity::from_page_owned_document_script(
                            activity,
                        ),
                    )
                });
            (run, task_effect)
        } else {
            (
                complete_prepared_script_execution_failure(
                    self.vm_mut(),
                    script_continuation.script,
                    completion_owner,
                    dynamic_script_owner_id,
                    error,
                    failure_classification,
                )
                .into_run(),
                MainParserContinuationTaskEffect::NotApplied,
            )
        };

        if let Some(binding) = load_delay_binding {
            let _ = self
                .vm_mut()
                .enqueue_main_document_script_load_delay_settlement_best_effort(
                    &settlement_script,
                    binding,
                );
        }
        ModuleScriptCompletionOutcome::Completed { run, task_effect }
    }

    pub(super) async fn finish_ready_completed_module_script(
        &mut self,
        loader: &ResourceRequestClient,
        mut script_continuation: ModuleScriptContinuation,
        terminal_disposition: ParserModuleTerminalDisposition,
    ) -> Result<MainParserModuleExecution> {
        let Some(graph) = script_continuation.completed_graph.take() else {
            self.handle_module_script_continuation_graph_advance(
                ModuleScriptContinuationGraphAdvance::Ready(Box::new(script_continuation)),
            )?;
            return Ok(MainParserModuleExecution::settled(
                DocumentScriptExecutionOutcome::NoProgress,
            ));
        };
        tracing::debug!(
            url = %script_continuation.script.url,
            completion_owner = ?script_continuation.completion_owner(),
            document_owner_before_run = ?script_continuation.document_owner(),
            active_fetch_load_id = ?script_continuation.active_fetch_load_id(),
            "finishing completed module script graph"
        );
        let outcome = match self.start_module_script_graph_evaluation(&graph) {
            Ok(ModuleScriptEvaluationStart::Completed(prepared_activity)) => {
                self.finalize_module_script_success(
                    loader,
                    script_continuation,
                    ParserModuleEvaluationSettlement::Completed,
                    terminal_disposition,
                    prepared_activity,
                )
                .await
            }
            Ok(ModuleScriptEvaluationStart::Pending {
                root_entry,
                reaction_id,
            }) => {
                let completion_applied_at_evaluation_start = true;
                let outcome = self
                    .finalize_module_script_completion(
                        loader,
                        &mut script_continuation,
                        ParserModuleEvaluationSettlement::Suspended,
                        terminal_disposition,
                        PreparedScriptBodyActivity::Entered,
                    )
                    .await;
                let (run, execution) = outcome.into_parser_execution();
                self.report.runs.push(run);
                self.push_module_evaluation_continuation(ModuleScriptEvaluationContinuation {
                    script_continuation,
                    root_entry,
                    reaction_id,
                    reaction_state: ModuleScriptEvaluationReactionState::Pending,
                    completion_applied_at_evaluation_start,
                });
                return Ok(execution);
            }
            Err(failure) => {
                let (error, prepared_activity) = failure.into_parts();
                let message = error.message().to_owned();
                let failure_classification =
                    PageOwnedScriptFailureClassification::from_module_load_error(
                        &script_continuation.script,
                        &error,
                    );
                self.complete_module_script_failure_for_terminal_disposition(
                    script_continuation,
                    message,
                    failure_classification,
                    prepared_activity,
                    terminal_disposition,
                )
            }
        };
        let (run, execution) = outcome.into_parser_execution();
        self.report.runs.push(run);
        Ok(execution)
    }

    async fn finalize_module_script_success(
        &mut self,
        loader: &ResourceRequestClient,
        mut script_continuation: ModuleScriptContinuation,
        evaluation: ParserModuleEvaluationSettlement,
        terminal_disposition: ParserModuleTerminalDisposition,
        prepared_activity: PreparedScriptBodyActivity,
    ) -> ModuleScriptCompletionOutcome {
        self.finalize_module_script_completion(
            loader,
            &mut script_continuation,
            evaluation,
            terminal_disposition,
            prepared_activity,
        )
        .await
    }

    async fn finalize_module_script_completion(
        &mut self,
        loader: &ResourceRequestClient,
        script_continuation: &mut ModuleScriptContinuation,
        evaluation: ParserModuleEvaluationSettlement,
        terminal_disposition: ParserModuleTerminalDisposition,
        prepared_activity: PreparedScriptBodyActivity,
    ) -> ModuleScriptCompletionOutcome {
        let script = script_continuation.script.clone();
        let settlement_script = script.clone();
        let document_owner_before_run = script_continuation.document_owner();
        let completion_owner = script_continuation.completion_owner();
        let parser_owner = script_continuation
            .parser_document_owner()
            .map(MainParserDocumentOwner::task_owner);
        let dynamic_script_owner_id = script_continuation.dynamic_script_owner_id();
        let load_delay_binding = script_continuation.take_main_document_load_delay_binding();
        let finish_result = self
            .vm_mut()
            .settle_prepared_module_success(
                loader,
                &script,
                document_owner_before_run,
                dynamic_script_owner_id,
                evaluation,
                terminal_disposition,
                prepared_activity,
            )
            .await;
        let outcome = match finish_result {
            Ok(settlement) => {
                if let Some(owner_id) = dynamic_script_owner_id {
                    self.vm_mut()
                        .finish_runtime_owned_script_success(owner_id, &script);
                }
                let run = ScriptRun::executed(
                    script.node_id,
                    script.kind,
                    script.mode,
                    script.source_kind,
                    script.url,
                );
                match settlement {
                    PreparedModuleSuccessSettlement::ParserOwned(terminal) => {
                        ModuleScriptCompletionOutcome::TerminalForSelectedTask { run, terminal }
                    }
                    PreparedModuleSuccessSettlement::ParserOwnedCompleted
                    | PreparedModuleSuccessSettlement::RuntimeOwned => {
                        ModuleScriptCompletionOutcome::Completed {
                            run,
                            task_effect: MainParserContinuationTaskEffect::NotApplied,
                        }
                    }
                    PreparedModuleSuccessSettlement::Stale => {
                        let task_effect = if terminal_disposition
                            == ParserModuleTerminalDisposition::ReturnToSelectedParserTask
                        {
                            script_continuation.parser_document_owner().map_or(
                                MainParserContinuationTaskEffect::NotApplied,
                                |owner| {
                                    MainParserContinuationTaskEffect::applied(
                                        owner.task_owner(),
                                        MainParserContinuationBodyActivity::from_prepared_script(
                                            prepared_activity,
                                        ),
                                    )
                                },
                            )
                        } else {
                            MainParserContinuationTaskEffect::NotApplied
                        };
                        ModuleScriptCompletionOutcome::Completed { run, task_effect }
                    }
                }
            }
            Err(error) => {
                if terminal_disposition
                    == ParserModuleTerminalDisposition::ReturnToSelectedParserTask
                {
                    let outcome = complete_page_owned_prepared_script_execution_failure_body(
                        self.vm_mut(),
                        script,
                        completion_owner,
                        dynamic_script_owner_id,
                        error,
                        PageOwnedScriptFailureClassification::LegacyMessageText,
                        prepared_activity,
                    );
                    let (run, activity) = outcome.into_parts();
                    let task_effect = parser_owner.map_or(
                        MainParserContinuationTaskEffect::NotApplied,
                        |owner| {
                            MainParserContinuationTaskEffect::applied(
                                owner,
                                MainParserContinuationBodyActivity::from_page_owned_document_script(
                                    activity,
                                ),
                            )
                        },
                    );
                    ModuleScriptCompletionOutcome::Completed { run, task_effect }
                } else {
                    ModuleScriptCompletionOutcome::Completed {
                        run: complete_prepared_script_execution_failure(
                            self.vm_mut(),
                            script,
                            completion_owner,
                            dynamic_script_owner_id,
                            error,
                            PageOwnedScriptFailureClassification::LegacyMessageText,
                        )
                        .into_run(),
                        task_effect: MainParserContinuationTaskEffect::NotApplied,
                    }
                }
            }
        };
        if let Some(binding) = load_delay_binding {
            let _ = self
                .vm_mut()
                .enqueue_main_document_script_load_delay_settlement_best_effort(
                    &settlement_script,
                    binding,
                );
        }
        outcome
    }

    pub(super) async fn run_ready_module_evaluation_completion(
        &mut self,
        loader: &ResourceRequestClient,
        evaluation: Option<ModuleScriptEvaluationContinuation>,
        terminal_disposition: ParserModuleTerminalDisposition,
    ) -> Result<MainParserModuleExecution> {
        let Some(evaluation) = evaluation else {
            return Ok(MainParserModuleExecution::settled(
                DocumentScriptExecutionOutcome::NoProgress,
            ));
        };
        if evaluation.completion_applied_at_evaluation_start {
            return match evaluation.reaction_state {
                ModuleScriptEvaluationReactionState::Fulfilled => {
                    tracing::debug!(
                        root_entry = ?evaluation.root_entry,
                        url = %evaluation.script_continuation.script.url,
                        "module TLA fulfilled after script completion was already applied"
                    );
                    let task_effect = if terminal_disposition
                        == ParserModuleTerminalDisposition::ReturnToSelectedParserTask
                    {
                        let owner = evaluation
                            .script_continuation
                            .parser_document_owner()
                            .map(MainParserDocumentOwner::task_owner)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "selected parser-owned TLA completion lost its exact Document owner"
                                )
                            })?;
                        MainParserContinuationTaskEffect::applied(
                            owner,
                            MainParserContinuationBodyActivity::NoPageCodeOrEventDispatch,
                        )
                    } else {
                        MainParserContinuationTaskEffect::NotApplied
                    };
                    Ok(MainParserModuleExecution::settled_with_task_effect(
                        DocumentScriptExecutionOutcome::Progressed,
                        task_effect,
                    ))
                }
                ModuleScriptEvaluationReactionState::Rejected {
                    reason,
                    error_constructor,
                } => {
                    let message = format!(
                        "NativeEsmEvaluateFailed: native module graph evaluation rejected: {reason}"
                    );
                    tracing::debug!(
                        root_entry = ?evaluation.root_entry,
                        url = %evaluation.script_continuation.script.url,
                        %reason,
                        "reporting module TLA rejection without redispatching script completion"
                    );
                    let task_effect = if terminal_disposition
                        == ParserModuleTerminalDisposition::ReturnToSelectedParserTask
                    {
                        let owner = evaluation
                            .script_continuation
                            .parser_document_owner()
                            .map(MainParserDocumentOwner::task_owner)
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "selected parser-owned TLA rejection lost its exact Document owner"
                                )
                            })?;
                        self.vm_mut().report_window_error_body_best_effort(
                            &message,
                            Some(evaluation.script_continuation.script.url.as_str()),
                            error_constructor,
                        );
                        MainParserContinuationTaskEffect::applied(
                            owner,
                            MainParserContinuationBodyActivity::PageCodeOrEventDispatch,
                        )
                    } else {
                        self.vm_mut()
                            .report_module_tla_rejection_and_finish_reaction_best_effort(
                                &message,
                                Some(evaluation.script_continuation.script.url.as_str()),
                                error_constructor,
                            );
                        MainParserContinuationTaskEffect::NotApplied
                    };
                    Ok(MainParserModuleExecution::settled_with_task_effect(
                        DocumentScriptExecutionOutcome::Progressed,
                        task_effect,
                    ))
                }
                ModuleScriptEvaluationReactionState::Pending => Ok(
                    MainParserModuleExecution::settled(DocumentScriptExecutionOutcome::NoProgress),
                ),
            };
        }
        let outcome = match evaluation.reaction_state {
            ModuleScriptEvaluationReactionState::Fulfilled => {
                let root_entry = evaluation.root_entry;
                tracing::debug!(
                    ?root_entry,
                    url = %evaluation.script_continuation.script.url,
                    "module script evaluation promise fulfilled"
                );
                Some(
                    self.finalize_module_script_success(
                        loader,
                        evaluation.script_continuation,
                        ParserModuleEvaluationSettlement::Completed,
                        terminal_disposition,
                        PreparedScriptBodyActivity::NotEntered,
                    )
                    .await,
                )
            }
            ModuleScriptEvaluationReactionState::Rejected {
                reason,
                error_constructor,
            } => Some(
                self.complete_module_script_failure_for_terminal_disposition(
                    evaluation.script_continuation,
                    format!(
                        "NativeEsmEvaluateFailed: native module graph evaluation rejected: {reason}"
                    ),
                    PageOwnedScriptFailureClassification::Typed {
                        dynamic_kind:
                            crate::dynamic_script_owner::DynamicScriptFailureKind::Immediate,
                        module_failure_policy: Some(
                            crate::host::ModuleFailurePolicy::EvaluationFailure,
                        ),
                        error_constructor,
                    },
                    PreparedScriptBodyActivity::NotEntered,
                    terminal_disposition,
                ),
            ),
            ModuleScriptEvaluationReactionState::Pending => None,
        };

        let Some(outcome) = outcome else {
            return Ok(MainParserModuleExecution::settled(
                DocumentScriptExecutionOutcome::NoProgress,
            ));
        };
        let (run, execution) = outcome.into_parser_execution();
        self.report.runs.push(run);
        Ok(execution)
    }
}
