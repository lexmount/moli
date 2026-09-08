use super::*;

impl ScriptVm {
    pub(crate) fn current_main_document_task_owner(&self) -> Option<FrameDocumentTaskOwner> {
        self._context_host
            .borrow()
            .current_main_document_task_owner()
    }
    pub(crate) fn current_main_parser_module_graph_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainParserModuleGraphFetchTarget> {
        let pending_script_id = self
            .document_runtime
            .parser_module_scripts()
            .pending_script_id_for_fetch(load_id)?;
        (self.current_main_document_task_owner() == Some(pending_script_id.owner().task_owner()))
            .then(|| {
                crate::page_resource_completion::MainParserModuleGraphFetchTarget::new(
                    pending_script_id,
                    load_id,
                )
            })
    }
    pub(crate) fn main_parser_module_graph_fetch_target_is_current(
        &self,
        target: crate::page_resource_completion::MainParserModuleGraphFetchTarget,
    ) -> bool {
        self.current_main_document_task_owner() == Some(target.document_owner())
            && (self.current_main_parser_module_graph_fetch_target(target.load_id())
                == Some(target)
                || self
                    .document_runtime
                    .has_inflight_native_module_script_fetch(target.load_id()))
    }
    pub(crate) fn current_main_runtime_module_graph_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainRuntimeModuleGraphFetchTarget> {
        let document_owner = self.current_main_document_task_owner()?;
        let dynamic_script_owner_id = self
            .document_runtime
            .runtime_script_work()
            .dynamic_scripts
            .module_script_owner_id_for_pending_fetch(load_id)?;
        Some(
            crate::page_resource_completion::MainRuntimeModuleGraphFetchTarget::new(
                document_owner,
                dynamic_script_owner_id,
                load_id,
            ),
        )
    }
    pub(crate) fn main_runtime_module_graph_fetch_target_is_current(
        &self,
        target: crate::page_resource_completion::MainRuntimeModuleGraphFetchTarget,
    ) -> bool {
        self.current_main_document_task_owner() == Some(target.document_owner())
            && (self.current_main_runtime_module_graph_fetch_target(target.load_id())
                == Some(target)
                || self
                    .document_runtime
                    .has_inflight_native_module_script_fetch(target.load_id()))
    }
    pub(super) fn main_dynamic_import_graph_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainDynamicImportGraphFetchTarget> {
        let import_owner = self.document_runtime.with_native_module_owner(|owner| {
            owner.inflight_native_dynamic_module_import_fetch_owner(load_id)
        })?;
        import_owner.child_handle().is_none().then(|| {
            crate::page_resource_completion::MainDynamicImportGraphFetchTarget::new(
                import_owner,
                load_id,
            )
        })
    }
    pub(crate) fn current_main_dynamic_import_graph_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainDynamicImportGraphFetchTarget> {
        let target = self.main_dynamic_import_graph_fetch_target(load_id)?;
        self.dynamic_module_import_owner_is_current(target.import_owner())
            .then_some(target)
    }
    /// Retires an old exact dynamic-import fetch after its network terminal is
    /// dequeued for a no-longer-current Document snapshot.
    ///
    /// A replacement PageVm may reuse `load_id`, so both the resolver-owned
    /// import identity and the load id must match before the old wait is
    /// removed. This preserves the pre-migration behavior without allowing a
    /// stale terminal to advance or delete replacement work.
    pub(crate) fn retire_stale_main_dynamic_import_graph_fetch(
        &mut self,
        target: crate::page_resource_completion::MainDynamicImportGraphFetchTarget,
    ) -> bool {
        if self.main_dynamic_import_graph_fetch_target(target.load_id()) != Some(target) {
            return false;
        }
        self.document_runtime
            .take_inflight_native_dynamic_module_import_fetch(target.load_id())
            .is_some()
    }
    pub(crate) fn current_main_modulepreload_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::page_resource_completion::MainModulepreloadFetchTarget> {
        let document_owner = self.current_main_document_task_owner()?;
        let is_inflight = self.document_runtime.with_native_module_owner(|owner| {
            owner.has_inflight_native_modulepreload_fetch_for(load_id)
        });
        is_inflight.then(|| {
            crate::page_resource_completion::MainModulepreloadFetchTarget::new(
                document_owner,
                load_id,
            )
        })
    }
    pub(crate) fn claim_main_parser_deferred_script(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        script: PreparedScript,
        shared_load: Option<crate::planning::SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        blocking_signatures_before: std::collections::HashSet<
            crate::DocumentBlockingStylesheetSignature,
        >,
    ) -> Result<bool> {
        let Some(start) = self.accept_main_parser_deferred_script(
            task_owner,
            script,
            shared_load,
            document_character_set,
            blocking_signatures_before,
        ) else {
            return Ok(false);
        };
        self.start_main_parser_deferred_script(start)?;
        Ok(true)
    }
    pub(in crate::script_vm) fn accept_main_parser_deferred_script(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        script: PreparedScript,
        shared_load: Option<crate::planning::SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        blocking_signatures_before: std::collections::HashSet<
            crate::DocumentBlockingStylesheetSignature,
        >,
    ) -> Option<crate::document_runtime::PendingMainParserDeferredScriptStart> {
        if self.current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.current_main_document_task_owner(),
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "dropping stale main parser-deferred preparation"
            );
            return None;
        }
        let Some(load_delay_token) = self
            ._context_host
            .borrow_mut()
            .acquire_current_main_parser_deferred_script_load_delay(task_owner)
        else {
            tracing::debug!(
                ?task_owner,
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "dropping main parser-deferred preparation without a current lifecycle owner"
            );
            return None;
        };
        let script_node_id = script.node_id;
        let script_url = script.url.clone();
        let start_action = self.document_runtime.accept_main_parser_deferred_script(
            task_owner,
            script,
            shared_load,
            document_character_set,
            blocking_signatures_before,
            load_delay_token,
        );
        let Some(start_action) = start_action else {
            let released = self
                ._context_host
                .borrow_mut()
                .release_main_parser_deferred_script_load_delay(task_owner, load_delay_token);
            debug_assert!(
                released,
                "rejected parser-deferred acceptance must release its lifecycle token"
            );
            return None;
        };
        tracing::debug!(
            ?task_owner,
            ?load_delay_token,
            ?script_node_id,
            script_url = %script_url,
            "accepted main parser-deferred PendingScript with lifecycle ownership"
        );
        Some(start_action)
    }
    pub(in crate::script_vm) fn start_main_parser_deferred_script(
        &mut self,
        start: crate::document_runtime::PendingMainParserDeferredScriptStart,
    ) -> Result<()> {
        let (task_owner, load_delay_token, start_action) = start.into_parts();
        if self.current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.current_main_document_task_owner(),
                "dropping stale accepted main parser-deferred start action"
            );
            return Ok(());
        }
        let document_loader = self.document_runtime.current_document_resource_loader();
        match start_action {
            ParserDeferredScriptStartAction::NoFetch => {}
            ParserDeferredScriptStartAction::ClassicSource(source_load_request) => {
                if let Some(document_loader) = document_loader.as_ref() {
                    let (document_url, request_url) =
                        source_load_request.network_attribution_urls();
                    let network_attribution = crate::page_resource_completion::
                        MainParserDeferredClassicSourceNetworkAttribution::new(
                            document_url,
                            request_url,
                        );
                    let source_load =
                        source_load_request.start(document_loader, document_loader.task_runner());
                    let completion_tx = self._context_host.borrow().resource_completion_sender();
                    let (pending_script_id, source_load) = source_load.into_parts();
                    let completed_source_load = source_load.clone();
                    source_load.register_completion_wake(move || {
                        let outcome = completed_source_load.try_outcome().expect(
                            "script source completion callback requires a terminal outcome",
                        );
                        let _ = completion_tx.send_main_parser_deferred_classic_source_load(
                            ParserDeferredClassicSourceLoadCompletion::new(
                                pending_script_id,
                                outcome,
                            ),
                            network_attribution,
                        );
                    });
                } else {
                    self.complete_main_parser_deferred_classic_source_load(
                        source_load_request.into_failure_completion(
                            "parser-deferred classic script accepted without an installed loader",
                        ),
                    );
                }
            }
            ParserDeferredScriptStartAction::ModuleGraph(start) => {
                let (pending_script_id, script) = start.into_parts();
                let start_result = self
                    .accept_registered_parser_pending_module_script_graph_for_document_owner(
                        pending_script_id,
                        &script,
                        None,
                    );
                let start_error = match start_result {
                    Ok(true) => None,
                    Ok(false) => Some(anyhow::anyhow!(
                        "registered parser module PendingScript {:?} did not start its graph",
                        pending_script_id
                    )),
                    Err(error) => Some(error),
                };
                if let Some(error) = start_error {
                    let canceled_token = self
                        .document_runtime
                        .parser_module_document_scripts_mut()
                        .cancel_parser_deferred_script(pending_script_id);
                    debug_assert_eq!(
                        canceled_token,
                        Some(load_delay_token),
                        "failed graph start must cancel the accepted PendingScript"
                    );
                    let released = self
                        ._context_host
                        .borrow_mut()
                        .release_main_parser_deferred_script_load_delay(
                            task_owner,
                            load_delay_token,
                        );
                    debug_assert!(
                        released,
                        "failed graph start must release its lifecycle token"
                    );
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    pub(crate) fn start_pending_main_parser_deferred_scripts(&mut self) -> Result<()> {
        let mut starts = self
            .document_runtime
            .take_main_parser_deferred_script_starts();
        while let Some(start) = starts.pop_front() {
            self.start_main_parser_deferred_script(start)?;
        }
        Ok(())
    }
    pub(crate) fn inline_module_script_source_for_graph_start(
        &mut self,
        script: &PreparedScript,
        source: &str,
    ) -> ModuleSource {
        let request = self.content_security_policy_script_element_request(script);
        // Module graphs compile their root before the later page-task
        // evaluation boundary. Feed that graph the Trusted Types/CSP-compliant
        // source now. A rejected inline source becomes an inert root so the
        // module owner can still retire its scheduling state without compiling
        // the rejected text.
        let source = self
            .inline_script_element_source_for_execution(script.node_id, source, request)
            .unwrap_or_default();
        self.inline_module_script_source_with_origin(script, source)
    }

    pub(crate) fn inline_module_script_source_with_origin(
        &self,
        script: &PreparedScript,
        source: String,
    ) -> ModuleSource {
        let position = self
            .document_runtime
            .parser_script_start_position(script.node_id);
        ModuleSource::text_with_origin(
            source,
            crate::document_module_graph::ModuleSourceOrigin {
                url: script.url.clone(),
                line_offset: position.map_or(0, |position| {
                    position.line.saturating_sub(1).min(i32::MAX as u64) as u32
                }),
                column_offset: position.map_or(0, |position| {
                    position.column.saturating_sub(1).min(i32::MAX as u64) as u32
                }),
            },
        )
    }
    pub(crate) fn seal_main_parser_deferred_scripts(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
    ) -> Option<crate::page_task_queue::PostParsePageOwnedWork> {
        if self.current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.current_main_document_task_owner(),
                "dropping stale main parser-deferred EOF finalization"
            );
            return None;
        }
        let owner = MainParserDocumentOwner::new(task_owner);
        let initial_count = match self
            .document_runtime
            .parser_module_document_scripts_mut()
            .seal_parser_deferred_scripts(owner)
        {
            Ok(initial_count) => initial_count,
            Err(missing) => {
                self.record_runtime_warning(format_args!(
                    "dropping parser-deferred script queue because module PendingScript is missing for parser position {} node {:?}",
                    missing.parser_position(),
                    missing.script_node_id()
                ));
                return None;
            }
        };
        self.arm_main_parser_deferred_scripts(owner, initial_count)
    }
    pub(crate) fn complete_main_parser_deferred_classic_source_load(
        &mut self,
        completion: MainParserDeferredClassicSourceLoadCompletion,
    ) {
        let pending_script_id = completion.pending_script_id();
        let result = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .complete_parser_deferred_classic_source_load(completion);
        match result {
            ParserDeferredClassicSourceLoadApplyResult::Applied => {}
            ParserDeferredClassicSourceLoadApplyResult::MissingDocument => {
                tracing::debug!(
                    owner = ?pending_script_id.owner(),
                    parser_position = pending_script_id.parser_position(),
                    script_node_id = ?pending_script_id.script_node_id(),
                    "dropping parser-deferred classic source terminal for retired document"
                );
            }
            ParserDeferredClassicSourceLoadApplyResult::MissingPendingScript => {
                tracing::warn!(
                    owner = ?pending_script_id.owner(),
                    parser_position = pending_script_id.parser_position(),
                    script_node_id = ?pending_script_id.script_node_id(),
                    "dropping parser-deferred classic source terminal without PendingScript"
                );
            }
        }
    }
    pub(super) fn arm_main_parser_deferred_scripts(
        &mut self,
        owner: MainParserDocumentOwner,
        initial_count: usize,
    ) -> Option<crate::page_task_queue::PostParsePageOwnedWork> {
        if initial_count == 0 {
            self.document_runtime
                .disarm_main_parser_deferred_scripts(owner.task_owner());
            return None;
        }
        self.document_runtime
            .arm_main_parser_deferred_scripts(owner.task_owner());
        Some(
            crate::page_task_queue::PostParsePageOwnedWork::main_parser_deferred_scripts(
                owner.task_owner(),
                initial_count,
            ),
        )
    }
    pub(super) fn queue_main_module_script_graph_ready_work(
        &mut self,
        continuation: ModuleScriptContinuation,
    ) -> bool {
        assert_eq!(
            continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "runtime-owned ready graph continuations should be owned by DynamicScriptOwner"
        );
        let work = continuation.into_main_document_graph_ready_work();
        self.document_runtime
            .parser_module_document_scripts_mut()
            .notify_module_script_graph_ready_work(work)
    }
    pub(super) fn queue_main_parser_module_graph_failure_work(
        &mut self,
        failure: ParserModuleScriptFailure,
    ) -> bool {
        assert_eq!(
            failure.continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "runtime-owned graph failures should be owned by DynamicScriptOwner"
        );
        self.document_runtime
            .parser_module_document_scripts_mut()
            .notify_module_script_graph_failed_action(DocumentOwnedScriptReadyAction::new(
                failure
                    .continuation
                    .parser_document_owner()
                    .expect("parser graph failure requires its original document owner"),
                failure,
            ))
    }
    pub(super) fn queue_main_parser_module_evaluation_work(
        &mut self,
        evaluation: ModuleScriptEvaluationContinuation,
    ) {
        assert_eq!(
            evaluation.script_continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "runtime-owned module evaluation continuations should be owned by DynamicScriptOwner"
        );
        let owner = evaluation
            .script_continuation
            .parser_document_owner()
            .expect("parser module evaluation requires its original document owner");
        if evaluation.reaction_state.is_pending() {
            self.document_runtime
                .parser_module_document_scripts_mut()
                .push_pending_parser_module_evaluation_with_reaction_id(
                    DocumentOwnedScriptReadyAction::new(owner, evaluation.script_continuation),
                    evaluation.root_entry,
                    evaluation.reaction_id,
                );
        } else {
            self.document_runtime
                .parser_module_document_scripts_mut()
                .notify_module_script_evaluation_completed(DocumentOwnedScriptReadyAction::new(
                    owner, evaluation,
                ));
        }
    }
    pub(crate) fn clear_pending_module_script_fetches_for_script(
        &mut self,
        node_id: NodeId,
        owner_error: &ModuleLoadError,
    ) {
        let (stale_load_ids, stale_joined_clients) = self
            .document_runtime
            .parser_module_scripts_mut()
            .clear_pending_fetches_for_script(node_id);
        for load_id in stale_load_ids {
            tracing::debug!(
                load_id,
                owner_error = owner_error.message(),
                "detached pending module script fetch from failed owner; network completion will settle the module map entry"
            );
        }
        for client in stale_joined_clients {
            let detached = self
                .document_runtime
                .detach_native_module_fetch_waiter(client);
            tracing::debug!(
                ?client,
                detached,
                owner_error = owner_error.message(),
                "detached joined module script fetch client from failed owner"
            );
        }
    }
    pub(super) fn clear_runtime_owned_module_script_graph_waits_for_owner(
        &mut self,
        continuation: &ModuleScriptContinuation,
        owner_error: &ModuleLoadError,
    ) {
        let Some(owner_id) = continuation.dynamic_script_owner_id() else {
            return;
        };
        let (stale_load_ids, stale_joined_clients) = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .clear_module_script_graph_pending_waits(owner_id);
        for load_id in stale_load_ids {
            tracing::debug!(
                load_id,
                dynamic_script_owner_id = ?owner_id,
                owner_error = owner_error.message(),
                "detached runtime-owned pending module script fetch from failed owner; network completion will settle the module map entry"
            );
        }
        for client in stale_joined_clients {
            let detached = self
                .document_runtime
                .detach_native_module_fetch_waiter(client);
            tracing::debug!(
                ?client,
                detached,
                dynamic_script_owner_id = ?owner_id,
                owner_error = owner_error.message(),
                "detached runtime-owned joined module script fetch client from failed owner"
            );
        }
    }
    #[cfg(test)]
    pub(crate) fn complete_native_module_graph_fetch(
        &mut self,
        completion: ModuleGraphFetchCompletion,
    ) -> Result<()> {
        let load_id = completion.load_id;
        let has_modulepreload = self.document_runtime.with_native_module_owner(|owner| {
            owner.has_inflight_native_modulepreload_fetch_for(completion.load_id)
        });
        if has_modulepreload {
            if let Some(network_result) = completion.network_result.as_deref() {
                self.record_module_graph_subresource_network_result(&completion, network_result);
                self.record_modulepreload_resource_performance_entry(
                    &completion.request_url,
                    network_result,
                );
            }
            self.warn_on_module_graph_fetch_metadata_mismatch(
                &completion,
                ModuleGraphFetchRequester::ModulePreload,
                ModuleGraphFetchOrdering::BackgroundPreload,
            );
            if self
                .apply_main_modulepreload_fetch_result(load_id, completion.result)?
                .is_none()
            {
                return Err(anyhow::anyhow!(
                    "known legacy modulepreload fetch lost its in-flight request before application"
                ));
            }
            return Ok(());
        }
        if matches!(
            completion.requester,
            ModuleGraphFetchRequester::ParserOwnedModuleScript
                | ModuleGraphFetchRequester::RuntimeOwnedModuleScript
        ) {
            self.complete_abandoned_module_script_graph_fetch(completion)?;
            return Ok(());
        }
        let result_summary = match &completion.result {
            Ok(source) => format!("ok({} bytes)", source.len()),
            Err(error) => format!("error({error})"),
        };
        self.record_runtime_warning(format_args!(
            "native module graph fetch completion {load_id} arrived without an in-flight module graph job: requester={:?} ordering={:?} {result_summary}",
            completion.requester,
            completion.ordering
        ));
        Ok(())
    }
    pub(super) fn apply_main_modulepreload_fetch_result(
        &mut self,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<Option<ModuleEntryId>> {
        let Some(preload) = self
            .document_runtime
            .take_inflight_native_modulepreload_fetch(load_id)
        else {
            return Ok(None);
        };
        let fetch_key = preload.module_key().clone();
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                preload.fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(ModuleLoadStage::Fetch, error)),
        };
        let application = match source {
            Ok(fetched_source) => {
                let effective_key = preload.effective_key_for_fetched_source(&fetched_source);
                let effective_fetch_metadata =
                    preload.effective_fetch_metadata_for_fetched_source(&fetched_source);
                self.record_css_modulepreload_source_for_owner(&effective_key, &fetched_source);
                self.document_runtime
                    .insert_native_module_source_for_request(
                        fetch_key,
                        effective_key,
                        fetched_source.into_source(),
                        effective_fetch_metadata,
                    )
            }
            Err(error) => {
                self.record_css_modulepreload_failure_for_owner(&fetch_key);
                self.document_runtime
                    .mark_native_module_failed(fetch_key, error)
            }
        };
        Ok(Some(application))
    }
    pub(crate) fn apply_live_main_modulepreload_fetch_completion(
        &mut self,
        authorization: crate::runtime::AuthorizedLiveMainModulepreloadFetchCompletion,
    ) -> Result<()> {
        let completion = authorization.into_completion();
        let target = completion.target();
        self.apply_main_modulepreload_fetch_result(target.load_id(), completion.into_result())?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "authorized main modulepreload terminal lost its exact in-flight request"
                )
            })?;
        Ok(())
    }
    pub(crate) fn accept_main_parser_async_module_script(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        script: &PreparedScript,
    ) -> Result<bool> {
        if script.kind != crate::types::ScriptKind::Module
            || script.mode != crate::types::ScriptMode::Async
        {
            return Ok(false);
        }
        let Some(binding) = self.accept_main_document_script_load_delay_binding(
            task_owner,
            MainDocumentScriptLoadDelayKind::Module,
        ) else {
            return Ok(false);
        };
        self.accept_main_parser_async_module_script_with_binding(task_owner, script, binding)
    }
    pub(crate) fn accept_main_parser_async_module_admission(
        &mut self,
        admission: crate::document_script_scheduler::MainParserAsyncModuleAdmission,
    ) -> Result<bool> {
        let (script, binding) = admission.into_parts();
        let task_owner = binding.owner();
        self.accept_main_parser_async_module_script_with_binding(task_owner, &script, binding)
    }
    pub(super) fn accept_main_parser_async_module_script_with_binding(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
        script: &PreparedScript,
        binding: MainDocumentScriptLoadDelayLease,
    ) -> Result<bool> {
        assert_eq!(
            binding.owner(),
            task_owner,
            "parser async-module admission lease must target its exact PendingScript owner"
        );
        assert_eq!(
            binding.kind(),
            MainDocumentScriptLoadDelayKind::Module,
            "parser async-module admission requires a module load-delay lease"
        );
        assert_eq!(
            (script.kind, script.mode),
            (
                crate::types::ScriptKind::Module,
                crate::types::ScriptMode::Async
            ),
            "parser async-module admission requires an async module"
        );
        if self.current_main_document_task_owner() != Some(task_owner) {
            tracing::debug!(
                ?task_owner,
                current_owner = ?self.current_main_document_task_owner(),
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "dropping stale main parser async module acceptance"
            );
            return Ok(false);
        }
        let owner = MainParserDocumentOwner::new(task_owner);
        let pending_script_id =
            ParserPendingScriptId::from_key(owner, ParserPendingScriptKey::from_script(script));
        if self
            .document_runtime
            .parser_module_document_scripts()
            .has_module_script(pending_script_id)
        {
            self.record_runtime_warning(format_args!(
                "rejecting duplicate main parser async module PendingScript {:?}",
                pending_script_id
            ));
            let _ = self
                ._context_host
                .borrow_mut()
                .release_main_document_script_load_delay(binding);
            return Ok(false);
        }
        let load_delay_token = binding.load_delay_token();
        let watch = self
            .document_runtime
            .parser_module_document_scripts_mut()
            .register_and_watch_module_script(owner, script);
        debug_assert_eq!(watch.pending_script_id(), pending_script_id);
        if !watch.watched() {
            let _ = self
                .document_runtime
                .parser_module_document_scripts_mut()
                .discard_module_script(pending_script_id);
            let settled = self
                ._context_host
                .borrow_mut()
                .release_main_document_script_load_delay(binding);
            tracing::debug!(
                ?task_owner,
                ?pending_script_id,
                ?load_delay_token,
                settled = ?settled,
                "cancelled main parser async module lifecycle binding after watch rejection"
            );
            return Ok(false);
        }
        let started = match self
            .accept_registered_parser_pending_module_script_graph_for_document_owner(
                pending_script_id,
                script,
                Some(binding),
            ) {
            Ok(started) => started,
            Err(error) => {
                let _ = self
                    .document_runtime
                    .parser_module_document_scripts_mut()
                    .discard_module_script(pending_script_id);
                return Err(error);
            }
        };
        if !started {
            let _ = self
                .document_runtime
                .parser_module_document_scripts_mut()
                .discard_module_script(pending_script_id);
            tracing::debug!(
                ?task_owner,
                ?pending_script_id,
                ?load_delay_token,
                "cancelled main parser async module after graph start rejected and released its lease"
            );
            return Ok(false);
        }
        tracing::debug!(
            ?task_owner,
            ?pending_script_id,
            ?load_delay_token,
            ready_before_graph_start = watch.queued_ready_work(),
            script_url = %script.url,
            "accepted and watched main parser async module before graph work"
        );
        if self
            .document_runtime
            .parser_module_document_scripts()
            .has_ready_work()
        {
            let _ = self.enqueue_parser_owned_module_continuation();
        }
        Ok(true)
    }
    pub(super) fn accept_registered_parser_pending_module_script_graph_for_document_owner(
        &mut self,
        pending_script_id: ParserPendingScriptId<MainParserDocumentOwner>,
        script: &PreparedScript,
        mut load_delay_binding: Option<crate::frame_owner_model::MainDocumentScriptLoadDelayLease>,
    ) -> Result<bool> {
        let owner = pending_script_id.owner();
        if script.kind != crate::types::ScriptKind::Module
            || ParserPendingScriptId::from_key(owner, ParserPendingScriptKey::from_script(script))
                != pending_script_id
        {
            self.record_runtime_warning(format_args!(
                "dropping parser module graph start whose script does not match PendingScript {:?}",
                pending_script_id
            ));
            if let Some(binding) = load_delay_binding.take() {
                let _ = self
                    ._context_host
                    .borrow_mut()
                    .release_main_document_script_load_delay(binding);
            }
            return Ok(false);
        }
        if self.current_main_document_task_owner() != Some(owner.task_owner()) {
            tracing::debug!(
                ?owner,
                current_owner = ?self.current_main_document_task_owner(),
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "dropping stale registered parser module graph start"
            );
            if let Some(binding) = load_delay_binding.take() {
                let _ = self
                    ._context_host
                    .borrow_mut()
                    .release_main_document_script_load_delay(binding);
            }
            return Ok(false);
        }
        if !self
            .document_runtime
            .parser_module_document_scripts()
            .has_module_script(pending_script_id)
        {
            self.record_runtime_warning(format_args!(
                "dropping parser module graph start without registered PendingScript {:?}",
                pending_script_id
            ));
            if let Some(binding) = load_delay_binding.take() {
                let _ = self
                    ._context_host
                    .borrow_mut()
                    .release_main_document_script_load_delay(binding);
            }
            return Ok(false);
        }
        let mut continuation =
            ModuleScriptContinuation::new_parser(script.clone(), pending_script_id);
        if let Some(binding) = load_delay_binding.take() {
            continuation = continuation.with_main_document_load_delay_binding(binding);
        }
        if self
            .document_runtime
            .current_document_resource_loader()
            .is_none()
        {
            let error = ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "parser module graph accepted without a Document resource authority",
            );
            let pending_failure =
                self.notify_module_script_graph_failure_for_owner(continuation, error);
            debug_assert!(
                pending_failure.is_none(),
                "parser module failure should stay on its watched PendingScript owner queue"
            );
            tracing::debug!(
                url = %script.url,
                position = script.position,
                node_id = ?script.node_id,
                "recorded missing-authority parser module graph failure on preparation-time PendingScript"
            );
            return Ok(true);
        }
        let job =
            match crate::module_runtime::parser_owned_module_script_graph_job_for_prepared_script(
                self, script,
            ) {
                Ok(Some(job)) => job,
                Ok(None) => {
                    let error = ModuleLoadError::new(
                        ModuleLoadStage::Fetch,
                        "registered parser module PendingScript produced no module graph job",
                    );
                    let pending_failure =
                        self.notify_module_script_graph_failure_for_owner(continuation, error);
                    debug_assert!(
                        pending_failure.is_none(),
                        "parser module no-job failure should stay on its watched PendingScript owner queue"
                    );
                    return Ok(true);
                }
                Err(error) => {
                    let pending_failure =
                        self.notify_module_script_graph_failure_for_owner(continuation, error);
                    debug_assert!(
                        pending_failure.is_none(),
                        "parser module graph failure should stay on its watched PendingScript owner queue"
                    );
                    return Ok(true);
                }
            };
        let continuation = continuation.with_resumed_graph_job(job);
        let advance = continuation.advance_graph(self);
        let actions = self.handle_module_script_graph_advance_for_owner(advance);
        let (ready_scripts, ready_evaluations, runtime_failures) = actions.into_parts();
        debug_assert!(
            ready_scripts.is_empty() && ready_evaluations.is_empty() && runtime_failures.is_empty(),
            "parser-owned graph start should not produce immediate ready owner actions"
        );
        Ok(true)
    }
    pub(crate) fn start_runtime_module_script_graph_for_owner(
        &mut self,
        script: &PreparedScript,
        dynamic_script_owner_id: crate::dynamic_script_owner::DynamicScriptOwnerId,
    ) -> RuntimeModuleScriptGraphStart {
        if script.kind != crate::types::ScriptKind::Module || script.url.scheme() == "data" {
            return RuntimeModuleScriptGraphStart::NotModuleScript;
        }
        let document_owner = self
            .current_main_document_task_owner()
            .expect("runtime module graph start requires a current main Document owner");
        let continuation = ModuleScriptContinuation::new_runtime(
            script.clone(),
            dynamic_script_owner_id,
            document_owner,
        );
        let job = match runtime_owned_module_script_graph_job_for_prepared_script(self, script) {
            Ok(job) => job,
            Err(error) => {
                let actions = self
                    .notify_module_script_graph_failure_for_owner(continuation, error)
                    .map(|(continuation, error)| {
                        NativeModuleOwnerActions::from_runtime_module_failure(continuation, error)
                    })
                    .unwrap_or_else(NativeModuleOwnerActions::empty);
                return RuntimeModuleScriptGraphStart::Started(actions);
            }
        };
        let continuation = continuation.with_resumed_graph_job(job);
        let advance = continuation.advance_graph(self);
        let actions = self.handle_runtime_module_script_graph_advance_for_dynamic_owner(advance);
        RuntimeModuleScriptGraphStart::Started(actions)
    }
}
