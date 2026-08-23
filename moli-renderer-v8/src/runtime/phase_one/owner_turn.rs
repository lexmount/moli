use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnerStepProgress {
    /// The current parse-time owner made progress and should run another step.
    Continue,
    /// The parser owns an external classic-script source load that is not
    /// terminal yet.
    BlockedOnParserScriptSourceLoad,
    /// An owner-attached Page has a concrete task in a stable source. Phase one
    /// must park instead of acquiring dequeue authority from the common Page
    /// scheduler.
    BlockedOnPageTask,
    /// The streaming parser consumed all currently buffered body input.
    NeedMoreInput,
    /// Phase one completed and can hand off to phase two.
    AdvancePhase,
    /// Script reentrancy installed a replacement Document.
    DocumentReplaced,
    /// Script execution requested a top-level navigation.
    TriggeredNavigation,
}

pub(super) fn owner_step_progress_after_current_document_stop(
    page_vm: &PageVm,
) -> OwnerStepProgress {
    if page_vm.vm().has_pending_location_navigation() {
        OwnerStepProgress::TriggeredNavigation
    } else {
        OwnerStepProgress::DocumentReplaced
    }
}

impl ConcurrentParseTimeRuntime {
    /// Selects the Document owner only when a stable parse-time source already
    /// contains a concrete payload.
    ///
    /// An open parser can otherwise be parked with a reevaluation credit whose
    /// producer has not completed yet. Routing that possibility to Document
    /// would bounce Parser and Document forever without runnable work.
    pub(super) fn admit_ready_open_stream_document_work(&mut self) -> bool {
        if !self
            .page_vm
            .page_task_queue
            .admit_ready_parse_time_document_work()
        {
            return false;
        }
        self.owner = ParseTimeOwner::Document;
        true
    }

    pub(super) async fn drive_owner_step(&mut self) -> Result<OwnerStepProgress> {
        self.admit_selected_main_parser_continuation();
        let ConcurrentParseTimeRuntime {
            loader,
            state,
            page_vm,
            owner,
            parser_step_ready,
            pending_parsing_blocking_wait,
            parser_document_owner,
            ..
        } = self;
        // Split the runtime once here so every owner step still runs against one
        // shared phase-1 object. Each owner step only carves out the narrower
        // parser/document view it needs; it does not create a second owner of
        // the underlying state.
        match *owner {
            ParseTimeOwner::Parser => {
                let mut parser_driver = ParserDriver {
                    loader,
                    final_url: &state.final_url,
                    parser_session: &mut state.parser_session,
                    scheduler: &mut state.scheduler,
                    pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                    buffered_document_preloads: &mut state.buffered_document_preloads,
                    service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                    input_closed: &state.input_closed,
                };
                parser_driver
                    .drive_owner_step(
                        owner,
                        parser_step_ready,
                        pending_parsing_blocking_wait,
                        *parser_document_owner,
                        page_vm,
                    )
                    .await
            }
            ParseTimeOwner::Document => {
                let parsing_blocking_wait = std::mem::replace(
                    pending_parsing_blocking_wait,
                    PendingParsingBlockingWait::None,
                );
                let has_pending_parsing_blocking_script = state
                    .pending_parsing_blocking_script
                    .has_parser_blocking_script();
                let mut context = DocumentTurnContext {
                    scheduler: &mut state.scheduler,
                    parser_session: &state.parser_session,
                };
                let wait_for_wake = parsing_blocking_wait.waits_for_legacy_document_processing();

                match context
                    .drain_parse_time_turns_until_idle(page_vm, wait_for_wake)
                    .await?
                {
                    PageTaskTurnResult::NoTask => {
                        let page_task_wait_is_pending = match parsing_blocking_wait {
                            PendingParsingBlockingWait::PageTaskBlockingStylesheet => {
                                !page_vm
                                    .vm()
                                    .document_runtime
                                    .has_all_blocking_stylesheets_resolved()
                            }
                            PendingParsingBlockingWait::PageNetworkingDocumentWriteExternalScript => {
                                page_vm
                                    .vm()
                                    .document_runtime
                                    .has_pending_document_write_external_script_load()
                            }
                            PendingParsingBlockingWait::None
                            | PendingParsingBlockingWait::LegacyDocumentProcessing => false,
                        };
                        if page_task_wait_is_pending {
                            *pending_parsing_blocking_wait = parsing_blocking_wait;
                            return Ok(OwnerStepProgress::BlockedOnPageTask);
                        }
                        if has_pending_parsing_blocking_script {
                            if !pending_parsing_blocking_wake_prefers_ready_task_drain(
                                context.observe_pending_parsing_blocking_wake(page_vm).await,
                            ) {
                                *owner = ParseTimeOwner::Parser;
                                return Ok(OwnerStepProgress::Continue);
                            }

                            *owner = ParseTimeOwner::Parser;
                            return match context
                                .drain_parse_time_turns_until_idle(page_vm, true)
                                .await?
                            {
                                PageTaskTurnResult::BlockedOnPageTask => {
                                    *pending_parsing_blocking_wait = parsing_blocking_wait;
                                    Ok(OwnerStepProgress::BlockedOnPageTask)
                                }
                                PageTaskTurnResult::StoppedCurrentDocument => {
                                    Ok(owner_step_progress_after_current_document_stop(page_vm))
                                }
                                PageTaskTurnResult::NoTask | PageTaskTurnResult::ExecutedTask => {
                                    Ok(OwnerStepProgress::Continue)
                                }
                            };
                        }

                        let mut drained_once = false;
                        loop {
                            let disposition = context.scheduler.plan_parse_visible_ready_turn(
                                ParseVisibleReadyTurnPhase::Parsing,
                                drained_once,
                            );
                            match disposition {
                                ParseVisibleReadyTurnDisposition::DrainReadyTasks => {
                                    match context
                                        // This is a readiness probe, not a wake wait.
                                        // Streaming input selection is already waiting on
                                        // parse-time injected task arrival when a credit is
                                        // outstanding; waiting here would let slow async script
                                        // tails keep ownership away from ready parser chunks.
                                        .drain_parse_time_turns_until_idle(page_vm, false)
                                        .await?
                                    {
                                        PageTaskTurnResult::BlockedOnPageTask => {
                                            *pending_parsing_blocking_wait = parsing_blocking_wait;
                                            return Ok(OwnerStepProgress::BlockedOnPageTask);
                                        }
                                        PageTaskTurnResult::NoTask => {
                                            drained_once = true;
                                            continue;
                                        }
                                        PageTaskTurnResult::ExecutedTask => {
                                            *owner = ParseTimeOwner::Parser;
                                            return Ok(OwnerStepProgress::Continue);
                                        }
                                        PageTaskTurnResult::StoppedCurrentDocument => {
                                            return Ok(
                                                owner_step_progress_after_current_document_stop(
                                                    page_vm,
                                                ),
                                            );
                                        }
                                    }
                                }
                                ParseVisibleReadyTurnDisposition::YieldToParserBoundary => {
                                    // Recheck the parser as soon as control returns. The
                                    // reevaluation credit remains armed until the async
                                    // completion is consumed, but ready document input should
                                    // still advance between wakeups.
                                    *parser_step_ready = true;
                                    *owner = ParseTimeOwner::Parser;
                                    return Ok(OwnerStepProgress::Continue);
                                }
                                ParseVisibleReadyTurnDisposition::FinishNoTask => {
                                    break;
                                }
                            }
                        }

                        if parsing_blocking_wait.is_pending() {
                            // A stylesheet parser boundary (with or without a
                            // following script) resumes only after document-owned
                            // work has gone fully idle and every blocker settled.
                            match context.run_parse_time_turn(page_vm).await? {
                                PageTaskTurnResult::BlockedOnPageTask => {
                                    *pending_parsing_blocking_wait = parsing_blocking_wait;
                                    Ok(OwnerStepProgress::BlockedOnPageTask)
                                }
                                PageTaskTurnResult::StoppedCurrentDocument => {
                                    Ok(owner_step_progress_after_current_document_stop(page_vm))
                                }
                                PageTaskTurnResult::NoTask => {
                                    if parsing_blocking_wait
                                        == PendingParsingBlockingWait::PageTaskBlockingStylesheet
                                        && !page_vm
                                            .vm()
                                            .document_runtime
                                            .has_all_blocking_stylesheets_resolved()
                                    {
                                        *pending_parsing_blocking_wait = parsing_blocking_wait;
                                        *owner = ParseTimeOwner::Document;
                                        return Ok(OwnerStepProgress::Continue);
                                    }
                                    *parser_step_ready = true;
                                    *owner = ParseTimeOwner::Parser;
                                    Ok(OwnerStepProgress::Continue)
                                }
                                PageTaskTurnResult::ExecutedTask => {
                                    *pending_parsing_blocking_wait = parsing_blocking_wait;
                                    *owner = ParseTimeOwner::Document;
                                    Ok(OwnerStepProgress::Continue)
                                }
                            }
                        } else {
                            Ok(
                                if state.parser_session.input_is_empty()
                                    && !state.parser_session.has_script_input()
                                {
                                    if state.input_closed {
                                        // Document work can expose EOF, but
                                        // only the phase-one parser owner may
                                        // request and claim finalization. Route
                                        // through it so the exact continuation
                                        // boundary remains mandatory.
                                        *parser_step_ready = true;
                                        *owner = ParseTimeOwner::Parser;
                                        OwnerStepProgress::Continue
                                    } else {
                                        *owner = ParseTimeOwner::Parser;
                                        OwnerStepProgress::NeedMoreInput
                                    }
                                } else {
                                    *parser_step_ready = true;
                                    *owner = ParseTimeOwner::Parser;
                                    OwnerStepProgress::Continue
                                },
                            )
                        }
                    }
                    other => match other {
                        PageTaskTurnResult::NoTask => {
                            unreachable!("non-idle page-task drain must map to parse-time progress")
                        }
                        PageTaskTurnResult::BlockedOnPageTask => {
                            *pending_parsing_blocking_wait = parsing_blocking_wait;
                            Ok(OwnerStepProgress::BlockedOnPageTask)
                        }
                        PageTaskTurnResult::StoppedCurrentDocument => {
                            Ok(owner_step_progress_after_current_document_stop(page_vm))
                        }
                        PageTaskTurnResult::ExecutedTask => {
                            if parsing_blocking_wait.is_pending() {
                                *pending_parsing_blocking_wait = parsing_blocking_wait;
                                *owner = ParseTimeOwner::Document;
                            } else {
                                *owner = ParseTimeOwner::Parser;
                            }
                            Ok(OwnerStepProgress::Continue)
                        }
                    },
                }
            }
        }
    }
}
