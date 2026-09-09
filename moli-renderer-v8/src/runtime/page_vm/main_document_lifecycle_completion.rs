//! Completion coordinator shared by every main-document lifecycle carrier.
//!
//! The exact lifecycle resident remains the sole ordinary DCL/load authority.
//! This component only reconciles an already-claimed body: typed checkpoint
//! continuations, milestone journal visibility, body settlement, and lifecycle
//! priming stay in one auditable order. DCL and load reach this coordinator
//! through the shared DOM task source after parser completion has admitted
//! them; interactive readiness remains part of parser completion.

use anyhow::Result;
use std::time::Instant;

use crate::frame_owner_model::MainDocumentLoadCompletionState;
use crate::runtime::{RendererDocumentLifecycleMilestone, RendererDocumentLifecycleTransition};
use crate::script_vm::{
    MainDocumentLifecycleBody, MainDocumentLifecycleBodyKind, MainDocumentLifecycleCompletion,
    MainDocumentLifecycleExecution, MainDocumentLifecycleFailure, MainDocumentLifecycleStep,
    MainDocumentLifecycleTargetRejection,
};

use super::super::access::run_named_owner_local_task;
use super::{AwaitedOwnerLocalPageVm, MainDocumentLifecycleTaskRun, PageVm};

fn milestone_for_body(
    body: MainDocumentLifecycleBody,
) -> Option<RendererDocumentLifecycleMilestone> {
    match body.kind() {
        MainDocumentLifecycleBodyKind::Interactive => None,
        MainDocumentLifecycleBodyKind::DomContentLoaded => {
            Some(RendererDocumentLifecycleMilestone::DomContentLoaded)
        }
        MainDocumentLifecycleBodyKind::WindowLoad => Some(RendererDocumentLifecycleMilestone::Load),
    }
}

pub(super) async fn execute_main_document_lifecycle_on_owner_local_task(
    page_vm: &mut PageVm,
    body: MainDocumentLifecycleBody,
) -> Result<MainDocumentLifecycleTaskRun> {
    execute_main_document_lifecycle_body_on_owner_local_task(page_vm, body).await
}

async fn execute_main_document_lifecycle_body_on_owner_local_task(
    page_vm: &mut PageVm,
    body: MainDocumentLifecycleBody,
) -> Result<MainDocumentLifecycleTaskRun> {
    // Lifecycle callbacks must enter V8 from a fresh owner-local task, not a
    // deep phase-one poll stack. This is the same boundary used before A1.
    let local_executor = page_vm.local_executor.clone();
    let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(page_vm);
    run_named_owner_local_task(
        local_executor,
        "main-document lifecycle local task channel closed",
        async move {
            let page_vm = page_vm_ref.get_mut();
            page_vm.ensure_document_replacement_lifecycle_journal_is_valid()?;
            let document_lifecycle_identity = page_vm.document_lifecycle.identity();
            let milestone = milestone_for_body(body);

            let mut checkpoint_elapsed_ms = 0;

            let current_owner = page_vm.vm().current_main_document_task_owner();
            if current_owner != Some(body.owner()) {
                let completion = MainDocumentLifecycleCompletion::not_applied(
                    body,
                    MainDocumentLifecycleTargetRejection::TransitionRejected,
                    current_owner,
                );
                return Ok(MainDocumentLifecycleTaskRun {
                    completion,
                    checkpoint_elapsed_ms,
                    lifecycle_task_elapsed_ms: 0,
                    lifecycle_elapsed_ms: 0,
                });
            }

            let navigation_started_before_lifecycle_dispatch =
                page_vm.request_pending_cross_document_navigation_termination();
            if milestone.is_some() && navigation_started_before_lifecycle_dispatch {
                if page_vm.vm().current_main_document_task_owner() == Some(body.owner()) {
                    let checkpoint_started = Instant::now();
                    let checkpoint_result =
                        page_vm.vm_mut().finish_main_document_lifecycle_checkpoint();
                    checkpoint_elapsed_ms += checkpoint_started.elapsed().as_millis();
                    page_vm.vm_mut().finish_main_document_lifecycle_turn(());
                    checkpoint_result?;
                }
                let completion = MainDocumentLifecycleCompletion::skipped_for_pending_navigation(
                    body,
                    page_vm.vm().current_main_document_task_owner(),
                );
                return Ok(MainDocumentLifecycleTaskRun {
                    completion,
                    checkpoint_elapsed_ms,
                    lifecycle_task_elapsed_ms: 0,
                    lifecycle_elapsed_ms: 0,
                });
            }

            let lifecycle_dispatch_started = milestone.is_some_and(|milestone| {
                let transition = page_vm
                    .document_lifecycle
                    .begin_milestone_dispatch(document_lifecycle_identity, milestone);
                if transition != RendererDocumentLifecycleTransition::DispatchStarted {
                    tracing::debug!(
                        ?transition,
                        ?milestone,
                        "renderer lifecycle journal rejected milestone dispatch start"
                    );
                }
                transition == RendererDocumentLifecycleTransition::DispatchStarted
            });

            let lifecycle_task_started = Instant::now();
            let execution_result: Result<MainDocumentLifecycleExecution> = {
                let mut step = page_vm.vm_mut().begin_main_document_lifecycle_body(body);
                loop {
                    match step {
                        MainDocumentLifecycleStep::Completed(execution) => {
                            break Ok(page_vm
                                .vm_mut()
                                .finish_main_document_lifecycle_turn(execution));
                        }
                        MainDocumentLifecycleStep::Checkpoint(checkpoint) => {
                            let checkpoint_started = Instant::now();
                            let checkpoint_result =
                                page_vm.vm_mut().finish_main_document_lifecycle_checkpoint();
                            checkpoint_elapsed_ms += checkpoint_started.elapsed().as_millis();
                            if let Err(error) = checkpoint_result {
                                page_vm.vm_mut().finish_main_document_lifecycle_turn(());
                                break Err(error);
                            }
                            step = page_vm
                                .vm_mut()
                                .resume_main_document_lifecycle_after_checkpoint(checkpoint);
                        }
                    }
                }
            };
            let execution = match execution_result {
                Ok(execution) => execution,
                Err(error) => {
                    if lifecycle_dispatch_started && let Some(milestone) = milestone {
                        let _ = page_vm
                            .document_lifecycle
                            .cancel_milestone_dispatch(document_lifecycle_identity, milestone);
                    }
                    return Err(anyhow::anyhow!(
                        "main-document lifecycle checkpoint failed: {error:#}"
                    ));
                }
            };
            let completion: MainDocumentLifecycleCompletion = match execution.into_completion() {
                Ok(completion) => completion,
                Err(failure) => {
                    let failure: MainDocumentLifecycleFailure = failure;
                    tracing::debug!(
                        kind = ?failure.kind(),
                        owner = ?failure.owner(),
                        target = ?failure.target(),
                        callback = ?failure.callback(),
                        "main-document lifecycle body failed after typed execution"
                    );
                    if lifecycle_dispatch_started && let Some(milestone) = milestone {
                        let _ = page_vm
                            .document_lifecycle
                            .cancel_milestone_dispatch(document_lifecycle_identity, milestone);
                    }
                    return Err(anyhow::anyhow!(failure.into_message()));
                }
            };

            page_vm.request_pending_cross_document_navigation_termination();
            if lifecycle_dispatch_started && let Some(milestone) = milestone {
                if milestone == RendererDocumentLifecycleMilestone::DomContentLoaded {
                    page_vm.prepare_dom_agent_for_main_document_dom_content_loaded(
                        document_lifecycle_identity,
                    );
                    page_vm.record_document_title_change_if_needed();
                }
                let waits_for_descendants = milestone == RendererDocumentLifecycleMilestone::Load
                    && page_vm
                        .vm()
                        .current_main_document_load_completion_state(body.owner())
                        == Some(MainDocumentLoadCompletionState::WaitingForDescendants);
                let transition = if waits_for_descendants {
                    page_vm
                        .document_lifecycle
                        .defer_milestone_completion(document_lifecycle_identity, milestone)
                } else {
                    page_vm
                        .document_lifecycle
                        .complete_milestone_dispatch(document_lifecycle_identity, milestone)
                };
                if !matches!(
                    transition,
                    RendererDocumentLifecycleTransition::Recorded(_)
                        | RendererDocumentLifecycleTransition::Deferred
                ) {
                    tracing::debug!(
                        ?transition,
                        ?milestone,
                        waits_for_descendants,
                        "renderer lifecycle journal rejected milestone completion boundary"
                    );
                }
            }
            let lifecycle_task_elapsed_ms = lifecycle_task_started.elapsed().as_millis();

            let lifecycle_started = Instant::now();
            page_vm
                .vm_mut()
                .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
            let lifecycle_elapsed_ms = lifecycle_started.elapsed().as_millis();

            Ok(MainDocumentLifecycleTaskRun {
                completion,
                checkpoint_elapsed_ms,
                lifecycle_task_elapsed_ms,
                lifecycle_elapsed_ms,
            })
        },
    )
    .await
}
