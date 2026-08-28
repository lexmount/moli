use crate::{
    document_runtime::DomHandle,
    frame_owner_model::{
        ChildDynamicModuleInflightFetch, FrameDocumentDynamicImportTerminalClientFinishResult,
        FrameDocumentModuleClientReservation, FrameDocumentModuleDependencyFetchTask,
        FrameDocumentModuleFetchTerminalResult, FrameDocumentModuleTerminalBatch,
        FrameDocumentModuleTerminalQueueFollowup, FrameDocumentModuleTerminalWarning,
        FrameDocumentModuleTerminalWarningRecord, FrameDocumentOwner,
        FrameDocumentParserRootModuleClient, FrameDocumentTaskOwner,
        FrameLocalWindowOwnerTransition, FrameRealmId,
    },
    module_runtime::{
        DynamicModuleFetchFinish, DynamicModuleInflightFetch, DynamicModuleJoinedFetch,
        ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource, ModuleLoadError,
        ModuleLoadStage, ModuleMapKey, ModuleRequestRecord, NativeDocumentModulator,
        NativeDynamicImportSingleModuleClient, NativeModuleGraphFetchRequest,
        NativeModuleTreeFrameDocumentOwner, NativeParserModuleTreeJobResume,
    },
    planning::PreparedScript,
};

use super::ScriptVm;
use moli_module_script_tree::ModuleTreeId;
use url::Url;

impl ScriptVm {
    pub(crate) fn resolve_child_frame_module_specifier(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        specifier: &str,
        base_url: &Url,
    ) -> Result<Url, String> {
        self._context_host
            .borrow_mut()
            .resolve_frame_document_module_specifier(document_owner, realm_id, specifier, base_url)
    }

    pub(crate) fn resolve_child_frame_module_integrity(
        &self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        url: &Url,
    ) -> Option<String> {
        self._context_host
            .borrow()
            .resolve_frame_document_module_integrity(document_owner, realm_id, url)
    }

    pub(super) fn apply_pending_child_document_owner_retirements(&mut self) {
        let transitions = self
            ._context_host
            .borrow_mut()
            .take_pending_child_document_owner_retirements();
        for transition in transitions {
            self.apply_child_document_owner_transition(transition);
        }
    }

    pub(super) fn apply_child_document_owner_transition(
        &mut self,
        transition: crate::frame_owner_model::FrameDocumentOwnerTransition,
    ) {
        let retired_local_window_id = match transition.local_window_owner_transition() {
            FrameLocalWindowOwnerTransition::Replaced { retired, .. }
            | FrameLocalWindowOwnerTransition::Retired { retired } => Some(retired),
            FrameLocalWindowOwnerTransition::Installed { .. }
            | FrameLocalWindowOwnerTransition::Preserved { .. } => None,
        };
        if let Some(retired_owner) = transition.retired_owner() {
            let execution_context_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(
                retired_owner.local_window_id,
            );
            let execution_context_retired = retired_local_window_id.is_some();
            debug_assert!(
                retired_local_window_id
                    .is_none_or(|local_window_id| local_window_id == retired_owner.local_window_id),
                "committed LocalWindow retirement must identify the retired document owner"
            );
            if execution_context_retired
                && let Some(execution_context_id) = self
                    .child_frame_realm_store
                    .context_id_for_local_window_id(retired_owner.local_window_id)
            {
                self.destroy_child_default_context(execution_context_id);
            }
            let retired_timer_count = if execution_context_retired {
                self.document_runtime
                    .cancel_window_execution_context_timers(execution_context_owner)
            } else {
                0
            };
            let retired_webcrypto_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_webcrypto_execution_context_owner(execution_context_owner)
            } else {
                0
            };
            if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_opfs_execution_context_owner(execution_context_owner);
            }
            let retired_worker_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_workers_for_execution_context_owner(execution_context_owner)
            } else {
                0
            };
            let retired_shared_worker_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .disconnect_shared_worker_clients_for_execution_context_owner(
                        execution_context_owner,
                    )
            } else {
                0
            };
            let retired_xhr_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_window_xhrs_for_execution_context_owner(execution_context_owner)
            } else {
                0
            };
            let retired_fetch_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_window_fetches_for_execution_context_owner(execution_context_owner)
            } else {
                (0, 0)
            };
            if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_window_event_sources_for_execution_context_owner(
                        execution_context_owner,
                    );
            }
            let retired_window_message_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_window_messages_for_execution_context_owner(execution_context_owner)
            } else {
                0
            };
            let retired_window_execution_context = execution_context_retired
                && self
                    ._context_host
                    .borrow_mut()
                    .retire_window_execution_context(execution_context_owner);
            let retired_image_decode_relevant_context_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_image_decode_requests_for_execution_context_owner(
                        execution_context_owner,
                    )
            } else {
                0
            };
            let retired_message_port_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_message_ports_for_execution_context_owner(execution_context_owner)
            } else {
                0
            };
            let context_host = self._context_host.clone();
            let image_decode_retirement = self
                .with_default_context_scope(move |scope, _host_ptr| {
                    Ok(context_host
                        .borrow_mut()
                        .retire_image_decode_requests_for_document_owner(scope, retired_owner))
                })
                .unwrap_or_default();
            let dynamic_import_retirement = if execution_context_retired {
                self.document_runtime
                    .retire_native_dynamic_module_import_execution_context(execution_context_owner)
            } else {
                crate::module_runtime::DynamicModuleExecutionContextRetirement::default()
            };
            let runtime_binding_retirement = if let Some(current_owner) = transition
                .current_owner()
                .filter(|_| !execution_context_retired)
            {
                self._context_host
                    .borrow_mut()
                    .rebind_runtime_binding_document_owner(retired_owner, current_owner)
            } else {
                self._context_host
                    .borrow_mut()
                    .retire_runtime_bindings_for_document_owner(retired_owner)
            };
            let retired_broadcast_channel_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .close_broadcast_channels_for_execution_context_owner(execution_context_owner)
            } else {
                0
            };
            let retired_websocket_count = if execution_context_retired {
                self._context_host
                    .borrow_mut()
                    .retire_websockets_for_execution_context_owner(execution_context_owner)
            } else {
                0
            };
            let (retired_isolated_world_count, rebound_isolated_world_count) =
                if execution_context_retired {
                    (
                        self.retire_isolated_worlds_for_document_owner(retired_owner),
                        0,
                    )
                } else if let Some(current_owner) = transition.current_owner() {
                    (
                        0,
                        self.rebind_isolated_worlds_for_document_owner_transition(
                            transition.browsing_context_id(),
                            retired_owner,
                            current_owner,
                        ),
                    )
                } else {
                    (0, 0)
                };
            let retired_modulator_count = if execution_context_retired {
                self.child_document_modulator_store
                    .remove_execution_context(retired_owner.local_window_id)
            } else {
                0
            };
            self._context_host
                .borrow_mut()
                .retire_service_worker_document_owner(retired_owner);
            tracing::debug!(
                ?retired_owner,
                pending_import_count = dynamic_import_retirement.pending_import_count(),
                pending_tree_count = dynamic_import_retirement.pending_tree_count(),
                inflight_fetch_count = dynamic_import_retirement.inflight_fetch_count(),
                joined_fetch_count = dynamic_import_retirement.joined_fetch_count(),
                pending_reaction_count = dynamic_import_retirement.pending_reaction_count(),
                retired_timer_count,
                retired_webcrypto_count,
                retired_worker_count,
                retired_shared_worker_count,
                retired_xhr_count,
                aborted_fetch_count = retired_fetch_count.0,
                detached_keepalive_fetch_count = retired_fetch_count.1,
                retired_window_message_count,
                retired_window_execution_context,
                retired_runtime_binding_context_count =
                    runtime_binding_retirement.retired_execution_context_count(),
                retired_broadcast_channel_count,
                retired_websocket_count,
                retired_isolated_world_count,
                rebound_isolated_world_count,
                rejected_image_decode_count = image_decode_retirement.rejected_count(),
                dropped_image_decode_context_count =
                    image_decode_retirement.dropped_context_count(),
                retired_image_decode_relevant_context_count,
                retired_message_port_count,
                retired_dynamic_import_state = dynamic_import_retirement.retired_anything(),
                retired_modulator_count,
                "retired child runtime state with document owner transaction"
            );
        }
        if let Some(current_owner) = transition.current_owner() {
            let child_handle = transition.child_handle();
            let browsing_context_id = transition.browsing_context_id();
            if !self
                ._context_host
                .borrow()
                .frame_document_projection_is_current(
                    child_handle,
                    browsing_context_id,
                    current_owner,
                )
            {
                tracing::debug!(
                    ?child_handle,
                    ?browsing_context_id,
                    ?current_owner,
                    "skipping realm projection for a superseded child owner transition"
                );
                return;
            }
            let should_materialize_default_realm = {
                let host = self._context_host.borrow();
                host.child_window_proxy_shell_is_exposed(child_handle)
                    || !host.child_current_document_is_initial_empty(child_handle)
            };
            if should_materialize_default_realm {
                self._context_host
                    .borrow_mut()
                    .request_child_frame_realm_materialization(child_handle);
                tracing::debug!(
                    child_handle = ?child_handle,
                    ?current_owner,
                    "queued child Document for its Page-owned realm turn"
                );
            }
        }
    }

    pub(super) fn push_child_module_terminal_batch_to_frame_lane(
        &mut self,
        batch: FrameDocumentModuleTerminalBatch,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let (tasks, modulepreload_terminal_works, dynamic_import_owner_actions, warnings) =
            batch.into_parts();
        let warning_recorded = !warnings.is_empty();
        for warning in warnings {
            self.record_child_module_terminal_warning(warning);
        }
        let mut followup = FrameDocumentModuleTerminalQueueFollowup::terminal_warning_from_recorded(
            warning_recorded,
        );
        // Before module-script terminals became a stable typed source, the
        // already-typed client actions were visible to Page arbitration before
        // the terminal's legacy-pump follow-up ticket. Preserve that observable
        // ordering at the producer boundary: modulepreload load/error and
        // dynamic-import settlement run before a same-batch terminal can make
        // DocumentScriptReady work executable.
        followup.merge(
            self.route_child_modulepreload_terminals_to_page_source(modulepreload_terminal_works),
        );
        followup.merge(
            self.route_child_dynamic_import_owner_actions_to_page_source(
                dynamic_import_owner_actions,
            ),
        );
        followup.merge(self.route_child_module_terminal_tasks_to_page_source(tasks));
        followup
    }

    pub(super) fn route_child_module_terminal_tasks_to_page_source(
        &mut self,
        tasks: Vec<crate::frame_owner_model::FrameDocumentModuleScriptTerminalBatchTask>,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let mut promoted = false;
        for task in tasks {
            promoted |= self
                ._context_host
                .borrow()
                .route_child_module_script_terminal(task);
        }
        FrameDocumentModuleTerminalQueueFollowup::module_script_terminal_from_queued(promoted)
    }

    pub(super) fn route_child_modulepreload_terminals_to_page_source(
        &mut self,
        works: Vec<crate::frame_owner_model::FrameDocumentModulepreloadTerminalWork>,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let mut promoted = false;
        for work in works {
            let host = self._context_host.borrow();
            let Some(action) = host.accept_child_modulepreload_terminal_event(work) else {
                continue;
            };
            promoted |= host.route_child_modulepreload_event_action(action);
        }
        FrameDocumentModuleTerminalQueueFollowup::modulepreload_event_action_from_queued(promoted)
    }

    pub(super) fn route_child_dynamic_import_owner_actions_to_page_source(
        &mut self,
        actions: Vec<crate::frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction>,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let promoted = self
            ._context_host
            .borrow_mut()
            .route_child_dynamic_import_owner_actions(actions);
        FrameDocumentModuleTerminalQueueFollowup::dynamic_import_owner_action_from_queued(promoted)
    }

    pub(super) fn post_current_child_document_modulator_terminals_to_frame_lane(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let Some(task_owner) = self
            ._context_host
            .borrow()
            .current_child_module_route_task_owner(document_owner, realm_id)
        else {
            return FrameDocumentModuleTerminalQueueFollowup::none();
        };
        let tasks = self
            .child_document_modulator_store
            .take_ready_document_modulator_terminal_batches(task_owner, realm_id);
        self.push_child_module_terminal_batch_to_frame_lane(tasks)
    }

    pub(super) fn restore_child_document_modulator(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        document_modulator: NativeDocumentModulator,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let Some(task_owner) = self
            ._context_host
            .borrow()
            .current_child_module_route_task_owner(document_owner, realm_id)
        else {
            self.child_document_modulator_store
                .restore_document_modulator_without_owner_events(
                    document_owner,
                    realm_id,
                    document_modulator,
                );
            return FrameDocumentModuleTerminalQueueFollowup::none();
        };
        let tasks = self
            .child_document_modulator_store
            .restore_document_modulator(task_owner, realm_id, document_modulator);
        self.push_child_module_terminal_batch_to_frame_lane(tasks)
    }

    pub(super) fn with_current_child_document_modulator<R>(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        operation: impl FnOnce(&mut Self, &mut NativeDocumentModulator) -> R,
    ) -> Option<R> {
        let mut document_modulator = self
            .child_document_modulator_store
            .take_current_document_modulator(document_owner, realm_id)?;
        let result = operation(self, &mut document_modulator);
        self.restore_child_document_modulator(document_owner, realm_id, document_modulator);
        Some(result)
    }

    pub(super) fn ensure_child_document_modulator_for_graph_start(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) {
        self.child_document_modulator_store
            .ensure_document_modulator(owner, realm_id);
    }

    fn record_child_module_terminal_warning(
        &mut self,
        warning_record: FrameDocumentModuleTerminalWarningRecord,
    ) {
        let (task_owner, realm_id, warning) = warning_record.into_parts();
        match warning {
            FrameDocumentModuleTerminalWarning::ParserRootTerminalWithoutOwnerWork {
                key,
                successful,
                parser_root_client_count,
            } => {
                self.record_runtime_warning(format_args!(
                    "child parser root terminal notification for {:?}/{:?} url={} successful={} parser_root_client_count={} produced no owner-local terminal work",
                    task_owner,
                    realm_id,
                    key.url(),
                    successful,
                    parser_root_client_count
                ));
            }
        }
    }

    pub(super) fn with_current_child_document_modulator_or_module_load_error<R>(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        operation: impl FnOnce(&mut Self, &mut NativeDocumentModulator) -> Result<R, ModuleLoadError>,
    ) -> Result<R, ModuleLoadError> {
        let Some(mut document_modulator) = self
            .child_document_modulator_store
            .take_current_document_modulator(document_owner, realm_id)
        else {
            return Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "child module graph had no current document modulator",
            ));
        };
        let result = operation(self, &mut document_modulator);
        self.restore_child_document_modulator(document_owner, realm_id, document_modulator);
        result
    }

    pub(super) fn child_module_request_initiator_url_for_owner(
        &self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        fallback_url: &Url,
    ) -> Url {
        self._context_host
            .borrow()
            .frame_owner_current_child_snapshot_for_realm(realm_id)
            .filter(|snapshot| {
                snapshot.local_window_id == document_owner.local_window_id
                    && snapshot.document_id == document_owner.document_id
            })
            .map(|snapshot| snapshot.document_base_url)
            .unwrap_or_else(|| fallback_url.clone())
    }

    pub(super) fn with_current_child_module_tree_owner_or_module_load_error<R>(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        fallback_url: &Url,
        operation: impl FnOnce(
            &mut NativeModuleTreeFrameDocumentOwner<'_>,
        ) -> Result<R, ModuleLoadError>,
    ) -> Result<R, ModuleLoadError> {
        let module_request_initiator_url = self.child_module_request_initiator_url_for_owner(
            document_owner,
            realm_id,
            fallback_url,
        );
        self.with_current_child_document_modulator_or_module_load_error(
            document_owner,
            realm_id,
            move |vm, document_modulator| {
                let mut module_owner = NativeModuleTreeFrameDocumentOwner::new(
                    vm,
                    document_modulator,
                    document_owner,
                    realm_id,
                    module_request_initiator_url,
                );
                operation(&mut module_owner)
            },
        )
    }

    pub(super) fn reserve_child_parser_root_module_client(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        client: FrameDocumentParserRootModuleClient,
    ) -> FrameDocumentModuleClientReservation {
        self.child_document_modulator_store
            .reserve_parser_root_module_client(owner, realm_id, key, client)
    }

    pub(super) fn finish_child_parser_root_module_fetch(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request_key: ModuleMapKey,
        result: Result<ModuleGraphFetchedSource, String>,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let tasks = self
            .child_document_modulator_store
            .finish_parser_root_module_fetch(owner, realm_id, request_key, result);
        self.push_child_module_terminal_batch_to_frame_lane(tasks)
    }

    pub(super) fn record_compiled_child_parser_root(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_id: crate::document_script_scheduler::ParserPendingScriptId<
            FrameDocumentOwner,
        >,
        script: PreparedScript,
        script_handle: DomHandle,
        request_key: ModuleMapKey,
        source_url: Url,
        entry_id: ModuleEntryId,
        parent_key: ModuleMapKey,
        requests: Vec<ModuleRequestRecord>,
        effective_fetch_metadata: ModuleFetchMetadata,
        load_delay_token: crate::frame_owner_model::DocumentLoadDelayTokenId,
    ) -> ModuleTreeId {
        self.child_document_modulator_store
            .record_compiled_parser_root(
                owner,
                realm_id,
                pending_script_id,
                script,
                script_handle,
                request_key,
                source_url,
                entry_id,
                parent_key,
                requests,
                effective_fetch_metadata,
                load_delay_token,
            )
    }

    pub(super) fn mark_child_parser_module_graph_evaluated(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        root_entry: ModuleEntryId,
    ) -> bool {
        self.child_document_modulator_store
            .mark_parser_module_graph_evaluated(owner.document_owner(), realm_id, root_entry)
    }

    pub(super) fn take_child_parser_module_tree_job(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        tree_id: ModuleTreeId,
    ) -> Option<NativeParserModuleTreeJobResume> {
        self.child_document_modulator_store
            .take_parser_module_tree_job(owner, realm_id, tree_id)
    }

    pub(super) fn restore_child_parser_module_tree_job(
        &mut self,
        resume: NativeParserModuleTreeJobResume,
    ) {
        self.child_document_modulator_store
            .restore_parser_module_tree_job(resume);
    }

    pub(super) fn record_child_parser_module_tree_fetches(
        &mut self,
        resume: &mut NativeParserModuleTreeJobResume,
        fetches: Vec<NativeModuleGraphFetchRequest>,
    ) -> Result<Vec<FrameDocumentModuleDependencyFetchTask>, ModuleLoadError> {
        self.child_document_modulator_store
            .record_parser_module_tree_fetches(resume, fetches)
    }

    pub(super) fn finish_child_parser_module_dependency_fetch(
        &mut self,
        realm_id: FrameRealmId,
        task: FrameDocumentModuleDependencyFetchTask,
        result: FrameDocumentModuleFetchTerminalResult,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let owner = task.owner().document_owner();
        let tasks = self
            .child_document_modulator_store
            .finish_parser_module_dependency_fetch(owner, realm_id, task, result);
        self.route_child_module_terminal_tasks_to_page_source(tasks)
    }

    pub(super) fn take_child_dynamic_module_import_fetch(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
    ) -> Option<ChildDynamicModuleInflightFetch> {
        self.child_document_modulator_store
            .take_inflight_dynamic_module_import_fetch(owner, realm_id, load_id)
    }

    pub(super) fn finish_child_dynamic_module_inflight_fetch_with_modulator(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        module_request_initiator_url: Url,
        inflight: DynamicModuleInflightFetch,
        source: Result<ModuleGraphFetchedSource, ModuleLoadError>,
        missing_message: &'static str,
    ) -> DynamicModuleFetchFinish {
        let Some(mut document_modulator) = self
            .child_document_modulator_store
            .take_current_document_modulator(owner, realm_id)
        else {
            return inflight.into_failure(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                missing_message,
            ));
        };
        let finish = {
            let mut owner_adapter = NativeModuleTreeFrameDocumentOwner::new(
                self,
                &mut document_modulator,
                owner,
                realm_id,
                module_request_initiator_url,
            );
            inflight.finish_with_owner_adapter(&mut owner_adapter, source)
        };
        self.restore_child_document_modulator(owner, realm_id, document_modulator);
        finish
    }

    pub(super) fn finish_child_dynamic_module_joined_fetch_with_modulator(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        module_request_initiator_url: Url,
        joined: DynamicModuleJoinedFetch,
        key: &ModuleMapKey,
        missing_message: &'static str,
    ) -> DynamicModuleFetchFinish {
        let Some(mut document_modulator) = self
            .child_document_modulator_store
            .take_current_document_modulator(owner, realm_id)
        else {
            return joined.into_failure(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                missing_message,
            ));
        };
        let finish = {
            let mut owner_adapter = NativeModuleTreeFrameDocumentOwner::new(
                self,
                &mut document_modulator,
                owner,
                realm_id,
                module_request_initiator_url,
            );
            joined.finish_with_owner_adapter(&mut owner_adapter, key)
        };
        self.restore_child_document_modulator(owner, realm_id, document_modulator);
        finish
    }

    pub(super) fn finish_child_dynamic_import_terminal_client(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        key: &ModuleMapKey,
        client: NativeDynamicImportSingleModuleClient,
    ) -> FrameDocumentDynamicImportTerminalClientFinishResult {
        let tree_client = client.token();
        let Some(joined) = self
            .child_document_modulator_store
            .take_joined_dynamic_module_import_fetch(owner, realm_id, tree_client)
        else {
            return FrameDocumentDynamicImportTerminalClientFinishResult::MissingJoinedClient;
        };
        debug_assert_eq!(
            joined.joined.client(),
            tree_client,
            "child dynamic import terminal client should recover the same pending tree client"
        );
        let module_request_initiator_url = self.child_module_request_initiator_url_for_owner(
            owner,
            realm_id,
            joined.joined.import_base_url(),
        );
        let owner = joined.owner;
        let realm_id = joined.realm_id;
        let finish = self.finish_child_dynamic_module_joined_fetch_with_modulator(
            owner,
            realm_id,
            module_request_initiator_url,
            joined.joined,
            key,
            "child dynamic import joined fetch has no current document modulator",
        );
        self.child_document_modulator_store
            .dynamic_import_fetch_finish_to_terminal_client_finish_result(owner, realm_id, finish)
    }

    pub(super) fn has_pending_child_dynamic_module_import(&self) -> bool {
        self.child_document_modulator_store
            .has_pending_dynamic_module_import()
    }

    #[cfg(test)]
    pub(super) fn has_inflight_child_dynamic_module_import_fetch(&self) -> bool {
        self.child_document_modulator_store
            .has_inflight_dynamic_module_import_fetch()
    }
}
