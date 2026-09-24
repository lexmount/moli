//! Execution and settlement of one ordered main-parser script continuation.

use anyhow::Result;

use crate::document_script_scheduler::ParserDeferredScriptReady;
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::module_script_continuation::MainParserDocumentOwner;
use crate::script_vm::ParserOwnedModuleSuccessTerminal;
use crate::types::ScriptRun;

use super::PageVm;
use super::parser_completion::MainParserFinishPermit;
use super::parser_deferred_classic::MainParserDeferredClassicDocumentScriptOwner;
use super::parser_owned_document_script::{
    MainParserModuleExecution, MainParserOwnedDocumentScriptOwner,
};
use super::parser_task_completion::{
    MainParserContinuationBodyActivity, MainParserContinuationTaskEffect,
};

/// The parser queue state produced by one selected continuation.
///
/// `Drained` is not durable lifecycle state. Its permit is a one-shot proof
/// that the exact parser queue became empty while this action still owned the
/// current Document.
#[derive(Debug)]
pub(super) enum MainParserQueueSettlement {
    Pending,
    Drained(MainParserFinishPermit),
}

#[derive(Debug)]
pub(in crate::runtime) struct MainParserContinuationCompletion {
    state: MainParserContinuationCompletionState,
}

#[derive(Debug)]
enum MainParserContinuationCompletionState {
    /// The ordered parser queue still owns work, or this selected claim did
    /// not apply. The execution-produced effect determines whether the
    /// selected task owes a checkpoint.
    Pending {
        task_effect: MainParserContinuationTaskEffect,
    },
    /// The exact ordered queue became empty after an applied task.
    ///
    /// Keeping only the permit and body activity makes the invalid
    /// `Drained + NotApplied` and cross-owner combinations unrepresentable.
    /// `into_parts()` reconstructs the applied task effect from the permit's
    /// exact owner.
    Drained {
        permit: MainParserFinishPermit,
        activity: MainParserContinuationBodyActivity,
    },
}

impl MainParserContinuationCompletion {
    fn pending(task_effect: MainParserContinuationTaskEffect) -> Self {
        Self {
            state: MainParserContinuationCompletionState::Pending { task_effect },
        }
    }

    fn drained(
        owner: FrameDocumentTaskOwner,
        task_effect: MainParserContinuationTaskEffect,
    ) -> Result<Self> {
        let MainParserContinuationTaskEffect::Applied {
            owner: effect_owner,
            activity,
        } = task_effect
        else {
            anyhow::bail!("a drained parser queue requires an applied parser task");
        };
        anyhow::ensure!(
            effect_owner == owner,
            "a drained parser queue must carry task completion for the same exact Document"
        );
        Ok(Self {
            state: MainParserContinuationCompletionState::Drained {
                permit: MainParserFinishPermit::new(owner),
                activity,
            },
        })
    }

    pub(super) fn into_parts(
        self,
    ) -> (MainParserQueueSettlement, MainParserContinuationTaskEffect) {
        match self.state {
            MainParserContinuationCompletionState::Pending { task_effect } => {
                (MainParserQueueSettlement::Pending, task_effect)
            }
            MainParserContinuationCompletionState::Drained { permit, activity } => {
                let owner = permit.owner();
                (
                    MainParserQueueSettlement::Drained(permit),
                    MainParserContinuationTaskEffect::applied(owner, activity),
                )
            }
        }
    }

    #[cfg(test)]
    pub(super) fn drained_for_test(
        owner: FrameDocumentTaskOwner,
        task_effect: MainParserContinuationTaskEffect,
    ) -> Self {
        Self::drained(owner, task_effect)
            .expect("a drained parser test completion requires an applied same-owner task")
    }

    #[cfg(test)]
    pub(super) fn pending_for_test(task_effect: MainParserContinuationTaskEffect) -> Self {
        Self::pending(task_effect)
    }
}

#[derive(Debug)]
pub(super) struct MainParserDeferredExecution {
    run: Option<ScriptRun>,
    completion: MainParserContinuationCompletion,
}

impl MainParserDeferredExecution {
    fn idle() -> Self {
        Self {
            run: None,
            completion: MainParserContinuationCompletion::pending(
                MainParserContinuationTaskEffect::NotApplied,
            ),
        }
    }

    fn pending(run: Option<ScriptRun>, task_effect: MainParserContinuationTaskEffect) -> Self {
        Self {
            run,
            completion: MainParserContinuationCompletion::pending(task_effect),
        }
    }

    fn drained(
        run: Option<ScriptRun>,
        owner: FrameDocumentTaskOwner,
        task_effect: MainParserContinuationTaskEffect,
    ) -> Result<Self> {
        Ok(Self {
            run,
            completion: MainParserContinuationCompletion::drained(owner, task_effect)?,
        })
    }

    pub(super) fn into_parts(self) -> (Option<ScriptRun>, MainParserContinuationCompletion) {
        (self.run, self.completion)
    }
}

/// Result of executing one page-owned work item.
///
/// Ordinary work cannot carry parser completion authority. The parser variant
/// must be handed to ParserCompletion before the selected task returns.
#[derive(Debug)]
pub(super) enum PostParsePageOwnedExecution {
    Ordinary(Option<ScriptRun>),
    DocumentScript(super::page_owned_document_script::MainPageOwnedDocumentScriptExecution),
    MainDocumentPostParse(crate::page_task_queue::MainDocumentPostParseExecution),
    MainParserContinuation(MainParserDeferredExecution),
}

impl PostParsePageOwnedExecution {
    pub(super) fn ordinary(run: Option<ScriptRun>) -> Self {
        Self::Ordinary(run)
    }

    pub(super) fn document_script(
        execution: super::page_owned_document_script::MainPageOwnedDocumentScriptExecution,
    ) -> Self {
        Self::DocumentScript(execution)
    }

    pub(super) fn main_document_post_parse(
        execution: crate::page_task_queue::MainDocumentPostParseExecution,
    ) -> Self {
        Self::MainDocumentPostParse(execution)
    }

    pub(super) fn main_parser_continuation(execution: MainParserDeferredExecution) -> Self {
        Self::MainParserContinuation(execution)
    }
}

impl PageVm {
    pub(super) fn dispatch_parser_module_terminal(
        &mut self,
        owner: FrameDocumentTaskOwner,
        terminal: ParserOwnedModuleSuccessTerminal,
    ) -> MainParserContinuationTaskEffect {
        let evaluation = terminal.evaluation();
        let (script_event, prepared_activity) = terminal.into_parts();
        let terminal_activity = if let Some(task) = script_event {
            tracing::debug!(
                ?evaluation,
                event = task.event_name(),
                "applying parser module terminal inside the selected parser action"
            );
            self.vm_mut().dispatch_script_event_body_best_effort(&task);
            self.vm_mut().drain_deferred_page_tasks_best_effort();
            crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted
        } else {
            tracing::debug!(
                ?evaluation,
                "parser module settlement has no observable script-element terminal"
            );
            crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch
        };
        MainParserContinuationTaskEffect::applied(
            owner,
            MainParserContinuationBodyActivity::from_prepared_script(prepared_activity)
                .note_terminal(terminal_activity),
        )
    }

    pub(super) async fn run_next_main_parser_deferred_script(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
    ) -> Result<MainParserDeferredExecution> {
        if self.vm().current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.vm().current_main_document_task_owner(),
                "dropping stale main parser-deferred owner turn"
            );
            self.vm_mut()
                .document_runtime
                .disarm_main_parser_deferred_scripts(task_owner);
            return Ok(MainParserDeferredExecution::idle());
        }

        let owner = MainParserDocumentOwner::new(task_owner);
        let ready = self
            .vm_mut()
            .document_runtime
            .parser_module_document_scripts_mut()
            .take_next_after_parsing_ready_script(owner);
        let (run, task_effect) = match ready {
            Some(ParserDeferredScriptReady::Classic(script)) => {
                let execution = MainParserDeferredClassicDocumentScriptOwner::new(self)
                    .run_work(task_owner, script)
                    .await?;
                let (run, effect) = execution.into_parts();
                (run, effect)
            }
            Some(ParserDeferredScriptReady::Module(ready)) => {
                let (terminal, load_delay_token) = ready.into_parts();
                let execution = MainParserOwnedDocumentScriptOwner::new(self)
                    .run_parser_deferred_module_terminal(task_owner, terminal, load_delay_token)
                    .await?;
                match execution {
                    MainParserModuleExecution::Settled {
                        outcome,
                        task_effect,
                    } => {
                        tracing::debug!(
                            ?task_owner,
                            ?load_delay_token,
                            ?outcome,
                            "completed main parser module-defer owner turn"
                        );
                        (None, task_effect)
                    }
                    MainParserModuleExecution::TerminalForSelectedTask { outcome, terminal } => {
                        tracing::debug!(
                            ?task_owner,
                            ?load_delay_token,
                            ?outcome,
                            "returned main parser module terminal to selected continuation"
                        );
                        (
                            None,
                            self.dispatch_parser_module_terminal(task_owner, terminal),
                        )
                    }
                }
            }
            None => {
                tracing::debug!(?task_owner, "main parser-deferred head is not ready");
                return Ok(MainParserDeferredExecution::idle());
            }
        };

        let owner_is_current = self.vm().current_main_document_task_owner() == Some(task_owner);
        let queue_is_empty = !self
            .vm()
            .document_runtime
            .parser_module_document_scripts()
            .has_after_parsing_script(owner);
        if queue_is_empty {
            self.vm_mut()
                .document_runtime
                .disarm_main_parser_deferred_scripts(task_owner);
        }

        if owner_is_current && queue_is_empty {
            MainParserDeferredExecution::drained(run, task_owner, task_effect)
        } else {
            Ok(MainParserDeferredExecution::pending(run, task_effect))
        }
    }
}
