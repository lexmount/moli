//! Completion of one selected main-parser continuation.
//!
//! Parser continuation execution owns ordered script work. This component
//! owns the bounded handoff from that work to parser completion: it consumes a
//! one-shot drained-queue permit, asks the lifecycle authority to claim the
//! exact DOMContentLoaded successor, closes the selected parser task's
//! checkpoint boundary, and only then hands that already-claimed successor to
//! the lifecycle coordinator. The coordinator applies it to the surviving
//! exact Document or stale-rejects it after replacement.
//! It is not a scheduler lane and stores no durable lifecycle state.

use anyhow::Result;

use crate::PageVmInitStage;
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::network::ResourceRequestClient;
use crate::page_task_queue::{PostParseLifecycleWork, PostParsePageOwnedWork};
use crate::runtime::{PendingDocumentLifecycleTurn, RendererDocumentLifecycleIdentity};
use crate::script_vm::{
    MainDocumentLifecycleBodyKind, ParserFinishDomContentLoadedTask, PostParsePageOwnedTask,
};

use super::super::document_lifecycle_turn::{
    DocumentLifecycleTurnAction, DocumentLifecycleTurnOutcome,
};
use super::PageVm;
use super::parser_continuation::{MainParserContinuationCompletion, MainParserQueueSettlement};
use super::parser_task_completion::MainParserContinuationTaskEffect;

/// One-shot authority to attempt the exact DOMContentLoaded successor of a
/// drained parser-deferred queue.
///
/// The permit proves only that the exact ordered parser queue became empty. It
/// carries no checkpoint policy and is never stored as a second parser-
/// completion authority.
#[derive(Debug)]
pub(in crate::runtime) struct MainParserFinishPermit {
    owner: FrameDocumentTaskOwner,
}

impl MainParserFinishPermit {
    pub(super) const fn new(owner: FrameDocumentTaskOwner) -> Self {
        Self { owner }
    }

    pub(in crate::runtime) const fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }
}

/// Whether parse-time parser completion retained the original Document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ParseTimeMainParserBoundaryOutcome {
    CurrentDocumentRetained,
    DocumentReplaced,
}

#[derive(Debug)]
pub(super) enum SelectedPostParsePageOwnedCompletion {
    Ordinary,
    MainDocumentPostParse(crate::page_task_queue::MainDocumentPostParseExecution),
    MainParser(MainParserContinuationCompletion),
}

struct ParserCompletion;

impl ParserCompletion {
    fn exact_domcontentloaded_owner(
        work: PostParseLifecycleWork,
        expected_owner: FrameDocumentTaskOwner,
    ) -> std::result::Result<FrameDocumentTaskOwner, &'static str> {
        let PostParseLifecycleWork::DispatchDomContentLoaded { owner } = work else {
            return Err("parser-finish successor is not DOMContentLoaded work");
        };
        if owner != expected_owner {
            return Err("parser-finish DOMContentLoaded successor targets another Document");
        }
        Ok(owner)
    }

    fn finish_task(
        page_vm: &mut PageVm,
        task_effect: MainParserContinuationTaskEffect,
    ) -> Result<()> {
        page_vm.finish_main_parser_continuation_task(task_effect)
    }

    fn finish_task_with_replacement_admission(
        page_vm: &mut PageVm,
        task_effect: MainParserContinuationTaskEffect,
    ) -> Result<()> {
        let replacement_lifecycle_snapshot =
            page_vm.document_replacement_lifecycle_action_snapshot();
        let completion = Self::finish_task(page_vm, task_effect);
        let admission = page_vm.take_document_replacement_lifecycle_admission_after_action(
            replacement_lifecycle_snapshot,
        );
        match (completion, admission) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(completion_error), Ok(_)) => Err(completion_error),
            (Ok(_), Err(admission_error)) => Err(admission_error),
            (Err(completion_error), Err(admission_error)) => Err(anyhow::anyhow!(
                "parser task completion failed ({completion_error:#}) and its Document replacement admission also failed ({admission_error:#})"
            )),
        }
    }

    fn finish_without_direct_domcontentloaded(
        page_vm: &mut PageVm,
        completion: SelectedPostParsePageOwnedCompletion,
    ) -> Result<()> {
        match completion {
            SelectedPostParsePageOwnedCompletion::Ordinary => Ok(()),
            SelectedPostParsePageOwnedCompletion::MainDocumentPostParse(execution) => {
                page_vm.finish_main_document_post_parse_execution(execution)
            }
            SelectedPostParsePageOwnedCompletion::MainParser(completion) => {
                let (_, task_effect) = completion.into_parts();
                Self::finish_task(page_vm, task_effect)
            }
        }
    }

    async fn finish_parse_time_inner(
        page_vm: &mut PageVm,
        completion: MainParserContinuationCompletion,
    ) -> Result<()> {
        let (queue, task_effect) = completion.into_parts();
        let MainParserQueueSettlement::Drained(permit) = queue else {
            return Self::finish_task(page_vm, task_effect);
        };
        let owner = permit.owner();
        let claimed_dcl = {
            let PageVm {
                vm,
                page_task_queue,
                ..
            } = page_vm;
            vm.as_mut()
                .expect("PageVm must retain a live ScriptVm until drop")
                .claim_parse_time_domcontentloaded_after_main_parser_finish(page_task_queue, owner)
        };
        let Some(claimed_dcl) = claimed_dcl else {
            return Self::finish_task(page_vm, task_effect);
        };
        if claimed_dcl.owner() != owner {
            Self::finish_task(page_vm, task_effect)?;
            anyhow::bail!(
                "parse-time parser completion claimed DOMContentLoaded for another Document"
            );
        }
        let successor_owner = match Self::exact_domcontentloaded_owner(
            claimed_dcl.into_work(),
            owner,
        ) {
            Ok(owner) => owner,
            Err(message) => {
                let completion = Self::finish_task(page_vm, task_effect);
                return match completion {
                    Ok(()) => Err(anyhow::anyhow!(message)),
                    Err(completion_error) => Err(anyhow::anyhow!(
                        "parse-time parser successor was invalid and its task completion also failed ({completion_error:#})"
                    )),
                };
            }
        };
        // The parser continuation and DOMContentLoaded are distinct HTML task
        // boundaries even though this Chromium-compatible direct successor
        // deliberately does not reopen ordinary scheduler arbitration. Drain
        // terminal reactions first; the lifecycle coordinator then owns DCL's
        // separate task-end checkpoint.
        Self::finish_task(page_vm, task_effect)?;
        let request_client = page_vm.request_client.clone();
        while page_vm
            .vm()
            .document_runtime
            .has_ready_timeout_queued_by_classic_defer_script()
        {
            page_vm
                .run_classic_defer_timer_before_domcontentloaded(&request_client)
                .await?;
        }
        let run = super::main_document_lifecycle_completion::execute_parser_exact_domcontentloaded_on_owner_local_task(
            page_vm,
            successor_owner,
        )
        .await?;
        anyhow::ensure!(
            run.completion.kind() == MainDocumentLifecycleBodyKind::DomContentLoaded,
            "parse-time DOMContentLoaded successor lost its typed lifecycle execution"
        );
        Ok(())
    }

    async fn finish_parse_time(
        page_vm: &mut PageVm,
        completion: MainParserContinuationCompletion,
    ) -> Result<ParseTimeMainParserBoundaryOutcome> {
        let source_document = page_vm.document_lifecycle.identity();
        let replacement_lifecycle_snapshot =
            page_vm.document_replacement_lifecycle_action_snapshot();
        let execution = Self::finish_parse_time_inner(page_vm, completion).await;
        let admission = page_vm.take_document_replacement_lifecycle_admission_after_action(
            replacement_lifecycle_snapshot,
        );
        match (execution, admission) {
            (Ok(()), Ok(_)) => Ok(
                if page_vm.document_lifecycle.identity() == source_document {
                    ParseTimeMainParserBoundaryOutcome::CurrentDocumentRetained
                } else {
                    ParseTimeMainParserBoundaryOutcome::DocumentReplaced
                },
            ),
            (Err(execution_error), Ok(_)) => Err(execution_error),
            (Ok(()), Err(admission_error)) => Err(admission_error),
            (Err(execution_error), Err(admission_error)) => Err(anyhow::anyhow!(
                "parse-time parser completion failed ({execution_error:#}) and its Document replacement admission also failed ({admission_error:#})"
            )),
        }
    }
}

impl PageVm {
    pub(in crate::runtime) async fn finish_parse_time_main_parser_boundary(
        &mut self,
        completion: MainParserContinuationCompletion,
    ) -> Result<ParseTimeMainParserBoundaryOutcome> {
        ParserCompletion::finish_parse_time(self, completion).await
    }

    async fn execute_domcontentloaded_after_main_parser_finish_on_named_owner_lane(
        &mut self,
        mut task: ParserFinishDomContentLoadedTask,
    ) -> Result<PostParsePageOwnedTask> {
        let owner = task.owner();
        let work = task.take_work_for_execution();
        let PostParsePageOwnedWork::Lifecycle(work) = work else {
            anyhow::bail!("parser-finish successor is not lifecycle work");
        };
        let successor_owner = match ParserCompletion::exact_domcontentloaded_owner(*work, owner) {
            Ok(owner) => owner,
            Err(message) => anyhow::bail!(message),
        };

        let run = super::main_document_lifecycle_completion::execute_parser_exact_domcontentloaded_on_owner_local_task(
            self,
            successor_owner,
        )
        .await?;
        anyhow::ensure!(
            run.completion.kind() == MainDocumentLifecycleBodyKind::DomContentLoaded,
            "DOMContentLoaded direct successor lost its typed lifecycle execution"
        );
        Ok(task.into_completed_task())
    }

    pub(super) async fn execute_and_complete_selected_post_parse_page_owned_task(
        &mut self,
        loader: &ResourceRequestClient,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        document: RendererDocumentLifecycleIdentity,
        stage: PageVmInitStage,
        mut task: Box<PostParsePageOwnedTask>,
    ) -> Result<DocumentLifecycleTurnOutcome> {
        let replacement_lifecycle_snapshot = self.document_replacement_lifecycle_action_snapshot();
        let execution = self
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                loader,
                task.take_work_for_execution(),
            )
            .await;
        // Generic post-parse callbacks must finish their old-realm task before
        // a synchronous `document.open()` replacement is admitted. MainParser
        // completion remains below because its exact-DCL handoff has its own
        // replacement-aware coordinator.
        let execution = match execution {
            Ok(SelectedPostParsePageOwnedCompletion::MainDocumentPostParse(execution)) => self
                .finish_main_document_post_parse_execution(execution)
                .map(|()| SelectedPostParsePageOwnedCompletion::Ordinary),
            other => other,
        };
        let admission = self.take_document_replacement_lifecycle_admission_after_action(
            replacement_lifecycle_snapshot,
        );
        let completion = match (execution, admission) {
            (Ok(completion), Ok(_)) => completion,
            (Err(execution_error), Ok(_)) => return Err(execution_error),
            (Ok(completion), Err(admission_error)) => {
                if let Err(checkpoint_error) =
                    ParserCompletion::finish_without_direct_domcontentloaded(self, completion)
                {
                    return Err(anyhow::anyhow!(
                        "Document replacement admission failed ({admission_error:#}) and parser task-end checkpoint also failed ({checkpoint_error:#})"
                    ));
                }
                return Err(admission_error);
            }
            (Err(execution_error), Err(admission_error)) => {
                return Err(anyhow::anyhow!(
                    "lifecycle action failed ({execution_error:#}) and its Document replacement admission also failed ({admission_error:#})"
                ));
            }
        };

        if self.vm().has_pending_location_navigation() {
            ParserCompletion::finish_without_direct_domcontentloaded(self, completion)?;
            pending_document_lifecycle_turn
                .as_mut()
                .expect("post-parse lifecycle state should remain installed")
                .completed_task = Some(*task);
            if let Some(outcome) = self.transition_lifecycle_for_pending_top_level_navigation(
                pending_document_lifecycle_turn,
                document,
                stage,
            ) {
                return Ok(outcome);
            }
            unreachable!("pending top-level navigation must transition the lifecycle turn");
        }

        match completion {
            SelectedPostParsePageOwnedCompletion::Ordinary => {
                pending_document_lifecycle_turn
                    .as_mut()
                    .expect("post-parse lifecycle state should remain installed")
                    .completed_task = Some(*task);
            }
            SelectedPostParsePageOwnedCompletion::MainDocumentPostParse(execution) => {
                self.finish_main_document_post_parse_execution(execution)?;
                pending_document_lifecycle_turn
                    .as_mut()
                    .expect("post-parse lifecycle state should remain installed")
                    .completed_task = Some(*task);
            }
            SelectedPostParsePageOwnedCompletion::MainParser(completion) => {
                let (queue, task_effect) = completion.into_parts();
                let has_sealed_main_parser_script_queue =
                    matches!(&queue, MainParserQueueSettlement::Pending)
                        && self
                            .vm()
                            .document_runtime
                            .main_parser_deferred_scripts_owner()
                            .is_some();
                pending_document_lifecycle_turn
                    .as_mut()
                    .expect("post-parse lifecycle state should remain installed")
                    .has_sealed_main_parser_script_queue = has_sealed_main_parser_script_queue;
                let MainParserQueueSettlement::Drained(permit) = queue else {
                    ParserCompletion::finish_task_with_replacement_admission(self, task_effect)?;
                    pending_document_lifecycle_turn
                        .as_mut()
                        .expect("post-parse lifecycle state should remain installed")
                        .completed_task = Some(*task);
                    return self.outcome_after_exact_post_parse_action(
                        pending_document_lifecycle_turn,
                        document,
                        stage,
                        DocumentLifecycleTurnAction::Progressed,
                        false,
                    );
                };

                let owner = permit.owner();
                let permit_is_current = self.document_lifecycle.identity() == document
                    && self.vm().current_main_document_task_owner() == Some(owner);
                if !permit_is_current {
                    ParserCompletion::finish_task_with_replacement_admission(self, task_effect)?;
                    pending_document_lifecycle_turn
                        .as_mut()
                        .expect("post-parse lifecycle state should remain installed")
                        .completed_task = Some(*task);
                    return self.outcome_after_exact_post_parse_action(
                        pending_document_lifecycle_turn,
                        document,
                        stage,
                        DocumentLifecycleTurnAction::Progressed,
                        false,
                    );
                }

                let claimed_dcl_result = {
                    let driver = pending_document_lifecycle_turn
                        .as_ref()
                        .expect("post-parse lifecycle state should remain installed")
                        .driver;
                    let PageVm {
                        vm,
                        page_task_queue,
                        report,
                        ..
                    } = self;
                    vm.as_mut()
                        .expect("PageVm must retain a live ScriptVm until drop")
                        .claim_domcontentloaded_after_main_parser_finish(
                            page_task_queue,
                            report,
                            driver,
                            *task,
                            owner,
                        )
                        .await
                        .map_err(anyhow::Error::msg)
                };
                let claimed_dcl = match claimed_dcl_result {
                    Ok(claimed_dcl) => claimed_dcl,
                    Err(claim_error) => {
                        let completion = ParserCompletion::finish_task_with_replacement_admission(
                            self,
                            task_effect,
                        );
                        return match completion {
                            Ok(()) => Err(claim_error),
                            Err(completion_error) => Err(anyhow::anyhow!(
                                "parser-finish DOMContentLoaded claim failed ({claim_error:#}) and its parser task completion also failed ({completion_error:#})"
                            )),
                        };
                    }
                };
                if let Some(dcl_task) = claimed_dcl {
                    ParserCompletion::finish_task_with_replacement_admission(self, task_effect)?;
                    while self
                        .vm()
                        .document_runtime
                        .has_ready_timeout_queued_by_classic_defer_script()
                    {
                        self.run_classic_defer_timer_before_domcontentloaded(loader)
                            .await?;
                    }
                    let replacement_lifecycle_snapshot =
                        self.document_replacement_lifecycle_action_snapshot();
                    let execution = self
                        .execute_domcontentloaded_after_main_parser_finish_on_named_owner_lane(
                            dcl_task,
                        )
                        .await;
                    let admission = self
                        .take_document_replacement_lifecycle_admission_after_action(
                            replacement_lifecycle_snapshot,
                        );
                    let completed_dcl_task = match (execution, admission) {
                        (Ok(task), Ok(_)) => task,
                        (Err(execution_error), Ok(_)) => return Err(execution_error),
                        (Ok(_), Err(admission_error)) => return Err(admission_error),
                        (Err(execution_error), Err(admission_error)) => {
                            return Err(anyhow::anyhow!(
                                "parser-finish DOMContentLoaded action failed ({execution_error:#}) and its Document replacement admission also failed ({admission_error:#})"
                            ));
                        }
                    };
                    pending_document_lifecycle_turn
                        .as_mut()
                        .expect("post-parse lifecycle state should remain installed")
                        .completed_task = Some(completed_dcl_task);
                    if let Some(outcome) = self
                        .transition_lifecycle_for_pending_top_level_navigation(
                            pending_document_lifecycle_turn,
                            document,
                            stage,
                        )
                    {
                        return Ok(outcome);
                    }
                } else {
                    ParserCompletion::finish_task_with_replacement_admission(self, task_effect)?;
                }
            }
        }

        self.outcome_after_exact_post_parse_action(
            pending_document_lifecycle_turn,
            document,
            stage,
            DocumentLifecycleTurnAction::Progressed,
            false,
        )
    }
}
