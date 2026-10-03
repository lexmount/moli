use super::*;

impl RendererOwnerLocalStore {
    pub(in crate::runtime) fn release_captured_document_environment(&mut self, id: u64) {
        self.captured_document_environments.remove(&id);
    }

    pub(super) fn stage_related_initial_empty_page(
        &mut self,
        owner: &RendererOwnerLocalContext,
        scope: &mut v8::PinScope<'_, '_>,
        pending: &RendererPendingAuxiliaryPage,
        source_environment: &RendererPageScriptEnvironment,
        source_bindings: &crate::native_bridge::bindings::NativeBridgeBindings,
        init: RendererRelatedInitialEmptyPageInit,
    ) -> Result<RendererPageScriptEnvironment> {
        let reservation = pending.page_reservation();
        ensure!(
            reservation.local_host_id() == owner.local_host_id,
            "related Page reservation belongs to another renderer owner"
        );
        let page_id = reservation.page_id();
        ensure!(
            !self.staged_auxiliary_pages.contains_key(&reservation),
            "related Page has already been staged"
        );
        ensure!(
            self.page_hosts
                .get(&owner.local_host_id)
                .and_then(|host| host.pages.get(&page_id))
                .is_none(),
            "related Page cannot replace an existing Page"
        );
        let RendererRelatedInitialEmptyPageInit {
            dom_host,
            loader,
            env,
            inherited_origin,
            inherited_security_token,
            opener,
            window,
            name,
        } = init;
        ensure!(
            pending.popup_id() == window.id(),
            "auxiliary identity changed before staging"
        );
        let token = renderer_page_token_for_owner_context(owner, page_id);
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            owner.owner_state.page_wake_tx.clone(),
            token,
        );
        let runtime_source =
            crate::page_task_queue::PageRuntimeTaskSource::new(Some(owner_wake.clone()));
        let (runtime_wake, stable_owner_wake) =
            runtime_source
                .owner_attached_page_source_wakes()
                .ok_or_else(|| anyhow!("related Page has no stable owner wake"))?;
        let (task_sources, producer_routes) =
            RendererPageOwnedTaskSources::new(runtime_wake, stable_owner_wake);
        runtime_source.bind_page_task_producer_routes(producer_routes)?;
        let sender = runtime_source
            .v8_foreground_task_sender()
            .ok_or_else(|| anyhow!("related Page has no V8 foreground route"))?;
        let bootstrap = source_environment.bootstrap_related_page_document_isolate_in_scope(
            scope,
            source_bindings,
            sender,
        )?;
        let isolate = bootstrap.clone_renderer_document_isolate_handle_for_owner_retention();
        let inspector_backend = bootstrap.inspector_isolate_backend_handle();
        let page_inspector = DocumentInspectorBinding::new(inspector_backend.clone());
        let stream = RendererOutputStreamIdentity::new_page(
            owner.local_host_id,
            page_id,
            page_inspector.agent_token(),
        );
        // The protocol target does not exist yet. Retain creator-script facts
        // locally until adoption binds this exact Page to its output owner.
        let journal = RendererTurnOutputJournal::new(stream);
        let environment = RendererPageScriptEnvironment::new(
            page_id.as_u64(),
            isolate.clone(),
            inspector_backend,
            runtime_source,
            journal.clone(),
        );
        environment
            .bind_auxiliary_allocator(RendererAuxiliaryPageAllocator::new(owner.clone(), page_id));
        environment.bind_window_identity(name, Some(window));
        environment.set_opener(opener);
        let bootstrap = bootstrap
            .with_page_inspector(page_inspector.with_output_journal(journal.clone()))
            .with_renderer_page_script_environment(environment.clone());
        let reservation_id = self.next_renderer_document_isolate_reservation_id;
        self.next_renderer_document_isolate_reservation_id = reservation_id
            .checked_add(1)
            .ok_or_else(|| anyhow!("renderer isolate reservation identity exhausted"))?;
        self.host_for_id(owner.local_host_id)
            .reserved_renderer_document_isolates
            .entry(page_id)
            .or_default()
            .push(RendererDocumentIsolateReservationEntry {
                id: reservation_id,
                handle: isolate,
                output_journal: journal,
                retire_output_journal_on_drop: true,
                initial_task_sources: Some(task_sources),
                _accounting: RendererDocumentIsolateReservationAccounting::new(),
            });
        let isolate_reservation = RendererDocumentIsolateReservation {
            inner: Rc::new(RendererDocumentIsolateReservationState {
                token,
                reservation_id,
                active: std::cell::Cell::new(true),
            }),
        };
        let hooks = PageVmRuntimeHooks::with_owner_wake(
            owner_wake,
            owner.owner_state.browser_context_runtime.clone(),
        )
        .with_renderer_document_isolate_allocator(RendererDocumentIsolateAllocator::new(
            owner.clone(),
            page_id,
        ))
        .with_auxiliary_page_bootstrap(bootstrap, isolate_reservation.clone());
        // Failure cleanup already owns the store; the reservation must not
        // recursively enter the bound store while unwinding construction.
        isolate_reservation.disarm_for_attach();
        let vm = match PageVm::new_related_initial_empty_in_scope(
            scope,
            page_id,
            owner.owner_state.local_executor.clone(),
            &loader,
            &env,
            hooks,
            dom_host,
            inherited_origin,
            &inherited_security_token,
        ) {
            Ok(vm) => vm,
            Err(error) => {
                self.remove_reserved_renderer_document_isolate(token, reservation_id);
                return Err(error);
            }
        };
        isolate_reservation.inner.active.set(true);
        assert!(
            self.staged_auxiliary_pages
                .insert(reservation, vm)
                .is_none()
        );
        Ok(environment)
    }

    pub(super) fn take_staged_auxiliary_page(
        &mut self,
        reservation: RendererPageReservationToken,
    ) -> Option<PageVm> {
        self.staged_auxiliary_pages.remove(&reservation)
    }

    pub(super) fn retire_staged_auxiliary_pages(&mut self) {
        for reservation in self
            .staged_auxiliary_pages
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            self.cancel_staged_auxiliary_page(reservation);
        }
    }

    pub(in crate::runtime) fn cancel_staged_auxiliary_page(
        &mut self,
        reservation: RendererPageReservationToken,
    ) {
        let Some(mut vm) = self.staged_auxiliary_pages.remove(&reservation) else {
            return;
        };
        if let Some(environment) = vm.renderer_page_script_environment() {
            environment.close_browsing_context();
        }
        let isolate_reservation = vm.take_renderer_document_isolate_reservation_for_attach();
        if let Some(reservation) = &isolate_reservation {
            reservation.disarm_for_attach();
        }
        vm.close_for_context_teardown();
        drop(vm);
        if let Some(reservation) = isolate_reservation {
            self.remove_reserved_renderer_document_isolate(
                reservation.token(),
                reservation.reservation_id(),
            );
        }
    }
}
