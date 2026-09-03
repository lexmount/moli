use super::*;

// This final thin slice keeps the runtime-core methods that are about owning `DocumentRuntime`
// itself rather than any specific subsystem.
//
// The methods here are intentionally tiny:
// - constructing the aggregate runtime state
// - tracking whether we are currently inside a custom-element reaction
//
// Splitting them out is mostly about finishing the file-level separation so `document_runtime.rs`
// stops carrying even a small "misc core methods" tail while the actual behavior remains unchanged.
impl DocumentRuntime {
    /// Constructs the runtime for an already-admitted main Document.
    ///
    /// The owner is part of the runtime's initial identity; production callers
    /// must not construct an ownerless runtime and attach it afterward.
    pub(crate) fn from_main_frame_dom_host(
        dom_host: DomHost,
        main_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        page_task_tx: Option<crate::page_task_queue::PageTaskSender>,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        stylesheet_task_sender: crate::page_task_queue::RendererPageStylesheetTaskSender,
        main_parser_continuation_sender: crate::page_task_queue::RendererPageMainParserContinuationSender,
    ) -> Self {
        let mut runtime = Self::from_dom_host_with_incarnation(
            dom_host,
            DocumentRuntimeIncarnationIdentity::MainFrame(main_document_owner),
            page_task_tx,
            page_task_parser_boundary_injection_tx,
            stylesheet_task_sender,
            main_parser_continuation_sender,
        );
        runtime.replace_main_document_task_capabilities(main_document_owner);
        runtime
    }

    pub(crate) fn main_frame_document_task_owner(
        &self,
    ) -> Option<crate::frame_owner_model::FrameDocumentTaskOwner> {
        match &self.document_incarnation {
            DocumentRuntimeIncarnationIdentity::MainFrame(owner) => Some(*owner),
            DocumentRuntimeIncarnationIdentity::Standalone(_) => None,
        }
    }

    fn from_dom_host_with_incarnation(
        dom_host: DomHost,
        document_incarnation: DocumentRuntimeIncarnationIdentity,
        page_task_tx: Option<crate::page_task_queue::PageTaskSender>,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        stylesheet_task_sender: crate::page_task_queue::RendererPageStylesheetTaskSender,
        main_parser_continuation_sender: crate::page_task_queue::RendererPageMainParserContinuationSender,
    ) -> Self {
        let document_url = dom_host
            .dom()
            .final_url()
            .expect("parsed native dom must retain a document url")
            .clone();
        let document = HostDocumentState::new(document_url);
        let mut dom_host = LiveRuntimeDomHost::from_dom_host(dom_host);
        let stylesheet_lifecycle = StylesheetLifecycleState::new(stylesheet_task_sender);
        // Sync the DomHost's document node readyState with HostDocumentState's
        // initial "loading". Without this, the DomHost keeps its default "complete"
        // (from NativeDom Document::new) and scripts see the wrong readyState.
        let _ = dom_host.set_document_ready_state(document.ready_state());
        let parser_boundary_lifecycle_tx = page_task_parser_boundary_injection_tx.clone();
        Self {
            dom_host,
            parser_reentry: ParserReentryState::default(),
            pending_parser_post_step_runtime_work: ParserPostStepRuntimeWork::default(),
            #[cfg(test)]
            parked_live_shadow_root_bindings: None,
            selector_engine: QueryEngine,
            selector_debug: SelectorDebugCounters::default(),
            document,
            design_mode_documents: HashSet::new(),
            script_execution_control: Default::default(),
            author_styles_disabled: false,
            bypass_content_security_policy: false,
            policy_container: DocumentPolicyContainer::default(),
            delivered_meta_content_security_policies: RefCell::new(HashMap::new()),
            processed_meta_content_security_policy_handles: RefCell::new(HashSet::new()),
            document_character_set: "UTF-8".to_owned(),
            resource_loader_binding: None,
            script_context_stack: Vec::new(),
            root_document_parser: None,
            post_parse_schedule_invalidated: false,
            stylesheet_lifecycle,
            main_parser_continuation:
                super::main_parser_continuation::MainParserContinuationState::new(
                    main_parser_continuation_sender,
                ),
            pending_stylesheet_source_css_projection_owners: Vec::new(),
            pending_connected_style_load_prime_result: ConnectedStyleLoadPrimeResult::default(),
            initial_connected_style_loads_queued: false,
            late_preload_stylesheet_handles: HashSet::new(),
            in_document_image_priority_boost_count: 0,
            parser_discovered_modulepreloads: HashSet::new(),
            modulepreload_invalid_as_link_errors: HashSet::new(),
            style_source_document_sync_pending: false,
            pending_devtools_dom_mutations: Vec::new(),
            #[cfg(test)]
            pending_runtime_binding_calls: Vec::new(),
            pending_inspector_issues: Vec::new(),
            quirks_mode_issue_reported: false,
            script_lifecycle: DocumentScriptLifecycle::with_scheduler(
                page_task_tx
                    .map(HostScriptScheduler::with_page_task_injection)
                    .unwrap_or_default(),
                parser_boundary_lifecycle_tx,
            ),
            parser_script_start_positions: HashMap::new(),
            timeouts: HostTimeoutScheduler::default(),
            events: HostEventTargetRegistry::default(),
            mutations: MutationCoordinator,
            meta_refresh_scheduler: super::meta_refresh::MetaRefreshScheduler::default(),
            custom_element_reaction_depth: 0,
            structural_mutation_depth: 0,
            dom_content_loaded_dispatched: false,
            document_incarnation,
            document_input_stream_opened: false,
            next_document_write_external_script_load_id: 0,
            document_write_script_preload_scanner: None,
            main_document_script_preloads: Default::default(),
            document_write_script_preloads: HashMap::new(),
            pending_document_write_external_script_load: None,
            pending_document_write_stylesheet_blocked_script: None,
            pending_document_write_stylesheet_parser_pause: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_document(document: NativeDom) -> Self {
        Self::from_document_with_resource_environment(document, None)
    }

    #[cfg(test)]
    fn from_document_with_resource_environment(
        document: NativeDom,
        resource_environment: Option<(
            ResourceRequestClient,
            crate::network::RendererResourceTaskRunner,
        )>,
    ) -> Self {
        let residence = crate::page_task_queue::RendererPageStylesheetTaskTestResidence::new();
        let sender = residence.sender();
        let main_parser_continuation_sender = residence.main_parser_continuation_sender();
        let parser_boundary_lifecycle_source =
            moli_owner_queue::OwnerTaskSource::<PageTask>::default();
        let parser_boundary_lifecycle_tx =
            parser_boundary_lifecycle_source.parser_boundary_sender();
        let owner = test_stylesheet_document_owner();
        let mut runtime = Self::from_dom_host_with_incarnation(
            DomHost::from_dom(document),
            DocumentRuntimeIncarnationIdentity::standalone(),
            None,
            parser_boundary_lifecycle_tx,
            sender,
            main_parser_continuation_sender,
        );
        runtime
            .script_lifecycle
            .retain_standalone_parser_boundary_lifecycle_source(parser_boundary_lifecycle_source);
        if let Some((loader, task_runner)) = resource_environment {
            runtime.set_author_styles_disabled(loader.author_styles_disabled());
            let document_url = runtime
                .dom_host
                .borrow()
                .dom()
                .final_url()
                .expect("test Document must retain its final URL")
                .clone();
            let document_handle = runtime.dom_host.borrow().document_handle();
            let base_url = runtime
                .dom_host
                .borrow()
                .document_base_url_for_handle(document_handle)
                .expect("test Document must expose its base URL");
            let document_loader = crate::network::context::DocumentResourceLoader::new(
                loader,
                task_runner,
                crate::network::context::DocumentFetchContext::new(
                    crate::native_bridge::WindowDocumentOwner::Frame(owner),
                    document_url.clone(),
                    base_url,
                    moli_url::origin_ascii_serialization(&document_url),
                ),
            );
            runtime.install_standalone_document_resource_loader(&document_loader);
        }
        runtime.stylesheet_lifecycle.task_test_residence = Some(residence);
        runtime.bind_stylesheet_task_producer(owner);
        runtime.bind_main_parser_continuation_producer(owner);
        runtime
    }

    pub(crate) fn into_dom_host(self) -> DomHost {
        let Self {
            dom_host,
            parser_reentry: _,
            pending_parser_post_step_runtime_work: _,
            #[cfg(test)]
                parked_live_shadow_root_bindings: _,
            selector_engine: _,
            selector_debug: _,
            document: _,
            design_mode_documents: _,
            script_execution_control: _,
            author_styles_disabled: _,
            bypass_content_security_policy: _,
            policy_container: _,
            delivered_meta_content_security_policies: _,
            processed_meta_content_security_policy_handles: _,
            document_character_set: _,
            resource_loader_binding: _,
            script_context_stack: _,
            root_document_parser: _,
            post_parse_schedule_invalidated: _,
            stylesheet_lifecycle: _,
            main_parser_continuation: _,
            pending_stylesheet_source_css_projection_owners: _,
            pending_connected_style_load_prime_result: _,
            initial_connected_style_loads_queued: _,
            late_preload_stylesheet_handles: _,
            in_document_image_priority_boost_count: _,
            parser_discovered_modulepreloads: _,
            modulepreload_invalid_as_link_errors: _,
            style_source_document_sync_pending: _,
            pending_devtools_dom_mutations: _,
            #[cfg(test)]
                pending_runtime_binding_calls: _,
            pending_inspector_issues: _,
            quirks_mode_issue_reported: _,
            script_lifecycle: _,
            parser_script_start_positions: _,
            timeouts: _,
            events: _,
            mutations: _,
            meta_refresh_scheduler: _,
            custom_element_reaction_depth: _,
            structural_mutation_depth: _,
            dom_content_loaded_dispatched: _,
            document_incarnation: _,
            document_input_stream_opened: _,
            next_document_write_external_script_load_id: _,
            document_write_script_preload_scanner: _,
            main_document_script_preloads: _,
            document_write_script_preloads: _,
            pending_document_write_external_script_load: _,
            pending_document_write_stylesheet_blocked_script: _,
            pending_document_write_stylesheet_parser_pause: _,
        } = self;
        dom_host.into_dom_host()
    }

    pub(crate) fn set_service_worker_connected_link_context(
        &mut self,
        browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
    ) {
        self.stylesheet_lifecycle
            .set_service_worker_connected_link_context(browser_context_runtime, client_id);
    }

    pub(crate) fn document_character_set(&self) -> &str {
        &self.document_character_set
    }

    pub(crate) fn set_script_execution_disabled(&mut self, disabled: bool) {
        self.script_execution_control.set_disabled(disabled);
    }

    pub(crate) fn script_execution_disabled(&self) -> bool {
        self.script_execution_control.is_disabled()
    }

    pub(crate) fn set_author_styles_disabled(&mut self, disabled: bool) {
        self.author_styles_disabled = disabled;
    }

    pub(crate) fn author_styles_disabled(&self) -> bool {
        self.author_styles_disabled
    }

    pub(crate) fn document_scripting_enabled(&self) -> bool {
        !self.script_execution_disabled() && self.document_sandbox_policy().allows_scripts
    }

    pub(crate) fn script_execution_control(
        &self,
    ) -> crate::script_execution_control::RendererScriptExecutionControl {
        self.script_execution_control.clone()
    }

    pub(crate) fn bind_script_execution_control(
        &mut self,
        control: crate::script_execution_control::RendererScriptExecutionControl,
    ) {
        self.script_execution_control = control;
    }

    pub(crate) fn set_document_character_set(&mut self, character_set: impl Into<String>) {
        self.document_character_set = character_set.into();
    }

    pub(crate) fn set_document_default_language(&mut self, language: Option<String>) {
        let document_handle = self.dom_host.document_handle();
        let _ = self
            .dom_host
            .set_document_default_language_for_handle(document_handle, language);
    }

    pub(crate) fn set_document_source_last_modified(&mut self, timestamp_ms: Option<f64>) {
        let document_handle = self.dom_host.document_handle();
        let _ = self
            .dom_host
            .set_document_source_last_modified_for_handle(document_handle, timestamp_ms);
    }

    pub(crate) fn take_style_source_document_sync_pending(&mut self) -> bool {
        std::mem::take(&mut self.style_source_document_sync_pending)
    }

    pub(crate) fn runtime_script_work_mut(&self) -> std::cell::RefMut<'_, RuntimeScriptWorkState> {
        self.script_lifecycle.runtime_script_work_mut()
    }

    pub(crate) fn runtime_script_work(&self) -> std::cell::Ref<'_, RuntimeScriptWorkState> {
        self.script_lifecycle.runtime_script_work()
    }

    pub(crate) fn runtime_script_work_handle(
        &self,
    ) -> crate::document_runtime::SharedRuntimeScriptWorkState {
        self.script_lifecycle.runtime_script_work_handle()
    }

    pub(crate) fn record_parser_no_execution_run(&self, run: crate::types::ScriptRun) {
        self.script_lifecycle.record_parser_no_execution_run(run);
    }

    pub(crate) fn take_parser_no_execution_runs(&self) -> Vec<crate::types::ScriptRun> {
        self.script_lifecycle.take_parser_no_execution_runs()
    }

    pub(crate) fn accept_ready_runtime_script_events(&mut self) {
        self.script_lifecycle.accept_ready_runtime_script_events();
    }

    pub(crate) fn deferred_page_tasks_mut(&mut self) -> &mut DeferredPageTaskState {
        self.script_lifecycle.deferred_page_tasks_mut()
    }

    #[cfg(test)]
    pub(crate) fn new(document: &NativeDom) -> Self {
        Self::from_document(document.clone())
    }

    #[cfg(test)]
    pub(crate) fn new_networked(document: &NativeDom, loader: &ResourceRequestClient) -> Self {
        let task_runner = crate::network::RendererResourceTaskRunner::from_current_tokio()
            .expect("networked DocumentRuntime test requires its Tokio runtime");
        Self::from_document_with_resource_environment(
            document.clone(),
            Some((loader.clone(), task_runner)),
        )
    }

    pub(crate) fn enter_custom_element_reaction(&mut self) {
        self.custom_element_reaction_depth += 1;
    }

    pub(crate) fn exit_custom_element_reaction(&mut self) {
        self.custom_element_reaction_depth = self.custom_element_reaction_depth.saturating_sub(1);
    }

    pub(crate) fn bind_document_resource_loader(
        &mut self,
        registry: crate::network::context::DocumentResourceLoaderRegistry,
        owner: crate::native_bridge::WindowDocumentOwner,
    ) {
        assert!(
            registry.get(owner).is_some(),
            "DocumentRuntime resource binding requires a registered authority"
        );
        self.resource_loader_binding = Some(DocumentResourceLoaderBinding { registry, owner });
    }

    pub(crate) fn current_document_resource_loader(
        &self,
    ) -> Option<crate::network::context::DocumentResourceLoader> {
        let binding = self.resource_loader_binding.as_ref()?;
        binding.registry.get(binding.owner)
    }

    #[cfg(test)]
    pub(crate) fn install_standalone_document_resource_loader(
        &mut self,
        loader: &crate::network::context::DocumentResourceLoader,
    ) {
        let registry = crate::network::context::DocumentResourceLoaderRegistry::default();
        let owner = loader.owner();
        registry.register(owner, loader.clone());
        self.bind_document_resource_loader(registry, owner);
    }

    pub(crate) fn note_parser_script_start_position(
        &mut self,
        handle: DomHandle,
        start_line: u64,
        start_column: u64,
    ) {
        self.parser_script_start_positions.insert(
            handle,
            ParserScriptStartPosition {
                line: start_line,
                column: start_column,
            },
        );
    }

    pub(crate) fn parser_script_start_line(&self, handle: DomHandle) -> Option<u64> {
        self.parser_script_start_position(handle)
            .map(|position| position.line)
    }

    pub(crate) fn parser_script_start_position(
        &self,
        handle: DomHandle,
    ) -> Option<ParserScriptStartPosition> {
        self.parser_script_start_positions.get(&handle).copied()
    }
}

#[cfg(test)]
pub(super) fn test_stylesheet_document_owner() -> crate::frame_owner_model::FrameDocumentTaskOwner {
    crate::frame_owner_model::FrameDocumentTaskOwner::new(
        crate::frame_owner_model::FrameSchedulerLaneId(0),
        crate::frame_owner_model::LocalWindowId(0),
        crate::frame_owner_model::DocumentId(1),
    )
}
