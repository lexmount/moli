use super::*;

impl ScriptVmDefaultWorldBootstrap {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn related_page_in_scope(
        scope: &mut v8::PinScope<'_, '_>,
        dom_host: DomHost,
        page_task_tx: RuntimePageTaskSender,
        parser_boundary_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        resource_completion_tx: RendererResourceCompletionSender,
        loader: crate::network::context::DocumentResourceLoaderBootstrap,
        browser_context_runtime: RendererBrowserContextRuntime,
        javascript_dialog_runtime: crate::runtime::RendererJavaScriptDialogRuntime,
        isolate_bootstrap: RendererDocumentIsolateBootstrap,
        backend_node_registry: SharedRendererBackendNodeRegistry,
        env: &crate::runtime::PageVmEnvConfig,
        inherited_origin: String,
        inherited_security_token: &v8::Global<v8::Value>,
    ) -> std::result::Result<ScriptVm, ScriptVmBootstrapError> {
        let global_template = isolate_bootstrap
            .bridge_bindings
            .window_global_template(scope);
        let bootstrap = ScriptVmPageRealmBootstrap::new_from_dom_host(
            dom_host,
            env.document_settings.bypass_content_security_policy,
            page_task_tx,
            parser_boundary_tx,
            resource_completion_tx,
            loader,
            browser_context_runtime,
            javascript_dialog_runtime,
            isolate_bootstrap,
            &[],
            backend_node_registry,
            None,
            None,
            env.top_level_storage_key.clone(),
            None,
            Some(ScriptVmInitialDocumentEnvironment {
                security_token: None,
                fallback_base_url: None,
                storage_key: None,
                origin: inherited_origin.clone(),
                policy_container: env.document_policy_container.clone(),
            }),
        )?;
        // Noopener blank pages commit a navigation; only pages exposing an
        // opener retain a provisional initial Document and empty history.
        if bootstrap
            .renderer_page_script_environment
            .as_ref()
            .and_then(|environment| environment.opener_in_scope(scope))
            .is_some()
        {
            bootstrap
                .context_host
                .borrow_mut()
                .mark_main_document_initial_empty();
        }
        let bootstrap = bootstrap.default_world_in_entered_scope(scope, global_template)?;
        let context = v8::Local::new(scope, &bootstrap.page_default_context);
        // A propagated origin sandbox gives the popup a fresh opaque origin.
        // Keep its unique token; copying the opener's token would bypass SOP.
        if !env.document_policy_container.sandbox.forces_opaque_origin {
            let token = v8::Local::new(scope, inherited_security_token);
            context.set_security_token(token);
        }
        {
            let scope = &mut v8::ContextScope::new(scope, context);
            crate::context_bootstrap::set_window_navigator_identity(scope, &env.navigator_identity)
                .map_err(|error| {
                    Box::new((error, bootstrap.document_runtime.dom_host().clone()))
                })?;
            let global = context.global(scope);
            crate::context_bootstrap::set_window_origin_runtime_state(
                scope,
                global,
                &inherited_origin,
            )
            .map_err(|error| Box::new((error, bootstrap.document_runtime.dom_host().clone())))?;
        }
        let mut vm = bootstrap.finish_in_scope(scope);
        vm.set_initial_storage_backends_in_scope(
            scope,
            env.indexed_db_manager.clone(),
            env.storage_bucket_store.clone(),
        );
        Ok(vm)
    }
}

impl ScriptVmPageRealmBootstrap {
    fn default_world_in_entered_scope<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        global_template: v8::Local<'s, v8::ObjectTemplate>,
    ) -> std::result::Result<ScriptVmDefaultWorldBootstrap, ScriptVmBootstrapError> {
        let Self {
            inherited_security_token: _,
            resource_owner_id,
            promise_reject_dispatch,
            page_inspector,
            renderer_document_isolate,
            renderer_document_isolate_teardown,
            document_runtime,
            root_frame_id,
            context_host,
            prebootstrapped_child_default_contexts,
            page_context_cancel_tx,
            post_domcontentloaded_page_task_tx,
            page_runtime_wake_tx,
            storage_bucket_store,
            renderer_page_script_environment,
            reuse_main_window_proxy,
        } = self;
        assert!(
            !reuse_main_window_proxy,
            "a synchronous popup creates its initial WindowProxy"
        );
        let environment = renderer_page_script_environment
            .clone()
            .expect("a related Page must retain its exact script environment");
        let context_bootstrap = match ScriptVmContextBootstrap::new_main_default_in_scope(
            scope,
            global_template,
            context_host.clone(),
            resource_owner_id,
            &promise_reject_dispatch,
            storage_bucket_store.clone(),
            environment,
        ) {
            Ok(context) => context,
            Err(error) => {
                return Err(Box::new((
                    error,
                    recover_bootstrap_dom_host_from_holder(
                        renderer_document_isolate,
                        renderer_document_isolate_teardown,
                        context_host,
                        document_runtime,
                    ),
                )));
            }
        };
        let runtime_observable_context_token = context_bootstrap.runtime_observable_context_token;
        let (context, bridge_ref) = context_bootstrap.into_context_and_bridge_ref();
        let local_context = v8::Local::new(scope, &context);
        context_host
            .borrow_mut()
            .install_page_default_context(scope, local_context);
        let scope = &mut v8::ContextScope::new(scope, local_context);
        crate::context_bootstrap::initialize_main_session_history(scope);
        let baseline = (|| {
            let baseline = ScriptVmDefaultWorldBootstrap::capture_baseline_globals_in_scope(scope)?;
            let mut host = context_host.borrow_mut();
            let binding = host
                .current_window_execution_context_binding(
                    scope,
                    crate::native_bridge::OwnerDispatchScope::Top,
                )
                .ok_or_else(|| {
                    anyhow!("related Page LocalWindow execution context is unavailable")
                })?;
            host.register_window_execution_context(binding);
            Ok(baseline)
        })();
        let baseline_globals = match baseline {
            Ok(baseline) => baseline,
            Err(error) => {
                drop(page_inspector);
                drop(bridge_ref);
                drop(context);
                drop(promise_reject_dispatch);
                return Err(Box::new((
                    error,
                    recover_bootstrap_dom_host_from_holder(
                        renderer_document_isolate,
                        renderer_document_isolate_teardown,
                        context_host,
                        document_runtime,
                    ),
                )));
            }
        };
        Ok(ScriptVmDefaultWorldBootstrap {
            resource_owner_id,
            promise_reject_dispatch,
            page_inspector,
            renderer_document_isolate,
            renderer_document_isolate_teardown,
            page_default_context: context,
            bridge_ref,
            runtime_observable_context_token,
            baseline_globals,
            document_runtime,
            root_frame_id,
            context_host,
            prebootstrapped_child_default_contexts,
            page_context_cancel_tx,
            post_domcontentloaded_page_task_tx,
            page_runtime_wake_tx,
            storage_bucket_store,
            renderer_page_script_environment,
        })
    }
}

impl ScriptVm {
    fn set_initial_storage_backends_in_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
        storage: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    ) {
        let context = v8::Local::new(scope, &self.page_default_context);
        self.indexed_db_manager = manager.clone();
        self._context_host
            .borrow_mut()
            .set_indexed_db_manager(manager.clone());
        crate::context_bootstrap::set_indexed_db_manager_for_context(context, manager);
        if let Some(storage) = storage {
            self.storage_bucket_store = storage.clone();
            self._context_host
                .borrow_mut()
                .set_storage_bucket_store(storage.clone());
            crate::context_bootstrap::set_storage_bucket_store_for_context(context, Some(storage));
        }
    }

    pub(crate) fn materialize_related_page_inspector_context(
        &mut self,
        root_frame_id: Option<String>,
        main_document_commit: Option<crate::runtime::RendererMainDocumentCommit>,
        restores: &[crate::runtime::RendererInspectorSessionRestoreSnapshot],
    ) -> Result<()> {
        let environment = self
            .renderer_page_script_environment
            .as_ref()
            .ok_or_else(|| anyhow!("related Page lost its script environment"))?
            .clone();
        let journal = environment.output_journal();
        let deferred_records = journal.take_unpublished_records_for_initial_context_adoption()?;
        if let Some(transport) = self
            ._context_host
            .borrow()
            .browser_context_runtime()
            .renderer_output_transport_sender()
        {
            journal.bind_transport(transport);
        }
        self.root_frame_id = root_frame_id;
        let isolate = self.renderer_document_isolate.clone();
        let inspector_isolate = isolate.clone();
        let context = &self.page_default_context;
        let document_url = self.document_runtime.document_url().clone();
        let root_frame_id = self.root_frame_id.as_deref();
        let page_inspector = &mut self.page_inspector;
        isolate.with_renderer_document_isolate_and_inspector_mut(|isolate, backend| {
            page_inspector.reattach_v8_sessions(backend, restores);
            if let Some(commit) = main_document_commit {
                journal.append(crate::runtime::PendingRendererOutputRecord::observation(
                    None,
                    crate::runtime::RendererProtocolObservation::MainDocumentCommit(commit),
                ));
            }
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, context);
            let default_context = v8::Global::new(scope, context);
            let registered_context = v8::Global::new(scope, context);
            let _scope = &mut v8::ContextScope::new(scope, context);
            page_inspector.attach_context(
                inspector_isolate,
                backend,
                context,
                default_context,
                registered_context,
                &document_url,
                root_frame_id,
            );
        });
        journal.append_records(deferred_records);
        Ok(())
    }
}
