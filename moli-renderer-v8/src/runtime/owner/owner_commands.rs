use super::*;

impl RendererOwnerHandle {
    /// Selects the immutable layout policy for this renderer owner.
    ///
    /// A browser owner may configure the default policy before its first Page
    /// is constructed. Page construction seals the value so a shared renderer
    /// owner can never host Pages with conflicting browser-level policies.
    pub fn configure_layout_policy(&self, policy: LayoutPolicy) -> Result<()> {
        self.configure_layout(policy, false)
    }

    /// Fixes both layout policy and scrollbar visibility before Page creation.
    pub fn configure_layout(&self, policy: LayoutPolicy, scrollbars_hidden: bool) -> Result<()> {
        self.configure_layout_configuration(moli_page_types::LayoutConfiguration {
            policy,
            scrollbars_hidden,
        })
    }

    /// Seals the complete immutable layout configuration for this owner.
    pub fn configure_layout_configuration(
        &self,
        configuration: moli_page_types::LayoutConfiguration,
    ) -> Result<()> {
        ensure!(
            !configuration.scrollbars_hidden || configuration.policy.uses_real_layout(),
            "hiding scrollbars requires real layout"
        );
        let mut state = self.state.layout_configuration.lock();
        if let Some(configured) = *state {
            ensure!(
                configured == configuration,
                "renderer owner layout policy is configured as {:?} (scrollbars hidden: {}), cannot change it to {:?} (scrollbars hidden: {})",
                configured.policy,
                configured.scrollbars_hidden,
                configuration.policy,
                configuration.scrollbars_hidden
            );
        } else {
            *state = Some(configuration);
        }
        Ok(())
    }

    pub fn layout_policy(&self) -> LayoutPolicy {
        self.layout_configuration().policy
    }

    pub fn scrollbars_hidden(&self) -> bool {
        self.layout_configuration().scrollbars_hidden
    }

    pub fn layout_configuration(&self) -> moli_page_types::LayoutConfiguration {
        self.state.layout_configuration.lock().unwrap_or_default()
    }

    pub(super) fn seal_layout_configuration_for_page_creation(
        &self,
    ) -> moli_page_types::LayoutConfiguration {
        let mut state = self.state.layout_configuration.lock();
        *state.get_or_insert_with(Default::default)
    }

    pub(crate) fn new(
        local_executor: JsLocalExecutor,
        next_page_id: Arc<AtomicU64>,
        browser_context_runtime: RendererBrowserContextRuntime,
    ) -> (Self, RenderRuntimeOwner) {
        let (page_wake_tx, page_wake_rx) = mpsc::unbounded_channel();
        let (inspector_io_wake_tx, inspector_io_wake_rx) = mpsc::unbounded_channel();
        let (shared_worker_wake_tx, shared_worker_wake_rx) = shared_worker_owner_wake_channel();
        browser_context_runtime.add_shared_worker_owner_wake_sender(shared_worker_wake_tx);
        let (service_worker_wake_tx, service_worker_wake_rx) = service_worker_owner_wake_channel();
        browser_context_runtime.add_service_worker_owner_wake_sender(service_worker_wake_tx);
        let owner_local_host_id = RendererOwnerLocalHostId::new(
            NEXT_RENDERER_OWNER_LOCAL_HOST_ID.fetch_add(1, Ordering::Relaxed),
        );
        browser_context_runtime.set_shared_worker_owner_local_host_id(owner_local_host_id);
        let state = Arc::new(RendererOwnerState {
            page_table: RendererPageTable::default(),
            local_executor,
            next_page_id,
            page_wake_tx,
            render_runtime_admission: std::sync::OnceLock::new(),
            runtime_handle: std::sync::OnceLock::new(),
            inspector_io_wake_tx,
            browser_context_runtime,
            devtools_target_shutdown_registry: Default::default(),
            owner_local_host_id,
            layout_configuration: Mutex::new(None),
            context_shutdown_notify: tokio::sync::Notify::new(),
            #[cfg(test)]
            command_dispatch_gate: Mutex::new(None),
            #[cfg(test)]
            publish_next_command_output_before_settlement: std::sync::atomic::AtomicBool::new(
                false,
            ),
            #[cfg(debug_assertions)]
            owner_local_thread_id: Mutex::new(None),
        });
        let provisional = Self {
            state,
            render_runtime: RenderRuntimeHandle::disconnected(),
        };
        let render_runtime_owner = RenderRuntimeOwner::spawn(
            provisional.clone(),
            page_wake_rx,
            inspector_io_wake_rx,
            shared_worker_wake_rx,
            service_worker_wake_rx,
        );
        let render_runtime = render_runtime_owner.handle();
        provisional
            .state
            .render_runtime_admission
            .set(render_runtime.clone())
            .expect("render runtime admission must be initialized exactly once");
        (
            Self {
                render_runtime,
                ..provisional
            },
            render_runtime_owner,
        )
    }

    #[cfg(debug_assertions)]
    pub(in crate::runtime) fn bind_or_check_local_runtime_thread(&self) -> Result<ThreadId> {
        let current_thread_id = std::thread::current().id();
        let mut owner_local_thread_id = self.state.owner_local_thread_id.lock();
        match *owner_local_thread_id {
            Some(thread_id) => {
                ensure!(
                    thread_id == current_thread_id,
                    "renderer owner local runtime was entered on a different thread"
                );
                Ok(thread_id)
            }
            None => {
                *owner_local_thread_id = Some(current_thread_id);
                Ok(current_thread_id)
            }
        }
    }

    pub(in crate::runtime) fn owner_local_context(&self) -> Result<RendererOwnerLocalContext> {
        let render_runtime = self
            .state
            .render_runtime_admission
            .get()
            .cloned()
            .ok_or_else(|| anyhow!("render runtime admission is not initialized"))?;
        Ok(RendererOwnerLocalContext {
            owner_state: self.state.clone(),
            render_runtime,
            local_host_id: self.state.owner_local_host_id,
            #[cfg(debug_assertions)]
            local_thread_id: self.bind_or_check_local_runtime_thread()?,
        })
    }

    pub fn refresh_page_view_for_testing(&self, view: RendererPageView) -> Result<()> {
        self.state.page_table.refresh(
            view.page_id,
            view.vm_creation_id,
            view.view_generation,
            view.page_state.requested_url.clone(),
            view.page_state.final_url.clone(),
            view.page_state.document_title.clone(),
            view.page_state.status,
        )
    }

    pub fn remove_page_for_testing(&self, page_id: PageId) {
        self.state.page_table.remove(page_id);
    }

    pub fn allocate_page_id(&self) -> PageId {
        PageId::new(self.state.next_page_id.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn allocate_page_reservation_token(&self) -> RendererPageReservationToken {
        RendererPageReservationToken::new(self.state.owner_local_host_id, self.allocate_page_id())
    }

    pub fn set_renderer_output_transport_sender(
        &self,
        sender: super::RendererOutputTransportSender,
    ) {
        self.state
            .browser_context_runtime
            .set_renderer_output_transport_sender(sender);
    }

    pub(super) fn publish_renderer_output(&self, output: RendererOutputPublication) {
        if let Some(sender) = self
            .state
            .browser_context_runtime
            .renderer_output_transport_sender()
        {
            let _ = output.publish_to(&sender);
        }
    }

    /// Completes the protocol-side owner reservation after renderer bootstrap
    /// has either opened its concrete stream or failed before doing so.
    ///
    /// The journal and this marker share one FIFO transport. Protocol therefore
    /// observes `Opened` before this release on success, while an early failure
    /// produces only the release. Never move this to the navigation completion
    /// channel: that independent channel cannot order against stream opening.
    pub(in crate::runtime) fn release_page_output_reservation(
        &self,
        reservation: RendererPageReservationToken,
    ) {
        if let Some(sender) = self
            .state
            .browser_context_runtime
            .renderer_output_transport_sender()
        {
            let _ = sender.send(RendererOutputTransportMessage::page_reservation_released(
                reservation.local_host_id(),
                reservation.page_id(),
            ));
        }
    }

    pub(super) fn signal_pending_phase_one_admission(
        &self,
        wake_token: RendererPageToken,
        admission: PhaseOneResidenceAdmission,
    ) {
        match admission {
            // The parser already froze and published the exact Fetch pause
            // record while restoring this Page residence. Protocol continuation,
            // not a second source-shaped wake, admits the parser afterwards.
            PhaseOneResidenceAdmission::ParserBlockingSourceLoad => {}
            PhaseOneResidenceAdmission::ReadyPageTurn => self.signal_internal_page_turn_source(
                wake_token,
                RendererOwnerWakeSource::SchedulerContinuation,
            ),
            PhaseOneResidenceAdmission::WaitingForProducer => {}
        }
    }

    pub(super) fn signal_internal_document_lifecycle_turn(&self, token: RendererPageToken) {
        self.signal_internal_page_turn_source(
            token,
            RendererOwnerWakeSource::Runtime(
                RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
            ),
        );
    }

    pub(super) fn post_response_owner_work_continuation(
        &self,
        token: RendererPageToken,
        work: RendererPostResponseOwnerWork,
    ) -> RendererPageCommandPostResponseContinuation {
        let page_wake_tx = self.state.page_wake_tx.clone();
        RendererPageCommandPostResponseContinuation::new(move || {
            if let Some(document) = work.document_lifecycle {
                let _ = page_wake_tx.send(RendererOwnerWake::post_response_document_lifecycle(
                    token, document,
                ));
            }
            if let Some(handoff) = work.navigation_handoff {
                let _ = page_wake_tx.send(RendererOwnerWake::top_level_navigation_handoff(
                    token, handoff,
                ));
            }
        })
    }

    pub(super) fn signal_internal_document_lifecycle_turn_if_resident(
        &self,
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
    ) {
        let Ok(mut entry) = take_entry_for_command_on_bound_owner_local_store(token) else {
            return;
        };
        let should_signal = entry.pending_document_lifecycle_identity() == Some(document);
        self.restore_live_page_entry(token, entry);
        if should_signal {
            self.signal_internal_document_lifecycle_turn(token);
        }
    }

    pub(super) fn signal_internal_page_turn_source(
        &self,
        token: RendererPageToken,
        source: RendererOwnerWakeSource,
    ) {
        let _ = self
            .state
            .page_wake_tx
            .send(RendererOwnerWake::page(token, source));
    }

    pub(super) fn signal_replacement_document_view_settled(
        &self,
        token: RendererPageToken,
        vm_creation_id: u64,
    ) {
        let _ = self
            .state
            .page_wake_tx
            .send(RendererOwnerWake::replacement_document_view_settled(
                token,
                vm_creation_id,
            ));
    }

    pub(super) fn owner_wake_sender_for_page(
        &self,
        owner_local_context: &RendererOwnerLocalContext,
        page_id: PageId,
    ) -> RendererOwnerWakeSender {
        RendererOwnerWakeSender::new(
            self.state.page_wake_tx.clone(),
            renderer_page_token_for_owner_context(owner_local_context, page_id),
        )
    }

    pub fn refresh_page_view_on_slot_for_testing(
        &self,
        slot: &RendererPageSlotHandle,
        view: RendererPageView,
    ) -> Result<()> {
        ensure!(
            self.state.page_table.owns_slot(slot),
            "renderer owner does not own slot for page {}",
            slot.page_id().as_u64()
        );
        slot.refresh_owned_view(view)
    }

    pub async fn dispatch_command(
        &self,
        command: RendererOwnerCommand,
    ) -> Result<RendererOwnerReply> {
        if moli_trace::cdp_nav_timing_enabled()
            && let Some(command_label) = owner_command_timing_label(&command)
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                command = command_label,
                stage = "owner_command_enqueue",
            );
        }
        let reply_rx = self.enqueue_command_with_reply(command)?;
        reply_rx
            .await
            .map_err(|_| anyhow!("render runtime reply channel closed"))?
    }

    /// Enqueue a renderer command without waiting for the reply, returning the
    /// reply channel so the caller can `await` it later (or hand it off to a
    /// different code path). This enables fire-then-defer patterns where the
    /// renderer thread can process the work in parallel with conn-side
    /// bookkeeping.
    pub fn enqueue_command_with_reply(
        &self,
        command: RendererOwnerCommand,
    ) -> Result<oneshot::Receiver<Result<RendererOwnerReply>>> {
        if let Err(error) = self.ensure_context_owner_accepts_commands() {
            self.release_rejected_command_output_reservation(&command);
            return Err(error);
        }
        if moli_trace::cdp_nav_timing_enabled()
            && let Some(command_label) = owner_command_timing_label(&command)
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                command = command_label,
                stage = "owner_command_enqueue_deferred",
            );
        }
        match self.render_runtime.enqueue_owned(command) {
            Ok(reply_rx) => Ok(reply_rx),
            Err(error) => {
                let (command, error) = error.into_parts();
                self.release_rejected_command_output_reservation(&command);
                Err(error)
            }
        }
    }

    pub(super) fn context_shutdown_started(&self) -> bool {
        self.state.page_table.is_terminal()
    }

    pub(super) fn ensure_context_owner_accepts_commands(&self) -> Result<()> {
        ensure!(
            !self.context_shutdown_started(),
            "renderer browser context owner has been dropped"
        );
        Ok(())
    }

    pub(crate) fn terminate_for_context_owner_shutdown(&self) {
        // Closing command admission and draining the queue use the same
        // RenderRuntimeState lock. Every command therefore belongs to exactly
        // one side of this boundary: rejected with ownership returned to the
        // caller, or durably enqueued for the renderer terminal drain.
        self.render_runtime.close_admission();
        self.state.devtools_target_shutdown_registry.terminate_all();
        self.state
            .page_table
            .terminate_and_cancel_all_contexts(RendererPageContextCancelReason::ContextDropped);
        self.state.context_shutdown_notify.notify_one();
    }

    #[cfg(test)]
    pub(crate) fn install_command_dispatch_gate_for_testing(
        &self,
    ) -> (
        crossbeam_channel::Receiver<()>,
        crossbeam_channel::Sender<()>,
    ) {
        let (entered_tx, entered_rx) = crossbeam_channel::bounded(1);
        let (release_tx, release_rx) = crossbeam_channel::bounded(1);
        let previous = self.state.command_dispatch_gate.lock().replace(
            RendererCommandDispatchGateForTesting {
                entered_tx,
                release_rx,
            },
        );
        assert!(
            previous.is_none(),
            "renderer command test gate already installed"
        );
        (entered_rx, release_tx)
    }

    #[cfg(test)]
    pub(crate) fn publish_next_command_output_before_settlement_for_testing(&self) {
        assert!(
            !self
                .state
                .publish_next_command_output_before_settlement
                .swap(true, Ordering::AcqRel),
            "renderer command output publication test hook already installed"
        );
    }

    #[cfg(test)]
    pub(crate) fn close_command_admission_for_testing(&self) {
        self.render_runtime.close_admission();
    }

    #[cfg(test)]
    pub(super) fn wait_on_command_dispatch_gate_for_testing(&self) {
        let gate = self.state.command_dispatch_gate.lock().take();
        let Some(gate) = gate else {
            return;
        };
        let _ = gate.entered_tx.send(());
        let _ = gate.release_rx.recv();
    }

    pub(super) fn release_rejected_command_output_reservation(
        &self,
        command: &RendererOwnerCommand,
    ) {
        let reservation = match command {
            RendererOwnerCommand::CreateHtmlPage(request) => Some(request.page_reservation),
            RendererOwnerCommand::PrepareStreamingRawDocument { token, .. } => Some(*token),
            _ => None,
        };
        if let Some(reservation) = reservation {
            self.release_page_output_reservation(reservation);
        }
    }

    pub(super) async fn dispatch_command_inline_on_owner_local_store(
        &self,
        command: RendererOwnerCommand,
        owner_local_store: &mut RendererOwnerLocalStore,
    ) -> RenderRuntimeDispatchOutcome {
        match command {
            RendererOwnerCommand::ReserveLivePageReplacement(request) => {
                let RendererPageReplacementReservationRequest {
                    token,
                    reservation_nonce,
                    replacement_scope: _replacement_scope,
                } = request;
                if token.local_host_id != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "replacement reservation belongs to another renderer owner"
                    ))
                    .into();
                }
                owner_local_store
                    .reserve_live_page_replacement(token, reservation_nonce)
                    .map(RendererOwnerReply::LivePageReplacementReserved)
                    .into()
            }
            RendererOwnerCommand::CancelLivePageReplacementReservation {
                token,
                reservation_nonce,
            } => {
                if token.local_host_id != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "reservation cancellation belongs to another renderer owner"
                    ))
                    .into();
                }
                owner_local_store
                    .cancel_live_page_replacement_reservation(token, reservation_nonce);
                Ok(RendererOwnerReply::PreparedRendererDocumentCanceled).into()
            }
            RendererOwnerCommand::CreateHtmlPage(request) => {
                let reservation = request.page_reservation;
                let outcome = self
                    .create_page_reply_from_html_request_on_owner_local_store(
                        request,
                        owner_local_store,
                    )
                    .await;
                self.release_page_output_reservation(reservation);
                outcome
            }
            RendererOwnerCommand::CancelPendingAuxiliaryPage { reservation } => {
                if reservation.local_host_id() != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "auxiliary cancellation belongs to another renderer owner"
                    ))
                    .into();
                }
                owner_local_store.cancel_staged_auxiliary_page(reservation);
                Ok(RendererOwnerReply::PendingAuxiliaryPageCanceled).into()
            }
            RendererOwnerCommand::ReleaseCapturedDocumentEnvironment { id } => {
                owner_local_store.release_captured_document_environment(id);
                Ok(RendererOwnerReply::CapturedDocumentEnvironmentReleased).into()
            }
            RendererOwnerCommand::PrepareStreamingRawDocument { token, request } => {
                let outcome = self
                    .prepare_renderer_document_on_owner_local_store(
                        token,
                        request,
                        owner_local_store,
                    )
                    .await;
                self.release_page_output_reservation(token);
                outcome
            }
            RendererOwnerCommand::UpdatePreparedRendererDocumentCommitConfiguration {
                token,
                configuration,
            } => {
                if token.local_host_id() != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "prepared document configuration update belongs to renderer owner {}, not {}",
                        token.local_host_id().as_u64(),
                        self.state.owner_local_host_id.as_u64()
                    ))
                    .into();
                }
                match owner_local_store
                    .update_prepared_document_commit_configuration(token, configuration)
                    .map(|()| {
                        RendererOwnerReply::PreparedRendererDocumentCommitConfigurationUpdated
                    }) {
                    Ok(reply) => Ok(reply).into(),
                    Err(error) => Err(error).into(),
                }
            }
            RendererOwnerCommand::CommitPreparedRendererDocument { permit } => {
                let token = permit.prepared_document();
                if token.replacement.is_some() {
                    owner_local_store.cancel_prepared_document(token);
                    return Err(anyhow!(
                        "a prepared Page replacement requires the stable Page commit entry point"
                    ))
                    .into();
                }
                if token.local_host_id() != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "prepared document commit permit belongs to renderer owner {}, not {}",
                        token.local_host_id().as_u64(),
                        self.state.owner_local_host_id.as_u64()
                    ))
                    .into();
                }
                match owner_local_store.take_prepared_document(token) {
                    Ok(residence) => {
                        self.create_page_reply_from_prepared_document_on_owner_local_store(
                            token.page_id(),
                            residence,
                            owner_local_store,
                        )
                        .await
                    }
                    Err(error) => Err(error).into(),
                }
            }
            RendererOwnerCommand::CommitPreparedPageReplacement { permit } => {
                let reservation = permit.prepared_document();
                if reservation.local_host_id() != self.state.owner_local_host_id
                    || reservation.replacement.is_none()
                {
                    return Err(anyhow!(
                        "Page replacement permit does not belong to this renderer owner"
                    ))
                    .into();
                }
                match owner_local_store.take_prepared_document(reservation) {
                    Ok(residence) => {
                        Box::pin(self.commit_prepared_page_replacement(reservation, residence))
                            .await
                    }
                    Err(error) => {
                        let document_preserved = reservation.replacement.is_some_and(|admission| {
                            self.state
                                .page_table
                                .active_vm_creation_id(reservation.page_id())
                                == Some(admission.expected_vm_creation_id)
                        });
                        let error = if document_preserved {
                            crate::runtime::RendererPageReplacementError::document_preserved(error)
                        } else {
                            crate::runtime::RendererPageReplacementError::document_unavailable(
                                error,
                            )
                        };
                        Err(error.into()).into()
                    }
                }
            }
            RendererOwnerCommand::CancelPreparedRendererDocument { token } => {
                if token.local_host_id() != self.state.owner_local_host_id {
                    return Err(anyhow!(
                        "prepared document cancellation belongs to renderer owner {}, not {}",
                        token.local_host_id().as_u64(),
                        self.state.owner_local_host_id.as_u64()
                    ))
                    .into();
                }
                owner_local_store.cancel_prepared_document(token);
                Ok(RendererOwnerReply::PreparedRendererDocumentCanceled).into()
            }
            command @ (RendererOwnerCommand::RunAsyncPageCommand { .. }
            | RendererOwnerCommand::RunProtocolPageCommand { .. }) => {
                let (token, command, capture_policy) = match command {
                    RendererOwnerCommand::RunAsyncPageCommand { token, command } => (
                        token,
                        command,
                        super::RendererPageStateCapturePolicy::FullReport,
                    ),
                    RendererOwnerCommand::RunProtocolPageCommand { token, command } => (
                        token,
                        command,
                        super::RendererPageStateCapturePolicy::ProtocolTurn,
                    ),
                    _ => unreachable!("combined renderer page command pattern must match"),
                };
                if moli_trace::cdp_nav_timing_enabled()
                    && let Some(command_label) = renderer_page_command_timing_label(&command)
                {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        command = command_label,
                        page_id = token.page_id.as_u64(),
                        stage = "owner_command_received",
                    );
                }
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::RunLivePageCommand {
                        token,
                        command,
                        capture_policy,
                    },
                ))
            }
            RendererOwnerCommand::FinalizeRuntimeInspectorSessionDetach {
                token,
                inspector_session_id,
                mut pause_guard,
            } => {
                let mut entry = match checkout_entry_for_owner_turn_on_bound_owner_local_store(
                    token,
                ) {
                    Ok(entry) => entry,
                    Err(
                        LivePageEntryCheckoutError::Retired | LivePageEntryCheckoutError::Missing,
                    ) => {
                        pause_guard.complete();
                        return Ok(RendererOwnerReply::RuntimeInspectorSessionDetachFinalized(
                            false,
                        ))
                        .into();
                    }
                    Err(LivePageEntryCheckoutError::Busy) => {
                        return Err(anyhow!(
                            "renderer page {} remained checked out while finalizing Inspector session detach",
                            token.page_id.as_u64()
                        ))
                        .into();
                    }
                };
                let detached = entry
                    .page_vm_mut()
                    .detach_runtime_inspector_session(inspector_session_id.as_deref());
                restore_entry_after_command_on_bound_owner_local_store(token, entry);
                pause_guard.complete();
                Ok(RendererOwnerReply::RuntimeInspectorSessionDetachFinalized(
                    detached,
                ))
                .into()
            }
            RendererOwnerCommand::WaitForNetworkIdle {
                token,
                timeout_ms,
                loader,
            } => {
                let deadline = match checked_live_page_wait_deadline(
                    Instant::now(),
                    timeout_ms,
                    "networkidle",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageNetworkIdle {
                        token,
                        state: PageVmNetworkIdleWaitState::default(),
                        deadline,
                        loader,
                    },
                ))
            }
            RendererOwnerCommand::WaitForDomStable {
                token,
                timeout_ms,
                loader,
            } => {
                let deadline = match checked_live_page_wait_deadline(
                    Instant::now(),
                    timeout_ms,
                    "domstable",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return Err(error).into(),
                };
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLivePageDomStable {
                        token,
                        state: PageVmDomStableWaitState::default(),
                        deadline,
                        loader,
                    },
                ))
            }
            RendererOwnerCommand::RemovePage { token } => {
                remove_page_on_bound_owner_local_store_via_local_task(
                    self.state.local_executor.clone(),
                    token,
                )
                .await
                .map(|()| RendererOwnerReply::PageRemoved)
                .into()
            }
            RendererOwnerCommand::TestingCurrentPageState { token } => {
                owner_local_store_session(owner_local_store)
                    .current_page_state_for_testing(token)
                    .map(RendererOwnerReply::TestingCurrentPageState)
                    .into()
            }
            RendererOwnerCommand::TestingRendererPageView { token } => {
                owner_local_store_session(owner_local_store)
                    .renderer_page_view_for_testing(token)
                    .map(RendererOwnerReply::TestingRendererPageView)
                    .into()
            }
            RendererOwnerCommand::TestingOwnerSlot { token } => {
                owner_local_store_session(owner_local_store)
                    .owner_slot_for_testing(token)
                    .map(RendererOwnerReply::TestingOwnerSlot)
                    .into()
            }
            RendererOwnerCommand::TestingHostInstanceKey { token } => {
                owner_local_store_session(owner_local_store)
                    .host_instance_key_for_testing(token)
                    .map(RendererOwnerReply::TestingHostInstanceKey)
                    .into()
            }
            RendererOwnerCommand::TestingHostUniqueDocumentIsolateCount { token } => {
                owner_local_store_session(owner_local_store)
                    .host_unique_document_isolate_count_for_testing(token)
                    .map(RendererOwnerReply::TestingHostUniqueDocumentIsolateCount)
                    .into()
            }
            #[cfg(test)]
            RendererOwnerCommand::TestingDeferredPageVmDropPendingCount => {
                Ok(RendererOwnerReply::TestingDeferredPageVmDropPendingCount(
                    deferred_page_vm_drop_pending_count_for_testing(),
                ))
                .into()
            }
        }
    }
}
