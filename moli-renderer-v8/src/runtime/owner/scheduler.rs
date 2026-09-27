use super::*;

impl RendererOwnerHandle {
    pub(super) fn restore_live_page_entry(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
    ) {
        // Freeze this owner turn before making the Page resident again. The
        // concrete publication is already source-bound, so a later lifecycle
        // turn never needs to rescan or publish output on this turn's behalf.
        let output = entry.page_vm_mut().settle_renderer_output_publication();
        restore_entry_after_command_on_bound_owner_local_store(token, entry);
        if let Some(output) = output {
            self.publish_renderer_output(output);
        }
    }

    pub(super) fn retire_pending_phase_one_page_entry(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        reason: &str,
    ) {
        // Retirement becomes sticky while the checked-out entry still owns
        // its PageVm. Rejecting the navigation then consumes the live type and
        // returns a teardown-only entry that cannot reach live restoration.
        remove_page_on_bound_owner_local_store(token);
        let entry = entry.reject_pending_phase_one_navigation(reason);
        restore_retiring_entry_after_command_on_bound_owner_local_store(token, entry);
    }

    pub(super) fn finish_retiring_phase_one_navigation_failure(
        &self,
        token: RendererPageToken,
        mut entry: RetiringPageEntry,
        error: anyhow::Error,
    ) -> RenderRuntimeDispatchOutcome {
        entry.settle_renderer_navigation_follow(false);
        tracing::warn!(
            page_id = token.page_id.as_u64(),
            failure = %error,
            "retiring page after phase-one navigation lost its active PageVm"
        );
        remove_page_on_bound_owner_local_store(token);
        restore_retiring_entry_after_command_on_bound_owner_local_store(token, entry);
        Err(error).into()
    }

    pub(super) fn finish_committed_navigation_failure(
        &self,
        token: RendererPageToken,
        mut entry: CommittedNavigationEntry,
        disposition: LivePageNavigationFailureDisposition,
    ) -> RenderRuntimeDispatchOutcome {
        // Page creation owns the strong failure observer while the stable Page
        // slot owns only its weak publisher. Publish before marking the slot
        // retiring so the parked creation turn observes this exact failure.
        let page_creation_publication = disposition.publish_page_creation_failure(token);
        tracing::warn!(
            page_id = token.page_id.as_u64(),
            failure = %disposition,
            "retiring page after committed navigation failed to bootstrap"
        );
        let cleanup_failure = disposition.to_string();
        entry.settle_renderer_navigation_follow(false);
        remove_page_on_bound_owner_local_store(token);
        let entry = entry.reject_and_retire(&cleanup_failure);
        restore_retiring_entry_after_command_on_bound_owner_local_store(token, entry);
        disposition.into_dispatch_outcome(token, page_creation_publication)
    }

    pub(super) async fn finish_live_page_navigation_failure(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        retire_page_on_failure: bool,
        disposition: LivePageNavigationFailureDisposition,
    ) -> RenderRuntimeDispatchOutcome {
        let had_uncommitted_page_vm = entry.has_uncommitted_page_vm();
        debug_assert!(
            entry
                .active_page_vm()
                .is_some_and(PageVm::has_live_script_vm),
            "live navigation failure handling requires an active PageVm"
        );
        let should_retire_page = retire_page_on_failure;
        // Page creation owns the strong failure observer while the stable Page
        // slot owns only its weak publisher. Publish before requesting Page
        // retirement: restoring a checked-out entry into a retiring slot tears
        // it down, but the parked creation turn must retain the concrete
        // terminal and be woken instead of falling through to its timeout.
        let page_creation_publication = disposition.publish_page_creation_failure(token);
        if should_retire_page {
            tracing::warn!(
                page_id = token.page_id.as_u64(),
                failure = %disposition,
                "retiring page after live navigation failure"
            );
            // Mark the stable slot retiring while this turn still owns the
            // entry. Restoring then tears down the detached shell without ever
            // publishing it as the page's active runtime.
            remove_page_on_bound_owner_local_store(token);
        }
        if had_uncommitted_page_vm && !should_retire_page {
            // Cross-creation publication is legal only at the typed response
            // commit transition. Reaching failure with a mismatched stable
            // identity means that transition itself failed; do not invent a
            // late commit or roll back to the terminated Document.
            tracing::error!(
                page_id = token.page_id.as_u64(),
                failure = %disposition,
                "retiring page after replacement Document commit failed to publish"
            );
            remove_page_on_bound_owner_local_store(token);
        }
        entry.settle_renderer_navigation_follow(false);
        self.restore_live_page_entry(token, entry);
        disposition.into_dispatch_outcome(token, page_creation_publication)
    }

    /// Compute the earliest Page-owned task deadline. This combines JavaScript
    /// timers with typed delayed sources and contains no renderer housekeeping
    /// deadline, so every admission has a concrete Page scheduler candidate.
    pub(super) fn compute_next_page_task_deadline(&self) -> Option<Instant> {
        next_page_task_deadline_on_bound_owner_local_store()
    }

    /// Compute the next renderer-owner housekeeping deadline. Maintenance is
    /// selected by its own residence and never enters Page task arbitration.
    pub(super) fn compute_next_owner_maintenance_deadline(&self) -> Option<Instant> {
        next_owner_maintenance_deadline_on_bound_owner_local_store()
    }

    pub(super) fn enqueue_page_creation_continuation(
        &self,
        continuation: RenderRuntimePageCreationContinuation,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
    ) {
        match continuation {
            RenderRuntimePageCreationContinuation::NextTurn(turn) => {
                pending_turns.push_back(RenderRuntimePendingTurn {
                    reply_tx: None,
                    turn: *turn,
                    allow_command_overtake: true,
                    command_admission_output_predecessor: None,
                });
            }
            RenderRuntimePageCreationContinuation::AfterCommittedDocumentResponse {
                turn,
                wake_token,
            } => {
                parked_turns.push_back(RenderRuntimeParkedTurn {
                    reply_tx: None,
                    turn: *turn,
                    wake_token,
                    ready_at: None,
                    condition: RenderRuntimeParkCondition::CommittedDocumentParserContinuation {
                        parser_unblocked: false,
                    },
                    command_admission_output_predecessor: None,
                });
            }
        }
    }

    pub(super) fn park_or_cancel_turn(
        &self,
        reply_tx: Option<oneshot::Sender<Result<RendererOwnerReply>>>,
        turn: RenderRuntimeTurn,
        wake_token: RendererPageToken,
        ready_at: Option<Instant>,
        condition: RenderRuntimeParkCondition,
        command_admission_output_predecessor: Option<RendererOutputFence>,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if reply_tx.as_ref().is_some_and(|tx| tx.is_closed()) {
            self.detach_navigation_or_cancel_pending_turn(turn, pending_turns);
            return;
        }
        parked_turns.push_back(RenderRuntimeParkedTurn {
            reply_tx,
            turn,
            wake_token,
            ready_at,
            condition,
            command_admission_output_predecessor,
        });
    }

    /// Admit one detached page turn. Producer wakes remain in the
    /// owner channel while a page turn is pending, so every legacy wake gets a
    /// bounded adapter turn without being merged into an inferred continuation.
    /// Already-arrived commands can preempt that turn at its owner boundary.
    pub(super) fn enqueue_page_turn(
        &self,
        token: RendererPageToken,
        trigger: PageTurnTrigger,
        allow_command_overtake: bool,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if let Some(turn) = self.admit_page_turn(token, trigger, allow_command_overtake) {
            pending_turns.push_back(turn);
        }
    }

    pub(super) fn enqueue_page_turn_before_commands(
        &self,
        token: RendererPageToken,
        trigger: PageTurnTrigger,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if let Some(turn) = self.admit_page_turn(token, trigger, false) {
            pending_turns.push_front(turn);
        }
    }

    pub(super) fn admit_page_turn(
        &self,
        token: RendererPageToken,
        trigger: PageTurnTrigger,
        allow_command_overtake: bool,
    ) -> Option<RenderRuntimePendingTurn> {
        match schedule_page_turn_on_bound_owner_local_store(token, trigger) {
            RendererPageTurnAdmission::EnqueueOwnerTurn => {}
            RendererPageTurnAdmission::AlreadyScheduled => {
                panic!(
                    "renderer page {} received a second owner admission before its scheduled turn",
                    token.page_id.as_u64()
                );
            }
            RendererPageTurnAdmission::Retired | RendererPageTurnAdmission::MissingPage => {
                tracing::trace!(
                    page_id = token.page_id.as_u64(),
                    ?trigger,
                    "discarded stale page turn trigger"
                );
                return None;
            }
        }
        Some(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunPageTurn { token },
            allow_command_overtake,
            command_admission_output_predecessor: None,
        })
    }

    pub(super) fn handle_page_owner_wake(
        &self,
        wake: RendererOwnerWake,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        match wake {
            RendererOwnerWake::Page { token, source } => {
                if parked_turns.iter().any(|turn| {
                    turn.wake_token == token
                        && turn.condition.blocks_page_activity_until_parser_unblocked()
                }) {
                    // The source payload remains resident. Its edge wake is
                    // reconciled after the commit caller observes the Page,
                    // so parser work cannot outrun the DocumentCommit reply.
                    return;
                }
                let allow_command_overtake = !parked_turns.iter().any(|turn| {
                    turn.wake_token == token
                        && turn.condition.admits_page_activity()
                        && !turn.condition.allows_command_overtake()
                });
                let committed_document_parser_handoff = self
                    .enqueue_unblocked_committed_document_parser_continuation_before_commands(
                        token,
                        parked_turns,
                        pending_turns,
                    );
                if matches!(source, RendererOwnerWakeSource::ParseTimeDocumentScriptWork) {
                    // Parse-time script payloads live inside the parked
                    // continuation. Admitting a generic Page turn for this
                    // source would manufacture a second consumer.
                    self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                    return;
                }
                if matches!(source, RendererOwnerWakeSource::InternalLoadingTask)
                    && !snapshot_due_page_task_tokens_on_bound_owner_local_store(Instant::now())
                        .contains(&token)
                {
                    // Delayed internal-loading payloads are posted while the
                    // Page is checked out. Restoration has already accepted
                    // the payload into the derived deadline index; the route
                    // wake only brings an idle owner loop back to that index.
                    // Do not manufacture an empty Page turn before it is due.
                    self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                    return;
                }
                if source
                    == RendererOwnerWakeSource::Runtime(
                        RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
                    )
                {
                    // The previous lifecycle turn has already published its
                    // milestone before admitting this follow-up. Let parked
                    // lifecycle observers capture that settled boundary
                    // before the next lifecycle turn can advance into load.
                    self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                    self.enqueue_page_turn(
                        token,
                        PageTurnTrigger::producer(source),
                        allow_command_overtake,
                        pending_turns,
                    );
                    return;
                }
                if committed_document_parser_handoff {
                    self.enqueue_page_turn_before_commands(
                        token,
                        PageTurnTrigger::producer(source),
                        pending_turns,
                    );
                    self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                    return;
                }
                self.enqueue_page_turn(
                    token,
                    PageTurnTrigger::producer(source),
                    allow_command_overtake,
                    pending_turns,
                );
                self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
            }
            RendererOwnerWake::PostResponseDocumentLifecycle { token, document } => {
                if !release_post_response_document_lifecycle_on_bound_owner_local_store(
                    token, document,
                ) {
                    tracing::trace!(
                        page_id = token.page_id.as_u64(),
                        ?document,
                        "discarded stale post-response lifecycle continuation"
                    );
                    return;
                }
                let trigger = PageTurnTrigger::producer(RendererOwnerWakeSource::Runtime(
                    RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
                ));
                match schedule_page_turn_on_bound_owner_local_store(token, trigger) {
                    RendererPageTurnAdmission::EnqueueOwnerTurn => {
                        pending_turns.push_back(RenderRuntimePendingTurn {
                            reply_tx: None,
                            turn: RenderRuntimeTurn::RunPageTurn { token },
                            allow_command_overtake: true,
                            command_admission_output_predecessor: None,
                        });
                    }
                    RendererPageTurnAdmission::AlreadyScheduled => {
                        // Opening the exact resident is enough. The previously
                        // scheduled Page turn will observe it during arbitration.
                    }
                    RendererPageTurnAdmission::Retired | RendererPageTurnAdmission::MissingPage => {
                        return;
                    }
                }
                self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
            }
            RendererOwnerWake::CommittedDocumentParserUnblocked { token } => {
                let mut unblocked = false;
                for parked_turn in parked_turns
                    .iter_mut()
                    .filter(|turn| turn.wake_token == token)
                {
                    unblocked |= parked_turn.condition.unblock_committed_document_parser();
                }
                if !unblocked {
                    tracing::trace!(
                        page_id = token.page_id.as_u64(),
                        "discarded stale committed-Document parser release"
                    );
                    return;
                }
                let admission =
                    pending_phase_one_admission_after_restore_on_bound_owner_local_store(token);
                self.signal_pending_phase_one_admission(token, admission);
            }
            RendererOwnerWake::RuntimeInspectorResponsePublication { token, publication } => {
                let renderer_output_predecessor =
                    renderer_output_fence_for_tail_on_bound_owner_local_store(token);
                if let Err(completion) = publication.commit(renderer_output_predecessor) {
                    tracing::debug!(
                        page_id = token.page_id.as_u64(),
                        call_id = completion.call_id,
                        "discarded late Runtime response because its protocol receiver was closed"
                    );
                }
            }
            RendererOwnerWake::TopLevelNavigationHandoff { token, handoff } => {
                // A command-owned wait that is already resident for this Page
                // gets first refusal. Its continuation can claim the exact
                // navigation descriptor and retain its own completion policy.
                // The queued owner turn is the explicit background fallback
                // when no such consumer takes ownership.
                self.enqueue_parked_turns_for_wake(token, parked_turns, pending_turns);
                pending_turns.push_back(RenderRuntimePendingTurn {
                    reply_tx: None,
                    turn: RenderRuntimeTurn::ClaimLivePageTopLevelNavigationHandoff {
                        token,
                        handoff,
                    },
                    allow_command_overtake: false,
                    command_admission_output_predecessor: None,
                });
            }
            RendererOwnerWake::ReplacementDocumentViewSettled {
                token,
                vm_creation_id,
            } => {
                self.enqueue_parked_turns_for_replacement_view_settlement(
                    token,
                    vm_creation_id,
                    parked_turns,
                    pending_turns,
                );
            }
        }
    }

    pub(super) fn claim_top_level_navigation_handoff(
        &self,
        token: RendererPageToken,
        handoff: RendererTopLevelNavigationHandoff,
    ) -> RenderRuntimeDispatchOutcome {
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => {
                tracing::trace!(
                    page_id = token.page_id.as_u64(),
                    ?handoff,
                    "discarded top-level navigation handoff: {error}"
                );
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
        };
        let claimed = entry.begin_renderer_navigation_follow_from_handoff(handoff);
        self.restore_live_page_entry(token, entry);
        if !claimed {
            tracing::trace!(
                page_id = token.page_id.as_u64(),
                ?handoff,
                "ignored stale, delegated, or already-owned top-level navigation handoff"
            );
            return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
        }
        RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                token,
                stage: PageVmInitStage::Load,
                follow_count: 0,
                completion: LivePagePendingNavigationCompletion::Background,
            },
        ))
    }

    pub(super) fn try_admit_ready_page_turn(
        &self,
        preference: &mut PageTurnAdmissionPreference,
        page_wake_rx: &mut mpsc::UnboundedReceiver<RendererOwnerWake>,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> ReadyPageTurnAdmission {
        let preferred = match preference {
            PageTurnAdmissionPreference::ProducerWake => self.try_admit_page_producer_wake(
                preference,
                page_wake_rx,
                parked_turns,
                pending_turns,
            ),
            PageTurnAdmissionPreference::Deadline => {
                self.try_admit_due_page_turn(preference, pending_turns)
            }
        };
        if preferred != ReadyPageTurnAdmission::NoneReady {
            return preferred;
        }
        match preference {
            PageTurnAdmissionPreference::ProducerWake => {
                self.try_admit_due_page_turn(preference, pending_turns)
            }
            PageTurnAdmissionPreference::Deadline => self.try_admit_page_producer_wake(
                preference,
                page_wake_rx,
                parked_turns,
                pending_turns,
            ),
        }
    }

    pub(super) fn try_admit_page_producer_wake(
        &self,
        preference: &mut PageTurnAdmissionPreference,
        page_wake_rx: &mut mpsc::UnboundedReceiver<RendererOwnerWake>,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> ReadyPageTurnAdmission {
        match page_wake_rx.try_recv() {
            Ok(wake) => {
                self.handle_page_owner_wake(wake, parked_turns, pending_turns);
                *preference = PageTurnAdmissionPreference::Deadline;
                ReadyPageTurnAdmission::Admitted
            }
            Err(mpsc::error::TryRecvError::Empty) => ReadyPageTurnAdmission::NoneReady,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                ReadyPageTurnAdmission::WakeChannelClosed
            }
        }
    }

    pub(super) fn try_admit_due_page_turn(
        &self,
        preference: &mut PageTurnAdmissionPreference,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> ReadyPageTurnAdmission {
        if self.enqueue_due_page_turns(pending_turns) {
            *preference = PageTurnAdmissionPreference::ProducerWake;
            ReadyPageTurnAdmission::Admitted
        } else {
            ReadyPageTurnAdmission::NoneReady
        }
    }

    pub(super) fn enqueue_shared_worker_service_lane_turn(
        &self,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        pending_turns.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::DrainSharedWorkerServiceLane,
            allow_command_overtake: false,
            command_admission_output_predecessor: None,
        });
    }

    pub(super) fn enqueue_service_worker_service_lane_turn(
        &self,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        pending_turns.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::DrainServiceWorkerServiceLane,
            allow_command_overtake: false,
            command_admission_output_predecessor: None,
        });
    }

    pub(super) fn handle_shared_worker_runtime_wake(
        &self,
        wake: SharedWorkerRuntimeOwnerWake,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        match wake {
            SharedWorkerRuntimeOwnerWake::ServiceLane => {
                self.enqueue_shared_worker_service_lane_turn(pending_turns);
            }
        }
    }

    pub(super) fn handle_service_worker_runtime_wake(
        &self,
        wake: ServiceWorkerRuntimeOwnerWake,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        match wake {
            ServiceWorkerRuntimeOwnerWake::ServiceLane => {
                self.enqueue_service_worker_service_lane_turn(pending_turns);
            }
        }
    }

    pub(super) fn next_parked_turn_deadline(
        &self,
        parked_turns: &VecDeque<RenderRuntimeParkedTurn>,
    ) -> Option<Instant> {
        parked_turns.iter().filter_map(|turn| turn.ready_at).min()
    }

    pub(super) fn enqueue_parked_turn(
        &self,
        parked_turn: RenderRuntimeParkedTurn,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if let Some(turn) = self.pending_turn_from_parked(parked_turn, pending_turns) {
            pending_turns.push_back(turn);
        }
    }

    pub(super) fn pending_turn_from_parked(
        &self,
        parked_turn: RenderRuntimeParkedTurn,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> Option<RenderRuntimePendingTurn> {
        if parked_turn
            .reply_tx
            .as_ref()
            .is_some_and(|tx| tx.is_closed())
        {
            self.detach_navigation_or_cancel_pending_turn(parked_turn.turn, pending_turns);
            None
        } else {
            let allow_command_overtake = parked_turn.condition.allows_command_overtake();
            Some(RenderRuntimePendingTurn {
                reply_tx: parked_turn.reply_tx,
                turn: parked_turn.turn,
                allow_command_overtake,
                command_admission_output_predecessor: parked_turn
                    .command_admission_output_predecessor,
            })
        }
    }

    pub(super) fn enqueue_unblocked_committed_document_parser_continuation_before_commands(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> bool {
        let mut index = 0;
        let mut admitted = 0usize;
        while index < parked_turns.len() {
            let matches = parked_turns[index].wake_token == token
                && parked_turns[index]
                    .condition
                    .is_unblocked_committed_document_parser_continuation();
            if matches {
                let parked_turn = parked_turns
                    .remove(index)
                    .expect("matching committed-Document continuation must remain parked");
                if let Some(turn) = self.pending_turn_from_parked(parked_turn, pending_turns) {
                    pending_turns.push_front(turn);
                    admitted += 1;
                }
            } else {
                index += 1;
            }
        }
        debug_assert!(
            admitted <= 1,
            "one Page cannot retain multiple committed-Document parser continuations"
        );
        admitted != 0
    }

    /// Complete the selected Networking continuation's one-way handoff.
    ///
    /// The Page turn only records admission in the resident phase-one parser;
    /// it must not let another ordinary Page task run before that parser
    /// consumes the admission. The logical phase-one driver may already be in
    /// the pending queue (from the producer wake that admitted this Page turn)
    /// or still be parked on Page activity, so promote either residence to the
    /// front without changing task-source arbitration globally.
    pub(super) fn promote_admitted_phase_one_parser_continuation(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        if pending_turns.promote_phase_one_parser_continuation(token) {
            return;
        }

        let index = parked_turns
            .iter()
            .position(|parked| {
                parked.wake_token == token && parked.turn.is_phase_one_continuation_for(token)
            })
            .expect("an admitted main-parser continuation must retain its phase-one driver");
        let parked = parked_turns
            .remove(index)
            .expect("selected phase-one continuation must remain parked");
        if let Some(mut pending) = self.pending_turn_from_parked(parked, pending_turns) {
            pending.allow_command_overtake = false;
            pending_turns.push_front(pending);
        } else {
            // A closed command observer may detach the same continuation into
            // background work while converting the parked turn. Promote that
            // detached driver too. Only an already-retired Page may
            // legitimately leave no driver; a live Page would otherwise hang
            // after consuming the one-shot parser admission.
            let promoted = pending_turns.promote_phase_one_parser_continuation(token);
            assert!(
                promoted || !self.state.page_table.contains_page(token.page_id()),
                "an admitted main-parser continuation lost its phase-one driver while the Page remained live"
            );
        }
    }

    pub(super) fn enqueue_due_parked_turns(
        &self,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let now = Instant::now();
        let mut index = 0;
        while index < parked_turns.len() {
            let ready = parked_turns[index]
                .ready_at
                .is_some_and(|ready_at| ready_at <= now)
                || parked_turns[index]
                    .reply_tx
                    .as_ref()
                    .is_some_and(|tx| tx.is_closed());
            if ready {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    pub(super) fn enqueue_parked_turns_for_wake(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let mut index = 0;
        while index < parked_turns.len() {
            let should_wake = (parked_turns[index].wake_token == token
                && parked_turns[index].condition.admits_page_activity())
                || parked_turns[index]
                    .reply_tx
                    .as_ref()
                    .is_some_and(|tx| tx.is_closed());
            if should_wake {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    pub(super) fn enqueue_parked_turns_for_replacement_view_settlement(
        &self,
        token: RendererPageToken,
        vm_creation_id: u64,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let mut index = 0;
        while index < parked_turns.len() {
            let should_wake = (parked_turns[index].wake_token == token
                && parked_turns[index]
                    .condition
                    .admits_replacement_view_settlement(vm_creation_id))
                || parked_turns[index]
                    .reply_tx
                    .as_ref()
                    .is_some_and(|tx| tx.is_closed());
            if should_wake {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    pub(super) fn enqueue_all_parked_turns_for_page(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let mut index = 0;
        while index < parked_turns.len() {
            let should_wake = parked_turns[index].wake_token == token
                || parked_turns[index]
                    .reply_tx
                    .as_ref()
                    .is_some_and(|tx| tx.is_closed());
            if should_wake {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    pub(super) fn enqueue_parked_page_creation_observers_after_page_turn(
        &self,
        token: RendererPageToken,
        parked_turns: &mut VecDeque<RenderRuntimeParkedTurn>,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let mut index = 0;
        while index < parked_turns.len() {
            let should_observe = parked_turns[index].wake_token == token
                && parked_turns[index]
                    .turn
                    .is_page_creation_lifecycle_observer_for(token);
            if should_observe {
                let Some(parked_turn) = parked_turns.remove(index) else {
                    break;
                };
                self.enqueue_parked_turn(parked_turn, pending_turns);
            } else {
                index += 1;
            }
        }
    }

    pub(super) async fn run_one_document_lifecycle_turn(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        source_label: &'static str,
        displaced_ordinary: RendererDisplacedOrdinaryTurn,
    ) -> RenderRuntimeDispatchOutcome {
        let executor = entry.page_vm().local_executor.clone();
        let (mut entry, advance_result) =
            advance_document_lifecycle_one_page_turn_via_local_task(executor, entry).await;
        let outcome = match advance_result {
            Ok(outcome) => outcome,
            Err(error) => {
                let output = entry.page_vm_mut().settle_renderer_output_publication();
                restore_entry_after_document_lifecycle_on_bound_owner_local_store(
                    token,
                    entry,
                    displaced_ordinary.requires_reconsideration(),
                );
                if let Some(output) = output {
                    self.publish_renderer_output(output);
                }
                if displaced_ordinary.requires_reconsideration() {
                    self.signal_internal_page_turn_source(
                        token,
                        RendererOwnerWakeSource::SchedulerContinuation,
                    );
                }
                tracing::error!(
                    source = source_label,
                    page_id = token.page_id.as_u64(),
                    "document lifecycle owner turn failed: {error}"
                );
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Err(error));
            }
        };
        let action = outcome.action;
        let readiness = outcome.readiness;
        let output_ordering = action.renderer_output_ordering();
        let top_level_navigation_dispatch = entry.top_level_navigation_dispatch();
        let should_resume_ordinary = displaced_ordinary.requires_reconsideration()
            && !matches!(
                action,
                DocumentLifecycleTurnAction::RequestedTopLevelNavigation { .. }
            )
            && !matches!(readiness, DocumentLifecycleTurnReadiness::Runnable { .. });
        let should_follow_pending_location_navigation = matches!(
            action,
            DocumentLifecycleTurnAction::RequestedTopLevelNavigation { .. }
        ) && entry
            .begin_renderer_navigation_follow();
        let delegated_top_level_navigation = if matches!(
            action,
            DocumentLifecycleTurnAction::RequestedTopLevelNavigation { .. }
        ) && matches!(
            top_level_navigation_dispatch,
            RendererTopLevelNavigationDispatch::DelegateToBrowser
        ) {
            match entry
                .page_vm_mut()
                .vm_mut()
                .publish_pending_document_location_navigation()
            {
                Ok(published) => published,
                Err(error) => {
                    let concrete_output = entry
                        .page_vm_mut()
                        .settle_renderer_output_publication()
                        .map(|output| output.with_ordering(output_ordering));
                    self.restore_live_page_entry(token, entry);
                    if let Some(output) = concrete_output {
                        self.publish_renderer_output(output);
                    }
                    return RenderRuntimeDispatchOutcome::BackgroundComplete(Err(error));
                }
            }
        } else {
            false
        };
        let concrete_output = entry
            .page_vm_mut()
            .settle_renderer_output_publication()
            .map(|output| output.with_ordering(output_ordering));
        restore_entry_after_document_lifecycle_on_bound_owner_local_store(
            token,
            entry,
            should_resume_ordinary,
        );
        if let Some(output) = concrete_output {
            self.publish_renderer_output(output);
        }

        match readiness {
            DocumentLifecycleTurnReadiness::Runnable { .. } => {
                self.signal_internal_document_lifecycle_turn(token);
            }
            DocumentLifecycleTurnReadiness::Blocked { .. }
            | DocumentLifecycleTurnReadiness::Idle
                if should_resume_ordinary =>
            {
                self.signal_internal_page_turn_source(
                    token,
                    RendererOwnerWakeSource::SchedulerContinuation,
                );
            }
            DocumentLifecycleTurnReadiness::Blocked { .. }
            | DocumentLifecycleTurnReadiness::Idle => {}
        }

        tracing::debug!(
            source = source_label,
            page_id = token.page_id.as_u64(),
            ?action,
            ?readiness,
            "completed one exact-Document lifecycle owner turn"
        );

        if let DocumentLifecycleTurnAction::RequestedTopLevelNavigation { stage, .. } = action {
            if delegated_top_level_navigation {
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
            if !should_follow_pending_location_navigation {
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
            return RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                    token,
                    stage,
                    follow_count: 0,
                    completion: LivePagePendingNavigationCompletion::Background,
                },
            ));
        }

        RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
    }

    pub(super) fn finish_page_turn(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        source_label: &'static str,
        output_ordering: RendererOutputPublicationOrdering,
        parser_continuation_admitted: bool,
    ) -> RenderRuntimeDispatchOutcome {
        let has_pending_document_lifecycle_turn =
            entry.pending_document_lifecycle_identity().is_some();
        if matches!(
            entry.top_level_navigation_dispatch(),
            RendererTopLevelNavigationDispatch::DelegateToBrowser
        ) && let Err(error) = entry
            .page_vm_mut()
            .vm_mut()
            .publish_pending_document_location_navigation()
        {
            let concrete_output = entry
                .page_vm_mut()
                .settle_renderer_output_publication()
                .map(|output| output.with_ordering(output_ordering));
            restore_entry_after_command_on_bound_owner_local_store(token, entry);
            if let Some(output) = concrete_output {
                self.publish_renderer_output(output);
            }
            return RenderRuntimeDispatchOutcome::PageTurnComplete {
                result: Err(error),
                parser_continuation_admitted,
            };
        }
        let concrete_output = entry
            .page_vm_mut()
            .settle_renderer_output_publication()
            .map(|output| output.with_ordering(output_ordering));
        restore_entry_after_command_on_bound_owner_local_store(token, entry);
        if let Some(output) = concrete_output {
            self.publish_renderer_output(output);
        }

        let readiness = page_turn_readiness_after_restore_on_bound_owner_local_store(token);
        let next_turn = readiness
            .map(|readiness| readiness.next_turn(has_pending_document_lifecycle_turn))
            .unwrap_or(PageOwnerNextTurn::None);

        match next_turn {
            PageOwnerNextTurn::Ordinary => {
                self.signal_internal_page_turn_source(
                    token,
                    RendererOwnerWakeSource::SchedulerContinuation,
                );
            }
            PageOwnerNextTurn::DocumentLifecycle => {
                self.signal_internal_document_lifecycle_turn(token);
            }
            PageOwnerNextTurn::None => {}
        }

        tracing::debug!(
            source = source_label,
            page_id = token.page_id.as_u64(),
            ?readiness,
            ?next_turn,
            "completed one ordinary Page turn"
        );
        RenderRuntimeDispatchOutcome::PageTurnComplete {
            result: Ok(()),
            parser_continuation_admitted,
        }
    }

    pub(super) async fn run_one_page_turn(
        &self,
        token: RendererPageToken,
    ) -> RenderRuntimeDispatchOutcome {
        let (entry, trigger, scheduled_turn) =
            match checkout_scheduled_page_turn_on_bound_owner_local_store(token) {
                Ok(turn) => turn,
                Err(RendererPageTurnCheckoutError::Busy) => {
                    panic!(
                        "renderer page {} remained checked out at a serialized page-turn boundary",
                        token.page_id.as_u64()
                    );
                }
                Err(RendererPageTurnCheckoutError::NotScheduled) => {
                    panic!(
                        "renderer page {} reached an owner turn without a page-local admission",
                        token.page_id.as_u64()
                    );
                }
                Err(
                    RendererPageTurnCheckoutError::Retired | RendererPageTurnCheckoutError::Missing,
                ) => {
                    return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
                }
            };
        let source_label = page_turn_trigger_log_label(trigger);
        let scheduled_task = match scheduled_turn {
            RendererPageScheduledTurn::DocumentLifecycle { displaced_ordinary } => {
                return self
                    .run_one_document_lifecycle_turn(token, entry, source_label, displaced_ordinary)
                    .await;
            }
            RendererPageScheduledTurn::SpentWake => {
                self.restore_live_page_entry(token, entry);
                tracing::trace!(
                    source = source_label,
                    page_id = token.page_id.as_u64(),
                    "settled spent Page wake without entering the local executor"
                );
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
            RendererPageScheduledTurn::Ordinary(scheduled_task) => *scheduled_task,
        };
        // Freeze the exact Document at task selection. A timer callback may
        // synchronously replace the Document; its protocol output still
        // belongs after the load boundary of the Document that authorized
        // this owner turn, never whichever Document is current at settlement.
        let turn_source_document = entry.page_vm().document_lifecycle.identity();
        let output_ordering = if matches!(
            &scheduled_task,
            crate::page_task_queue::RendererPageSchedulerTask::Timer { .. }
        ) {
            RendererOutputPublicationOrdering::AfterPendingPageLoad {
                source_document: turn_source_document,
            }
        } else {
            RendererOutputPublicationOrdering::Unconstrained
        };
        let executor = entry.page_vm().local_executor.clone();
        let loader = entry.page_vm().request_client.clone();
        let (mut entry, advance_result) =
            advance_page_owner_one_turn_via_local_task(executor, entry, scheduled_task, loader)
                .await;
        let parser_continuation_admitted = entry
            .page_vm()
            .vm()
            .document_runtime
            .has_main_parser_continuation_admission();

        if let Err(error) = advance_result {
            let has_pending_document_lifecycle_turn =
                has_pending_document_lifecycle_turn_on_entry(&mut entry);
            let output = entry
                .page_vm_mut()
                .settle_renderer_output_publication()
                .map(|output| output.with_ordering(output_ordering));
            self.restore_live_page_entry(token, entry);
            if let Some(output) = output {
                self.publish_renderer_output(output);
            }
            let readiness = page_turn_readiness_after_restore_on_bound_owner_local_store(token);
            let next_turn = readiness
                .map(|readiness| readiness.next_turn(has_pending_document_lifecycle_turn))
                .unwrap_or(PageOwnerNextTurn::None);
            tracing::debug!(
                source = source_label,
                page_id = token.page_id.as_u64(),
                ?readiness,
                ?next_turn,
                "Page scheduler turn failed: {error}"
            );
            match next_turn {
                PageOwnerNextTurn::Ordinary => self.signal_internal_page_turn_source(
                    token,
                    RendererOwnerWakeSource::SchedulerContinuation,
                ),
                PageOwnerNextTurn::DocumentLifecycle => {
                    self.signal_internal_document_lifecycle_turn(token);
                }
                PageOwnerNextTurn::None => {}
            }
            return RenderRuntimeDispatchOutcome::PageTurnComplete {
                result: Ok(()),
                parser_continuation_admitted,
            };
        }

        self.finish_page_turn(
            token,
            entry,
            source_label,
            output_ordering,
            parser_continuation_admitted,
        )
    }

    pub(super) async fn run_one_owner_maintenance_turn(
        &self,
        task: RendererOwnerMaintenanceTask,
    ) -> RenderRuntimeDispatchOutcome {
        let token = task.token();
        let entry = match checkout_entry_for_owner_turn_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(LivePageEntryCheckoutError::Busy) => {
                // A maintenance task may wait behind other bounded owner
                // turns, but those turns always restore the Page entry before
                // returning or parking their continuation. Reaching this arm
                // would therefore mean two owner turns retained the same Page
                // entry concurrently, not a condition maintenance can retry.
                panic!(
                    "renderer page {} remained checked out at a serialized owner-maintenance boundary",
                    token.page_id.as_u64()
                );
            }
            Err(LivePageEntryCheckoutError::Retired | LivePageEntryCheckoutError::Missing) => {
                return RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()));
            }
        };
        let executor = entry.page_vm().local_executor.clone();
        let (entry, action_result) =
            execute_owner_maintenance_task_on_local_lane(executor, entry, task).await;

        // Rearm the stable maintenance residence regardless of the V8
        // notification result. Leaving the admitted deadline unresolved would
        // either stop future maintenance or recreate the expired-deadline
        // owner spin this lane is designed to eliminate.
        let settlement_result =
            settle_owner_maintenance_task_on_bound_owner_local_store(task, Instant::now());
        // Maintenance does not own protocol/runtime output publication. A
        // plain residence restore keeps that boundary with the Page task or
        // command that produced the output.
        restore_entry_after_command_on_bound_owner_local_store(token, entry);

        let result = match (action_result, settlement_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(action_error), Ok(())) => Err(action_error),
            (Ok(()), Err(settlement_error)) => Err(settlement_error),
            (Err(action_error), Err(settlement_error)) => Err(anyhow!(
                "owner maintenance failed ({action_error:#}) and its residence settlement also failed ({settlement_error:#})"
            )),
        };
        RenderRuntimeDispatchOutcome::BackgroundComplete(result)
    }

    /// Admit every Page whose scheduler-owned task deadline is due.
    ///
    /// The selected turn still arbitrates and executes exactly one ready
    /// source; future deadlines remain resident in the owner-local index.
    pub(super) fn enqueue_due_page_turns(
        &self,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> bool {
        // Future work remains only in its Page-owned residence and the
        // owner-local deadline index. A deadline must actually be due before
        // admission.
        let due_at_or_before = Instant::now();
        let tokens = snapshot_due_page_task_tokens_on_bound_owner_local_store(due_at_or_before);
        let admitted = !tokens.is_empty();
        for token in tokens {
            self.enqueue_page_turn(token, PageTurnTrigger::Deadline, true, pending_turns);
        }
        admitted
    }

    /// Claim at most one due housekeeping residence. Other due Pages remain
    /// indexed and are admitted on subsequent owner-loop iterations, so the
    /// lane is bounded and cannot drain ahead of Page work or commands.
    pub(super) fn enqueue_one_due_owner_maintenance_turn(
        &self,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) -> bool {
        let Some(task) =
            claim_due_owner_maintenance_task_on_bound_owner_local_store(Instant::now())
        else {
            return false;
        };
        pending_turns.push_back(RenderRuntimePendingTurn {
            reply_tx: None,
            turn: RenderRuntimeTurn::RunOwnerMaintenance { task },
            allow_command_overtake: true,
            command_admission_output_predecessor: None,
        });
        true
    }
}
