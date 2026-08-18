use super::*;
use crate::page_task_queue::RendererPageModuleReactionEvent;

fn slotchange_microtask_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    crate::observer_runtime::flush_slotchange_microtask(scope, host_ptr);
}

impl JsContextHost {
    pub(crate) fn capture_node_creation_stack_trace(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        if !self.dom_agent_state.has_node_stack_trace_capture_interest() {
            return;
        }
        let Some(document_id) = self.document_id_for_backend_node_identity_handle(handle) else {
            return;
        };
        let Some(stack) = v8::StackTrace::current_stack_trace(scope, 32) else {
            return;
        };
        let mut call_frames = Vec::with_capacity(stack.get_frame_count());
        for index in 0..stack.get_frame_count() {
            let Some(frame) = stack.get_frame(scope, index) else {
                continue;
            };
            let function_name = frame
                .get_function_name(scope)
                .map(|name| name.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let url = frame
                .get_script_name_or_source_url(scope)
                .map(|name| name.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let script_id = frame.get_script_id().to_string();
            let line_number = u64::try_from(frame.get_line_number())
                .unwrap_or_default()
                .saturating_sub(1);
            let column_number = u64::try_from(frame.get_column())
                .unwrap_or_default()
                .saturating_sub(1);
            call_frames.push(crate::RendererDomNodeCreationStackFrame {
                function_name,
                script_id,
                url,
                line_number,
                column_number,
            });
        }
        if call_frames.is_empty() {
            return;
        }
        self.dom_agent_state.record_node_creation_stack_trace(
            document_id,
            handle,
            crate::RendererDomNodeCreationStackTrace { call_frames },
        );
    }

    pub(crate) fn capture_node_creation_stack_traces_since(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        first_node_index: usize,
    ) {
        let node_count = self.dom_host().dom().len();
        for index in first_node_index..node_count {
            self.capture_node_creation_stack_trace(scope, DomHandle::new(index));
        }
    }

    pub(crate) fn install_page_default_context(
        &mut self,
        scope: &mut v8::PinScope<'_, '_, ()>,
        context: v8::Local<'_, v8::Context>,
    ) {
        self.page_default_context = Some(v8::Weak::new(scope, context));
    }

    pub(crate) fn page_default_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Context>> {
        self.page_default_context
            .as_ref()
            .and_then(|context| context.to_local(scope))
    }

    pub(crate) fn initialize_new_native_node_owner_document(
        &mut self,
        document_handle: DomHandle,
        handle: DomHandle,
    ) -> Option<DomHandle> {
        // This is creation-time owner assignment for a freshly created native
        // node. User-visible adoption must go through DocumentRuntime::adopt_node(...),
        // which snapshots registry/adoptedCallback side effects before mutating.
        self.dom_host_mut().adopt_node(document_handle, handle)
    }

    pub(crate) fn set_popover_focus_restore_target(
        &mut self,
        popover: DomHandle,
        target: Option<DomHandle>,
    ) {
        self.popover_focus_restore_targets.insert(popover, target);
    }

    pub(crate) fn take_popover_focus_restore_target(
        &mut self,
        popover: DomHandle,
    ) -> Option<DomHandle> {
        self.popover_focus_restore_targets
            .remove(&popover)
            .flatten()
    }

    pub(crate) fn focus_change_epoch(&self) -> u64 {
        self.focus_change_epoch
    }

    pub(crate) fn mark_focus_changed(&mut self) {
        self.focus_change_epoch = self.focus_change_epoch.wrapping_add(1);
    }
}

impl JsContextHost {
    pub(crate) fn new(
        runtime: &mut DocumentRuntime,
        bindings: NativeBridgeBindings,
        backend_node_registry: SharedRendererBackendNodeRegistry,
        dom_debugger_pause_scheduler: crate::script_vm::RendererDomDebuggerPauseScheduler,
        resource_completion_tx: RendererResourceCompletionSender,
        top_level_navigation_handoff_tx: crate::page_task_queue::RendererTopLevelNavigationHandoffSender,
        service_worker_task_tx: crate::page_task_queue::RendererPageServiceWorkerTaskSender,
        browser_context_runtime: RendererBrowserContextRuntime,
        javascript_dialog_runtime: crate::runtime::RendererJavaScriptDialogRuntime,
        page_context_cancel_rx: RendererPageContextCancelReceiver,
        top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
        reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
    ) -> Self {
        let message_port_registry = browser_context_runtime.message_port_registry();
        let broadcast_channel_registry = browser_context_runtime.broadcast_channel_registry();
        let shared_worker_client_owner_id =
            browser_context_runtime.next_shared_worker_client_owner_id();
        let javascript_dialog_handler_enabled =
            browser_context_runtime.javascript_dialog_handler_enabled();
        let document_handle = runtime.document_handle();
        let document_url = runtime.document_url().clone();
        let document_base_url = runtime
            .dom_host()
            .document_base_url_for_handle(document_handle)
            .unwrap_or_else(|| document_url.clone());
        let mut frame_owner_store = FrameOwnerStore::default();
        frame_owner_store.ensure_main_frame(
            document_handle,
            document_url.clone(),
            document_base_url,
            moli_url::origin_ascii_serialization(&document_url),
            runtime.document_policy_container().clone(),
            crate::types::SubresourcePolicyContext {
                cross_origin_embedder_policy: runtime.cross_origin_embedder_policy(),
                document_isolation_policy: runtime.document_isolation_policy(),
                cross_origin_isolated: runtime.cross_origin_isolated(),
            },
            None,
        );
        let main_owner_snapshot = frame_owner_store
            .current_main_owner_snapshot()
            .expect("main frame owner must exist before client projection");
        let main_document_owner = FrameDocumentTaskOwner::new(
            main_owner_snapshot.scheduler_lane_id,
            main_owner_snapshot.local_window_id,
            main_owner_snapshot.document_id,
        );
        let service_worker_storage_key = top_level_storage_key
            .as_ref()
            .map(moli_storage_key::MoliStorageKey::serialized_storage_key)
            .unwrap_or_else(|| {
                super::service_workers::service_worker_first_party_storage_key(&document_url)
            });
        let service_worker_client_id = reserved_service_worker_client_id
            .filter(|client_id| {
                browser_context_runtime.update_service_worker_client_document_and_page_endpoint(
                    *client_id,
                    document_url.clone(),
                    service_worker_storage_key.clone(),
                    crate::service_worker_runtime::ServiceWorkerClientFrameType::TopLevel,
                    Some(super::WindowDocumentOwner::Frame(main_document_owner)),
                    service_worker_task_tx.clone(),
                )
            })
            .unwrap_or_else(|| {
                browser_context_runtime.register_service_worker_client(
                    document_url,
                    service_worker_storage_key,
                    crate::service_worker_runtime::ServiceWorkerClientFrameType::TopLevel,
                    Some(super::WindowDocumentOwner::Frame(main_document_owner)),
                    service_worker_task_tx.clone(),
                )
            });
        assert!(
            frame_owner_store
                .set_current_main_service_worker_client_id(Some(service_worker_client_id,)),
            "main service worker client projection requires a current owner"
        );
        let mut host = Self {
            runtime: runtime as *mut DocumentRuntime,
            layout_policy: moli_page_types::LayoutPolicy::default(),
            document_layout_state: RefCell::new(super::layout_state::DocumentLayoutState::default()),
            layout_pass_active: Cell::new(false),
            completed_layout_pass_count: Cell::new(0),
            completed_layout_pass_time: Cell::new(std::time::Duration::ZERO),
            last_layout_pass_metrics: Cell::new(None),
            layout_snapshot_cache_hits: Cell::new(0),
            layout_snapshot_cache_misses: Cell::new(0),
            layout_snapshot_cache_publishes: Cell::new(0),
            #[cfg(test)]
            force_fresh_layout_reads_for_test: false,
            root_document_lifecycle: None,
            output_journal: None,
            page_context_resources_closed: false,
            page_default_context: None,
            v8_finalizers: crate::v8_finalizer::V8FinalizerRegistry::default(),
            bridge: NativeDomBridge::new(bindings),
            dom_agent_state: crate::runtime::RendererDomAgentState::new(
                backend_node_registry.clone(),
            ),
            backend_node_registry,
            dom_debugger_state: super::dom_debugger::DomDebuggerState::new(
                dom_debugger_pause_scheduler,
            ),
            #[cfg(test)]
            bridge_ref_count: std::rc::Rc::new(std::cell::Cell::new(0)),
            range_record_registry: range_records::RangeRecordRegistry::new(),
            selection_record_registry: selection_records::SelectionRecordRegistry::new(),
            custom_elements: CustomElementStore::default(),
            custom_element_reactions: CustomElementReactionCoordinator::default(),
            child_custom_elements: HashMap::new(),
            scoped_custom_elements: HashMap::new(),
            parser_defined_autonomous_custom_elements: VecDeque::new(),
            parser_custom_element_handoff_replacements: HashMap::new(),
            scoped_custom_element_registry_wrappers: HashMap::new(),
            custom_element_registry_associations: IndexMap::new(),
            next_scoped_custom_elements_registry_id: 1,
            observers: ObserverStore::default(),
            text_codecs: TextCodecStore::default(),
            child_browsing_contexts: IndexMap::new(),
            frame_owner_store,
            frame_parser_classic_scripts: FrameParserClassicScriptRunnerStore::default(),
            frame_parser_deferred_script_order: FrameParserDeferredScriptOrderStore::default(),
            frame_document_blocking_stylesheets: FrameDocumentBlockingStylesheetStore::default(),
            child_document_script_schedulers: FrameDocumentScriptSchedulerStore::default(),
            child_document_parsers: ChildDocumentParserStore::default(),
            child_window_proxy_records: ChildWindowProxyRecords::default(),
            child_default_context_bootstrap: None,
            #[cfg(test)]
            force_child_default_context_preflight_failure: false,
            child_browsing_context_document_handles: HashMap::new(),
            document_domain_override: None,
            next_child_browsing_context_id: 1,
            next_child_document_load_id: 0,
            next_child_classic_script_load_id: 0,
            pending_child_document_navigations: HashMap::new(),
            document_resource_loaders: DocumentResourceLoaderRegistry::default(),
            web_storage_store: new_shared_web_storage_store(),
            session_storage_store: new_shared_web_storage_store(),
            indexed_db_manager: None,
            storage_bucket_store: new_shared_storage_bucket_store(),
            stored_document_start_scripts: Vec::new(),
            stored_runtime_bindings: Vec::new(),
            app_manifest_link_change_epoch: 0,
            extra_http_headers: Vec::new(),
            permission_overrides: Vec::new(),
            locale_override: None,
            timezone_override: None,
            idle_override: None,
            protocol_user_gesture_activation_depth: 0,
            webdriver_bidi_file_prompt_handler_stack: Vec::new(),
            emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
            viewport_surface: None,
            wpt_extensions_enabled: false,
            network_offline: false,
            blocked_url_patterns: Vec::new(),
            service_worker_client_id,
            service_worker_control: None,
            next_service_worker_register_request_id: 1,
            pending_service_worker_registers: HashMap::new(),
            pending_service_worker_unregisters: HashMap::new(),
            pending_service_worker_ready: HashMap::new(),
            service_worker_registration_watchers: Vec::new(),
            service_worker_lifecycle_watched_scopes: HashSet::new(),
            service_worker_popup_clients: HashMap::new(),
            pending_service_worker_clients_open_window_popups: HashMap::new(),
            pending_window_messages: VecDeque::new(),
            next_window_message_task_id:
                crate::page_task_queue::RendererPageWindowMessageTaskId::from_raw(1),
            indexed_db_context_tasks: IndexedDbContextState::default(),
            window_execution_contexts: HashMap::new(),
            current_window_message_source: None,
            pending_active_child_window_restore: None,
            pending_active_lightweight_popup_restore: None,
            pending_child_subresource_request_scope_pop: false,
            pending_text_control_change_commit: None,
            directory_reader_callbacks:
                super::directory_reader_callbacks::DirectoryReaderCallbackState::default(),
            misc_platform_api_tasks:
                super::misc_platform_api_tasks::MiscPlatformApiTaskState::default(),
            file_entry_file_callbacks:
                super::file_entry_file_callbacks::FileEntryFileCallbackState::default(),
            user_interaction_tasks:
                super::user_interaction_tasks::UserInteractionTaskState::default(),
            pending_image_load_events: HashMap::new(),
            next_image_load_event_id: 1,
            pending_media_load_sequences: HashMap::new(),
            next_media_load_sequence_id: 1,
            pending_text_track_load_sequences: HashMap::new(),
            next_text_track_load_sequence_id: 1,
            pending_media_text_track_gates: HashMap::new(),
            active_pointer_capture_ids: HashSet::new(),
            pending_pointer_capture_targets: HashMap::new(),
            pointer_capture_targets: HashMap::new(),
            lazy_media_load_candidates: HashSet::new(),
            canvas_resources: super::canvas_resources::CanvasResourceStore::default(),
            image_resources: super::image_resources::ImageResourceStore::default(),
            next_image_decode_id: 1,
            pending_image_decode_requests: HashMap::new(),
            resource_timing_buffers: SharedResourceTimingBufferRegistry::new(),
            next_webcrypto_task_id: crate::page_task_queue::RendererPageWebCryptoTaskId::first(),
            pending_webcrypto_tasks: HashMap::new(),
            opfs_owner_state: None,
            history_queue: HistoryQueueState::default(),
            rendering_updates: super::rendering_updates::RenderingUpdateState::default(),
            scroll_observable_effect_batch:
                super::interaction_batch::ScrollObservableEffectBatchState::default(),
            view_transition_updates:
                super::view_transition_updates::ViewTransitionUpdateState::default(),
            media_element_events: super::media_element_events::MediaElementEventState::default(),
            element_toggle_events: super::element_toggle_events::ElementToggleEventState::default(),
            text_track_default_modes:
                super::text_track_default_modes::TextTrackDefaultModeState::default(),
            child_document_script_ready_tasks:
                super::document_script_ready_inputs::ChildDocumentScriptReadyTaskLedger::default(),
            pending_child_external_classic_document_scripts: HashMap::new(),
            pending_child_modulepreload_work_awaiting_realm: VecDeque::new(),
            active_child_browsing_context_host_loads: Vec::new(),
            character_data_utf16_overrides: HashMap::new(),
            child_meta_refresh_navigations: HashMap::new(),
            disconnected_shadow_roots: HashSet::new(),
            live_stylesheets: crate::live_stylesheet::LiveStylesheetRegistry::default(),
            style_engine: MoliStyleEngine::new(),
            inline_style_declarations: HashMap::new(),
            css_module_texts_by_url: HashMap::new(),
            css_module_failed_urls: HashSet::new(),
            popover_focus_restore_targets: HashMap::new(),
            pending_slotchange_slots: Vec::new(),
            deferred_slotchange_slots: Vec::new(),
            slotchange_flush_scheduled: false,
            mutation_observer_delivery_depth: 0,
            #[cfg(test)]
            pending_child_frame_tree_events: Vec::new(),
            internal_node_references: HashMap::new(),
            internal_inspector_value_references: HashMap::new(),
            #[cfg(test)]
            completed_child_browsing_context_loads: Vec::new(),
            #[cfg(test)]
            completed_child_document_networks: Vec::new(),
            active_child_subresource_request_scopes: Vec::new(),
            child_window_event_listeners: HashMap::new(),
            next_child_window_event_registration_id: 0,
            event_callbacks: Default::default(),
            browser_context_runtime,
            top_level_navigation_handoff_tx,
            service_worker_task_tx,
            message_port_registry,
            message_port_wrappers: HashMap::new(),
            broadcast_channel_registry,
            shared_worker_client_owner_id,
            child_shared_worker_client_owner_ids: HashMap::new(),
            shared_worker_clients: SharedWorkerClientEndpointOwner::default(),
            top_level_storage_key,
            web_storage_opaque_context_nonce: None,
            child_web_storage_opaque_context_nonces: HashMap::new(),
            broadcast_channel_wrappers: HashMap::new(),
            form_past_named_items: HashMap::new(),
            button_element_targets: HashMap::new(),
            constructing_form_data_forms: Vec::new(),
            active_form_submission_forms: Vec::new(),
            pending_form_submission_child_targets: HashMap::new(),
            active_image_submitter_coordinate: None,
            current_inline_script_stack: Vec::new(),
            compiled_string_provenance: Vec::new(),
            active_runtime_command_cause: None,
            active_inspector_dispatch: false,
            pending_top_level_navigation: None,
            ordinary_page_turn_navigation_handoff_active: false,
            next_navigation_attempt_id: 1,
            active_navigation_attempts: HashMap::new(),
            navigation_lifecycle_trace: VecDeque::new(),
            command_turn_output: None,
            runtime_binding_execution_context_owners: HashMap::new(),
            window_execution_context_realms: Default::default(),
            #[cfg(test)]
            pending_runtime_binding_calls: Vec::new(),
            next_runtime_observable_context_token: super::RuntimeObservableContextToken::first(),
            pending_runtime_observable_console_source_events: Vec::new(),
            #[cfg(test)]
            pending_file_chooser_activations: Vec::new(),
            #[cfg(test)]
            pending_download_activations: Vec::new(),
            #[cfg(test)]
            pending_popup_activations: Vec::new(),
            next_lightweight_popup_id: 1,
            next_lightweight_popup_local_window_id: 1,
            next_lightweight_popup_document_id: 1,
            next_lightweight_popup_document_load_id: 0,
            next_lightweight_popup_classic_script_load_id: 0,
            lightweight_popup_browsing_contexts: HashMap::new(),
            lightweight_popup_window_names: HashMap::new(),
            lightweight_popup_document_handles: HashMap::new(),
            pending_lightweight_popup_document_loads: HashMap::new(),
            pending_lightweight_popup_classic_script_loads: HashMap::new(),
            #[cfg(test)]
            pending_javascript_dialogs: Vec::new(),
            javascript_dialog_runtime,
            next_javascript_dialog_id: 1,
            javascript_dialog_handler_enabled,
            pending_network_output: Vec::new(),
            focus_change_epoch: 0,
            next_subresource_network_request_handle: 1,
            subresource_activity_epoch: 0,
            subresource_last_activity_at: std::time::Instant::now(),
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
            active_subresource_requests: 0,
            next_pending_subresource_fetch_id: 0,
            pending_subresource_fetches: HashMap::new(),
            pending_subresource_auths: HashMap::new(),
            pending_subresource_responses: HashMap::new(),
            pending_websocket_responses: HashMap::new(),
            #[cfg(test)]
            pending_subresource_fetch_infos: Vec::new(),
            running_subresource_fetches: HashMap::new(),
            streaming_subresource_fetches: HashMap::new(),
            in_flight_worker_subresource_fetches: HashMap::new(),
            #[cfg(test)]
            pending_subresource_continue_events: Vec::new(),
            pending_network_body_sources: HashMap::new(),
            pending_network_body_clones: HashMap::new(),
            page_task_capabilities: std::cell::OnceCell::new(),
            resource_scheduler: RendererResourceScheduler::new(resource_completion_tx.clone()),
            resource_completion_tx,
            next_worker_id: 1,
            workers: HashMap::new(),
            next_websocket_id: 1,
            websockets: HashMap::new(),
            synchronous_xhr_request_counts: HashMap::new(),
            page_context_cancel_rx,
            layout_metric_trace: RefCell::default(),
            layout_rect_cache: RefCell::default(),
            layout_flow_top_cache: RefCell::default(),
            layout_mock_rendered_element_cache: RefCell::default(),
            layout_preceding_flow_count_cache: RefCell::default(),
            layout_flow_prefix_cursor_cache: RefCell::default(),
            #[cfg(test)]
            layout_flow_subtree_node_visits: Cell::new(0),
            #[cfg(test)]
            stylo_computed_style_input_builds: Cell::new(0),
            #[cfg(test)]
            stylo_style_system_key_builds: Cell::new(0),
            #[cfg(test)]
            stylo_computed_style_property_reads: Cell::new(0),
        };
        host.sync_owner_style_sheet_texts_for_document_tree_scopes(host.document_handle());
        host
    }

    pub(crate) fn set_root_document_lifecycle(
        &mut self,
        lifecycle: RendererDocumentLifecycleJournalHandle,
    ) {
        self.root_document_lifecycle = Some(lifecycle);
    }

    /// Returns the exact root Document that owns Page-scoped protocol
    /// handoffs produced by this host.
    ///
    /// Production PageVm instances and ordinary standalone integration
    /// fixtures install a journal before JavaScript can produce browser-owner
    /// work. Only tests that explicitly exercise the no-Page fallback omit it.
    pub(crate) fn root_document_lifecycle_identity(
        &self,
    ) -> Option<crate::runtime::RendererDocumentLifecycleIdentity> {
        self.root_document_lifecycle
            .as_ref()
            .map(RendererDocumentLifecycleJournalHandle::identity)
    }

    pub(crate) fn open_root_document(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        let descendant_count_before = self.child_browsing_contexts.len();
        for child_handle in self.top_level_child_browsing_context_handles_in_document_order() {
            self.drop_child_browsing_context_subtree_with_window_realm(scope, child_handle);
        }
        let retired_document_handle = self.document_handle();
        let retired_image_event_count =
            self.retire_image_state_for_document(retired_document_handle);
        let retired_media_load_count =
            self.cancel_pending_media_loads_for_document(retired_document_handle);
        let retired_text_track_load_count =
            self.cancel_pending_text_track_loads_for_document(retired_document_handle);
        let retired_stylesheet_subresource_count = self
            .current_main_document_task_owner()
            .map(|owner| self.cancel_stylesheet_subresource_fetches_for_document_owner(owner))
            .unwrap_or_default();
        {
            let runtime: &mut DocumentRuntime = self;
            runtime.open_document();
        }
        let retired_descendant_count =
            descendant_count_before.saturating_sub(self.child_browsing_contexts.len());
        let document_handle = self.document_handle();
        let document_url = self.document_url().clone();
        let document_base_url = self.document_base_url_for_handle(document_handle);
        let transition = self
            .frame_owner_store
            .replace_main_document(document_handle, document_url, document_base_url)
            .expect("main document owner must exist before document.open() replacement");
        {
            let runtime: &mut DocumentRuntime = self;
            runtime.start_root_document_parser_stream();
        }
        self.dom_agent_state
            .reset_for_document_replacement(transition.current_owner().document_id);
        self.replace_main_document_resource_loader(transition);
        let main_runtime_route_expected = {
            let runtime: &DocumentRuntime = self;
            runtime.has_main_document_runtime_route()
        };
        let main_runtime_route_bound = {
            let runtime: &mut DocumentRuntime = self;
            runtime.bind_main_document_runtime_producer(transition.current_owner())
        };
        assert_eq!(
            main_runtime_route_bound, main_runtime_route_expected,
            "document.open() must replace the exact main runtime producer with the owner transaction"
        );
        {
            let runtime: &mut DocumentRuntime = self;
            runtime.bind_stylesheet_task_producer(transition.current_owner());
            runtime.bind_main_parser_continuation_producer(transition.current_owner());
        }
        let rebound_meta_refresh = {
            let runtime: &mut DocumentRuntime = self;
            runtime.rebind_top_level_meta_refresh_after_document_open(transition.current_owner())
        };
        if let Some(scheduled) = rebound_meta_refresh {
            let owner = scheduled.owner;
            let url = scheduled.navigation.url.clone();
            let delay_ms = scheduled.navigation.delay_ms;
            let (task, ready_at) = scheduled.into_internal_loading_task();
            tracing::debug!(
                ?owner,
                %url,
                delay_ms,
                ?ready_at,
                "rebound active top-level meta refresh across document.open()"
            );
            if self
                .page_internal_loading_sender()
                .schedule_at(task, ready_at)
                .is_err()
            {
                tracing::debug!("dropped rebound meta refresh because its Page source closed");
            }
        }
        let runtime_binding_transition = self.rebind_runtime_binding_document_owner(
            transition.retired_owner(),
            transition.current_owner(),
        );
        let image_decode_retirement =
            self.retire_image_decode_requests_for_document_owner(scope, transition.retired_owner());
        self.apply_main_service_worker_document_owner_transition(transition);
        self.update_main_service_worker_client_after_document_replacement(transition);
        tracing::debug!(
            retired_owner = ?transition.retired_owner(),
            current_owner = ?transition.current_owner(),
            retired_descendant_count,
            retired_image_event_count,
            retired_media_load_count,
            retired_text_track_load_count,
            retired_stylesheet_subresource_count,
            retired_runtime_binding_context_count = runtime_binding_transition
                .retired_execution_context_count(),
            rebound_runtime_binding_context_count = runtime_binding_transition
                .rebound_execution_context_count(),
            rejected_image_decode_count = image_decode_retirement.rejected_count(),
            dropped_image_decode_context_count = image_decode_retirement.dropped_context_count(),
            "applied main document replacement owner transaction"
        );
        self.inline_style_declarations.clear();
        self.style_engine
            .clear_for_document_replacement(document_handle);
        if let Some(lifecycle) = &self.root_document_lifecycle {
            lifecycle.did_open_document();
        }
    }

    pub(crate) fn install_page_task_capabilities(
        &self,
        capabilities: super::JsContextHostPageTaskCapabilities,
    ) {
        assert!(
            self.page_task_capabilities.set(capabilities).is_ok(),
            "PageVm must install its complete Page task capability set exactly once"
        );
    }

    pub(crate) fn page_websocket_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageWebSocketSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before opening a WebSocket",
            )
            .websocket()
    }

    pub(crate) fn page_modulepreload_start_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageModulepreloadStartSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before child modulepreload discovery",
            )
            .modulepreload_start()
    }

    pub(crate) fn page_child_module_dependency_fetch_start_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageChildModuleDependencyFetchStartSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before child module dependency discovery",
            )
            .child_module_dependency_fetch_start()
    }

    pub(crate) fn page_child_module_script_terminal_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageChildModuleScriptTerminalSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before child module terminal routing",
            )
            .child_module_script_terminal()
    }

    pub(crate) fn page_dynamic_import_owner_action_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageDynamicImportOwnerActionSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before child dynamic-import work",
            )
            .dynamic_import_owner_action()
    }

    pub(crate) fn page_broadcast_channel_delivery_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageBroadcastChannelDeliverySender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before script execution",
            )
            .dom_manipulation()
            .broadcast_channel_delivery()
    }

    pub(crate) fn page_storage_event_delivery_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageStorageEventDeliverySender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before Web Storage mutation",
            )
            .dom_manipulation()
            .storage_event_delivery()
    }

    pub(crate) fn page_hash_change_delivery_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageHashChangeDeliverySender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before fragment navigation",
            )
            .dom_manipulation()
            .hash_change_delivery()
    }

    pub(crate) fn page_popup_load_event_sender(
        &self,
    ) -> crate::page_task_queue::RendererPagePopupLoadEventSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before popup load admission",
            )
            .dom_manipulation()
            .popup_load_event()
    }

    pub(crate) fn page_file_entry_file_callback_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageFileEntryFileCallbackSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before FileSystemFileEntry.file callback admission",
            )
            .dom_manipulation()
            .file_entry_file_callback()
    }

    pub(crate) fn page_file_reading_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageFileReadingSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before FileSystemDirectoryReader.readEntries admission",
            )
            .file_reading()
            .clone()
    }

    pub(crate) fn page_misc_platform_api_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageMiscPlatformApiSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before miscellaneous-platform callback admission",
            )
            .misc_platform_api()
            .clone()
    }

    pub(crate) fn page_view_transition_update_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageViewTransitionUpdateSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before queuing a view-transition update callback",
            )
            .dom_manipulation()
            .view_transition_update()
    }

    pub(crate) fn page_element_toggle_event_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageElementToggleEventSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before queuing an element toggle event",
            )
            .dom_manipulation()
            .element_toggle_event()
    }

    pub(crate) fn page_image_load_event_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageImageLoadEventSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before queuing an image load event",
            )
            .dom_manipulation()
            .image_load_event()
    }

    pub(crate) fn page_text_track_default_mode_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageTextTrackDefaultModeSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before queuing text-track default-mode work",
            )
            .dom_manipulation()
            .text_track_default_mode()
    }

    pub(crate) fn page_text_track_load_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageTextTrackLoadSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before queuing text-track load work",
            )
            .text_track_load()
            .clone()
    }

    pub(crate) fn page_user_interaction_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageUserInteractionSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before queuing a user-interaction task",
            )
            .user_interaction()
            .clone()
    }

    pub(crate) fn page_history_traversal_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageHistoryTraversalSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before history traversal",
            )
            .navigation_and_traversal()
            .history_traversal()
    }

    pub(crate) fn page_child_navigation_commit_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageChildNavigationCommitSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before child navigation admission",
            )
            .navigation_and_traversal()
            .child_navigation_commit()
    }

    pub(crate) fn page_navigation_api_task_sender(
        &self,
    ) -> crate::page_task_queue::RendererPageNavigationApiTaskSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before Navigation API task admission",
            )
            .navigation_and_traversal()
            .navigation_api_task()
    }

    pub(crate) fn page_rendering_update_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageRenderingUpdateSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before rendering-update admission",
            )
            .rendering_update()
    }

    pub(crate) fn page_media_element_event_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageMediaElementEventSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before media-element event admission",
            )
            .media_element_event()
    }

    pub(crate) fn page_dedicated_worker_client_event_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageDedicatedWorkerClientEventSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before Worker construction",
            )
            .dedicated_worker_client_event()
    }

    pub(crate) fn page_shared_worker_client_event_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageSharedWorkerClientEventSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before SharedWorker construction",
            )
            .shared_worker_client_event()
    }

    pub(crate) fn page_worker_host_bridge_event_sender(
        &self,
    ) -> &crate::page_task_queue::RendererWorkerHostBridgeEventSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before Worker construction",
            )
            .worker_host_bridge()
    }

    pub(crate) fn page_webcrypto_task_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageWebCryptoTaskSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before WebCrypto registration",
            )
            .webcrypto_task()
    }

    pub(crate) fn page_indexed_db_task_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageIndexedDbTaskSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before IndexedDB task registration",
            )
            .indexed_db_task()
    }

    pub(crate) fn page_opfs_task_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageOpfsTaskSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before OPFS task registration",
            )
            .opfs_task()
    }

    pub(crate) fn page_internal_loading_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageInternalLoadingSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before internal-loading work",
            )
            .internal_loading()
    }

    pub(crate) fn page_child_modulepreload_event_action_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageChildModulepreloadEventActionSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before child modulepreload event routing",
            )
            .child_modulepreload_event_action()
    }

    pub(crate) fn page_child_frame_task_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageChildFrameTaskSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before child realm materialization",
            )
            .child_frame_task()
    }

    pub(crate) fn page_module_reaction_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageModuleReactionSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before module evaluation",
            )
            .module_reaction()
    }

    /// Readmit a queued child-realm request after an exact child owner is
    /// retired. Bootstrap unwind can run before Page capabilities are
    /// installed, when no typed request could have been accepted.
    pub(super) fn signal_page_child_realm_materialization_reconsideration_if_installed(&self) {
        let Some(capabilities) = self.page_task_capabilities.get() else {
            return;
        };
        capabilities.child_frame_task().signal_reconsideration();
    }

    /// Readmits already-queued IndexedDB work after a realm retirement when
    /// this host reached the Page-owned task bootstrap boundary.
    ///
    /// Teardown can also run while Page bootstrap is unwinding, before the
    /// atomic capability bundle is installed. In that state no stable Page
    /// IndexedDB route could have accepted work, so there is nothing to
    /// readmit. New IndexedDB work continues to use
    /// [`Self::page_indexed_db_task_sender`] and therefore cannot silently
    /// bypass the required capability or fall back to a legacy queue.
    pub(super) fn signal_page_indexed_db_task_reconsideration_if_installed(&self) {
        let Some(capabilities) = self.page_task_capabilities.get() else {
            return;
        };
        capabilities.indexed_db_task().signal_reconsideration();
    }

    pub(crate) fn page_window_message_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageWindowMessageSender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before script execution",
            )
            .window_message()
    }

    pub(crate) fn page_message_port_delivery_sender(
        &self,
    ) -> &crate::page_task_queue::RendererPageMessagePortDeliverySender {
        self.page_task_capabilities
            .get()
            .expect(
                "a live Page Window must install its complete Page task capabilities before script execution",
            )
            .message_port_delivery()
    }

    #[cfg(test)]
    pub(crate) fn bridge_ref_count_for_test(&self) -> usize {
        self.bridge_ref_count.get()
    }

    #[cfg(test)]
    pub(crate) fn custom_element_registry_association_count_for_test(&self) -> usize {
        self.custom_element_registry_associations.len()
    }

    #[cfg(test)]
    pub(crate) fn scoped_custom_element_registry_wrapper_count_for_test(&self) -> usize {
        self.scoped_custom_element_registry_wrappers.len()
    }

    #[cfg(test)]
    pub(crate) fn scoped_custom_elements_store_count_for_test(&self) -> usize {
        self.scoped_custom_elements.len()
    }

    #[cfg(test)]
    pub(crate) fn remove_scoped_custom_element_registry_wrapper_for_test(&mut self, id: u64) {
        self.scoped_custom_element_registry_wrappers.remove(&id);
    }

    #[cfg(test)]
    pub(crate) fn compact_scoped_custom_element_registry_wrappers_for_test(&mut self) {
        self.compact_scoped_custom_element_registry_wrappers();
    }

    #[cfg(test)]
    pub(crate) fn has_pending_image_load_event_for_test(&self, handle: DomHandle) -> bool {
        self.pending_image_load_events.contains_key(&handle)
    }

    fn queue_page_module_reaction(&self, event: RendererPageModuleReactionEvent) {
        if self.page_module_reaction_sender().send(event).is_err() {
            tracing::debug!("discarded module reaction after its stable Page route closed");
        }
    }

    pub(crate) fn queue_document_module_script_evaluation_fulfilled(
        &mut self,
        document_owner: FrameDocumentTaskOwner,
        reaction_id: u64,
    ) {
        self.queue_page_module_reaction(
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationFulfilled {
                document_owner,
                reaction_id,
            },
        );
    }

    pub(crate) fn queue_document_module_script_evaluation_rejected(
        &mut self,
        document_owner: FrameDocumentTaskOwner,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) {
        self.queue_page_module_reaction(
            RendererPageModuleReactionEvent::DocumentModuleScriptEvaluationRejected {
                document_owner,
                reaction_id,
                reason,
                error_constructor,
            },
        );
    }

    pub(crate) fn queue_child_parser_module_script_evaluation_fulfilled(
        &mut self,
        document_owner: FrameDocumentTaskOwner,
        realm_id: crate::frame_owner_model::FrameRealmId,
        reaction_id: u64,
    ) {
        self.queue_page_module_reaction(
            RendererPageModuleReactionEvent::ChildParserModuleEvaluationFulfilled {
                document_owner,
                realm_id,
                reaction_id,
            },
        );
    }

    pub(crate) fn queue_child_parser_module_script_evaluation_rejected(
        &mut self,
        document_owner: FrameDocumentTaskOwner,
        realm_id: crate::frame_owner_model::FrameRealmId,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) {
        self.queue_page_module_reaction(
            RendererPageModuleReactionEvent::ChildParserModuleEvaluationRejected {
                document_owner,
                realm_id,
                reaction_id,
                reason,
                error_constructor,
            },
        );
    }

    pub(crate) fn queue_native_dynamic_module_evaluation_fulfilled(&mut self, reaction_id: u64) {
        let import_owner =
            unsafe { &*self.runtime }.native_dynamic_module_evaluation_reaction_owner(reaction_id);
        let Some(import_owner) = import_owner else {
            tracing::debug!(
                reaction_id,
                "ignored dynamic module fulfillment without a pending execution-context reaction"
            );
            return;
        };
        self.queue_page_module_reaction(
            RendererPageModuleReactionEvent::DynamicModuleEvaluationFulfilled {
                import_owner,
                reaction_id,
            },
        );
    }

    pub(crate) fn queue_native_dynamic_module_evaluation_rejected(
        &mut self,
        reaction_id: u64,
        reason: v8::Global<v8::Value>,
    ) {
        let import_owner =
            unsafe { &*self.runtime }.native_dynamic_module_evaluation_reaction_owner(reaction_id);
        let Some(import_owner) = import_owner else {
            tracing::debug!(
                reaction_id,
                "ignored dynamic module rejection without a pending execution-context reaction"
            );
            return;
        };
        self.queue_page_module_reaction(
            RendererPageModuleReactionEvent::DynamicModuleEvaluationRejected {
                import_owner,
                reaction_id,
                reason,
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn queue_native_dynamic_module_evaluation_fulfilled_for_owner_for_test(
        &mut self,
        import_owner: crate::module_runtime::DynamicModuleImportOwner,
        reaction_id: u64,
    ) {
        self.queue_page_module_reaction(
            RendererPageModuleReactionEvent::DynamicModuleEvaluationFulfilled {
                import_owner,
                reaction_id,
            },
        );
    }

    pub(crate) fn native_bridge_mut(&mut self) -> &mut NativeDomBridge {
        &mut self.bridge
    }

    pub(crate) fn native_bridge(&self) -> &NativeDomBridge {
        &self.bridge
    }

    pub(crate) fn character_data_utf16_units(&self, handle: DomHandle) -> Option<Vec<u16>> {
        self.character_data_utf16_overrides
            .get(&handle)
            .map(|value| value.as_slice().to_vec())
            .or_else(|| {
                self.dom_host()
                    .node(handle)
                    .and_then(moli_dom::native::Node::data_value)
                    .map(utf16_units)
            })
    }

    pub(crate) fn set_character_data_utf16_units(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        units: &[u16],
    ) -> bool {
        self.set_character_data_utf16_units_with_options(scope, host_ptr, handle, units, false)
    }

    pub(crate) fn set_character_data_utf16_units_for_edit(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        units: &[u16],
    ) -> bool {
        self.set_character_data_utf16_units_with_options(scope, host_ptr, handle, units, true)
    }

    fn set_character_data_utf16_units_with_options(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        units: &[u16],
        force_mutation_record: bool,
    ) -> bool {
        let old_value = force_mutation_record
            .then(|| {
                self.dom_host()
                    .node(handle)
                    .and_then(moli_dom::native::Node::node_value)
                    .map(str::to_owned)
            })
            .flatten();
        let value = string_from_utf16_units_lossy(units);
        let changed = self.set_text_content(scope, host_ptr, handle, &value);
        if !changed && force_mutation_record && self.dom_host().mutation_records_enabled() {
            let runtime: &mut DocumentRuntime = self;
            let _ =
                runtime.queue_character_data_mutation_record(scope, host_ptr, handle, old_value);
        }
        if utf16_units_contain_unpaired_surrogate(units) {
            self.character_data_utf16_overrides
                .insert(handle, U16String::from_vec(units.to_vec()));
        }
        changed
    }

    pub(crate) fn set_text_content(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        value: &str,
    ) -> bool {
        self.character_data_utf16_overrides.remove(&handle);
        let runtime: &mut DocumentRuntime = self;
        runtime.set_text_content(scope, host_ptr, handle, value)
    }

    pub(crate) fn push_current_inline_script(&mut self, handle: DomHandle) {
        self.current_inline_script_stack.push(handle);
    }

    pub(crate) fn pop_current_inline_script(&mut self, handle: DomHandle) {
        let popped = self.current_inline_script_stack.pop();
        debug_assert_eq!(popped, Some(handle));
    }

    pub(crate) fn current_inline_script_handle(&self) -> Option<DomHandle> {
        self.current_inline_script_stack.last().copied()
    }

    pub(crate) fn register_compiled_string_provenance(
        &mut self,
        provenance: &crate::script_provenance::CompiledStringProvenance,
    ) {
        if self
            .compiled_string_provenance
            .iter()
            .any(|entry| entry == provenance)
        {
            return;
        }
        self.compiled_string_provenance.push(provenance.clone());
    }

    pub(crate) fn script_base_url_for_compiled_string_resource(
        &self,
        script_url: &Url,
    ) -> Option<Url> {
        self.compiled_string_provenance
            .iter()
            .rev()
            .find(|entry| entry.source_url() == script_url && entry.module_base_url() != script_url)
            .map(|entry| entry.module_base_url().clone())
    }

    pub(crate) fn queue_slotchange_events(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        slots: &[DomHandle],
    ) {
        for &slot in slots {
            if self.pending_slotchange_slots.contains(&slot) {
                if self.mutation_observer_delivery_depth > 0
                    && !self.deferred_slotchange_slots.contains(&slot)
                {
                    self.deferred_slotchange_slots.push(slot);
                }
            } else {
                self.pending_slotchange_slots.push(slot);
            }
        }
        self.schedule_slotchange_flush(scope);
    }

    pub(crate) fn take_pending_slotchange_slots(&mut self) -> Vec<DomHandle> {
        self.slotchange_flush_scheduled = false;
        std::mem::take(&mut self.pending_slotchange_slots)
    }

    pub(crate) fn promote_deferred_slotchange_events(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        for slot in std::mem::take(&mut self.deferred_slotchange_slots) {
            if !self.pending_slotchange_slots.contains(&slot) {
                self.pending_slotchange_slots.push(slot);
            }
        }
        self.schedule_slotchange_flush(scope);
    }

    fn schedule_slotchange_flush(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        if self.slotchange_flush_scheduled || self.pending_slotchange_slots.is_empty() {
            return;
        }
        let Some(callback) = v8::Function::new(scope, slotchange_microtask_callback) else {
            return;
        };
        self.slotchange_flush_scheduled = true;
        crate::util::enqueue_host_microtask(scope, callback);
    }

    pub(crate) fn has_scheduled_mutation_delivery(&self) -> bool {
        self.observers.has_scheduled_mutation_delivery()
    }

    pub(crate) fn defer_slotchange_flush(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        self.slotchange_flush_scheduled = false;
        self.schedule_slotchange_flush(scope);
    }

    pub(crate) fn begin_mutation_observer_delivery(&mut self) {
        self.mutation_observer_delivery_depth += 1;
    }

    pub(crate) fn end_mutation_observer_delivery(&mut self) {
        self.mutation_observer_delivery_depth =
            self.mutation_observer_delivery_depth.saturating_sub(1);
    }

    pub(crate) fn allow_synchronous_xhr_request(&mut self, url: &Url) -> bool {
        const MAX_SYNC_XHR_REQUESTS_PER_URL: u32 = 32;

        let count = self
            .synchronous_xhr_request_counts
            .entry(url.as_str().to_owned())
            .or_insert(0);
        *count = count.saturating_add(1);
        *count <= MAX_SYNC_XHR_REQUESTS_PER_URL
    }

    pub(crate) fn page_context_cancel_receiver(&self) -> RendererPageContextCancelReceiver {
        self.page_context_cancel_rx.clone()
    }

    pub(in crate::native_bridge) fn remember_form_past_named_item(
        &mut self,
        form_handle: DomHandle,
        key: String,
        item_handle: DomHandle,
    ) {
        self.form_past_named_items
            .insert((form_handle, key), item_handle);
    }

    pub(in crate::native_bridge) fn form_past_named_item(
        &self,
        form_handle: DomHandle,
        key: &str,
    ) -> Option<DomHandle> {
        let item_handle = self
            .form_past_named_items
            .get(&(form_handle, key.to_owned()))
            .copied()?;
        self.dom_host()
            .form_control_elements(form_handle)
            .contains(&item_handle)
            .then_some(item_handle)
    }

    pub(in crate::native_bridge) fn remember_button_element_target(
        &mut self,
        source_handle: DomHandle,
        slot: &str,
        target_handle: DomHandle,
    ) {
        self.button_element_targets
            .insert((source_handle, slot.to_owned()), target_handle);
    }

    pub(in crate::native_bridge) fn clear_button_element_target(
        &mut self,
        source_handle: DomHandle,
        slot: &str,
    ) {
        self.button_element_targets
            .remove(&(source_handle, slot.to_owned()));
    }

    pub(in crate::native_bridge) fn button_element_target(
        &self,
        source_handle: DomHandle,
        slot: &str,
    ) -> Option<DomHandle> {
        self.button_element_targets
            .get(&(source_handle, slot.to_owned()))
            .copied()
            .filter(|handle| self.dom_host().node(*handle).is_some())
    }

    pub(in crate::native_bridge) fn replace_active_image_submitter_coordinate(
        &mut self,
        coordinate: Option<(DomHandle, u32, u32)>,
    ) -> Option<(DomHandle, u32, u32)> {
        std::mem::replace(&mut self.active_image_submitter_coordinate, coordinate)
    }

    pub(crate) fn active_image_submitter_coordinate(
        &self,
        handle: DomHandle,
    ) -> Option<(u32, u32)> {
        self.active_image_submitter_coordinate
            .filter(|(active_handle, _, _)| *active_handle == handle)
            .map(|(_, x, y)| (x, y))
    }

    pub(crate) fn mark_text_control_change_pending(
        &mut self,
        handle: DomHandle,
        committed_value: &str,
    ) {
        if self
            .pending_text_control_change_commit
            .as_ref()
            .is_some_and(|commit| commit.handle == handle)
        {
            return;
        }
        self.pending_text_control_change_commit = Some(PendingTextControlChangeCommit {
            handle,
            committed_value: committed_value.to_owned(),
        });
    }

    pub(crate) fn take_text_control_change_commit(&mut self, handle: DomHandle) -> Option<String> {
        if self
            .pending_text_control_change_commit
            .as_ref()
            .is_some_and(|commit| commit.handle == handle)
        {
            return self
                .pending_text_control_change_commit
                .take()
                .map(|commit| commit.committed_value);
        }
        None
    }

    pub(crate) fn clear_text_control_change_pending(&mut self, handle: DomHandle) {
        if self
            .pending_text_control_change_commit
            .as_ref()
            .is_some_and(|commit| commit.handle == handle)
        {
            self.pending_text_control_change_commit = None;
        }
    }

    pub(crate) fn pending_image_load_event(
        &self,
        handle: DomHandle,
    ) -> Option<super::PendingImageLoadEvent> {
        self.pending_image_load_events.get(&handle).copied()
    }

    pub(crate) fn next_image_load_event_id(&mut self) -> super::ImageLoadEventId {
        let id = super::ImageLoadEventId::new(self.next_image_load_event_id);
        self.next_image_load_event_id = self.next_image_load_event_id.wrapping_add(1).max(1);
        id
    }

    pub(crate) fn insert_pending_image_load_event(
        &mut self,
        handle: DomHandle,
        pending: super::PendingImageLoadEvent,
    ) -> bool {
        let std::collections::hash_map::Entry::Vacant(entry) =
            self.pending_image_load_events.entry(handle)
        else {
            return false;
        };
        entry.insert(pending);
        true
    }

    pub(crate) fn take_pending_image_load_event(
        &mut self,
        handle: DomHandle,
    ) -> Option<super::PendingImageLoadEvent> {
        self.pending_image_load_events.remove(&handle)
    }

    pub(crate) fn take_pending_image_load_event_if_matches(
        &mut self,
        handle: DomHandle,
        id: super::ImageLoadEventId,
    ) -> Option<super::PendingImageLoadEvent> {
        if !self
            .pending_image_load_events
            .get(&handle)
            .is_some_and(|pending| pending.id() == id)
        {
            return None;
        }
        self.pending_image_load_events.remove(&handle)
    }

    pub(crate) fn pending_media_load_sequence(
        &self,
        handle: DomHandle,
    ) -> Option<super::PendingMediaLoadSequence> {
        self.pending_media_load_sequences.get(&handle).copied()
    }

    pub(crate) fn next_media_load_sequence_id(&mut self) -> super::MediaLoadSequenceId {
        let id = super::MediaLoadSequenceId::new(self.next_media_load_sequence_id);
        self.next_media_load_sequence_id = self.next_media_load_sequence_id.wrapping_add(1).max(1);
        id
    }

    pub(crate) fn insert_pending_media_load_sequence(
        &mut self,
        handle: DomHandle,
        pending: super::PendingMediaLoadSequence,
    ) -> bool {
        let std::collections::hash_map::Entry::Vacant(entry) =
            self.pending_media_load_sequences.entry(handle)
        else {
            return false;
        };
        entry.insert(pending);
        true
    }

    pub(crate) fn take_pending_media_load_sequence(
        &mut self,
        handle: DomHandle,
    ) -> Option<super::PendingMediaLoadSequence> {
        self.pending_media_load_sequences.remove(&handle)
    }

    pub(crate) fn take_pending_media_load_sequence_if_matches(
        &mut self,
        handle: DomHandle,
        id: super::MediaLoadSequenceId,
    ) -> Option<super::PendingMediaLoadSequence> {
        if !self
            .pending_media_load_sequences
            .get(&handle)
            .is_some_and(|pending| pending.id() == id)
        {
            return None;
        }
        self.pending_media_load_sequences.remove(&handle)
    }

    pub(crate) fn pending_text_track_load_sequence(
        &self,
        handle: DomHandle,
    ) -> Option<super::PendingTextTrackLoadSequence> {
        self.pending_text_track_load_sequences.get(&handle).cloned()
    }

    pub(crate) fn next_text_track_load_sequence_id(&mut self) -> super::TextTrackLoadSequenceId {
        let id = super::TextTrackLoadSequenceId::new(self.next_text_track_load_sequence_id);
        self.next_text_track_load_sequence_id =
            self.next_text_track_load_sequence_id.wrapping_add(1).max(1);
        id
    }

    pub(crate) fn take_pending_text_track_load_sequence_if_matches(
        &mut self,
        handle: DomHandle,
        id: super::TextTrackLoadSequenceId,
    ) -> Option<super::PendingTextTrackLoadSequence> {
        if !self
            .pending_text_track_load_sequences
            .get(&handle)
            .is_some_and(|pending| pending.id() == id)
        {
            return None;
        }
        self.pending_text_track_load_sequences.remove(&handle)
    }

    pub(crate) fn register_lazy_media_load_candidate(&mut self, handle: DomHandle) {
        self.lazy_media_load_candidates.insert(handle);
    }

    pub(crate) fn remove_lazy_media_load_candidate(&mut self, handle: DomHandle) {
        self.lazy_media_load_candidates.remove(&handle);
    }

    pub(crate) fn lazy_media_load_candidates(&self) -> Vec<DomHandle> {
        self.lazy_media_load_candidates.iter().copied().collect()
    }

    pub(crate) fn begin_form_data_construction(&mut self, form_handle: DomHandle) -> bool {
        if self.constructing_form_data_forms.contains(&form_handle) {
            return false;
        }
        self.constructing_form_data_forms.push(form_handle);
        true
    }

    pub(crate) fn end_form_data_construction(&mut self, form_handle: DomHandle) {
        if let Some(index) = self
            .constructing_form_data_forms
            .iter()
            .rposition(|handle| *handle == form_handle)
        {
            self.constructing_form_data_forms.remove(index);
        }
    }

    pub(crate) fn begin_form_submission(&mut self, form_handle: DomHandle) -> bool {
        if self.active_form_submission_forms.contains(&form_handle) {
            return false;
        }
        self.active_form_submission_forms.push(form_handle);
        true
    }

    pub(crate) fn end_form_submission(&mut self, form_handle: DomHandle) {
        if let Some(index) = self
            .active_form_submission_forms
            .iter()
            .rposition(|handle| *handle == form_handle)
        {
            self.active_form_submission_forms.remove(index);
        }
    }

    pub(crate) fn observers_mut(
        &mut self,
        _access: &ObserverStoreAccessToken,
    ) -> &mut ObserverStore {
        &mut self.observers
    }

    pub(crate) fn custom_element_definition_for_constructor(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        constructor: v8::Local<'_, v8::Function>,
    ) -> Option<(CustomElementRegistryKey, String, Option<String>)> {
        if let Some((name, extends_local_name)) = self
            .custom_elements
            .definition_for_constructor(scope, constructor)
        {
            return Some((CustomElementRegistryKey::Global, name, extends_local_name));
        }
        if let Some((child_handle, name, extends_local_name)) = self
            .child_custom_elements
            .iter()
            .find_map(|(child_handle, store)| {
                store
                    .definition_for_constructor(scope, constructor)
                    .map(|(name, extends_local_name)| (*child_handle, name, extends_local_name))
            })
        {
            return Some((
                CustomElementRegistryKey::Child(child_handle),
                name,
                extends_local_name,
            ));
        }
        if let Some((scoped_id, name, extends_local_name)) = self
            .scoped_custom_elements
            .iter()
            .find_map(|(scoped_id, store)| {
                store
                    .definition_for_constructor(scope, constructor)
                    .map(|(name, extends_local_name)| (*scoped_id, name, extends_local_name))
            })
        {
            return Some((
                CustomElementRegistryKey::Scoped(scoped_id),
                name,
                extends_local_name,
            ));
        }
        None
    }

    pub(crate) fn create_scoped_custom_elements_registry(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        registry: v8::Local<'_, v8::Object>,
    ) -> u64 {
        let id = self.next_scoped_custom_elements_registry_id;
        self.next_scoped_custom_elements_registry_id = self
            .next_scoped_custom_elements_registry_id
            .checked_add(1)
            .expect("scoped custom element registry id overflowed");
        self.scoped_custom_element_registry_wrappers
            .insert(id, v8::Weak::new(scope, registry));
        if id.is_multiple_of(64) {
            self.compact_scoped_custom_element_registry_wrappers();
        }
        id
    }

    fn compact_scoped_custom_element_registry_wrappers(&mut self) {
        self.scoped_custom_element_registry_wrappers
            .retain(|_, registry| !registry.is_empty());
        let wrappers = &self.scoped_custom_element_registry_wrappers;
        self.scoped_custom_elements
            .retain(|id, _| wrappers.contains_key(id));
    }

    fn remove_scoped_custom_element_registry(&mut self, id: u64) {
        self.scoped_custom_element_registry_wrappers.remove(&id);
        self.scoped_custom_elements.remove(&id);
    }

    pub(crate) fn set_custom_element_registry_association(
        &mut self,
        handle: DomHandle,
        association: CustomElementRegistryAssociation,
    ) {
        match self
            .custom_element_registry_associations
            .get(&handle)
            .copied()
        {
            Some(current) if current == association => return,
            Some(_) => {
                self.custom_element_registry_associations
                    .shift_remove(&handle);
            }
            None => {}
        }
        self.custom_element_registry_associations
            .insert(handle, association);
    }

    pub(crate) fn apply_custom_element_registry_association_retargets(
        &mut self,
        retargets: &[RegistryAssociationRetarget],
    ) {
        if retargets.is_empty() {
            return;
        }

        // Re-association is observable even when the registry itself does not
        // change: adopting a scoped-registry tree into another document moves
        // that document to the end of the registry's association order. Build
        // the final touched-handle order once so a large adopted subtree does
        // not repeatedly shift the IndexMap.
        let mut touched = HashSet::with_capacity(retargets.len());
        let mut ordered_retargets = Vec::with_capacity(retargets.len());
        for retarget in retargets.iter().rev() {
            if touched.insert(retarget.handle) {
                ordered_retargets.push(*retarget);
            }
        }
        ordered_retargets.reverse();

        self.custom_element_registry_associations
            .retain(|handle, _| !touched.contains(handle));
        for retarget in ordered_retargets {
            self.custom_element_registry_associations
                .insert(retarget.handle, retarget.association);
        }
    }

    pub(crate) fn custom_element_registry_association(
        &self,
        handle: DomHandle,
    ) -> Option<CustomElementRegistryAssociation> {
        self.custom_element_registry_associations
            .get(&handle)
            .copied()
    }

    pub(crate) fn custom_element_registry_associations_in_order(
        &self,
    ) -> impl Iterator<Item = (DomHandle, CustomElementRegistryAssociation)> + '_ {
        self.custom_element_registry_associations
            .iter()
            .map(|(handle, association)| (*handle, *association))
    }

    pub(crate) fn clear_custom_element_registry_associations_for_child_context(
        &mut self,
        child_handle: DomHandle,
    ) {
        let document_handle = self
            .child_browsing_context_document_handles
            .get(&child_handle)
            .copied();
        let stale_handles = self
            .custom_element_registry_associations
            .iter()
            .filter_map(|(associated_handle, association)| {
                let registry_matches_child = matches!(
                    association,
                    CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Child(
                        handle
                    )) if *handle == child_handle
                );
                let document_matches_child = document_handle.is_some_and(|document_handle| {
                    *associated_handle == document_handle
                        || self.dom_host().owner_document_handle(*associated_handle)
                            == Some(document_handle)
                });
                if *associated_handle == child_handle
                    || registry_matches_child
                    || document_matches_child
                {
                    Some(*associated_handle)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for handle in stale_handles {
            self.custom_element_registry_associations
                .shift_remove(&handle);
        }
    }

    pub(crate) fn clear_custom_element_registry_associations_for_document(
        &mut self,
        document_handle: DomHandle,
    ) {
        let stale_handles = self
            .custom_element_registry_associations
            .keys()
            .copied()
            .filter(|associated_handle| {
                *associated_handle == document_handle
                    || self.dom_host().owner_document_handle(*associated_handle)
                        == Some(document_handle)
            })
            .collect::<Vec<_>>();
        for handle in stale_handles {
            self.custom_element_registry_associations
                .shift_remove(&handle);
        }
    }

    pub(crate) fn effective_custom_element_registry_association(
        &self,
        handle: DomHandle,
    ) -> CustomElementRegistryAssociation {
        self.custom_element_registry_association(handle)
            .map(|association| {
                self.normalize_explicit_custom_element_registry_association(handle, association)
            })
            .unwrap_or_else(|| self.inferred_custom_element_registry_association(handle))
    }

    pub(crate) fn should_serialize_shadow_root_registry_attribute(
        &self,
        shadow_root: DomHandle,
    ) -> bool {
        let document_registry = self
            .dom_host()
            .owner_document_handle(shadow_root)
            .map(|document| self.effective_custom_element_registry_association(document))
            .unwrap_or(CustomElementRegistryAssociation::Null);
        let shadow_registry = self.effective_custom_element_registry_association(shadow_root);
        match (document_registry, shadow_registry) {
            (CustomElementRegistryAssociation::Null, CustomElementRegistryAssociation::Null) => {
                false
            }
            (
                CustomElementRegistryAssociation::Registry(document_key),
                CustomElementRegistryAssociation::Registry(shadow_key),
            ) if document_key == shadow_key && document_key.is_document_default_backed() => false,
            _ => true,
        }
    }

    fn inferred_custom_element_registry_association(
        &self,
        handle: DomHandle,
    ) -> CustomElementRegistryAssociation {
        if let Some(association) = self.inherited_custom_element_registry_association(handle) {
            return association;
        }
        let Some(document_handle) = self.dom_host().owner_document_handle(handle) else {
            return CustomElementRegistryAssociation::Null;
        };
        if let Some(child_handle) =
            self.child_browsing_context_host_for_document_handle(document_handle)
        {
            return CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Child(
                child_handle,
            ));
        }
        if document_handle == self.dom_host().document_handle() {
            return CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Global);
        }
        CustomElementRegistryAssociation::Null
    }

    fn normalize_explicit_custom_element_registry_association(
        &self,
        handle: DomHandle,
        association: CustomElementRegistryAssociation,
    ) -> CustomElementRegistryAssociation {
        match association {
            CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Global) => {}
            CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Child(_)) => {
                return association;
            }
            CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Scoped(_))
            | CustomElementRegistryAssociation::Null => return association,
        }
        let Some(document_handle) = self.dom_host().owner_document_handle(handle) else {
            return association;
        };
        let owner_default =
            self.default_custom_element_registry_association_for_document(document_handle);
        if owner_default
            == CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Global)
        {
            association
        } else {
            owner_default
        }
    }

    pub(crate) fn default_custom_element_registry_association_for_document(
        &self,
        document_handle: DomHandle,
    ) -> CustomElementRegistryAssociation {
        if let Some(child_handle) =
            self.child_browsing_context_host_for_document_handle(document_handle)
        {
            return CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Child(
                child_handle,
            ));
        }
        if document_handle == self.dom_host().document_handle() {
            return CustomElementRegistryAssociation::Registry(CustomElementRegistryKey::Global);
        }
        CustomElementRegistryAssociation::Null
    }

    fn inherited_custom_element_registry_association(
        &self,
        handle: DomHandle,
    ) -> Option<CustomElementRegistryAssociation> {
        let mut current = Some(handle);
        while let Some(candidate) = current {
            if candidate != handle
                && let Some(association) = self.custom_element_registry_association(candidate)
            {
                return Some(association);
            }
            if self
                .dom_host()
                .shadow_root_uses_null_custom_element_registry(candidate)
                .unwrap_or(false)
            {
                return Some(CustomElementRegistryAssociation::Null);
            }
            current = self
                .dom_host()
                .node(candidate)
                .and_then(|node| node.parent_node())
                .or_else(|| self.dom_host().shadow_root_host(candidate));
        }
        None
    }

    pub(crate) fn custom_element_registry_value_for_handle<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Value>> {
        match self.effective_custom_element_registry_association(handle) {
            CustomElementRegistryAssociation::Null => Some(v8::null(scope).into()),
            CustomElementRegistryAssociation::Registry(key) => self
                .custom_element_registry_object_for_key(scope, key)
                .map(Into::into),
        }
    }

    pub(crate) fn custom_element_registry_object_for_key<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        key: CustomElementRegistryKey,
    ) -> Option<v8::Local<'s, v8::Object>> {
        match key {
            CustomElementRegistryKey::Global => {
                let global = scope.get_current_context().global(scope);
                global
                    .get(scope, crate::util::v8str(scope, "customElements").into())
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            }
            CustomElementRegistryKey::Child(handle) => {
                let window = self
                    .child_browsing_context_window_wrapper(scope, handle)
                    .or_else(|| self.cached_detached_iframe_content_window(scope, handle))?;
                window
                    .get(scope, crate::util::v8str(scope, "customElements").into())
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            }
            CustomElementRegistryKey::Scoped(id) => {
                let registry = self
                    .scoped_custom_element_registry_wrappers
                    .get(&id)
                    .and_then(|registry| registry.to_local(scope));
                if registry.is_none() {
                    self.remove_scoped_custom_element_registry(id);
                }
                registry
            }
        }
    }

    pub(crate) fn child_browsing_context_constructor_prototype<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        child_handle: DomHandle,
        prototype_name: &str,
    ) -> Option<v8::Local<'s, v8::Value>> {
        let window = self.existing_child_browsing_context_window_wrapper(scope, child_handle)?;
        let constructor =
            window.get(scope, crate::util::v8_string(scope, prototype_name)?.into())?;
        let constructor = v8::Local::<v8::Object>::try_from(constructor).ok()?;
        constructor.get(scope, crate::util::v8str(scope, "prototype").into())
    }

    pub(crate) fn custom_elements_for_registry_key(
        &self,
        key: CustomElementRegistryKey,
    ) -> Option<&CustomElementStore> {
        match key {
            CustomElementRegistryKey::Global => Some(&self.custom_elements),
            CustomElementRegistryKey::Child(handle) => self.child_custom_elements.get(&handle),
            CustomElementRegistryKey::Scoped(id) => self.scoped_custom_elements.get(&id),
        }
    }

    pub(crate) fn custom_elements_mut_for_registry_key(
        &mut self,
        key: CustomElementRegistryKey,
    ) -> &mut CustomElementStore {
        match key {
            CustomElementRegistryKey::Global => &mut self.custom_elements,
            CustomElementRegistryKey::Child(handle) => {
                self.child_custom_elements.entry(handle).or_default()
            }
            CustomElementRegistryKey::Scoped(id) => {
                self.scoped_custom_elements.entry(id).or_default()
            }
        }
    }

    pub(crate) fn custom_element_reactions_mut(&mut self) -> &mut CustomElementReactionCoordinator {
        &mut self.custom_element_reactions
    }

    pub(crate) fn custom_element_reactions(&self) -> &CustomElementReactionCoordinator {
        &self.custom_element_reactions
    }

    pub(crate) fn enter_parser_pause(&mut self) -> crate::document_runtime::ParserPauseGuard {
        unsafe { &mut *self.runtime }.enter_parser_pause()
    }

    pub(crate) fn should_checkpoint_before_parser_custom_element_constructor(&self) -> bool {
        unsafe { &*self.runtime }.should_checkpoint_before_parser_custom_element_constructor()
    }

    pub(crate) fn custom_elements_for_node_handle(
        &self,
        handle: DomHandle,
    ) -> Option<&CustomElementStore> {
        if self.custom_elements.owns_custom_element_handle(handle) {
            return Some(&self.custom_elements);
        }
        if let Some((_, store)) = self
            .child_custom_elements
            .iter()
            .find(|(_, store)| store.owns_custom_element_handle(handle))
        {
            return Some(store);
        }
        if let Some((_, store)) = self
            .scoped_custom_elements
            .iter()
            .find(|(_, store)| store.owns_custom_element_handle(handle))
        {
            return Some(store);
        }
        match self.effective_custom_element_registry_association(handle) {
            CustomElementRegistryAssociation::Null => None,
            CustomElementRegistryAssociation::Registry(key) => {
                self.custom_elements_for_registry_key(key)
            }
        }
    }

    pub(crate) fn custom_element_registry_key_for_owned_handle(
        &self,
        handle: DomHandle,
    ) -> Option<CustomElementRegistryKey> {
        if self.custom_elements.owns_custom_element_handle(handle) {
            return Some(CustomElementRegistryKey::Global);
        }
        if let Some(child_handle) =
            self.child_custom_elements
                .iter()
                .find_map(|(child_handle, store)| {
                    store
                        .owns_custom_element_handle(handle)
                        .then_some(*child_handle)
                })
        {
            return Some(CustomElementRegistryKey::Child(child_handle));
        }
        if let Some(scoped_id) =
            self.scoped_custom_elements
                .iter()
                .find_map(|(scoped_id, store)| {
                    store
                        .owns_custom_element_handle(handle)
                        .then_some(*scoped_id)
                })
        {
            return Some(CustomElementRegistryKey::Scoped(scoped_id));
        }
        match self.effective_custom_element_registry_association(handle) {
            CustomElementRegistryAssociation::Null => None,
            CustomElementRegistryAssociation::Registry(key) => Some(key),
        }
    }

    pub(crate) fn custom_element_handle_is_upgraded(&self, handle: DomHandle) -> bool {
        self.custom_elements_for_node_handle(handle)
            .is_some_and(|store| store.is_upgraded_handle(handle))
    }

    pub(crate) fn custom_elements_mut_for_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> &mut CustomElementStore {
        if self.custom_elements.owns_custom_element_handle(handle) {
            return &mut self.custom_elements;
        }
        if let Some(child_handle) =
            self.child_custom_elements
                .iter()
                .find_map(|(child_handle, store)| {
                    store
                        .owns_custom_element_handle(handle)
                        .then_some(*child_handle)
                })
        {
            return self
                .child_custom_elements
                .get_mut(&child_handle)
                .expect("child custom element store disappeared");
        }
        if let Some(scoped_id) =
            self.scoped_custom_elements
                .iter()
                .find_map(|(scoped_id, store)| {
                    store
                        .owns_custom_element_handle(handle)
                        .then_some(*scoped_id)
                })
        {
            return self
                .scoped_custom_elements
                .get_mut(&scoped_id)
                .expect("scoped custom element store disappeared");
        }
        match self.effective_custom_element_registry_association(handle) {
            CustomElementRegistryAssociation::Null => &mut self.custom_elements,
            CustomElementRegistryAssociation::Registry(key) => {
                self.custom_elements_mut_for_registry_key(key)
            }
        }
    }

    pub(crate) fn custom_elements_subtree_lifecycle_quiescent(&self) -> bool {
        self.custom_elements.is_subtree_lifecycle_quiescent()
            && self
                .child_custom_elements
                .values()
                .all(CustomElementStore::is_subtree_lifecycle_quiescent)
            && self
                .scoped_custom_elements
                .values()
                .all(CustomElementStore::is_subtree_lifecycle_quiescent)
    }

    pub(crate) fn note_parser_defined_autonomous_custom_element(&mut self, name: &str) {
        if self
            .parser_defined_autonomous_custom_elements
            .iter()
            .any(|existing| existing == name)
        {
            return;
        }
        self.parser_defined_autonomous_custom_elements
            .push_back(name.to_owned());
    }

    pub(crate) fn drain_parser_defined_autonomous_custom_elements(&mut self) -> Vec<String> {
        self.parser_defined_autonomous_custom_elements
            .drain(..)
            .collect()
    }

    pub(crate) fn note_parser_custom_element_handoff_replacement(
        &mut self,
        placeholder: DomHandle,
        constructed: DomHandle,
    ) {
        self.parser_custom_element_handoff_replacements
            .insert(placeholder, constructed);
    }

    pub(crate) fn parser_custom_element_handoff_replacements_snapshot(
        &self,
    ) -> Vec<(DomHandle, DomHandle)> {
        self.parser_custom_element_handoff_replacements
            .iter()
            .map(|(placeholder, constructed)| (*placeholder, *constructed))
            .collect()
    }

    pub(crate) fn compact_parser_custom_element_handoff_replacements(&mut self) {
        let stale = self
            .parser_custom_element_handoff_replacements
            .iter()
            .filter_map(|(placeholder, constructed)| {
                let placeholder_alive = self.dom_host().node(*placeholder).is_some();
                let constructed_alive = self.dom_host().node(*constructed).is_some();
                (!placeholder_alive || !constructed_alive).then_some(*placeholder)
            })
            .collect::<Vec<_>>();
        for placeholder in stale {
            self.parser_custom_element_handoff_replacements
                .remove(&placeholder);
        }
    }

    pub(crate) fn take_pending_custom_element_wrapper_for<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        new_target: v8::Local<'_, v8::Function>,
    ) -> Option<crate::custom_elements::PendingCustomElementConstruction<'s>> {
        self.custom_elements
            .take_pending_wrapper_for(scope, new_target)
            .or_else(|| {
                self.child_custom_elements
                    .values_mut()
                    .find_map(|store| store.take_pending_wrapper_for(scope, new_target))
            })
            .or_else(|| {
                self.scoped_custom_elements
                    .values_mut()
                    .find_map(|store| store.take_pending_wrapper_for(scope, new_target))
            })
    }

    pub(crate) fn has_pending_custom_element_construction_for(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        new_target: v8::Local<'_, v8::Function>,
    ) -> bool {
        if self
            .custom_elements
            .has_pending_wrapper_for(scope, new_target)
        {
            return true;
        }
        for store in self.child_custom_elements.values() {
            if store.has_pending_wrapper_for(scope, new_target) {
                return true;
            }
        }
        for store in self.scoped_custom_elements.values() {
            if store.has_pending_wrapper_for(scope, new_target) {
                return true;
            }
        }
        false
    }

    pub(crate) fn text_codecs_mut(&mut self) -> &mut TextCodecStore {
        &mut self.text_codecs
    }

    pub(crate) fn resolve_node_wrapper_handle(
        &self,
        reflector_id: ReflectorId,
    ) -> Option<DomHandle> {
        self.bridge.resolve_node_handle(reflector_id)
    }

    pub(crate) fn renderer_backend_node_id_for_live_handle(
        &mut self,
        handle: DomHandle,
    ) -> Option<u32> {
        let identity_handle = self.dom_host().document_identity_handle(handle)?;
        let document_id = self.document_id_for_backend_node_identity_handle(identity_handle)?;
        Some(
            self.backend_node_registry
                .borrow_mut()
                .id_for_node(document_id, handle),
        )
    }

    pub(crate) fn renderer_dom_agent_state(&self) -> crate::runtime::RendererDomAgentState {
        self.dom_agent_state.clone()
    }

    pub(crate) fn document_id_for_backend_node_identity_handle(
        &self,
        handle: DomHandle,
    ) -> Option<crate::frame_owner_model::DocumentId> {
        let document_handle = self.dom_host().owner_document_handle(handle)?;
        if document_handle == self.dom_host().document_handle() {
            return self
                .current_main_document_task_owner()
                .map(|owner| owner.document_id);
        }
        let child_handle = self.child_browsing_context_host_for_document_handle(document_handle)?;
        self.frame_owner_current_child_snapshot(child_handle)
            .map(|snapshot| snapshot.document_id)
    }
}
