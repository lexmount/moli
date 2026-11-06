use super::page_vm::ParseTimeLiveExecution;
use super::scaffold::debug_assert_phase_one_execution_context_for;
use super::*;
use crate::document_script_scheduler::{
    DocumentScriptExecutionLane, DocumentScriptSourceFailureLane, ParseTimeDocumentScriptEvent,
    ParseTimeDocumentScriptTask,
};

pub(super) struct DocumentTurnContext<'driver> {
    pub(super) scheduler: &'driver mut DocumentScriptScheduler,
    pub(super) parser_session: &'driver DocumentParserSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingParsingBlockingWake {
    ReadyNow,
    Source(crate::document_runtime::DocumentProcessingWakeSource),
    NoWake,
}

pub(super) fn pending_parsing_blocking_wake_prefers_ready_task_drain(
    wake: PendingParsingBlockingWake,
) -> bool {
    match wake {
        PendingParsingBlockingWake::ReadyNow | PendingParsingBlockingWake::NoWake => false,
        PendingParsingBlockingWake::Source(
            crate::document_runtime::DocumentProcessingWakeSource::InjectedPageTask
            | crate::document_runtime::DocumentProcessingWakeSource::TaskSourceLoadCompletion,
        ) => true,
    }
}

impl<'driver> DocumentTurnContext<'driver> {
    pub(super) async fn run_parse_time_turn(
        &mut self,
        page_vm: &mut PageVm,
    ) -> Result<PageTaskTurnResult> {
        debug_assert_phase_one_execution_context_for(
            &page_vm.local_executor,
            "parse-time document turns",
        );
        // A completed script/runtime task may have posted parser-boundary work
        // while this owner still held the PageVm. Materialize only work that is
        // already ready before arbitrating the next turn; future async arrivals
        // remain producer-driven and do not block parser progress.
        page_vm.page_task_queue.accept_ready_parse_time_wakes();
        if page_vm.has_ready_page_networking_task() {
            return Ok(PageTaskTurnResult::BlockedOnPageTask);
        }
        let parse_time_document_script_event = {
            let PageVm {
                page_task_queue, ..
            } = &mut *page_vm;
            let lifecycle_front = page_task_queue.parse_time_front();
            if lifecycle_front.is_none_or(|task| task.is_window_load_task()) {
                page_task_queue.parse_time_document_script_pop_front()
            } else {
                None
            }
        };
        if let Some(event) = parse_time_document_script_event {
            return self
                .run_parse_time_document_script_event(page_vm, event)
                .await;
        }

        let next_action = {
            let PageVm {
                vm,
                page_task_queue,
                ..
            } = &mut *page_vm;
            vm.as_mut()
                .expect("PageVm must retain a live ScriptVm until drop")
                .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
            self.parser_session
                .with_stylesheet_blocking_read_view(|document| {
                    vm.as_mut()
                        .expect("PageVm must retain a live ScriptVm until drop")
                        .document_runtime
                        .poll_document_processing_action(page_task_queue, Some(document))
                })
        };
        let Some(action) = next_action else {
            return Ok(PageTaskTurnResult::NoTask);
        };
        match action {
            DocumentProcessingAction::PostParsePageOwnedWork(work) => {
                return execute_page_owned_work_turn_on_local_task(page_vm, *work).await;
            }
            DocumentProcessingAction::DispatchConnectedStyleLoad(ready) => {
                let outcome = page_vm
                    .execute_parse_time_on_existing_live_document_on_named_owner_local_task(
                        ParseTimeLiveExecution::ConnectedStyleLoad { ready },
                    )
                    .await?;
                Ok(if outcome.navigation_triggered() {
                    PageTaskTurnResult::StoppedCurrentDocument
                } else {
                    PageTaskTurnResult::ExecutedTask
                })
            }
        }
    }

    async fn run_parse_time_document_script_event(
        &mut self,
        page_vm: &mut PageVm,
        event: ParseTimeDocumentScriptEvent,
    ) -> Result<PageTaskTurnResult> {
        match event {
            ParseTimeDocumentScriptEvent::AsyncCompletion(completion) => {
                let (node_id, outcome) = completion.into_parts();
                let ready_task = self
                    .scheduler
                    .accept_injected_parse_time_async_completion(node_id, outcome);
                page_vm
                    .page_task_queue
                    .enqueue_parse_time_document_script_task(ready_task);
                Ok(PageTaskTurnResult::ExecutedTask)
            }
            ParseTimeDocumentScriptEvent::ReadyTask(task) => {
                self.run_parse_time_document_script_task(page_vm, *task)
                    .await
            }
        }
    }

    async fn run_parse_time_document_script_task(
        &mut self,
        page_vm: &mut PageVm,
        task: ParseTimeDocumentScriptTask,
    ) -> Result<PageTaskTurnResult> {
        let outcome = match task {
            ParseTimeDocumentScriptTask::ClassicAsyncScript(script) => {
                let (script, network_result, load_delay_binding) = script.into_parts();
                execute_page_owned_document_script_turn_on_local_task(
                    page_vm,
                    DocumentScriptExecutionLane::ParseTimeAsync,
                    script,
                    network_result,
                    load_delay_binding,
                )
                .await?
            }
            ParseTimeDocumentScriptTask::AsyncScriptFailure(failure) => {
                let (script, error, source_network_result, load_delay_binding) =
                    failure.into_parts();
                execute_page_owned_document_script_failure_turn_on_local_task(
                    page_vm,
                    DocumentScriptSourceFailureLane::ParseTimeAsync,
                    script,
                    error,
                    source_network_result,
                    load_delay_binding,
                )
                .await?
            }
        };
        if matches!(outcome, PageTaskTurnResult::ExecutedTask) {
            let ParseTimeTurn {
                parser_step_bytes: _,
                ready_task,
            } = self
                .scheduler
                .parse_time_turn(ParseTimeTurnTrigger::AfterClassicAsyncTaskExecuted);
            page_vm
                .page_task_queue
                .enqueue_parse_time_document_script_task(ready_task);
        }
        Ok(outcome)
    }

    pub(super) async fn observe_pending_parsing_blocking_wake(
        &mut self,
        page_vm: &mut PageVm,
    ) -> PendingParsingBlockingWake {
        debug_assert_phase_one_execution_context_for(
            &page_vm.local_executor,
            "pending parsing-blocking wake observation",
        );
        page_vm
            .vm_mut()
            .document_runtime
            .drain_document_processing_wakes();
        let ready_now = {
            let PageVm {
                vm,
                page_task_queue,
                ..
            } = &mut *page_vm;
            vm.as_mut()
                .expect("PageVm must retain a live ScriptVm until drop")
                .document_runtime
                .has_ready_parse_time_document_processing_wake(page_task_queue)
        };
        if ready_now {
            return PendingParsingBlockingWake::ReadyNow;
        }
        // If all blocking stylesheets have already resolved (their completions
        // were consumed by an earlier drain), the pending parsing-blocking
        // script is no longer blocked. Return immediately so the caller can
        // re-evaluate instead of waiting on a channel message that will never
        // arrive.
        if page_vm
            .vm()
            .document_runtime
            .has_all_blocking_stylesheets_resolved()
        {
            return PendingParsingBlockingWake::ReadyNow;
        }
        let wake_source = {
            let PageVm {
                vm,
                page_task_queue,
                ..
            } = &mut *page_vm;
            vm.as_mut()
                .expect("PageVm must retain a live ScriptVm until drop")
                .document_runtime
                .wait_for_parse_time_document_processing_wake_source(page_task_queue)
                .await
        };
        match wake_source {
            Some(source) => {
                page_vm
                    .vm_mut()
                    .document_runtime
                    .drain_document_processing_wakes();
                PendingParsingBlockingWake::Source(source)
            }
            None => PendingParsingBlockingWake::NoWake,
        }
    }

    pub(super) async fn drain_parse_time_turns_until_idle(
        &mut self,
        page_vm: &mut PageVm,
        wait_for_wake: bool,
    ) -> Result<PageTaskTurnResult> {
        let mut ran_any_task = false;
        loop {
            match self.run_parse_time_turn(page_vm).await? {
                PageTaskTurnResult::NoTask => {
                    if wait_for_wake {
                        if page_vm
                            .vm()
                            .document_runtime
                            .has_pending_document_write_external_script_load()
                        {
                            if page_vm.wait_for_initial_page_runtime_wake().await {
                                continue;
                            }
                            return Ok(if ran_any_task {
                                PageTaskTurnResult::ExecutedTask
                            } else {
                                PageTaskTurnResult::NoTask
                            });
                        }
                        let pending_processing = {
                            let PageVm {
                                vm,
                                page_task_queue,
                                ..
                            } = &mut *page_vm;
                            vm.as_mut()
                                .expect("PageVm must retain a live ScriptVm until drop")
                                .document_runtime
                                .has_pending_parse_time_document_processing(page_task_queue)
                        };
                        // The scheduler's parse-visible async reevaluation credit is a parser
                        // boundary signal, not a document-processing blocker. It keeps the
                        // streaming input wait interested in parse-time injected tasks, but it
                        // must not make the document owner wait for a slow async fetch tail:
                        // classic async scripts do not delay DOMContentLoaded.
                        let has_expected_async_arrival = self
                            .scheduler
                            .has_outstanding_parse_visible_reevaluation_credit();
                        if pending_processing {
                            let probe_started =
                                moli_trace::dcl_wait_probe_enabled().then(std::time::Instant::now);
                            if probe_started.is_some() {
                                tracing::info!(
                                    target: "moli_dcl_wait_probe",
                                    wait_for_wake,
                                    pending_processing,
                                    has_expected_async_arrival,
                                    stage = "parse_time_document_wait_enter"
                                );
                            }
                            page_vm
                                .vm_mut()
                                .document_runtime
                                .drain_document_processing_wakes();
                            let ready_now = {
                                let PageVm {
                                    vm,
                                    page_task_queue,
                                    ..
                                } = &mut *page_vm;
                                vm.as_mut()
                                    .expect("PageVm must retain a live ScriptVm until drop")
                                    .document_runtime
                                    .has_ready_parse_time_document_processing_wake(page_task_queue)
                            };
                            if let Some(started) = probe_started {
                                tracing::info!(
                                    target: "moli_dcl_wait_probe",
                                    ready_now,
                                    elapsed_ms = started.elapsed().as_millis(),
                                    stage = "parse_time_document_wait_ready_probe"
                                );
                            }
                            if ready_now {
                                continue;
                            }
                            let wake_source = {
                                let PageVm {
                                    vm,
                                    page_task_queue,
                                    ..
                                } = &mut *page_vm;
                                vm.as_mut()
                                    .expect("PageVm must retain a live ScriptVm until drop")
                                    .document_runtime
                                    .wait_for_parse_time_document_processing_wake_source(
                                        page_task_queue,
                                    )
                                    .await
                            };
                            if let Some(started) = probe_started {
                                tracing::info!(
                                    target: "moli_dcl_wait_probe",
                                    ?wake_source,
                                    elapsed_ms = started.elapsed().as_millis(),
                                    stage = "parse_time_document_wait_wake"
                                );
                            };
                            if wake_source.is_some() {
                                page_vm
                                    .vm_mut()
                                    .document_runtime
                                    .drain_document_processing_wakes();
                                continue;
                            }
                        }
                    }
                    return Ok(if ran_any_task {
                        PageTaskTurnResult::ExecutedTask
                    } else {
                        PageTaskTurnResult::NoTask
                    });
                }
                PageTaskTurnResult::StoppedCurrentDocument => {
                    return Ok(PageTaskTurnResult::StoppedCurrentDocument);
                }
                PageTaskTurnResult::BlockedOnPageTask => {
                    return Ok(PageTaskTurnResult::BlockedOnPageTask);
                }
                PageTaskTurnResult::ExecutedTask => {
                    ran_any_task = true;
                }
            }
        }
    }
}
