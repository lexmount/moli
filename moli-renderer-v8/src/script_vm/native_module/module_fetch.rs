use super::*;

impl ScriptVm {
    pub(crate) fn dynamic_module_import_owner_is_current(
        &self,
        owner: crate::module_runtime::DynamicModuleImportOwner,
    ) -> bool {
        self._context_host
            .borrow()
            .dynamic_module_import_owner_is_current(owner)
    }
    pub(super) fn advance_native_dynamic_module_import_job_with_body<Body>(
        &mut self,
        mut job: NativeModuleGraphJob,
        body: &mut Body,
    ) -> std::result::Result<(), String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        if job
            .dynamic_import_request()
            .and_then(PendingDynamicModuleImport::child_browsing_context_handle)
            .is_some()
        {
            return self.advance_child_native_dynamic_module_import_job(job);
        }
        let advance = match job.advance_dynamic_import_owner_lane(self) {
            Ok(advance) => advance,
            Err(error) => {
                let mut fanout = NativeDynamicModuleTerminalFanout::default();
                fanout.push_graph_advance_failure(job, error);
                return self
                    .handle_dynamic_module_terminal_fanout_for_owner_with_body(fanout, body)
                    .map(|_| ())
                    .map_err(|error| error.to_string());
            }
        };
        self.continue_native_dynamic_module_import_after_tree_advance_with_body(job, advance, body)
    }
    pub(super) fn continue_native_dynamic_module_import_after_tree_advance_with_body<Body>(
        &mut self,
        job: NativeModuleGraphJob,
        advance: NativeModuleGraphJobAdvance,
        body: &mut Body,
    ) -> std::result::Result<(), String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        if job
            .dynamic_import_request()
            .and_then(PendingDynamicModuleImport::child_browsing_context_handle)
            .is_some()
        {
            return self
                .continue_child_native_dynamic_module_import_after_tree_advance(job, advance);
        }
        self.continue_main_native_dynamic_module_import_after_tree_advance_with_body(
            job, advance, body,
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }
    pub(super) fn continue_main_native_dynamic_module_import_after_tree_advance_with_body<Body>(
        &mut self,
        mut job: NativeModuleGraphJob,
        advance: NativeModuleGraphJobAdvance,
        body: &mut Body,
    ) -> Result<NativeDynamicModuleTerminalFanoutOutcome>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let mut outcome = NativeDynamicModuleTerminalFanoutOutcome::default();
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        match advance {
            NativeModuleGraphJobAdvance::NeedFetches(requests) => {
                let joined_clients = job.take_pending_joined_clients();
                let owner_module_fetch_starts = vec![None; requests.len()];
                let scheduled = self
                    .document_runtime
                    .suspend_native_dynamic_module_import_fetches(
                        requests,
                        joined_clients,
                        job,
                        owner_module_fetch_starts,
                    );
                fanout.extend_scheduled_dynamic_import_fetches(scheduled);
            }
            NativeModuleGraphJobAdvance::WaitingForFetches => {
                let joined_clients = job.take_pending_joined_clients();
                if joined_clients.is_empty() {
                    self.document_runtime
                        .resume_native_dynamic_module_import_front(job);
                    outcome.record_dynamic_import_job_resumed();
                } else {
                    self.document_runtime
                        .suspend_native_dynamic_module_import_fetches(
                            Vec::new(),
                            joined_clients,
                            job,
                            Vec::new(),
                        );
                    outcome.record_dynamic_import_wait_retained();
                }
            }
            NativeModuleGraphJobAdvance::Complete(graph) => {
                fanout.push_ready_import(NativeDynamicModuleImportReady { job, graph });
            }
        }
        outcome
            .merge(self.handle_dynamic_module_terminal_fanout_for_owner_with_body(fanout, body)?);
        Ok(outcome)
    }
    pub(super) fn run_completed_native_dynamic_module_import_action_with_body<Body>(
        &mut self,
        job: NativeModuleGraphJob,
        graph: ModuleGraphHandle,
        body: &mut Body,
    ) -> std::result::Result<FrameDocumentDynamicImportTerminalOutcome, String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let document_owner = job
            .dynamic_import_request()
            .expect("completed dynamic import graph must retain its request")
            .owner();
        if let Some((_child_handle, task_owner, realm_id)) = document_owner.child_parts() {
            let owner = task_owner.document_owner();
            let followup = self
                .child_document_modulator_store
                .dynamic_import_ready_followup(
                    owner,
                    realm_id,
                    NativeDynamicModuleImportReady { job, graph },
                );
            return Ok(
                FrameDocumentDynamicImportTerminalOutcome::from_owner_action_queue_followup(
                    self.apply_child_dynamic_import_followup(followup),
                ),
            );
        }
        self.run_main_dynamic_import_owner_action_with_body(
            FrameDocumentDynamicImportOwnerAction::ready(NativeDynamicModuleImportReady {
                job,
                graph,
            }),
            body,
        )
    }
    pub(super) fn run_main_dynamic_import_owner_action_with_body<Body>(
        &mut self,
        action: FrameDocumentDynamicImportOwnerAction,
        body: &mut Body,
    ) -> std::result::Result<FrameDocumentDynamicImportTerminalOutcome, String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        FrameDocumentDynamicImportOwnerActionRunner::new(
            ScriptVmMainDynamicImportOwnerActionHooks::new(self, body),
        )
        .run_owner_action(action)
    }
    pub(crate) fn has_ready_native_module_owner_actions(&mut self) -> bool {
        self.document_runtime.has_ready_native_module_owner_event()
    }
    #[cfg(test)]
    pub(crate) fn drain_ready_native_module_owner_actions(
        &mut self,
    ) -> Result<(
        NativeModuleOwnerActions,
        FrameDocumentModuleTerminalQueueFollowup,
    )> {
        self.drain_ready_native_module_owner_actions_with_body(
            &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
        )
    }
    pub(super) fn drain_ready_native_module_owner_actions_with_body<Body>(
        &mut self,
        body: &mut Body,
    ) -> Result<(
        NativeModuleOwnerActions,
        FrameDocumentModuleTerminalQueueFollowup,
    )>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        if !self.has_ready_native_module_owner_actions() {
            return Ok((
                NativeModuleOwnerActions::empty(),
                FrameDocumentModuleTerminalQueueFollowup::none(),
            ));
        }
        let Some(event) = self.document_runtime.take_next_native_module_owner_event() else {
            return Ok((
                NativeModuleOwnerActions::empty(),
                FrameDocumentModuleTerminalQueueFollowup::none(),
            ));
        };
        self.dispatch_native_module_owner_event_with_body(event, body)
    }
    pub(super) fn dispatch_native_module_owner_event_with_body<Body>(
        &mut self,
        event: NativeModuleOwnerEvent,
        body: &mut Body,
    ) -> Result<(
        NativeModuleOwnerActions,
        FrameDocumentModuleTerminalQueueFollowup,
    )>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        match event {
            NativeModuleOwnerEvent::ModuleMapTerminalNotification(notification) => {
                let fanout =
                    self.dispatch_native_document_modulator_terminal_notification(notification)?;
                self.handle_document_modulator_terminal_fanout_for_owner_with_body(fanout, body)
            }
            NativeModuleOwnerEvent::ModulepreloadLinkError(handle) => {
                body.note_page_realm_body_attempted();
                self.dispatch_preload_like_link_error_event(handle);
                Ok((
                    NativeModuleOwnerActions::empty(),
                    FrameDocumentModuleTerminalQueueFollowup::none(),
                ))
            }
        }
    }
    pub(crate) fn has_ready_runtime_owned_module_owner_actions(&mut self) -> bool {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .has_ready_module_script_continuation()
    }
    pub(crate) fn register_native_modulepreload_for_owner(
        &mut self,
        request: NativeModuleSingleFetchRequest,
    ) -> std::result::Result<Option<crate::module_runtime::ModulePreloadJobRun>, String> {
        let document_owner = self.current_main_document_task_owner().ok_or_else(|| {
            "main modulepreload fetch started without a current Document owner".to_owned()
        })?;
        let resource_scheduler = self.resource_scheduler();
        let outcome = self
            .document_runtime
            .start_main_document_modulepreload_fetch(document_owner, &resource_scheduler, request)
            .map_err(|error| error.message().to_owned())?;
        Ok(self.consume_main_document_modulepreload_fetch_outcome(outcome))
    }
    pub(crate) fn register_native_modulepreload_link_for_owner(
        &mut self,
        request: NativeModuleSingleFetchRequest,
        link_client: std::sync::Arc<crate::module_runtime::NativeModulepreloadLinkClient>,
    ) -> std::result::Result<Option<crate::module_runtime::ModulePreloadJobRun>, String> {
        let document_owner = self.current_main_document_task_owner().ok_or_else(|| {
            "main modulepreload fetch started without a current Document owner".to_owned()
        })?;
        let resource_scheduler = self.resource_scheduler();
        let outcome = self
            .document_runtime
            .start_main_document_modulepreload_link_fetch(
                document_owner,
                &resource_scheduler,
                request,
                link_client,
            )
            .map_err(|error| error.message().to_owned())?;
        Ok(self.consume_main_document_modulepreload_fetch_outcome(outcome))
    }
    pub(super) fn enqueue_main_modulepreload_link_event(
        &mut self,
        pending_event: crate::document_runtime::PendingNativeModulepreloadLinkEvent,
    ) {
        let Some(event_owner) = pending_event.client().main_document_event_owner() else {
            tracing::debug!(
                element = ?pending_event.client().owner(),
                "discarded main modulepreload terminal without an exact Document owner"
            );
            return;
        };
        debug_assert_eq!(event_owner.element(), pending_event.client().owner());
        if !self
            ._context_host
            .borrow()
            .main_document_task_owner_is_current(event_owner.owner())
        {
            tracing::debug!(
                owner = ?event_owner.owner(),
                element = ?event_owner.element(),
                "discarded modulepreload terminal for a retired main Document"
            );
            return;
        }
        let ready = pending_event.into_ready_event();
        self.document_runtime
            .enqueue_ready_native_modulepreload_link_event(ready);
    }
    pub(crate) fn drain_ready_runtime_owned_module_owner_actions(
        &mut self,
    ) -> Result<NativeModuleOwnerActions> {
        let Some(work) = self.take_ready_runtime_owned_module_script_continuation_work() else {
            return Ok(NativeModuleOwnerActions::empty());
        };
        Ok(Self::runtime_owned_module_owner_actions_from_work(work))
    }
    pub(super) fn runtime_owned_module_owner_actions_from_work(
        work: crate::dynamic_script_owner::DynamicModuleScriptContinuationWork,
    ) -> NativeModuleOwnerActions {
        let mut actions = NativeModuleOwnerActions::empty();
        match work {
            crate::dynamic_script_owner::DynamicModuleScriptContinuationWork::Graph {
                continuation,
            } => actions.push_ready_module_script(*continuation),
            crate::dynamic_script_owner::DynamicModuleScriptContinuationWork::Evaluation {
                evaluation,
            } => actions.push_ready_module_evaluation(*evaluation),
        }
        actions
    }
    pub(super) fn consume_main_document_modulepreload_fetch_outcome(
        &mut self,
        outcome: crate::document_runtime::MainDocumentModulepreloadFetchOutcome,
    ) -> Option<crate::module_runtime::ModulePreloadJobRun> {
        let (job_run, csp_violations, runtime_warning) = outcome.into_parts();
        for violation in csp_violations {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
        }
        if let Some(runtime_warning) = runtime_warning {
            self.record_runtime_warning(format_args!("{runtime_warning}"));
        }
        job_run
    }
    pub(super) fn handle_document_modulator_terminal_fanout_for_owner_with_body<Body>(
        &mut self,
        fanout: ModuleMapTerminalFanout,
        body: &mut Body,
    ) -> Result<(
        NativeModuleOwnerActions,
        FrameDocumentModuleTerminalQueueFollowup,
    )>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let (module_script_results, dynamic_import_fanout) = fanout.into_parts();
        let mut actions = NativeModuleOwnerActions::empty();
        for result in module_script_results {
            actions.merge(self.handle_module_script_graph_advance_for_owner(result));
        }
        let dynamic_import_outcome = self
            .handle_dynamic_module_terminal_fanout_for_owner_with_body(
                dynamic_import_fanout,
                body,
            )?;
        Ok((actions, dynamic_import_outcome.child_followup()))
    }
    #[cfg(test)]
    pub(super) fn handle_dynamic_module_terminal_fanout_for_owner(
        &mut self,
        fanout: NativeDynamicModuleTerminalFanout,
    ) -> Result<NativeDynamicModuleTerminalFanoutOutcome> {
        self.handle_dynamic_module_terminal_fanout_for_owner_with_body(
            fanout,
            &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
        )
    }
    pub(super) fn handle_dynamic_module_terminal_fanout_for_owner_with_body<Body>(
        &mut self,
        fanout: NativeDynamicModuleTerminalFanout,
        body: &mut Body,
    ) -> Result<NativeDynamicModuleTerminalFanoutOutcome>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let (
            dynamic_imports,
            scheduled_dynamic_import_fetches,
            failed_fetches,
            graph_advance_failures,
            restored_after_unexpected_complete,
        ) = fanout.into_parts();
        let mut outcome = NativeDynamicModuleTerminalFanoutOutcome::default();
        let scheduled_fetch_count = scheduled_dynamic_import_fetches.len();
        self.schedule_native_dynamic_module_import_fetches(scheduled_dynamic_import_fetches);
        outcome.record_scheduled_dynamic_import_fetches(scheduled_fetch_count);
        for dynamic_import in dynamic_imports {
            let ready_outcome = self
                .run_completed_native_dynamic_module_import_action_with_body(
                    dynamic_import.job,
                    dynamic_import.graph,
                    body,
                )
                .map_err(|error| anyhow::anyhow!(error))?;
            outcome.record_ready_import_outcome(ready_outcome);
        }
        for failure in failed_fetches {
            let failure_outcome = self
                .run_failed_native_dynamic_module_fetch_action_with_body(failure, body)
                .map_err(|error| anyhow::anyhow!(error))?;
            outcome.record_failed_fetch_rejection_outcome(failure_outcome);
        }
        for (job, error) in graph_advance_failures {
            if job
                .dynamic_import_request()
                .and_then(PendingDynamicModuleImport::child_browsing_context_handle)
                .is_some()
            {
                let followup = self
                    .enqueue_child_dynamic_import_graph_advance_failed_owner_action(job, error)
                    .map_err(|error| anyhow::anyhow!(error))?;
                outcome.record_graph_advance_failure_followup(followup);
            } else {
                let failure_outcome = self
                    .run_main_dynamic_import_owner_action_with_body(
                        FrameDocumentDynamicImportOwnerAction::graph_advance_failed(job, error),
                        body,
                    )
                    .map_err(|error| anyhow::anyhow!(error))?;
                outcome.record_graph_advance_failure_outcome(failure_outcome);
            }
        }
        if restored_after_unexpected_complete {
            self.record_runtime_warning(format_args!(
                "native dynamic import tree completed while its pending tree still had clients"
            ));
            outcome.record_restored_after_unexpected_complete();
        }
        Ok(outcome)
    }
    pub(super) fn handle_module_script_graph_fetch_resume_for_owner(
        &mut self,
        resume: ModuleScriptGraphFetchResume,
    ) -> Result<NativeModuleOwnerActions> {
        match resume {
            ModuleScriptGraphFetchResume::Finished { result } => {
                Ok(self.handle_module_script_graph_advance_for_owner(*result))
            }
            ModuleScriptGraphFetchResume::RestoredMissingGraphContinuation => {
                Ok(NativeModuleOwnerActions::empty())
            }
        }
    }
    pub(super) fn schedule_native_dynamic_module_import_fetches(
        &mut self,
        scheduled: Vec<DynamicModuleScheduledFetch>,
    ) {
        let document_loader = self
            .document_runtime
            .current_document_resource_loader()
            .expect("main dynamic import requires the committed Document resource authority");
        for scheduled_fetch in scheduled {
            let (load_id, request, _) = scheduled_fetch.into_parts();
            let import_owner = self
                .document_runtime
                .with_native_module_owner(|owner| {
                    owner.inflight_native_dynamic_module_import_fetch_owner(load_id)
                })
                .expect("scheduled dynamic-import fetch must retain its resolver owner");
            assert!(
                import_owner.child_handle().is_none(),
                "main dynamic-import scheduler cannot publish a child fetch"
            );
            let target = crate::page_resource_completion::MainDynamicImportGraphFetchTarget::new(
                import_owner,
                load_id,
            );
            let document_url = self.document_runtime.document_url().clone();
            self.resource_scheduler()
                .schedule_main_dynamic_import_graph_fetch(
                    document_loader.clone(),
                    target,
                    request,
                    document_url,
                );
        }
    }
    pub(super) fn complete_shared_module_map_fetch_result(
        &mut self,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<()> {
        let graph_continuation = self
            .document_runtime
            .take_inflight_native_module_script_fetch(load_id)
            .expect("authorized shared module-map terminal must retain its in-flight fetch");
        let fetch_key = graph_continuation
            .request()
            .pending_fetch_key()
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "authorized shared module-map terminal {load_id} has no module map key"
                )
            })?;
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                graph_continuation.request().fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!("native module script fetch completion {load_id} failed: {error}"),
            )),
        };
        match source {
            Ok(fetched_source) => {
                let effective_key = graph_continuation
                    .request()
                    .effective_key_for_fetched_source(&fetched_source)
                    .unwrap_or_else(|| fetch_key.clone());
                let effective_fetch_metadata = graph_continuation
                    .request()
                    .effective_fetch_metadata_for_fetched_source(&fetched_source);
                self.document_runtime
                    .insert_native_module_source_for_request(
                        fetch_key,
                        effective_key,
                        fetched_source.into_source(),
                        effective_fetch_metadata,
                    );
            }
            Err(error) => {
                self.document_runtime
                    .mark_native_module_failed(fetch_key, error);
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn complete_abandoned_module_script_graph_fetch(
        &mut self,
        completion: ModuleGraphFetchCompletion,
    ) -> Result<()> {
        let load_id = completion.load_id;
        if !self
            .document_runtime
            .has_inflight_native_module_script_fetch(load_id)
        {
            let result_summary = match &completion.result {
                Ok(source) => format!("ok({} bytes)", source.len()),
                Err(error) => format!("error({error})"),
            };
            self.record_runtime_warning(format_args!(
                "native module script fetch completion {load_id} arrived without an owner or in-flight module map continuation: requester={:?} ordering={:?} {result_summary}",
                completion.requester,
                completion.ordering
            ));
            return Ok(());
        }
        if let Some(network_result) = completion.network_result.as_deref() {
            self.record_module_graph_subresource_network_result(&completion, network_result);
        }
        self.complete_shared_module_map_fetch_result(load_id, completion.result)
    }
    pub(super) fn dynamic_module_fetch_resume_advance_for_owner_with_body<Body>(
        &mut self,
        continuation: DynamicModuleFetchContinuation,
        body: &mut Body,
    ) -> Result<FrameDocumentModuleTerminalQueueFollowup>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let owner_module_fetch_starts =
            native_dynamic_import_owner_fetch_starts_for_continuation(&continuation);
        let advance = self
            .document_runtime
            .continue_native_dynamic_module_import_fetch(continuation, owner_module_fetch_starts);
        Ok(self
            .handle_dynamic_module_terminal_fanout_for_owner_with_body(
                NativeDynamicModuleTerminalFanout::from_owner_advance(advance),
                body,
            )?
            .child_followup())
    }
    pub(super) fn dynamic_module_fetch_finish_to_owner_actions_with_body<Body>(
        &mut self,
        finish: DynamicModuleFetchFinish,
        body: &mut Body,
    ) -> Result<FrameDocumentModuleTerminalQueueFollowup>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        match finish {
            DynamicModuleFetchFinish::Advanced(continuation) => {
                self.dynamic_module_fetch_resume_advance_for_owner_with_body(continuation, body)
            }
            DynamicModuleFetchFinish::Failed(failure) => {
                let mut fanout = NativeDynamicModuleTerminalFanout::default();
                fanout.push_failed_fetch(failure);
                Ok(self
                    .handle_dynamic_module_terminal_fanout_for_owner_with_body(fanout, body)?
                    .child_followup())
            }
        }
    }
    pub(super) fn dispatch_native_document_modulator_terminal_notification(
        &mut self,
        notification: ModuleMapTerminalNotification,
    ) -> Result<ModuleMapTerminalFanout> {
        let (key, clients, successful) = notification.into_parts();
        let (fetch_clients, _, modulepreload_link_clients) = clients.into_parts();
        debug_assert!(
            modulepreload_link_clients
                .iter()
                .all(|client| client.frame_document_client().is_none())
        );
        let pending_events = self
            .document_runtime
            .accept_native_modulepreload_link_client_terminals(
                &key,
                modulepreload_link_clients,
                successful,
            );
        for pending_event in pending_events {
            self.enqueue_main_modulepreload_link_event(pending_event);
        }
        self.resume_native_document_modulator_fetch_clients(&key, fetch_clients)
    }
    pub(super) fn resume_native_document_modulator_fetch_clients(
        &mut self,
        key: &ModuleMapKey,
        clients: Vec<NativeModuleMapSingleModuleClient>,
    ) -> Result<ModuleMapTerminalFanout> {
        let mut fanout = ModuleMapTerminalFanout::empty();
        if clients.is_empty() {
            return Ok(fanout);
        }
        tracing::debug!(
            url = %key.url(),
            client_count = clients.len(),
            "resuming native module map fetch clients"
        );
        for client in clients {
            tracing::trace!(
                url = %key.url(),
                client_name = client.client_name(),
                import_phase = ?client.import_phase(),
                token = ?client.token(),
                "dispatching native module map single-module client"
            );
            match client {
                NativeModuleMapSingleModuleClient::ModuleScript(client) => {
                    if let Some(result) = self.resume_module_script_fetch_join_waiter(key, client) {
                        fanout.push_module_script_result(result);
                    }
                }
                NativeModuleMapSingleModuleClient::DynamicImport(client) => {
                    self.resume_native_dynamic_module_fetch_waiter(key, client, &mut fanout)?;
                }
            }
        }
        Ok(fanout)
    }
    pub(super) fn resume_native_dynamic_module_fetch_waiter(
        &mut self,
        key: &ModuleMapKey,
        client: NativeDynamicImportSingleModuleClient,
        fanout: &mut ModuleMapTerminalFanout,
    ) -> Result<()> {
        let client_token = client.token();
        let Some(joined) = self
            .document_runtime
            .take_joined_native_dynamic_module_import_fetch(client_token)
        else {
            self.record_runtime_warning(format_args!(
                "native module joined fetch client {:?} had no dynamic import continuation",
                client_token
            ));
            return Ok(());
        };
        debug_assert_eq!(
            joined.client(),
            client_token,
            "dynamic pending tree should return the requested joined client"
        );
        let document_owner = joined.owner();
        if !self.dynamic_module_import_owner_is_current(document_owner) {
            self.record_runtime_warning(format_args!(
                "dropped stale joined dynamic import terminal: client={client_token:?} owner={document_owner:?}"
            ));
            return Ok(());
        }
        match self.finish_native_dynamic_module_joined_fetch(joined, key) {
            DynamicModuleFetchFinish::Advanced(continuation) => {
                self.push_dynamic_module_fetch_resume_advance_into_fanout(continuation, fanout);
            }
            DynamicModuleFetchFinish::Failed(failure) => {
                fanout.push_dynamic_import_fetch_failure(failure);
            }
        }
        Ok(())
    }
    pub(super) fn push_dynamic_module_fetch_resume_advance_into_fanout(
        &mut self,
        continuation: DynamicModuleFetchContinuation,
        fanout: &mut ModuleMapTerminalFanout,
    ) {
        let owner_module_fetch_starts =
            native_dynamic_import_owner_fetch_starts_for_continuation(&continuation);
        let advance = self
            .document_runtime
            .continue_native_dynamic_module_import_fetch(continuation, owner_module_fetch_starts);
        fanout.absorb_dynamic_import_owner_advance(advance);
    }
    pub(super) fn run_failed_native_dynamic_module_fetch_action_with_body<Body>(
        &mut self,
        failure: DynamicModuleFetchFailure,
        body: &mut Body,
    ) -> std::result::Result<FrameDocumentDynamicImportTerminalOutcome, String>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let (request, error) = self
            .document_runtime
            .clear_failed_native_dynamic_module_import_fetch(failure);
        self.run_main_dynamic_import_owner_action_with_body(
            FrameDocumentDynamicImportOwnerAction::fetch_failed(request, error),
            body,
        )
    }
    #[cfg(test)]
    pub(crate) fn record_module_graph_subresource_network_result(
        &mut self,
        completion: &ModuleGraphFetchCompletion,
        network_result: &std::result::Result<crate::types::NavigationResponse, String>,
    ) {
        let document_url = self.document_runtime.document_url().clone();
        self._context_host
            .borrow_mut()
            .record_staged_get_subresource_network_result_with_initiator(
                None,
                document_url,
                completion.request_url.clone(),
                SubresourceResourceType::Script,
                match completion.requester {
                    ModuleGraphFetchRequester::DynamicImport => {
                        SubresourceRequestInitiatorType::Script
                    }
                    ModuleGraphFetchRequester::ParserOwnedModuleScript
                    | ModuleGraphFetchRequester::RuntimeOwnedModuleScript
                    | ModuleGraphFetchRequester::ModulePreload => {
                        SubresourceRequestInitiatorType::Parser
                    }
                },
                network_result,
            );
    }
    pub(crate) fn record_main_modulepreload_network_result(
        &mut self,
        document_url: Url,
        request_url: Url,
        network_result: &std::result::Result<crate::types::NavigationResponse, String>,
    ) {
        self._context_host
            .borrow_mut()
            .record_get_subresource_network_result_with_initiator(
                None,
                document_url,
                request_url.clone(),
                SubresourceResourceType::Script,
                SubresourceRequestInitiatorType::Parser,
                network_result,
            );
        self.record_modulepreload_resource_performance_entry(&request_url, network_result);
    }
    pub(crate) fn record_historical_main_modulepreload_network_result(
        &mut self,
        document_url: Url,
        request_url: Url,
        network_result: &std::result::Result<crate::types::NavigationResponse, String>,
    ) {
        self._context_host
            .borrow_mut()
            .record_historical_get_subresource_network_result_with_initiator(
                None,
                document_url,
                request_url,
                SubresourceResourceType::Script,
                SubresourceRequestInitiatorType::Parser,
                network_result,
            );
    }
    pub(super) fn record_modulepreload_resource_performance_entry(
        &mut self,
        request_url: &Url,
        network_result: &std::result::Result<crate::types::NavigationResponse, String>,
    ) {
        let performance_entry =
            crate::context_bootstrap::ResourcePerformanceEntry::from_network_result(
                request_url.as_str(),
                "link",
                None,
                network_result,
            );
        if let Err(error) = self.with_default_context_scope(|scope, _host_ptr| {
            crate::context_bootstrap::record_resource_performance_entry(scope, performance_entry);
            Ok(())
        }) {
            self.record_runtime_warning(format_args!(
                "failed to record native modulepreload performance entry for `{}`: {}",
                request_url, error
            ));
        }
    }
    pub(super) fn record_css_modulepreload_source_for_owner(
        &mut self,
        key: &ModuleMapKey,
        fetched_source: &ModuleGraphFetchedSource,
    ) {
        if key.kind() != ModuleKind::Css {
            return;
        }
        let Some(source) = fetched_source.source().text_source() else {
            self.record_css_modulepreload_failure_for_owner(key);
            return;
        };
        self._context_host
            .borrow_mut()
            .record_css_module_text_for_url(key.url(), source.to_owned());
    }
    pub(super) fn record_css_modulepreload_failure_for_owner(&mut self, key: &ModuleMapKey) {
        if key.kind() != ModuleKind::Css {
            return;
        }
        self._context_host
            .borrow_mut()
            .record_css_module_failure_for_url(key.url());
    }
    pub(crate) fn module_graph_fetched_source_or_csp_error(
        &mut self,
        load_id: u64,
        fetched_source: ModuleGraphFetchedSource,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> std::result::Result<ModuleGraphFetchedSource, ModuleLoadError> {
        if fetched_source.redirected() {
            let redirect_status =
                crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect;
            let request = module_fetch_csp_request(fetch_metadata);
            if let Some(violation) = self
                .document_runtime
                .script_element_request_csp_report_only_violation_with_redirect_status(
                    fetched_source.final_url(),
                    redirect_status,
                    request,
                )
            {
                self.queue_content_security_policy_violation_event_best_effort(&violation);
            }
            if let Some(violation) = self
                .document_runtime
                .script_element_request_csp_violation_with_redirect_status(
                    fetched_source.final_url(),
                    redirect_status,
                    request,
                )
            {
                self.queue_content_security_policy_violation_event_best_effort(&violation);
                return Err(ModuleLoadError::new(
                    ModuleLoadStage::Fetch,
                    format!(
                        "native module graph fetch completion {load_id} blocked by document Content Security Policy for `{}`",
                        violation.blocked_uri
                    ),
                ));
            }
        }
        Ok(fetched_source)
    }
    pub(crate) fn csp_blocked_module_fetch_error_for_owner(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) -> Option<ModuleLoadError> {
        if !matches!(key.kind(), ModuleKind::JavaScript | ModuleKind::WebAssembly) {
            return None;
        }
        let violation = self
            .document_runtime
            .script_element_request_csp_violation_with_request(
                key.url(),
                module_fetch_csp_request(fetch_metadata),
            )?;
        self.queue_content_security_policy_violation_event_best_effort(&violation);
        Some(ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            format!(
                "Refused to load module `{}` because it violates the document Content Security Policy directive `{}`",
                key.url(),
                violation.effective_directive
            ),
        ))
    }
    pub(crate) fn dispatch_module_fetch_csp_report_only_violation_for_owner(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &crate::module_runtime::ModuleFetchMetadata,
    ) {
        if !matches!(key.kind(), ModuleKind::JavaScript | ModuleKind::WebAssembly) {
            return;
        }
        if let Some(violation) = self
            .document_runtime
            .script_element_request_csp_report_only_violation_with_request(
                key.url(),
                module_fetch_csp_request(fetch_metadata),
            )
        {
            self.queue_content_security_policy_violation_event_best_effort(&violation);
        }
    }
    #[cfg(test)]
    pub(in crate::script_vm) fn warn_on_module_graph_fetch_metadata_mismatch(
        &mut self,
        completion: &ModuleGraphFetchCompletion,
        expected_requester: ModuleGraphFetchRequester,
        expected_ordering: ModuleGraphFetchOrdering,
    ) {
        if completion.requester == expected_requester && completion.ordering == expected_ordering {
            return;
        }
        self.record_runtime_warning(format_args!(
            "module graph fetch completion {} metadata mismatch: got requester={:?} ordering={:?}, expected requester={:?} ordering={:?}",
            completion.load_id,
            completion.requester,
            completion.ordering,
            expected_requester,
            expected_ordering
        ));
    }
    pub(crate) fn note_module_script_evaluation_suspended_for_owner(
        &mut self,
        evaluation: ModuleScriptEvaluationContinuation,
    ) {
        match evaluation.script_continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => {
                self.queue_main_parser_module_evaluation_work(evaluation)
            }
            ModuleScriptCompletionOwner::Runtime => {
                let owner_id = evaluation
                    .script_continuation
                    .dynamic_script_owner_id()
                    .expect("runtime-owned module evaluation should carry dynamic owner id");
                self.note_runtime_owned_module_script_evaluation_suspended(
                    owner_id,
                    Box::new(evaluation),
                );
            }
        }
    }
    pub(super) fn mark_module_evaluation_reaction_fulfilled_for_owner(
        &mut self,
        reaction_id: u64,
    ) -> Option<DocumentModuleReactionUpdate> {
        let parser_update = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .mark_parser_module_evaluation_fulfilled(
                reaction_id,
                parser_module_evaluation_continuation_into_ready_action,
            )
            .map(DocumentModuleReactionUpdate::ParserOwned);
        parser_update.or_else(|| {
            self.mark_runtime_owned_module_script_evaluation_fulfilled(reaction_id)
                .map(DocumentModuleReactionUpdate::RuntimeOwned)
        })
    }
    pub(super) fn mark_module_evaluation_reaction_rejected_for_owner(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_value: Option<ScriptErrorValue>,
    ) -> Option<DocumentModuleReactionUpdate> {
        let parser_update = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .mark_parser_module_evaluation_rejected(
                reaction_id,
                reason.clone(),
                error_value,
                parser_module_evaluation_continuation_into_ready_action,
            )
            .map(DocumentModuleReactionUpdate::ParserOwned);
        parser_update.or_else(|| {
            self.mark_runtime_owned_module_script_evaluation_rejected(
                reaction_id,
                reason,
                error_value,
            )
            .map(DocumentModuleReactionUpdate::RuntimeOwned)
        })
    }
    #[cfg(test)]
    pub(crate) fn has_pending_parser_owned_module_script(&self) -> bool {
        let Some(owner) = self.current_main_document_task_owner() else {
            return false;
        };
        self._context_host
            .borrow()
            .current_main_document_has_parser_deferred_script_load_delay(owner)
            .unwrap_or(false)
    }
    pub(crate) fn release_main_parser_deferred_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        token: crate::frame_owner_model::DocumentLoadDelayTokenId,
    ) -> bool {
        let released = self
            ._context_host
            .borrow_mut()
            .release_main_parser_deferred_script_load_delay(owner, token);
        tracing::debug!(
            ?owner,
            ?token,
            released,
            "settled main parser-deferred lifecycle ownership"
        );
        released
    }
    #[cfg(test)]
    pub(crate) fn has_pending_parser_owned_module_fetch(&self) -> bool {
        self.document_runtime
            .parser_module_scripts()
            .has_pending_fetch(self.document_runtime.parser_module_document_scripts())
            || self
                .document_runtime
                .has_native_module_script_fetch_waiters()
    }
    pub(crate) fn note_module_script_graph_waits_suspended_for_owner(
        &mut self,
        load_ids: Vec<u64>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        continuation: ModuleScriptContinuation,
    ) {
        match continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => self
                .document_runtime
                .parser_module_scripts_mut()
                .insert_parser_pending_waits(load_ids, joined_clients, continuation),
            ModuleScriptCompletionOwner::Runtime => {
                let owner_id = continuation
                    .dynamic_script_owner_id()
                    .expect("runtime-owned module graph fetch should carry dynamic owner id");
                self.note_runtime_owned_module_script_graph_fetch_suspended(
                    owner_id,
                    load_ids,
                    joined_clients,
                    Box::new(continuation),
                );
            }
        }
    }
    pub(crate) fn note_or_restore_module_script_graph_waits_for_owner(
        &mut self,
        load_ids: Vec<u64>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        continuation: ModuleScriptContinuation,
    ) {
        if load_ids.is_empty() && joined_clients.is_empty() {
            self.restore_module_script_graph_pending_continuation_for_owner(continuation);
            return;
        }
        self.note_module_script_graph_waits_suspended_for_owner(
            load_ids,
            joined_clients,
            continuation,
        );
    }
    pub(crate) fn suspend_and_schedule_module_script_graph_fetches_for_owner(
        &mut self,
        continuation: ModuleScriptContinuation,
        job: NativeModuleGraphJob,
        fetches: Vec<ModuleScriptGraphFetchContinuation>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        trace_message: &'static str,
    ) {
        #[derive(Clone, Copy)]
        enum FetchScheduleOwner {
            Parser(
                crate::document_script_scheduler::ParserPendingScriptId<
                    crate::module_script_continuation::MainParserDocumentOwner,
                >,
            ),
            Runtime {
                document_owner: FrameDocumentTaskOwner,
                dynamic_script_owner_id: crate::dynamic_script_owner::DynamicScriptOwnerId,
            },
        }

        let completion_owner = continuation.completion_owner();
        let fetch_schedule_owner = match completion_owner {
            ModuleScriptCompletionOwner::Parser => FetchScheduleOwner::Parser(
                continuation
                    .parser_pending_script_id()
                    .expect("parser module continuation must retain its PendingScript owner"),
            ),
            ModuleScriptCompletionOwner::Runtime => FetchScheduleOwner::Runtime {
                document_owner: self.current_main_document_task_owner().expect(
                    "runtime-created module graph fetch must start with a current main Document owner",
                ),
                dynamic_script_owner_id: continuation.dynamic_script_owner_id().expect(
                    "runtime-created module continuation must retain its dynamic script owner",
                ),
            },
        };
        let mut load_ids = Vec::with_capacity(fetches.len());
        let mut scheduled = Vec::with_capacity(fetches.len());
        for fetch in fetches {
            let request = fetch.request().clone();
            let load_id = self
                .document_runtime
                .suspend_native_module_script_fetch(fetch);
            load_ids.push(load_id);
            scheduled.push((load_id, request));
        }
        let continuation = continuation.with_pending_graph_fetches(job, load_ids.first().copied());
        tracing::debug!(
            url = %continuation.script.url,
            completion_owner = ?continuation.completion_owner(),
            dynamic_script_owner_id = ?continuation.dynamic_script_owner_id(),
            fetch_count = load_ids.len(),
            joined_fetch_count = joined_clients.len(),
            trace_message,
            "module script graph fetch waits installed"
        );
        self.note_or_restore_module_script_graph_waits_for_owner(
            load_ids,
            joined_clients,
            continuation,
        );
        let document_loader = self
            .document_runtime
            .current_document_resource_loader()
            .expect("main module graph requires the committed Document resource authority");
        for (load_id, request) in scheduled {
            match fetch_schedule_owner {
                FetchScheduleOwner::Parser(pending_script_id) => {
                    self.resource_scheduler()
                        .schedule_main_parser_module_graph_fetch(
                            document_loader.clone(),
                            crate::page_resource_completion::MainParserModuleGraphFetchTarget::new(
                                pending_script_id,
                                load_id,
                            ),
                            request,
                            self.document_runtime.document_url().clone(),
                        );
                }
                FetchScheduleOwner::Runtime {
                    document_owner,
                    dynamic_script_owner_id,
                } => {
                    self.resource_scheduler()
                        .schedule_main_runtime_module_graph_fetch(
                            document_loader.clone(),
                            crate::page_resource_completion::MainRuntimeModuleGraphFetchTarget::new(
                                document_owner,
                                dynamic_script_owner_id,
                                load_id,
                            ),
                            request,
                            self.document_runtime.document_url().clone(),
                        );
                }
            }
        }
    }
    pub(crate) fn handle_module_script_graph_advance_for_owner(
        &mut self,
        advance: ModuleScriptContinuationGraphAdvance,
    ) -> NativeModuleOwnerActions {
        match advance {
            ModuleScriptContinuationGraphAdvance::Ready(script_continuation) => {
                if script_continuation.completed_graph.is_some() {
                    tracing::debug!(
                        url = %script_continuation.script.url,
                        completion_owner = ?script_continuation.completion_owner(),
                        active_fetch_load_id = ?script_continuation.active_fetch_load_id(),
                        "module script graph completed after fetch"
                    );
                }
                self.note_module_script_graph_ready_for_owner(*script_continuation);
                NativeModuleOwnerActions::empty()
            }
            ModuleScriptContinuationGraphAdvance::NeedFetches {
                continuation,
                mut job,
                fetches,
            } => {
                let joined_clients = job.take_pending_joined_clients();
                self.suspend_and_schedule_module_script_graph_fetches_for_owner(
                    *continuation,
                    *job,
                    fetches,
                    joined_clients,
                    "module script graph requested parallel fetches",
                );
                NativeModuleOwnerActions::empty()
            }
            ModuleScriptContinuationGraphAdvance::Failed {
                continuation,
                error,
            } => {
                self.clear_pending_module_script_fetches_for_script(
                    continuation.script.node_id,
                    &error,
                );
                self.notify_module_script_graph_failure_for_owner(*continuation, error)
                    .map(|(continuation, error)| {
                        NativeModuleOwnerActions::from_runtime_module_failure(continuation, error)
                    })
                    .unwrap_or_else(NativeModuleOwnerActions::empty)
            }
        }
    }
    pub(super) fn handle_runtime_module_script_graph_advance_for_dynamic_owner(
        &mut self,
        advance: ModuleScriptContinuationGraphAdvance,
    ) -> NativeModuleOwnerActions {
        match advance {
            ModuleScriptContinuationGraphAdvance::Ready(script_continuation) => {
                NativeModuleOwnerActions::from_ready_module_script(*script_continuation)
            }
            ModuleScriptContinuationGraphAdvance::NeedFetches {
                continuation,
                mut job,
                fetches,
            } => {
                let joined_clients = job.take_pending_joined_clients();
                self.suspend_and_schedule_module_script_graph_fetches_for_owner(
                    *continuation,
                    *job,
                    fetches,
                    joined_clients,
                    "runtime module script graph requested parallel fetches",
                );
                NativeModuleOwnerActions::empty()
            }
            ModuleScriptContinuationGraphAdvance::Failed {
                continuation,
                error,
            } => {
                self.clear_runtime_owned_module_script_graph_waits_for_owner(&continuation, &error);
                NativeModuleOwnerActions::from_runtime_module_failure(*continuation, error)
            }
        }
    }
    pub(crate) fn restore_module_script_graph_pending_continuation_for_owner(
        &mut self,
        continuation: ModuleScriptContinuation,
    ) {
        match continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => self
                .document_runtime
                .parser_module_scripts_mut()
                .restore_pending_continuation(continuation),
            ModuleScriptCompletionOwner::Runtime => {
                let owner_id = continuation.dynamic_script_owner_id().expect(
                    "runtime-owned module graph continuation should carry dynamic owner id",
                );
                let restored = self.restore_runtime_owned_module_script_graph_pending_continuation(
                    owner_id,
                    Box::new(continuation),
                );
                debug_assert!(
                    restored,
                    "runtime-owned module graph pending continuation should restore into owner state"
                );
            }
        }
    }
    pub(crate) fn note_module_script_graph_ready_for_owner(
        &mut self,
        continuation: ModuleScriptContinuation,
    ) {
        match continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => {
                self.queue_main_module_script_graph_ready_work(continuation);
            }
            ModuleScriptCompletionOwner::Runtime => {
                let owner_id = continuation
                    .dynamic_script_owner_id()
                    .expect("runtime-owned module continuation should carry dynamic owner id");
                let ready = self
                    .note_runtime_owned_module_script_graph_ready(owner_id, Box::new(continuation));
                debug_assert!(
                    ready,
                    "dynamic owner should accept runtime-owned ready graph continuation"
                );
            }
        }
    }
    pub(crate) fn notify_module_script_graph_failure_for_owner(
        &mut self,
        continuation: ModuleScriptContinuation,
        error: ModuleLoadError,
    ) -> Option<(ModuleScriptContinuation, ModuleLoadError)> {
        match continuation.completion_owner() {
            ModuleScriptCompletionOwner::Parser => {
                self.queue_main_parser_module_graph_failure_work(ParserModuleScriptFailure {
                    continuation,
                    error,
                });
                None
            }
            ModuleScriptCompletionOwner::Runtime => Some((continuation, error)),
        }
    }
    pub(super) fn complete_parser_owned_module_script_graph_fetch_result(
        &mut self,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<Option<ModuleScriptGraphFetchResume>> {
        let Some(active_tree) = self
            .document_runtime
            .parser_module_scripts_mut()
            .take_active_tree_for_fetch_completion(load_id)
        else {
            return Ok(None);
        };
        let Some(graph_continuation) = self
            .document_runtime
            .take_inflight_native_module_script_fetch(load_id)
        else {
            self.record_runtime_warning(format_args!(
                "module script continuation {load_id} had no graph fetch continuation"
            ));
            self.document_runtime
                .parser_module_scripts_mut()
                .restore_active_tree_for_fetch(load_id, active_tree);
            return Ok(Some(
                ModuleScriptGraphFetchResume::RestoredMissingGraphContinuation,
            ));
        };
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                graph_continuation.request().fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!("native module script fetch completion {load_id} failed: {error}"),
            )),
        };
        let result = active_tree.finish_fetch_into_graph(self, graph_continuation, source);
        Ok(Some(ModuleScriptGraphFetchResume::finished(result)))
    }
    pub(crate) fn apply_current_main_parser_module_graph_fetch_completion(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentMainParserModuleGraphFetchCompletion,
    ) -> Result<()> {
        let completion = authorization.into_completion();
        let target = completion.target();
        assert!(
            self.main_parser_module_graph_fetch_target_is_current(target),
            "authorized main parser module terminal must retain its exact Document fetch"
        );
        let result = completion.into_result();
        if self.current_main_parser_module_graph_fetch_target(target.load_id()) != Some(target) {
            return self.complete_shared_module_map_fetch_result(target.load_id(), result);
        }
        let resume = self
            .complete_parser_owned_module_script_graph_fetch_result(target.load_id(), result)?
            .expect("active main parser module terminal must retain its PendingScript fetch");
        let actions = self.handle_module_script_graph_fetch_resume_for_owner(resume)?;
        let (ready_scripts, ready_evaluations, runtime_failures) = actions.into_parts();
        assert!(
            ready_scripts.is_empty() && ready_evaluations.is_empty() && runtime_failures.is_empty(),
            "parser-owned module fetch application must publish follow-up work through the parser owner, not inline runtime actions"
        );
        Ok(())
    }
    pub(super) fn complete_runtime_owned_module_script_graph_fetch_result(
        &mut self,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<Option<ModuleScriptGraphFetchResume>> {
        let Some(script_continuation) =
            self.take_runtime_owned_module_script_graph_pending_fetch(load_id)
        else {
            return Ok(None);
        };
        let Some(graph_continuation) = self
            .document_runtime
            .take_inflight_native_module_script_fetch(load_id)
        else {
            self.record_runtime_warning(format_args!(
                "module script continuation {load_id} had no graph fetch continuation"
            ));
            self.note_module_script_graph_waits_suspended_for_owner(
                vec![load_id],
                Vec::new(),
                script_continuation,
            );
            return Ok(Some(
                ModuleScriptGraphFetchResume::RestoredMissingGraphContinuation,
            ));
        };

        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                graph_continuation.request().fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!("native module script fetch completion {load_id} failed: {error}"),
            )),
        };
        let result =
            script_continuation.finish_fetch_into_resumed_graph(self, graph_continuation, source);
        Ok(Some(ModuleScriptGraphFetchResume::finished(result)))
    }
    pub(crate) fn apply_current_main_runtime_module_graph_fetch_completion(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentMainRuntimeModuleGraphFetchCompletion,
    ) -> Result<NativeModuleOwnerActions> {
        let completion = authorization.into_completion();
        let target = completion.target();
        assert!(
            self.main_runtime_module_graph_fetch_target_is_current(target),
            "authorized main runtime module terminal must retain its exact Document fetch"
        );
        let result = completion.into_result();
        if self.current_main_runtime_module_graph_fetch_target(target.load_id()) != Some(target) {
            self.complete_shared_module_map_fetch_result(target.load_id(), result)?;
            return Ok(NativeModuleOwnerActions::empty());
        }
        let resume = self
            .complete_runtime_owned_module_script_graph_fetch_result(target.load_id(), result)?
            .expect("active main runtime module terminal must retain its dynamic-script fetch");
        self.handle_module_script_graph_fetch_resume_for_owner(resume)
    }
    #[cfg(test)]
    pub(super) fn complete_current_main_dynamic_import_graph_fetch_result(
        &mut self,
        target: crate::page_resource_completion::MainDynamicImportGraphFetchTarget,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<NativeModuleOwnerActions> {
        self.complete_current_main_dynamic_import_graph_fetch_result_with_body(
            target,
            result,
            &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
        )
    }
    pub(super) fn complete_current_main_dynamic_import_graph_fetch_result_with_body<Body>(
        &mut self,
        target: crate::page_resource_completion::MainDynamicImportGraphFetchTarget,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
        body: &mut Body,
    ) -> Result<NativeModuleOwnerActions>
    where
        Body: ScriptVmMainNativeModuleTaskBody,
    {
        let inflight = self
            .document_runtime
            .take_inflight_native_dynamic_module_import_fetch(target.load_id())
            .expect("authorized main dynamic-import terminal must retain its resolver fetch claim");
        assert_eq!(
            inflight.owner(),
            target.import_owner(),
            "dynamic-import resolver claim must match the authorized exact import owner"
        );
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                target.load_id(),
                fetched_source,
                inflight.fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!(
                    "native dynamic import fetch completion {} failed: {error}",
                    target.load_id()
                ),
            )),
        };
        let finish = self.finish_native_dynamic_module_inflight_fetch(inflight, source);
        let mut terminal_followup =
            self.dynamic_module_fetch_finish_to_owner_actions_with_body(finish, body)?;
        let mut owner_actions = NativeModuleOwnerActions::empty();
        while self.has_ready_native_module_owner_actions() {
            let (actions, followup) =
                self.drain_ready_native_module_owner_actions_with_body(body)?;
            owner_actions.merge(actions);
            terminal_followup.merge(followup);
        }
        // `terminal_followup` describes already-published child/module-owner
        // work. The stable sources own that work; this exact Page terminal only
        // returns main runtime actions to its caller.
        let _ = terminal_followup;
        Ok(owner_actions)
    }
    pub(crate) fn resume_parser_owned_module_script_joined_fetch(
        &mut self,
        key: &ModuleMapKey,
        client: NativeModuleScriptSingleModuleClient,
    ) -> Option<ModuleScriptGraphResumeResult> {
        let client_token = client.token();
        let active_tree = self
            .document_runtime
            .parser_module_scripts_mut()
            .take_active_tree_for_joined_client(client_token)?;
        Some(active_tree.finish_joined_fetch_into_graph(
            self,
            chromium_module_key(key),
            client_token,
        ))
    }
    pub(crate) fn resume_runtime_owned_module_script_joined_fetch(
        &mut self,
        key: &ModuleMapKey,
        client: NativeModuleScriptSingleModuleClient,
    ) -> Option<ModuleScriptGraphResumeResult> {
        let client_token = client.token();
        let script_continuation =
            self.take_runtime_owned_module_script_graph_pending_joined_client(client_token)?;
        let result = script_continuation.finish_joined_fetch_into_resumed_graph(
            self,
            chromium_module_key(key),
            client_token,
        );
        Some(result)
    }
    pub(super) fn resume_module_script_fetch_join_waiter(
        &mut self,
        key: &ModuleMapKey,
        client: NativeModuleScriptSingleModuleClient,
    ) -> Option<ModuleScriptGraphResumeResult> {
        let client_token = client.token();
        if let Some(result) = self.resume_parser_owned_module_script_joined_fetch(key, client) {
            return Some(result);
        }
        if let Some(result) = self.resume_runtime_owned_module_script_joined_fetch(key, client) {
            return Some(result);
        }
        self.record_runtime_warning(format_args!(
            "module script joined fetch client {:?} had no graph continuation",
            client_token
        ));
        None
    }
}
