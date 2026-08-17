//! Exact-Document post-parse lifecycle continuations.
//!
//! The renderer page residence is stable across navigation; executable
//! lifecycle semantics are not. A continuation captures the complete
//! renderer lifecycle identity when it is created, and every owner resume has
//! to present that identity again. A page wake is only a liveness hint.

use std::time::Instant;

use anyhow::Result;

use crate::PageVmInitStage;
use crate::local_executor::is_on_named_owner_execution_lane_for;
use crate::page_task_queue::PostParsePageOwnedWork;
use crate::runtime::{RendererDocumentLifecycleIdentity, RendererDocumentLifecycleWaitOutcome};
use crate::script_vm::PostParseLifecycleAdvance;

use super::super::document_lifecycle_turn::{
    DocumentLifecycleTurnAction, DocumentLifecycleTurnOutcome, PendingDocumentLifecycleTurn,
};
use super::{PageVm, renderer_document_lifecycle_milestone_for_stage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct DocumentReplacementLifecycleActionSnapshot {
    pending_admission: Option<crate::runtime::RendererDocumentLifecycleDriveAdmission>,
    ready_admission: Option<crate::runtime::RendererDocumentLifecycleDriveAdmission>,
}

impl PageVm {
    pub(in crate::runtime) fn ready_document_replacement_lifecycle_admission(
        &self,
    ) -> Option<crate::runtime::RendererDocumentLifecycleDriveAdmission> {
        let admission = self
            .document_lifecycle
            .pending_document_replacement_drive_admission()?;
        (!self
            .vm()
            .document_runtime
            .document_replacement_parser_is_blocked())
        .then_some(admission)
    }

    pub(in crate::runtime) fn document_replacement_lifecycle_action_snapshot(
        &self,
    ) -> DocumentReplacementLifecycleActionSnapshot {
        DocumentReplacementLifecycleActionSnapshot {
            pending_admission: self
                .document_lifecycle
                .pending_document_replacement_drive_admission(),
            ready_admission: self.ready_document_replacement_lifecycle_admission(),
        }
    }

    pub(in crate::runtime) fn has_blocked_document_replacement_lifecycle_admission(
        &self,
        document: RendererDocumentLifecycleIdentity,
    ) -> bool {
        self.document_lifecycle
            .pending_document_replacement_drive_admission()
            .is_some_and(|admission| admission.to == document)
            && self
                .vm()
                .document_runtime
                .document_replacement_parser_is_blocked()
    }

    pub(super) fn take_document_replacement_lifecycle_admission_after_action(
        &mut self,
        snapshot: DocumentReplacementLifecycleActionSnapshot,
    ) -> Result<Option<crate::runtime::RendererDocumentLifecycleDriveAdmission>> {
        if let Some(transition) = self.document_lifecycle.take_pending_document_open_error() {
            anyhow::bail!(
                "renderer document lifecycle rejected document.open restart: {transition:?}"
            );
        }
        let ready_admission = self.ready_document_replacement_lifecycle_admission();
        if ready_admission == snapshot.ready_admission {
            return Ok(None);
        }
        anyhow::ensure!(
            snapshot.ready_admission.is_none(),
            "a Page action replaced an unconsumed Document lifecycle admission"
        );
        let Some(admission) = ready_admission else {
            return Ok(None);
        };
        self.activate_ready_document_replacement_lifecycle_admission(admission)?;
        Ok(Some(admission))
    }

    fn activate_ready_document_replacement_lifecycle_admission(
        &mut self,
        admission: crate::runtime::RendererDocumentLifecycleDriveAdmission,
    ) -> Result<()> {
        let document = self.document_lifecycle.identity();
        anyhow::ensure!(
            admission.to == document,
            "Document replacement lifecycle admission targets a non-current identity"
        );
        anyhow::ensure!(
            self.document_lifecycle
                .activate_document_replacement_drive_admission(admission),
            "Document replacement lifecycle admission changed before activation"
        );
        Ok(())
    }

    pub(in crate::runtime) async fn reconcile_document_replacement_lifecycle_after_owner_action(
        &mut self,
        snapshot: DocumentReplacementLifecycleActionSnapshot,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
    ) -> Result<Option<DocumentLifecycleTurnOutcome>> {
        // A replacement has two distinct owner-boundary transitions. Opening
        // D2 retires D1's exact continuation immediately, even if D2's input
        // stream remains open and its lifecycle admission is still blocked.
        // Closing that stream later changes the same durable admission to
        // ready and authorizes installation of D2's resident.
        let pending_admission = self
            .document_lifecycle
            .pending_document_replacement_drive_admission();
        let replacement_admission = if pending_admission == snapshot.pending_admission {
            None
        } else {
            Some(pending_admission.ok_or_else(|| {
                anyhow::anyhow!(
                    "a pending Document replacement admission disappeared outside owner settlement"
                )
            })?)
        };
        if let Some(replacement) = replacement_admission {
            anyhow::ensure!(
                replacement.to == self.document_lifecycle.identity(),
                "Document replacement admission targets a non-current identity"
            );
            self.retire_document_actions(replacement.from)?;
        }

        let ready_admission =
            self.take_document_replacement_lifecycle_admission_after_action(snapshot)?;

        if let Some(replacement) = replacement_admission {
            if let Some(stale) = pending_document_lifecycle_turn.take() {
                anyhow::ensure!(
                    stale.document == replacement.from,
                    "replacement lifecycle admission did not name the retired resident"
                );
                tracing::debug!(
                    stale_document = ?stale.document,
                    replacement_document = ?replacement.to,
                    "retired old lifecycle resident at an explicit Document replacement boundary"
                );
            }

            // These queues are owned by the exact Document. They must retire
            // at document.open(), not wait for document.close(), because an
            // old completion token cannot be allowed to settle D2 while its
            // input stream is still open. Stable Page-owned sources survive.
            self.page_task_queue.clear_document_owned_tasks();
        }

        let Some(admission) = ready_admission else {
            return Ok(None);
        };
        let document = admission.to;
        if !self.repeated_document_lifecycle_load_is_pending() {
            return Ok(None);
        }

        tracing::debug!(
            admission_id = ?admission.id,
            from = ?admission.from,
            to = ?admission.to,
            "activating exact Document replacement lifecycle admission"
        );

        if self.has_pending_runtime_command_lifecycle() {
            self.bind_pending_runtime_command_lifecycle_observer(document)?;
        }
        if pending_document_lifecycle_turn
            .as_ref()
            .is_some_and(|pending| pending.document == document)
        {
            return Ok(None);
        }
        if let Some(stale) = pending_document_lifecycle_turn.take() {
            anyhow::ensure!(
                stale.document == admission.from,
                "replacement lifecycle admission did not name the retired resident"
            );
            tracing::debug!(
                stale_document = ?stale.document,
                replacement_document = ?document,
                "retired old lifecycle resident at an explicit Document replacement boundary"
            );
        }

        self.begin_post_parse_lifecycle_on_named_owner_lane(
            pending_document_lifecycle_turn,
            Vec::new(),
            PageVmInitStage::Load,
            Instant::now(),
        )
        .await
        .map(Some)
    }

    pub(in crate::runtime) async fn begin_post_parse_lifecycle_on_named_owner_lane(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        work: Vec<PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
    ) -> Result<DocumentLifecycleTurnOutcome> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "post-parse lifecycle must start on the matching named owner lane"
        );
        anyhow::ensure!(
            pending_document_lifecycle_turn.is_none(),
            "post-parse lifecycle is already active for this page"
        );
        if let Some(admission) = self.ready_document_replacement_lifecycle_admission() {
            self.activate_ready_document_replacement_lifecycle_admission(admission)?;
        }
        let document = self.document_lifecycle.identity();
        let has_sealed_main_parser_script_queue = work
            .iter()
            .any(|item| item.main_parser_deferred_scripts_owner().is_some());
        self.set_target_stage(stage);
        let driver = {
            let PageVm {
                vm,
                page_task_queue,
                report,
                ..
            } = self;
            vm.as_mut()
                .expect("PageVm must retain a live ScriptVm until drop")
                .start_post_parse_lifecycle_round(stage, page_task_queue, report, work)
                .await
        };
        anyhow::ensure!(
            self.document_lifecycle.identity() == document,
            "renderer Document changed while creating its post-parse continuation"
        );
        *pending_document_lifecycle_turn = Some(PendingDocumentLifecycleTurn {
            document,
            stage,
            owner_turn_is_runnable: true,
            driver,
            completed_task: None,
            completion_action: None,
            has_sealed_main_parser_script_queue,
            started,
        });
        if let Some(outcome) = self.transition_lifecycle_for_pending_top_level_navigation(
            pending_document_lifecycle_turn,
            document,
            stage,
        ) {
            return Ok(outcome);
        }
        Ok(DocumentLifecycleTurnOutcome::runnable(
            DocumentLifecycleTurnAction::None,
            document,
        ))
    }

    pub(in crate::runtime) async fn advance_post_parse_lifecycle_one_owner_turn(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        document: RendererDocumentLifecycleIdentity,
    ) -> Result<DocumentLifecycleTurnOutcome> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "post-parse lifecycle must advance on the matching named owner lane"
        );
        let Some(pending) = pending_document_lifecycle_turn.as_ref() else {
            return Ok(DocumentLifecycleTurnOutcome::idle(
                DocumentLifecycleTurnAction::None,
            ));
        };
        if pending.document != document {
            tracing::debug!(
                ?document,
                pending_document = ?pending.document,
                "discarded stale post-parse owner turn without touching the current continuation"
            );
            return Ok(DocumentLifecycleTurnOutcome::idle(
                DocumentLifecycleTurnAction::None,
            ));
        }
        let current_document = self.document_lifecycle.identity();
        if current_document != document {
            *pending_document_lifecycle_turn = None;
            tracing::debug!(
                ?document,
                ?current_document,
                "discarded stale post-parse continuation without advancing the replacement document"
            );
            return Ok(DocumentLifecycleTurnOutcome::idle(
                DocumentLifecycleTurnAction::None,
            ));
        }
        let stage = pending.stage;
        let completion_action = pending.completion_action;
        if let Some(outcome) = self.transition_lifecycle_for_pending_top_level_navigation(
            pending_document_lifecycle_turn,
            document,
            stage,
        ) {
            return Ok(outcome);
        }
        if let Some(completion_action) = completion_action {
            let milestone = renderer_document_lifecycle_milestone_for_stage(stage);
            if matches!(
                self.document_lifecycle_wait_outcome(milestone),
                RendererDocumentLifecycleWaitOutcome::Pending
            ) {
                return Ok(DocumentLifecycleTurnOutcome::blocked(
                    DocumentLifecycleTurnAction::None,
                    document,
                ));
            }
            let pending = pending_document_lifecycle_turn
                .take()
                .expect("post-parse lifecycle state should remain installed");
            self.finish_post_parse_lifecycle_completion_on_named_owner_lane(
                stage,
                pending.started,
                completion_action,
            )
            .await?;
            if self.vm().has_pending_location_navigation()
                && !self
                    .vm()
                    .pending_location_navigation_scheme_is("javascript")
            {
                return Ok(self
                    .transition_lifecycle_for_pending_top_level_navigation(
                        pending_document_lifecycle_turn,
                        document,
                        stage,
                    )
                    .expect("pending cross-Document navigation must retire lifecycle residence"));
            }
            let settled = if let Some(replacement_document) = self
                .install_replacement_post_parse_lifecycle_after_terminal_action(
                    pending_document_lifecycle_turn,
                    document,
                    stage,
                ) {
                DocumentLifecycleTurnOutcome::runnable(
                    DocumentLifecycleTurnAction::DocumentReplaced {
                        previous: document,
                        current: replacement_document,
                    },
                    replacement_document,
                )
            } else {
                self.complete_stage_and_prepare_followup(
                    pending_document_lifecycle_turn,
                    document,
                    stage,
                )
            };
            if let Some(outcome) = self.transition_lifecycle_for_pending_top_level_navigation(
                pending_document_lifecycle_turn,
                document,
                stage,
            ) {
                return Ok(outcome);
            }
            return Ok(settled);
        }

        // Feed the previous exact task result into the driver before other
        // page work is admitted. `AwaitProgress` parks immediately; a producer
        // wake returns to owner arbitration instead of waiting in this method.
        let request_client = self.request_client.clone();
        let (driver, completed_task) = {
            let pending = pending_document_lifecycle_turn
                .as_mut()
                .expect("post-parse lifecycle state should remain installed");
            (pending.driver, pending.completed_task.take())
        };
        let advance = {
            let PageVm {
                vm,
                page_task_queue,
                report,
                ..
            } = self;
            vm.as_mut()
                .expect("PageVm must retain a live ScriptVm until drop")
                .advance_post_parse_lifecycle(
                    &request_client,
                    page_task_queue,
                    report,
                    driver,
                    completed_task,
                )
                .await
                .map_err(anyhow::Error::msg)?
        };
        match advance {
            PostParseLifecycleAdvance::PageOwnedTask(task) => {
                self.execute_and_complete_selected_post_parse_page_owned_task(
                    &request_client,
                    pending_document_lifecycle_turn,
                    document,
                    stage,
                    task,
                )
                .await
            }
            PostParseLifecycleAdvance::NeedsContinuation => self
                .outcome_after_exact_post_parse_action(
                    pending_document_lifecycle_turn,
                    document,
                    stage,
                    DocumentLifecycleTurnAction::Progressed,
                    false,
                ),
            PostParseLifecycleAdvance::AwaitProgress => self.outcome_after_exact_post_parse_action(
                pending_document_lifecycle_turn,
                document,
                stage,
                DocumentLifecycleTurnAction::None,
                true,
            ),
            PostParseLifecycleAdvance::Complete(completion_action) => {
                if matches!(
                    self.document_lifecycle_wait_outcome(
                        renderer_document_lifecycle_milestone_for_stage(stage),
                    ),
                    RendererDocumentLifecycleWaitOutcome::Pending
                ) {
                    pending_document_lifecycle_turn
                        .as_mut()
                        .expect("post-parse lifecycle state should remain installed")
                        .completion_action = Some(completion_action);
                    return Ok(DocumentLifecycleTurnOutcome::blocked(
                        DocumentLifecycleTurnAction::None,
                        document,
                    ));
                }
                let pending = pending_document_lifecycle_turn
                    .take()
                    .expect("post-parse lifecycle state should remain installed");
                self.finish_post_parse_lifecycle_completion_on_named_owner_lane(
                    stage,
                    pending.started,
                    completion_action,
                )
                .await?;
                if self.vm().has_pending_location_navigation()
                    && !self
                        .vm()
                        .pending_location_navigation_scheme_is("javascript")
                {
                    return Ok(self
                        .transition_lifecycle_for_pending_top_level_navigation(
                            pending_document_lifecycle_turn,
                            document,
                            stage,
                        )
                        .expect(
                            "pending cross-Document navigation must retire lifecycle residence",
                        ));
                }
                let settled = if let Some(replacement_document) = self
                    .install_replacement_post_parse_lifecycle_after_terminal_action(
                        pending_document_lifecycle_turn,
                        document,
                        stage,
                    ) {
                    DocumentLifecycleTurnOutcome::runnable(
                        DocumentLifecycleTurnAction::DocumentReplaced {
                            previous: document,
                            current: replacement_document,
                        },
                        replacement_document,
                    )
                } else {
                    self.complete_stage_and_prepare_followup(
                        pending_document_lifecycle_turn,
                        document,
                        stage,
                    )
                };
                if let Some(outcome) = self.transition_lifecycle_for_pending_top_level_navigation(
                    pending_document_lifecycle_turn,
                    document,
                    stage,
                ) {
                    Ok(outcome)
                } else {
                    Ok(settled)
                }
            }
        }
    }

    pub(super) fn outcome_after_exact_post_parse_action(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        previous_document: RendererDocumentLifecycleIdentity,
        stage: PageVmInitStage,
        action: DocumentLifecycleTurnAction,
        blocked: bool,
    ) -> Result<DocumentLifecycleTurnOutcome> {
        if let Some(document) = self.retire_and_install_post_parse_lifecycle_after_exact_action(
            pending_document_lifecycle_turn,
            previous_document,
        ) {
            Ok(DocumentLifecycleTurnOutcome::runnable(
                DocumentLifecycleTurnAction::DocumentReplaced {
                    previous: previous_document,
                    current: document,
                },
                document,
            ))
        } else if blocked {
            Ok(DocumentLifecycleTurnOutcome::blocked(
                action,
                previous_document,
            ))
        } else {
            debug_assert_eq!(
                pending_document_lifecycle_turn
                    .as_ref()
                    .map(|pending| pending.stage),
                Some(stage)
            );
            Ok(DocumentLifecycleTurnOutcome::runnable(
                action,
                previous_document,
            ))
        }
    }

    fn complete_stage_and_prepare_followup(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        document: RendererDocumentLifecycleIdentity,
        stage: PageVmInitStage,
    ) -> DocumentLifecycleTurnOutcome {
        let action = DocumentLifecycleTurnAction::ReachedStage(stage);
        if matches!(stage, PageVmInitStage::Load) {
            return DocumentLifecycleTurnOutcome::idle(action);
        }

        debug_assert!(pending_document_lifecycle_turn.is_none());
        debug_assert_eq!(self.document_lifecycle.identity(), document);
        self.set_target_stage(PageVmInitStage::Load);
        let driver = self
            .vm()
            .resume_post_parse_lifecycle_driver_for_existing_queue(PageVmInitStage::Load);
        *pending_document_lifecycle_turn = Some(PendingDocumentLifecycleTurn {
            document,
            stage: PageVmInitStage::Load,
            owner_turn_is_runnable: true,
            driver,
            completed_task: None,
            completion_action: None,
            has_sealed_main_parser_script_queue: false,
            started: Instant::now(),
        });
        DocumentLifecycleTurnOutcome::runnable(action, document)
    }

    fn retire_and_install_post_parse_lifecycle_after_exact_action(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        previous_document: RendererDocumentLifecycleIdentity,
    ) -> Option<RendererDocumentLifecycleIdentity> {
        let document = self.document_lifecycle.identity();
        if document == previous_document {
            return None;
        }
        // Only the exact action admitted under `previous_document` may cross
        // this boundary. Explicitly retire that continuation and install a new
        // identity-bound one; a later stale wake never reaches this helper.
        // If a page-owned task was just executed, its work payload has already
        // been consumed. Carrying its completion token lets ScriptVm observe
        // invalidation and rebuild the replacement Document's queue.
        let PendingDocumentLifecycleTurn {
            document: retired_document,
            stage,
            owner_turn_is_runnable: _,
            driver,
            completed_task,
            completion_action,
            has_sealed_main_parser_script_queue: _,
            started: _,
        } = pending_document_lifecycle_turn
            .take()
            .expect("post-parse lifecycle should remain installed after an exact action");
        debug_assert_eq!(retired_document, previous_document);
        // The priority belongs to the exact Document whose parser-deferred
        // queue was sealed. A script action may replace that Document, so
        // derive the replacement's state from its own runtime instead of
        // carrying authorization across generations.
        let has_sealed_main_parser_script_queue = self
            .vm()
            .document_runtime
            .main_parser_deferred_scripts_owner()
            .is_some();
        *pending_document_lifecycle_turn = Some(PendingDocumentLifecycleTurn {
            document,
            stage,
            owner_turn_is_runnable: true,
            driver,
            completed_task,
            completion_action,
            has_sealed_main_parser_script_queue,
            started: Instant::now(),
        });
        Some(document)
    }

    fn install_replacement_post_parse_lifecycle_after_terminal_action(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        previous_document: RendererDocumentLifecycleIdentity,
        stage: PageVmInitStage,
    ) -> Option<RendererDocumentLifecycleIdentity> {
        let document = self.document_lifecycle.identity();
        if document == previous_document {
            return None;
        }
        debug_assert!(pending_document_lifecycle_turn.is_none());
        let driver = self
            .vm()
            .resume_post_parse_lifecycle_driver_for_existing_queue(stage);
        *pending_document_lifecycle_turn = Some(PendingDocumentLifecycleTurn {
            document,
            stage,
            owner_turn_is_runnable: true,
            driver,
            completed_task: None,
            completion_action: None,
            has_sealed_main_parser_script_queue: self
                .vm()
                .document_runtime
                .main_parser_deferred_scripts_owner()
                .is_some(),
            started: Instant::now(),
        });
        Some(document)
    }

    /// Report whether the exact Document's sealed defer/module queue can make
    /// progress now.
    ///
    /// Merely owning a sealed queue is not enough: an external script may
    /// still need its Networking completion. Keeping readiness at this owner
    /// boundary prevents internal-script priority from starving the very task
    /// that makes the script executable.
    pub(in crate::runtime) fn sealed_main_parser_script_continuation_is_ready(&mut self) -> bool {
        let PageVm {
            vm,
            page_task_queue,
            ..
        } = self;
        vm.as_mut()
            .expect("PageVm must retain a live ScriptVm until drop")
            .document_runtime
            .main_parser_deferred_script_continuation_is_ready(page_task_queue)
    }
}
