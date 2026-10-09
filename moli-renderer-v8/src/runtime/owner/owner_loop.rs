use super::*;

/// Scheduling facts must reach the deadline index before arbitration reads it.
/// Runnable source wakes retain their FIFO and one-turn admission boundary.
pub(super) struct RendererOwnerWakeReceiver {
    receiver: mpsc::UnboundedReceiver<RendererOwnerWake>,
    pending: VecDeque<RendererOwnerWake>,
}

impl RendererOwnerWakeReceiver {
    fn synchronize_deadlines(&mut self) {
        let mut changed = std::collections::HashSet::new();
        // Timer producers run on this owner thread. All their changes are
        // already queued; concurrent producers cannot extend this drain forever.
        for _ in 0..self.receiver.len() {
            let Ok(wake) = self.receiver.try_recv() else {
                break;
            };
            match wake {
                RendererOwnerWake::PageTaskDeadlineChanged { token } => {
                    changed.insert(token);
                }
                wake => self.pending.push_back(wake),
            }
        }
        for token in changed {
            reindex_page_deadline_on_bound_owner_local_store(token);
        }
    }

    pub(super) fn try_recv(&mut self) -> Result<RendererOwnerWake, mpsc::error::TryRecvError> {
        self.pending
            .pop_front()
            .map_or_else(|| self.receiver.try_recv(), Ok)
    }

    async fn recv(&mut self) -> Option<RendererOwnerWake> {
        if let Some(wake) = self.pending.pop_front() {
            Some(wake)
        } else {
            self.receiver.recv().await
        }
    }
}

impl RendererOwnerHandle {
    pub(super) fn enqueue_inspector_main_receiver_wake(
        wake: RendererInspectorMainOwnerWake,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        pending_turns.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunInspectorMainReceiver { wake },
            allow_command_overtake: false,
            command_admission_output_predecessor: None,
        });
    }

    pub(crate) async fn run_render_runtime_loop(
        &self,
        mut rx: mpsc::UnboundedReceiver<RenderRuntimeEnvelope>,
        page_wake_rx: mpsc::UnboundedReceiver<RendererOwnerWake>,
        mut inspector_io_wake_rx: mpsc::UnboundedReceiver<RendererInspectorIoOwnerWake>,
        mut shared_worker_wake_rx: mpsc::UnboundedReceiver<SharedWorkerRuntimeOwnerWake>,
        mut service_worker_wake_rx: mpsc::UnboundedReceiver<ServiceWorkerRuntimeOwnerWake>,
    ) {
        let loop_future = async {
            let mut owner_local_store = RendererOwnerLocalStore::default();
            let _owner_local_store_binding =
                bind_render_runtime_owner_local_store(&mut owner_local_store);
            let mut pending_turns = RenderRuntimePendingTurnQueue::default();
            let mut parked_turns: VecDeque<RenderRuntimeParkedTurn> = VecDeque::new();
            let mut page_wake_rx = RendererOwnerWakeReceiver {
                receiver: page_wake_rx,
                pending: VecDeque::new(),
            };
            // A ready producer and an expired deadline are two admission
            // reasons for the same Page scheduler, not two priority classes.
            // Alternate which one is polled first so neither can remain
            // hidden behind a continuously ready command queue or the other
            // admission reason.
            let mut page_admission_preference = PageTurnAdmissionPreference::ProducerWake;
            loop {
                page_wake_rx.synchronize_deadlines();
                if self.context_shutdown_started() {
                    while let Some(pending_turn) = pending_turns.pop_front() {
                        self.cancel_pending_turn_on_owner_local_store(pending_turn.turn);
                        if let Some(reply_tx) = pending_turn.reply_tx {
                            let _ = reply_tx.send(Err(anyhow!(
                                "renderer browser context was dropped while work was pending"
                            )));
                        }
                    }
                    for parked_turn in parked_turns.drain(..) {
                        self.cancel_pending_turn_on_owner_local_store(parked_turn.turn);
                        if let Some(reply_tx) = parked_turn.reply_tx {
                            let _ = reply_tx.send(Err(anyhow!(
                                "renderer browser context was dropped while work was parked"
                            )));
                        }
                    }
                    while let Ok(envelope) = rx.try_recv() {
                        if let RenderRuntimeEnvelope::Command { command, reply_tx } = envelope {
                            self.release_rejected_command_output_reservation(&command);
                            let _ = reply_tx.send(Err(anyhow!(
                                "renderer browser context was dropped before command dispatch"
                            )));
                        }
                    }
                    break;
                }
                self.enqueue_due_parked_turns(&mut parked_turns, &mut pending_turns);
                // Commands are also represented as bounded pending turns. A
                // Page wake/deadline must be admitted before selecting the
                // next pending turn, otherwise a stream of command
                // continuations can keep the Page scheduler invisible even
                // though its durable source is ready. At most one Page turn
                // is admitted here; that turn itself still lets one ready
                // command overtake at its owner boundary.
                if !pending_turns.has_page_owner_turn() {
                    match self.try_admit_ready_page_turn(
                        &mut page_admission_preference,
                        &mut page_wake_rx,
                        &mut parked_turns,
                        &mut pending_turns,
                    ) {
                        ReadyPageTurnAdmission::Admitted | ReadyPageTurnAdmission::NoneReady => {}
                        ReadyPageTurnAdmission::WakeChannelClosed => break,
                    }
                }
                // Renderer housekeeping is a separate owner lane. It is
                // admitted after Page work so an expired memory-maintenance
                // deadline cannot masquerade as, or outrank, an HTML task.
                if !pending_turns.has_owner_maintenance_turn() {
                    self.enqueue_one_due_owner_maintenance_turn(&mut pending_turns);
                }
                // IO Inspector ingress is independent of normal owner command
                // admission. Give at most one command an owner execution
                // chance at each boundary, then continue with the ordinary
                // pending turn so sustained IO cannot starve MainThread work.
                match inspector_io_wake_rx.try_recv() {
                    Ok(wake) => dispatch_inspector_io_owner_wake(wake),
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => {}
                }
                if let Some(mut pending_turn) = pending_turns.pop_front() {
                    if pending_turn.allow_command_overtake
                        && pending_turn.turn.page_turn_should_yield_to_ready_command()
                    {
                        // A protocol reply may observe page progress, but it does not
                        // own the page scheduler. Every bounded page turn returns the
                        // entry to its stable slot, so already-arrived commands can be
                        // admitted before the same continuation runs another turn.
                        match rx.try_recv() {
                            Ok(envelope) => {
                                pending_turn.allow_command_overtake = false;
                                pending_turns.push_front(pending_turn);
                                self.dispatch_envelope_on_owner_local_store(
                                    envelope,
                                    &mut owner_local_store,
                                    &mut pending_turns,
                                    &mut parked_turns,
                                )
                                .await;
                                continue;
                            }
                            Err(mpsc::error::TryRecvError::Empty) => {}
                            Err(mpsc::error::TryRecvError::Disconnected) => {}
                        }
                    }
                    if pending_turn
                        .reply_tx
                        .as_ref()
                        .is_some_and(|tx| tx.is_closed())
                    {
                        self.detach_navigation_or_cancel_pending_turn(
                            pending_turn.turn,
                            &mut pending_turns,
                        );
                        continue;
                    }
                    if let Some(token) = pending_turn.turn.committed_page_view_command_token()
                        && parked_turns.iter().any(|turn| {
                            turn.wake_token == token
                                && matches!(
                                    turn.condition,
                                    RenderRuntimeParkCondition::CommittedDocumentParserContinuation {
                                        ..
                                    }
                                )
                        })
                    {
                        // A command observed the committed Page through the
                        // browser-side response path, but its parser handoff
                        // has not run yet. Park the command on the same durable
                        // Page activity edge. The response-release wake puts
                        // the parser turn and continuation in front of it.
                        parked_turns.push_back(RenderRuntimeParkedTurn {
                            reply_tx: pending_turn.reply_tx,
                            turn: pending_turn.turn,
                            wake_token: token,
                            ready_at: None,
                            condition: RenderRuntimeParkCondition::PageActivity,
                            command_admission_output_predecessor: pending_turn
                                .command_admission_output_predecessor
                                .take(),
                        });
                        continue;
                    }
                    if let Some(token) = pending_turn.turn.committed_page_view_command_token()
                        && let Some(expected_vm_creation_id) =
                            owner_local_store.page_uncommitted_vm_creation_id(token)
                        && pending_turn
                            .turn
                            .committed_page_view_deadline()
                            .is_none_or(|deadline| deadline > Instant::now())
                    {
                        tracing::trace!(
                            page_id = token.page_id.as_u64(),
                            expected_vm_creation_id,
                            "parked page command until the replacement PageVm commits"
                        );
                        let ready_at = pending_turn.turn.committed_page_view_deadline();
                        parked_turns.push_back(RenderRuntimeParkedTurn {
                            reply_tx: pending_turn.reply_tx,
                            turn: pending_turn.turn,
                            wake_token: token,
                            ready_at,
                            condition:
                                RenderRuntimeParkCondition::ReplacementDocumentViewSettlement {
                                    expected_vm_creation_id,
                                },
                            command_admission_output_predecessor: pending_turn
                                .command_admission_output_predecessor
                                .take(),
                        });
                        continue;
                    }
                    let completed_page_turn_token = pending_turn.page_owner_token();
                    let outcome = self
                        .run_pending_turn_on_owner_local_store(pending_turn.turn)
                        .await;
                    let parser_continuation_admitted = matches!(
                        &outcome,
                        RenderRuntimeDispatchOutcome::PageTurnComplete {
                            parser_continuation_admitted: true,
                            ..
                        }
                    );
                    match outcome {
                        RenderRuntimeDispatchOutcome::Reply(result) => {
                            let result = merge_command_admission_output_predecessor(
                                *result,
                                pending_turn.command_admission_output_predecessor.take(),
                            );
                            if let Some(reply_tx) = pending_turn.reply_tx {
                                let _ = reply_tx.send(result);
                            } else if let Err(error) = result {
                                tracing::debug!(
                                    "background pending turn finished with error: {error}"
                                );
                            }
                        }
                        RenderRuntimeDispatchOutcome::InspectorMainCommandClaimed {
                            reply_tx,
                            turn,
                            command_admission_output_predecessor,
                        } => {
                            debug_assert!(pending_turn.reply_tx.is_none());
                            pending_turns.push_front(RenderRuntimePendingTurn {
                                reply_tx: Some(reply_tx),
                                turn: *turn,
                                allow_command_overtake: false,
                                command_admission_output_predecessor,
                            });
                        }
                        RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                            mut page,
                            continuation,
                        } => {
                            match pending_turn.reply_tx {
                                Some(reply_tx) => {
                                    let token = page.token;
                                    if continuation.requires_committed_document_response_release() {
                                        page.defer_committed_document_parser_until_response(
                                            self.state.page_wake_tx.clone(),
                                        );
                                    }
                                    if reply_tx
                                        .send(Ok(RendererOwnerReply::PageCreated(page)))
                                        .is_ok()
                                    {
                                        self.enqueue_page_creation_continuation(
                                            continuation,
                                            &mut pending_turns,
                                            &mut parked_turns,
                                        );
                                    } else {
                                        // The attached Page never crossed its
                                        // ownership handoff, so there is no
                                        // browser-side handle that can retire
                                        // it later. This differs from a live
                                        // navigation command observer going
                                        // away after Page publication: that
                                        // continuation is detached below and
                                        // keeps running in the background.
                                        remove_page_on_bound_owner_local_store(token);
                                    }
                                }
                                None => {
                                    // A Page already published at an earlier
                                    // reply boundary owns this background
                                    // navigation. Its replacement commit may
                                    // produce another attached-page boundary,
                                    // but there is intentionally no command
                                    // observer to notify. Keep the renderer
                                    // continuation alive instead of treating
                                    // the absent observer as a cancelled
                                    // initial Page creation.
                                    drop(page);
                                    pending_turns.push_back(RenderRuntimePendingTurn {
                                        reply_tx: None,
                                        turn: continuation.into_turn(),
                                        allow_command_overtake: true,
                                        command_admission_output_predecessor: None,
                                    });
                                }
                            }
                        }
                        RenderRuntimeDispatchOutcome::PageReplacementCommittedAndContinueNavigation { mut replacement, continuation } => {
                            if continuation.requires_committed_document_response_release() {
                                replacement.defer_committed_document_parser_until_response(self.state.page_wake_tx.clone());
                            }
                            if let Some(reply_tx) = pending_turn.reply_tx {
                                let _ = reply_tx.send(Ok(RendererOwnerReply::PageReplacementCommitted(replacement)));
                            }
                            // An existing Page retains its close owner even if this
                            // navigation's observer has gone away. Continue its parser.
                            self.enqueue_page_creation_continuation(continuation, &mut pending_turns, &mut parked_turns);
                        }
                        RenderRuntimeDispatchOutcome::BackgroundComplete(result)
                        | RenderRuntimeDispatchOutcome::PageTurnComplete { result, .. } => {
                            if let Some(reply_tx) = pending_turn.reply_tx {
                                let reply = match result {
                                    Ok(()) => Err(anyhow!(
                                        "command-owned turn unexpectedly completed as background work"
                                    )),
                                    Err(error) => Err(anyhow!(
                                        "command-owned turn unexpectedly completed as background work: {error}"
                                    )),
                                };
                                let _ = reply_tx.send(reply);
                            } else if let Err(error) = result {
                                tracing::debug!(
                                    "background pending turn finished with error: {error}"
                                );
                            }
                        }
                        RenderRuntimeDispatchOutcome::PageCreationNavigationFailurePublished {
                            token,
                            failure,
                        } => {
                            if let Some(reply_tx) = pending_turn.reply_tx {
                                let _ = reply_tx.send(Err(anyhow!(failure.to_string())));
                            } else {
                                tracing::debug!(
                                    "background navigation published a concrete failure: {failure}"
                                );
                            }
                            self.enqueue_parked_turns_for_wake(
                                token,
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                        RenderRuntimeDispatchOutcome::ContinueNextTurn(turn) => {
                            if pending_turn
                                .reply_tx
                                .as_ref()
                                .is_some_and(|tx| tx.is_closed())
                            {
                                self.detach_navigation_or_cancel_pending_turn(
                                    *turn,
                                    &mut pending_turns,
                                );
                            } else {
                                pending_turns.push_back(RenderRuntimePendingTurn {
                                    reply_tx: pending_turn.reply_tx,
                                    turn: *turn,
                                    allow_command_overtake: true,
                                    command_admission_output_predecessor: pending_turn
                                        .command_admission_output_predecessor
                                        .take(),
                                });
                            }
                        }
                        RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                            turn,
                            wake_token,
                            ready_at,
                        } => {
                            self.park_or_cancel_turn(
                                pending_turn.reply_tx,
                                *turn,
                                wake_token,
                                Some(ready_at),
                                RenderRuntimeParkCondition::PageActivity,
                                pending_turn.command_admission_output_predecessor.take(),
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                        RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                            turn,
                            wake_token,
                        } => {
                            self.park_or_cancel_turn(
                                pending_turn.reply_tx,
                                *turn,
                                wake_token,
                                None,
                                RenderRuntimeParkCondition::PageActivity,
                                pending_turn.command_admission_output_predecessor.take(),
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                        RenderRuntimeDispatchOutcome::ContinueCommittedDocumentParserAfterPageWake {
                            turn,
                            wake_token,
                        } => {
                            self.park_or_cancel_turn(
                                pending_turn.reply_tx,
                                *turn,
                                wake_token,
                                None,
                                RenderRuntimeParkCondition::CommittedDocumentParserContinuation {
                                    parser_unblocked: true,
                                },
                                pending_turn.command_admission_output_predecessor.take(),
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                    }
                    if let Some(token) = completed_page_turn_token {
                        if parser_continuation_admitted {
                            self.promote_admitted_phase_one_parser_continuation(
                                token,
                                &mut parked_turns,
                                &mut pending_turns,
                            );
                        }
                        // A bounded Page turn may have published the exact
                        // lifecycle milestone page creation is waiting for.
                        // Re-admit only that one-shot observer at this owner
                        // boundary, before a producer wake can advance the
                        // Page through a follow-up turn. Other waits retain
                        // their existing wake/deadline contracts.
                        self.enqueue_parked_page_creation_observers_after_page_turn(
                            token,
                            &mut parked_turns,
                            &mut pending_turns,
                        );
                    }
                    continue;
                }

                // SharedWorker service-lane completions can make a page-visible
                // command result ready. Do not let sustained CDP polling starve
                // this owner-level wake behind the command queue.
                match shared_worker_wake_rx.try_recv() {
                    Ok(wake) => {
                        self.handle_shared_worker_runtime_wake(wake, &mut pending_turns);
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }

                match service_worker_wake_rx.try_recv() {
                    Ok(wake) => {
                        self.handle_service_worker_runtime_wake(wake, &mut pending_turns);
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }

                match rx.try_recv() {
                    Ok(envelope) => {
                        self.dispatch_envelope_on_owner_local_store(
                            envelope,
                            &mut owner_local_store,
                            &mut pending_turns,
                            &mut parked_turns,
                        )
                        .await;
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }

                let next_owner_deadline = earliest_deadline(
                    earliest_deadline(
                        self.compute_next_page_task_deadline(),
                        self.compute_next_owner_maintenance_deadline(),
                    ),
                    self.next_parked_turn_deadline(&parked_turns),
                );
                tokio::select! {
                    _ = self.state.context_shutdown_notify.notified() => {}
                    envelope_opt = rx.recv() => {
                        let Some(envelope) = envelope_opt else {
                            break;
                        };
                        self.dispatch_envelope_on_owner_local_store(
                            envelope,
                            &mut owner_local_store,
                            &mut pending_turns,
                            &mut parked_turns,
                        )
                        .await;
                    }
                    wake_opt = page_wake_rx.recv() => {
                        let Some(wake) = wake_opt else {
                            break;
                        };
                        page_admission_preference = PageTurnAdmissionPreference::Deadline;
                        self.handle_page_owner_wake(
                            wake,
                            &mut parked_turns,
                            &mut pending_turns,
                        );
                    }
                    inspector_io_wake_opt = inspector_io_wake_rx.recv() => {
                        let Some(wake) = inspector_io_wake_opt else {
                            break;
                        };
                        dispatch_inspector_io_owner_wake(wake);
                    }
                    shared_worker_wake_opt = shared_worker_wake_rx.recv() => {
                        let Some(wake) = shared_worker_wake_opt else {
                            break;
                        };
                        self.handle_shared_worker_runtime_wake(wake, &mut pending_turns);
                    }
                    service_worker_wake_opt = service_worker_wake_rx.recv() => {
                        let Some(wake) = service_worker_wake_opt else {
                            break;
                        };
                        self.handle_service_worker_runtime_wake(wake, &mut pending_turns);
                    }
                    _ = sleep_until_or_forever(next_owner_deadline) => {
                        self.enqueue_due_parked_turns(&mut parked_turns, &mut pending_turns);
                        if self.enqueue_due_page_turns(&mut pending_turns) {
                            page_admission_preference = PageTurnAdmissionPreference::ProducerWake;
                        }
                        self.enqueue_one_due_owner_maintenance_turn(&mut pending_turns);
                    }
                }
            }
        };
        loop_future.await
    }

    pub(super) async fn dispatch_envelope_on_owner_local_store(
        &self,
        envelope: RenderRuntimeEnvelope,
        owner_local_store: &mut RendererOwnerLocalStore,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
    ) {
        let (command, reply_tx) = match envelope {
            RenderRuntimeEnvelope::Command { command, reply_tx } => (*command, reply_tx),
            RenderRuntimeEnvelope::InspectorMainReceiverWake(wake) => {
                Self::enqueue_inspector_main_receiver_wake(wake, pending_turns);
                return;
            }
        };
        #[cfg(test)]
        self.wait_on_command_dispatch_gate_for_testing();
        if self.context_shutdown_started() {
            self.release_rejected_command_output_reservation(&command);
            let _ = reply_tx.send(Err(anyhow!(
                "renderer browser context was dropped before command dispatch"
            )));
            return;
        }
        if reply_tx.is_closed()
            && matches!(
                &command,
                RendererOwnerCommand::RunAsyncPageCommand { command, .. }
                    | RendererOwnerCommand::RunProtocolPageCommand { command, .. }
                    if command.interruptible_by_javascript_dialog()
            )
        {
            return;
        }
        let removed_page_token = match &command {
            RendererOwnerCommand::RemovePage { token } => Some(*token),
            _ => None,
        };
        let mut command_admission_output_predecessor =
            renderer_command_admission_page_token(&command)
                .and_then(renderer_output_fence_for_tail_on_bound_owner_local_store);
        let command_future =
            Box::pin(self.dispatch_command_inline_on_owner_local_store(command, owner_local_store));
        let outcome = command_future.await;
        if self.context_shutdown_started() {
            self.cancel_dispatch_outcome_for_context_shutdown(outcome);
            let _ = reply_tx.send(Err(anyhow!(
                "renderer browser context was dropped during command dispatch"
            )));
            return;
        }
        match outcome {
            RenderRuntimeDispatchOutcome::Reply(result) => {
                let result = merge_command_admission_output_predecessor(
                    *result,
                    command_admission_output_predecessor.take(),
                );
                let _ = reply_tx.send(result);
            }
            RenderRuntimeDispatchOutcome::InspectorMainCommandClaimed {
                reply_tx: main_reply_tx,
                turn,
                command_admission_output_predecessor: _,
            } => {
                self.cancel_pending_turn_on_owner_local_store(*turn);
                let _ = main_reply_tx.send(Err(anyhow!(
                    "Inspector Main receiver outcome escaped into owner command dispatch"
                )));
                let _ = reply_tx.send(Err(anyhow!(
                    "renderer command unexpectedly entered the Inspector Main receiver"
                )));
            }
            RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                mut page,
                continuation,
            } => {
                let token = page.token;
                if continuation.requires_committed_document_response_release() {
                    page.defer_committed_document_parser_until_response(
                        self.state.page_wake_tx.clone(),
                    );
                }
                if reply_tx
                    .send(Ok(RendererOwnerReply::PageCreated(page)))
                    .is_ok()
                {
                    self.enqueue_page_creation_continuation(
                        continuation,
                        pending_turns,
                        parked_turns,
                    );
                } else {
                    remove_page_on_bound_owner_local_store(token);
                }
            }
            RenderRuntimeDispatchOutcome::PageReplacementCommittedAndContinueNavigation {
                mut replacement,
                continuation,
            } => {
                if continuation.requires_committed_document_response_release() {
                    replacement.defer_committed_document_parser_until_response(
                        self.state.page_wake_tx.clone(),
                    );
                }
                let _ = reply_tx.send(Ok(RendererOwnerReply::PageReplacementCommitted(
                    replacement,
                )));
                self.enqueue_page_creation_continuation(continuation, pending_turns, parked_turns);
            }
            RenderRuntimeDispatchOutcome::BackgroundComplete(result)
            | RenderRuntimeDispatchOutcome::PageTurnComplete { result, .. } => {
                let reply = match result {
                    Ok(()) => Err(anyhow!(
                        "renderer command unexpectedly completed as background work"
                    )),
                    Err(error) => Err(anyhow!(
                        "renderer command unexpectedly completed as background work: {error}"
                    )),
                };
                let _ = reply_tx.send(reply);
            }
            RenderRuntimeDispatchOutcome::PageCreationNavigationFailurePublished {
                token,
                failure,
            } => {
                let _ = reply_tx.send(Err(anyhow!(failure.to_string())));
                self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
            }
            RenderRuntimeDispatchOutcome::ContinueNextTurn(turn) => {
                if reply_tx.is_closed() {
                    self.detach_navigation_or_cancel_pending_turn(*turn, pending_turns);
                } else {
                    pending_turns.push_back(RenderRuntimePendingTurn {
                        reply_tx: Some(reply_tx),
                        turn: *turn,
                        // This is the command's first owner turn, not a
                        // continuation returning to the queue. Run it before
                        // admitting another envelope so a Page wake produced
                        // by the command cannot inherit an already-pending
                        // command and then yield to a second one.
                        allow_command_overtake: false,
                        command_admission_output_predecessor: command_admission_output_predecessor
                            .take(),
                    });
                }
            }
            RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
                turn,
                wake_token,
                ready_at,
            } => {
                self.park_or_cancel_turn(
                    Some(reply_tx),
                    *turn,
                    wake_token,
                    Some(ready_at),
                    RenderRuntimeParkCondition::PageActivity,
                    command_admission_output_predecessor.take(),
                    parked_turns,
                    pending_turns,
                );
            }
            RenderRuntimeDispatchOutcome::ContinueAfterPageWake { turn, wake_token } => {
                self.park_or_cancel_turn(
                    Some(reply_tx),
                    *turn,
                    wake_token,
                    None,
                    RenderRuntimeParkCondition::PageActivity,
                    command_admission_output_predecessor.take(),
                    parked_turns,
                    pending_turns,
                );
            }
            RenderRuntimeDispatchOutcome::ContinueCommittedDocumentParserAfterPageWake {
                turn,
                wake_token,
            } => {
                self.park_or_cancel_turn(
                    Some(reply_tx),
                    *turn,
                    wake_token,
                    None,
                    RenderRuntimeParkCondition::CommittedDocumentParserContinuation {
                        parser_unblocked: true,
                    },
                    command_admission_output_predecessor.take(),
                    parked_turns,
                    pending_turns,
                );
            }
        }
        if let Some(token) = removed_page_token {
            self.enqueue_all_parked_turns_for_page(token, parked_turns, pending_turns);
        }
    }

    pub(super) fn cancel_dispatch_outcome_for_context_shutdown(
        &self,
        outcome: RenderRuntimeDispatchOutcome,
    ) {
        match outcome {
            RenderRuntimeDispatchOutcome::PageCreatedAndContinueNavigation {
                page,
                continuation,
            } => {
                drop(page);
                self.cancel_pending_turn_on_owner_local_store(continuation.into_turn());
            }
            RenderRuntimeDispatchOutcome::PageReplacementCommittedAndContinueNavigation {
                replacement,
                continuation,
            } => {
                drop(replacement);
                self.cancel_pending_turn_on_owner_local_store(continuation.into_turn());
            }
            RenderRuntimeDispatchOutcome::ContinueNextTurn(turn)
            | RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline { turn, .. }
            | RenderRuntimeDispatchOutcome::ContinueAfterPageWake { turn, .. }
            | RenderRuntimeDispatchOutcome::ContinueCommittedDocumentParserAfterPageWake {
                turn,
                ..
            } => {
                self.cancel_pending_turn_on_owner_local_store(*turn);
            }
            RenderRuntimeDispatchOutcome::InspectorMainCommandClaimed {
                reply_tx,
                turn,
                command_admission_output_predecessor: _,
            } => {
                drop(reply_tx);
                self.cancel_pending_turn_on_owner_local_store(*turn);
            }
            RenderRuntimeDispatchOutcome::Reply(_)
            | RenderRuntimeDispatchOutcome::BackgroundComplete(_)
            | RenderRuntimeDispatchOutcome::PageTurnComplete { .. }
            | RenderRuntimeDispatchOutcome::PageCreationNavigationFailurePublished { .. } => {}
        }
    }

    pub(super) async fn run_pending_turn_on_owner_local_store(
        &self,
        turn: RenderRuntimeTurn,
    ) -> RenderRuntimeDispatchOutcome {
        // Keep large navigation futures out of this shared dispatcher's state machine.
        match turn {
            RenderRuntimeTurn::FinishHtmlCreatePage {
                requested_url,
                navigation_initiator_url,
                navigation_redirected,
                navigation_redirect_count,
                response_status,
                response_headers,
                page_vm,
                page_tasks,
                stage,
                started,
                reply_boundary,
                lifecycle_decider,
                top_level_navigation_dispatch,
                navigation_reply_policy,
            } => {
                let (pending, begin_outcome) = match self
                    .install_page_vm_and_begin_post_parse_lifecycle(
                        requested_url,
                        navigation_initiator_url,
                        navigation_redirected,
                        navigation_redirect_count,
                        response_status,
                        response_headers,
                        *page_vm,
                        page_tasks,
                        stage,
                        started,
                        reply_boundary,
                        lifecycle_decider,
                        top_level_navigation_dispatch,
                    )
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(error) => return Err(error).into(),
                };
                match begin_outcome {
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                        ..
                    } => {
                        let target_stage = stage;
                        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
                            let token = pending.token;
                            self.signal_internal_document_lifecycle_turn(token);
                            self.publish_pending_page_creation_and_continue(
                                pending,
                                RenderRuntimePageCreationContinuation::next_turn(
                                    RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                                        token,
                                        document,
                                        target_stage,
                                        follow_count: 0,
                                        completion:
                                            LivePagePendingNavigationCompletion::PublishedPageCreation {
                                                navigation_reply_policy,
                                            },
                                    },
                                ),
                            )
                            .await
                        } else {
                            let token = pending.token;
                            self.signal_internal_document_lifecycle_turn(token);
                            RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                                wake_token: token,
                                turn: Box::new(
                                    RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle {
                                        pending,
                                        document,
                                        target_stage,
                                        navigation_reply_policy,
                                    },
                                ),
                            }
                        }
                    }
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Blocked { document },
                        ..
                    } => {
                        let target_stage = stage;
                        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
                            let token = pending.token;
                            self.publish_pending_page_creation_and_continue(
                                pending,
                                RenderRuntimePageCreationContinuation::next_turn(
                                    RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                                        token,
                                        document,
                                        target_stage,
                                        follow_count: 0,
                                        completion:
                                            LivePagePendingNavigationCompletion::PublishedPageCreation {
                                                navigation_reply_policy,
                                            },
                                    },
                                ),
                            )
                            .await
                        } else {
                            RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                                wake_token: pending.token,
                                turn: Box::new(
                                    RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle {
                                        pending,
                                        document,
                                        target_stage,
                                        navigation_reply_policy,
                                    },
                                ),
                            }
                        }
                    }
                    DocumentLifecycleTurnOutcome {
                        action:
                            DocumentLifecycleTurnAction::RequestedTopLevelNavigation { stage, .. },
                        ..
                    } => {
                        if matches!(reply_boundary, crate::RendererReplyBoundary::DocumentCommit) {
                            if navigation_reply_policy.returns_with_pending_navigation() {
                                self.finish_pending_page_creation(pending).await
                            } else {
                                let token = pending.token;
                                self.publish_pending_page_creation_and_continue(
                                    pending,
                                    RenderRuntimePageCreationContinuation::next_turn(
                                        RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                                            token,
                                            stage,
                                            follow_count: 0,
                                            completion:
                                                LivePagePendingNavigationCompletion::PublishedPageCreation {
                                                    navigation_reply_policy,
                                                },
                                        },
                                    ),
                                )
                                .await
                            }
                        } else if navigation_reply_policy.returns_with_pending_navigation() {
                            self.finish_pending_page_creation(pending).await
                        } else {
                            let token = pending.token;
                            let entry =
                                match take_entry_for_command_on_bound_owner_local_store(token) {
                                    Ok(entry) => entry,
                                    Err(error) => return Err(error).into(),
                                };
                            self.continue_live_page_pending_navigation(
                                token,
                                entry,
                                stage,
                                0,
                                LivePagePendingNavigationCompletion::CompletePageCreation {
                                    pending,
                                    navigation_reply_policy,
                                },
                            )
                        }
                    }
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Idle,
                        ..
                    } => self.finish_pending_page_creation(pending).await,
                }
            }
            RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle {
                pending,
                document,
                target_stage,
                navigation_reply_policy,
            } => {
                self.continue_attached_page_creation_lifecycle_turn(
                    pending,
                    document,
                    target_stage,
                    navigation_reply_policy,
                )
                .await
            }
            RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                token,
                vm_creation_id,
                follow_count,
                completion,
            } => {
                let retire_page_on_failure = completion.retires_page_on_navigation_failure();
                let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
                    Ok(entry) => entry,
                    Err(error) => return Err(error).into(),
                };
                if entry.page_vm().creation_id != vm_creation_id {
                    self.restore_live_page_entry(token, entry);
                    return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
                }
                let advance = advance_pending_phase_one_navigation_on_entry_via_local_task(
                    self.state.local_executor.clone(),
                    entry,
                )
                .await;
                let (entry, advance_result) = match advance {
                    PendingPhaseOneEntryAdvance::Live { entry, result } => (entry, result),
                    PendingPhaseOneEntryAdvance::Retiring { entry, error } => {
                        return self
                            .finish_retiring_phase_one_navigation_failure(token, entry, error);
                    }
                };
                match advance_result {
                    Ok(LivePagePendingNavigationPhaseOneAdvance::Pending { wake_token }) => {
                        self.restore_live_page_entry(token, entry);
                        let admission =
                            pending_phase_one_admission_after_restore_on_bound_owner_local_store(
                                wake_token,
                            );
                        let continues_committed_document_parser_prefix = completion
                            .continues_committed_document_parser_prefix()
                            && admission == PhaseOneResidenceAdmission::ReadyPageTurn;
                        self.signal_pending_phase_one_admission(wake_token, admission);
                        let turn = Box::new(
                            RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                                token,
                                vm_creation_id,
                                follow_count,
                                completion,
                            },
                        );
                        if continues_committed_document_parser_prefix {
                            RenderRuntimeDispatchOutcome::ContinueCommittedDocumentParserAfterPageWake {
                                turn,
                                wake_token,
                            }
                        } else {
                            RenderRuntimeDispatchOutcome::ContinueAfterPageWake { turn, wake_token }
                        }
                    }
                    Ok(LivePagePendingNavigationPhaseOneAdvance::TriggeredNavigation { stage }) => {
                        if completion.returns_with_pending_location_navigation() {
                            Box::pin(
                                self.finish_live_page_navigation_completion(
                                    token, entry, completion,
                                ),
                            )
                            .await
                        } else {
                            self.continue_live_page_pending_navigation(
                                token,
                                entry,
                                stage,
                                follow_count + 1,
                                completion,
                            )
                        }
                    }
                    Ok(LivePagePendingNavigationPhaseOneAdvance::PostParseLifecycle {
                        target_stage,
                        outcome: lifecycle_outcome,
                    }) => {
                        self.handle_live_page_post_parse_lifecycle_outcome(
                            token,
                            entry,
                            target_stage,
                            follow_count,
                            completion,
                            lifecycle_outcome,
                        )
                        .await
                    }
                    Err(error) => {
                        self.finish_live_page_navigation_failure(
                            token,
                            entry,
                            retire_page_on_failure,
                            LivePageNavigationFailureDisposition::ReturnToInitiator(error),
                        )
                        .await
                    }
                }
            }
            RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                token,
                document,
                target_stage,
                follow_count,
                completion,
            } => {
                self.continue_live_page_navigation_post_parse_lifecycle_turn(
                    token,
                    document,
                    target_stage,
                    follow_count,
                    completion,
                )
                .await
            }
            RenderRuntimeTurn::DrainSharedWorkerServiceLane => {
                self.state
                    .browser_context_runtime
                    .drain_shared_worker_service_lane();
                RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
            }
            RenderRuntimeTurn::DrainServiceWorkerServiceLane => {
                self.state
                    .browser_context_runtime
                    .drain_service_worker_service_lane();
                RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
            }
            RenderRuntimeTurn::RunPageTurn { token } => self.run_one_page_turn(token).await,
            RenderRuntimeTurn::RunOwnerMaintenance { task } => {
                self.run_one_owner_maintenance_turn(task).await
            }
            RenderRuntimeTurn::RunInspectorMainReceiver { wake } => {
                let Some(dispatch) = dispatch_inspector_main_owner_wake(wake) else {
                    return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
                };
                let (token, capture_policy, envelope, first_dispatch, reply_tx) =
                    dispatch.into_parts();
                let command_admission_output_predecessor =
                    renderer_output_fence_for_tail_on_bound_owner_local_store(token);
                RenderRuntimeDispatchOutcome::InspectorMainCommandClaimed {
                    reply_tx,
                    command_admission_output_predecessor,
                    turn: Box::new(RenderRuntimeTurn::RunDevToolsMainCommand {
                        token,
                        command: envelope.into_payload(),
                        first_dispatch,
                        capture_policy,
                    }),
                }
            }
            RenderRuntimeTurn::RunDevToolsMainCommand {
                token,
                command,
                mut first_dispatch,
                capture_policy,
            } => {
                // First dispatch ends at backend entry. Producer publication
                // orders frontend output; Browser polling of an internal reply
                // must never keep this renderer's Main receiver blocked.
                first_dispatch.release();
                self.run_live_page_command_turn(token, command, capture_policy)
                    .await
            }
            RenderRuntimeTurn::RunLivePageCommand {
                token,
                command,
                capture_policy,
            } => {
                self.run_live_page_command_turn(token, command, capture_policy)
                    .await
            }
            RenderRuntimeTurn::ContinueLivePageRuntimeCommandLifecycle {
                token,
                scope_id,
                reply,
                should_follow_pending_navigation,
                turn_records,
                capture_policy,
            } => {
                self.continue_live_page_runtime_command_lifecycle_turn(
                    token,
                    scope_id,
                    *reply,
                    should_follow_pending_navigation,
                    turn_records,
                    capture_policy,
                )
                .await
            }
            RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply { token, document } => {
                self.signal_internal_document_lifecycle_turn_if_resident(token, document);
                RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
            }
            RenderRuntimeTurn::WaitLivePageNetworkIdle {
                token,
                state,
                deadline,
                loader,
            } => {
                self.wait_live_page_network_idle_turn(token, state, deadline, loader)
                    .await
            }
            RenderRuntimeTurn::WaitLivePageDomStable {
                token,
                state,
                deadline,
                loader,
            } => {
                self.wait_live_page_dom_stable_turn(token, state, deadline, loader)
                    .await
            }
            RenderRuntimeTurn::WaitLifecycleNavigation(wait) => {
                self.wait_lifecycle_navigation_turn(wait).await
            }
            RenderRuntimeTurn::WaitLivePageSelector {
                token,
                selector,
                deadline,
                loader,
                capture_policy,
            } => {
                self.wait_live_page_selector_turn(token, selector, deadline, loader, capture_policy)
                    .await
            }
            RenderRuntimeTurn::WaitLivePageScriptTruthy {
                token,
                expression,
                pending_call,
                deadline,
                loader,
                capture_policy,
            } => {
                self.wait_live_page_script_truthy_turn(
                    token,
                    expression,
                    pending_call,
                    deadline,
                    loader,
                    capture_policy,
                )
                .await
            }
            RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                token,
                execution_context_id,
                expression,
                pending_call,
                deadline,
                result_mode,
                navigation_policy,
                capture_policy,
            } => {
                self.wait_live_page_runtime_expression_await_turn(
                    token,
                    execution_context_id,
                    expression,
                    pending_call,
                    deadline,
                    result_mode,
                    navigation_policy,
                    capture_policy,
                )
                .await
            }
            RenderRuntimeTurn::WaitLivePageSubresourceResponse {
                token,
                criteria,
                deadline,
                loader,
                capture_policy,
            } => {
                self.wait_live_page_subresource_response_turn(
                    token,
                    criteria,
                    deadline,
                    loader,
                    capture_policy,
                )
                .await
            }
            RenderRuntimeTurn::WaitLivePageChildFrameLifecycle {
                token,
                deadline,
                capture_policy,
            } => {
                self.wait_live_page_child_frame_lifecycle_turn(token, deadline, capture_policy)
                    .await
            }
            RenderRuntimeTurn::ClaimLivePageTopLevelNavigationHandoff { token, handoff } => {
                self.claim_top_level_navigation_handoff(token, handoff)
            }
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                token,
                stage,
                follow_count,
                completion,
            } => {
                Box::pin(self.follow_live_page_pending_location_navigation_turn(
                    token,
                    stage,
                    follow_count,
                    completion,
                ))
                .await
            }
        }
    }
}
