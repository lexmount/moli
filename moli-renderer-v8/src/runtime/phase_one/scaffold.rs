use super::access::run_named_owner_local_task;
use super::loop_protocol::{ParseTimeOwnerCompletion, ParseTimePageVmStreamingProgress};
use super::*;

fn is_current_phase_one_execution_context_backend(local_executor: &JsLocalExecutor) -> bool {
    is_on_named_owner_execution_lane_for(local_executor)
}

async fn scope_phase_one_execution_context_backend<F>(
    local_executor: &JsLocalExecutor,
    future: F,
) -> F::Output
where
    F: std::future::Future,
{
    debug_assert!(
        is_on_named_owner_execution_lane_for(local_executor),
        "phase-one execution context must execute on the matching named owner lane"
    );
    future.await
}

fn prepare_phase_one_runtime_on_execution_context_backend(
    mut runtime: ConcurrentParseTimeRuntime,
) -> ConcurrentParseTimeRuntime {
    runtime
        .state
        .scheduler
        .bind_parse_time_completion_event_injection(
            runtime
                .page_vm
                .page_task_queue
                .parse_time_document_script_sender(),
            runtime.page_vm.runtime_hooks.owner_wake(),
        );
    runtime
        .state
        .scheduler
        .bind_owner_wake(runtime.page_vm.runtime_hooks.owner_wake());
    runtime
        .state
        .buffered_document_preloads
        .bind_resource_runtime(
            runtime.page_vm.runtime_hooks.owner_wake(),
            runtime.page_vm.runtime_hooks.resource_task_runner(),
        );
    runtime
}

pub(super) async fn run_phase_one_local_task<R, F>(
    local_executor: &JsLocalExecutor,
    operation: &'static str,
    future: F,
) -> Result<R>
where
    R: 'static,
    F: std::future::Future<Output = Result<R>> + 'static,
{
    let assert_executor = local_executor.clone();
    run_named_owner_local_task(
        local_executor.clone(),
        "phase-one page-vm local task channel closed",
        async move {
            debug_assert_phase_one_execution_context_for(&assert_executor, operation);
            future.await
        },
    )
    .await
}

pub(super) fn debug_assert_phase_one_execution_context_for(
    local_executor: &JsLocalExecutor,
    operation: &str,
) {
    debug_assert!(
        is_current_phase_one_execution_context_backend(local_executor),
        "{operation} must execute in the phase-one execution context"
    );
}

async fn run_phase_one_creation_session_on_execution_context(
    runtime: ConcurrentParseTimeRuntime,
    operation: &'static str,
) -> Result<ParseTimeOwnerCompletion> {
    let runtime = prepare_phase_one_runtime_on_execution_context_backend(runtime);
    let local_executor = runtime.page_vm.local_executor.clone();
    let run_executor = local_executor.clone();
    scope_phase_one_execution_context_backend(&local_executor, async move {
        debug_assert_phase_one_execution_context_for(&run_executor, operation);
        let mut runtime = runtime;
        let main_parser_continuation_is_admitted = runtime
            .page_vm
            .vm()
            .document_runtime
            .has_main_parser_continuation_admission();
        if !main_parser_continuation_is_admitted
            && (runtime.page_vm.has_ready_page_networking_task()
                || runtime
                    .page_vm
                    .vm()
                    .document_runtime
                    .has_pending_document_write_external_script_load())
        {
            // The unique typed consumer already lives in the owner-local
            // isolate reservation. Park creation so the stable Page arbiter
            // can select and apply the terminal after attach; phase one owns
            // neither dequeue authority nor a hidden legacy fallback.
            return Ok(ParseTimeOwnerCompletion::PendingPageTask(runtime));
        }
        if runtime.has_unready_pending_parser_blocking_source_load() {
            match runtime.run_one_page_creation_event_loop_turn().await? {
                PageTaskTurnResult::BlockedOnPageTask => {
                    return Ok(ParseTimeOwnerCompletion::PendingPageTask(runtime));
                }
                PageTaskTurnResult::StoppedCurrentDocument => {
                    return Ok(
                        match owner_step_progress_after_current_document_stop(&runtime.page_vm) {
                            OwnerStepProgress::TriggeredNavigation => {
                                let stage = runtime.stage;
                                ParseTimeOwnerCompletion::TriggeredNavigation {
                                    page_vm: Box::new(runtime.into_navigation_triggered_page_vm()),
                                    stage,
                                }
                            }
                            OwnerStepProgress::DocumentReplaced => {
                                ParseTimeOwnerCompletion::AdvancePhase {
                                    runtime,
                                    reason: ParseTimePhaseTransitionReason::DocumentReplaced,
                                }
                            }
                            _ => unreachable!(
                                "stopped parse-time document must navigate or install a replacement"
                            ),
                        },
                    );
                }
                PageTaskTurnResult::NoTask | PageTaskTurnResult::ExecutedTask => {}
            }
            if runtime.has_unready_pending_parser_blocking_source_load() {
                return Ok(ParseTimeOwnerCompletion::PendingParserBlockingSourceLoad(
                    runtime,
                ));
            }
        }
        ParseTimePhaseOnePump::new(runtime)
            .run_to_completion()
            .await
    })
    .await
}

pub(super) async fn finish_phase_one_creation_on_execution_context(
    runtime: ConcurrentParseTimeRuntime,
    started: Instant,
) -> Result<ParseTimePageVmCreationOutcome> {
    let completion =
        run_phase_one_creation_session_on_execution_context(runtime, "parse-time owner loop")
            .await?;

    match completion {
        ParseTimeOwnerCompletion::NeedMoreInput(runtime)
        | ParseTimeOwnerCompletion::PendingParserBlockingSourceLoad(runtime) => {
            if runtime.has_pending_parser_blocking_source_load() {
                Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(
                    PendingPhaseOneResidence::parser_blocking_source_load(
                        Box::new(runtime),
                        started,
                    ),
                ))
            } else {
                Err(anyhow!(
                    "full-body phase-one creation should not stall waiting for more input"
                ))
            }
        }
        ParseTimeOwnerCompletion::PendingPageTask(runtime) => {
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::closed_input_page_work(Box::new(runtime), started),
            ))
        }
        ParseTimeOwnerCompletion::TriggeredNavigation { page_vm, stage } => {
            Ok(ParseTimePageVmCreationOutcome::TriggeredNavigation {
                page_vm: *page_vm,
                stage,
            })
        }
        ParseTimeOwnerCompletion::AdvancePhase {
            mut runtime,
            reason,
        } => {
            if reason == ParseTimePhaseTransitionReason::ParserCompleted {
                runtime.state.scheduler.seal_parse_visible_async_cutoff();
            }
            let local_executor = runtime.page_vm.local_executor.clone();
            let (page_vm, page_tasks, stage, started) = run_phase_one_local_task(
                &local_executor,
                "parse-time phase transition handoff",
                async move { runtime.into_phase_two_execution(started, reason).await },
            )
            .await?;
            Ok(ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                page_vm,
                page_tasks,
                stage,
                started,
            })
        }
    }
}

pub(super) async fn continue_phase_one_until_streaming_boundary_on_execution_context(
    runtime: ConcurrentParseTimeRuntime,
    started: Instant,
) -> Result<ParseTimePageVmStreamingProgress> {
    let completion =
        run_phase_one_creation_session_on_execution_context(runtime, "parse-time owner loop")
            .await?;

    match completion {
        ParseTimeOwnerCompletion::NeedMoreInput(runtime)
        | ParseTimeOwnerCompletion::PendingParserBlockingSourceLoad(runtime) => Ok(
            ParseTimePageVmStreamingProgress::NeedMoreInput(Box::new(runtime)),
        ),
        ParseTimeOwnerCompletion::PendingPageTask(runtime) => Ok(
            ParseTimePageVmStreamingProgress::PendingPageTask(Box::new(runtime)),
        ),
        ParseTimeOwnerCompletion::TriggeredNavigation { page_vm, stage } => {
            Ok(ParseTimePageVmStreamingProgress::TriggeredNavigation {
                page_vm: *page_vm,
                stage,
            })
        }
        ParseTimeOwnerCompletion::AdvancePhase {
            mut runtime,
            reason,
        } => {
            if reason == ParseTimePhaseTransitionReason::ParserCompleted {
                runtime.state.scheduler.seal_parse_visible_async_cutoff();
            }
            let local_executor = runtime.page_vm.local_executor.clone();
            let (page_vm, page_tasks, stage, started) = run_phase_one_local_task(
                &local_executor,
                "parse-time phase transition handoff",
                async move { runtime.into_phase_two_execution(started, reason).await },
            )
            .await?;
            Ok(ParseTimePageVmStreamingProgress::ContinuePhaseTwo {
                page_vm,
                page_tasks,
                stage,
                started,
            })
        }
    }
}
