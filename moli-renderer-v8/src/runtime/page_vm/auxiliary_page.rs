use super::*;

impl PageVm {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime) fn new_related_initial_empty_in_scope(
        scope: &mut v8::PinScope<'_, '_>,
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        mut hooks: PageVmRuntimeHooks,
        dom_host: DomHost,
        inherited_origin: String,
        inherited_security_token: &v8::Global<v8::Value>,
    ) -> Result<Self> {
        let started = Instant::now();
        let (lifecycle, identity) = hooks.install_document_lifecycle(page_id)?;
        let environment = hooks
            .renderer_page_script_environment
            .as_ref()
            .ok_or_else(|| anyhow!("related Page has no script environment"))?
            .clone();
        lifecycle.bind_output_journal(environment.output_journal());
        let source = environment.page_runtime_task_source();
        let queue = PageTaskQueue::new_with_page_runtime_task_source(source.clone());
        let PageVmRendererDocumentIsolateBootstrap {
            renderer_document_isolate_bootstrap,
        } = hooks.create_renderer_document_isolate_bootstrap(source.clone())?;
        let (capabilities, runtime, completion, parser, stylesheet, worker) = source
            .bound_task_producer_senders(identity.document)
            .ok_or_else(|| anyhow!("related Page has no typed producer routes"))?
            .into_parts();
        let runtime_sender = queue.owner_attached_post_domcontentloaded_runtime_page_task_sender(
            runtime, parser, stylesheet, worker,
        );
        let completion =
            RendererResourceCompletionSender::for_page_scheduler(completion, identity.document);
        let runner = hooks
            .resource_task_runner
            .clone()
            .ok_or_else(|| anyhow!("related Page has no resource task runner"))?;
        let loader_bootstrap =
            crate::network::context::DocumentResourceLoaderBootstrap::new(loader.clone(), runner);
        let mut vm = ScriptVmDefaultWorldBootstrap::related_page_in_scope(
            scope,
            dom_host,
            runtime_sender,
            queue.parser_boundary_sender(),
            completion,
            loader_bootstrap,
            hooks.browser_context_runtime.clone(),
            hooks.javascript_dialog_runtime.clone(),
            renderer_document_isolate_bootstrap,
            new_shared_renderer_backend_node_registry(),
            env,
            inherited_origin,
            inherited_security_token,
        )
        .map_err(|error| error.0)?;
        vm.set_layout_policy(env.layout_configuration.policy);
        vm.set_scrollbars_hidden(
            env.layout_configuration
                .scrollbars_hidden_for(env.document_settings.scrollbars_hidden),
        );
        vm.install_page_task_capabilities(capabilities);
        vm.set_root_document_lifecycle(lifecycle.clone());
        vm.set_document_ready_state(crate::dom::native::DocumentReadyState::Complete)?;
        for milestone in [
            RendererDocumentLifecycleMilestone::DomContentLoaded,
            RendererDocumentLifecycleMilestone::Load,
        ] {
            ensure!(
                matches!(
                    lifecycle.begin_milestone_dispatch(identity, milestone),
                    RendererDocumentLifecycleTransition::DispatchStarted
                ),
                "related initial Document rejected {milestone:?} dispatch"
            );
            ensure!(
                matches!(
                    lifecycle.complete_milestone_dispatch(identity, milestone),
                    RendererDocumentLifecycleTransition::Recorded(_)
                ),
                "related initial Document rejected {milestone:?} completion"
            );
        }
        let mut page = Self::finish_construction(
            page_id,
            local_executor,
            env,
            hooks,
            lifecycle,
            queue,
            vm,
            started,
        );
        page.apply_environment_config(env, AuxiliaryEnvironmentApply::InitialRealm)?;
        Ok(page)
    }

    pub(in crate::runtime) fn adopt_initial_auxiliary_page(
        &mut self,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
    ) -> Result<()> {
        // The initial Document keeps its inherited origin and policy, while
        // future requests use the actual target's network configuration.
        self.request_client = self
            .vm_mut()
            .replace_document_resource_runtime_with_navigator_identity(
                loader,
                &env.navigator_identity,
            )
            .request_client()
            .clone();
        self.vm_mut().materialize_related_page_inspector_context(
            env.root_frame_id.clone(),
            env.main_document_commit.clone(),
            &env.runtime_inspector_session_restore_snapshots,
        )?;
        self.apply_environment_config(env, AuxiliaryEnvironmentApply::Adoption)?;
        self.navigator_identity = env.navigator_identity.clone();
        self.runtime_isolated_worlds = env.runtime_isolated_worlds.clone();
        self.permission_overrides = env.permission_overrides.clone();
        self.document_start_scripts = env.document_start_scripts.clone();
        self.runtime_bindings = env.runtime_bindings.clone();
        self.runtime_inspector_protocol_configurations = env.inspector_protocol_configurations();
        self.extra_http_headers = env.extra_http_headers.clone();
        self.document_settings = env.document_settings.clone();
        self.network_offline = env.network_offline;
        self.blocked_url_patterns = env.blocked_url_patterns.clone();
        self.indexed_db_manager = env.indexed_db_manager.clone();
        self.storage_bucket_store = env.storage_bucket_store.clone();
        self.fetch_subresource_interception_enabled = env.fetch_subresource_interception_enabled;
        self.fetch_subresource_interception_resource_type =
            env.fetch_subresource_interception_resource_type;
        self.layout_configuration = env.layout_configuration;
        self.wpt_extensions_enabled = env.wpt_extensions_enabled;
        self.vm_mut()
            .set_layout_policy(env.layout_configuration.policy);
        self.vm_mut()
            .set_wpt_extensions_enabled(env.wpt_extensions_enabled)?;
        self.restore_runtime_inspector_sessions_on_named_owner_lane(
            &env.runtime_inspector_session_restore_snapshots,
        )?;
        self.install_stored_runtime_isolated_worlds_on_named_owner_lane()?;
        self.install_stored_runtime_bindings_on_named_owner_lane()?;
        self.run_document_start_scripts_on_named_owner_lane(&env.document_start_scripts, |_| {})?;
        Ok(())
    }
}
