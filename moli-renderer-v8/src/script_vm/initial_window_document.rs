use super::*;

impl ScriptVm {
    pub(crate) fn initial_window_transition_for_commit(
        &self,
        origin: &str,
        policy: &crate::document_runtime::DocumentPolicyContainer,
        inherited: Option<&ScriptVmInitialDocumentEnvironment>,
    ) -> Result<crate::frame_owner_model::FrameDocumentLocalWindowTransition> {
        // Serialized opaque origins are all "null". Compare the inherited
        // identity instead, so a shared opaque origin can reuse its Window
        // without treating independently sandboxed documents as same-origin.
        let inherited_opaque_origin_matches = if origin == "null"
            && let Some(token) =
                inherited.and_then(|environment| environment.security_token.as_ref())
        {
            self.renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = v8::Local::new(scope, &self.page_default_runtime.context);
                    let token = v8::Local::new(scope, token);
                    Ok(context.get_security_token(scope).strict_equals(token))
                })?
        } else {
            false
        };
        let inherited_domain =
            inherited.map(|environment| environment.document_domain_override.borrow());
        Ok(self
            ._context_host
            .borrow()
            .main_document_local_window_transition_for_commit(
                origin,
                inherited_opaque_origin_matches,
                inherited_domain
                    .as_ref()
                    .and_then(|domain| domain.as_deref()),
                policy,
            ))
    }

    /// Replace the initial Document while retaining this Window's actual V8
    /// Context, native host and DOM handles. No globals or intrinsics are copied.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_initial_empty_document_in_current_window(
        &mut self,
        bootstrap_document: &DomHost,
        page_task_tx: RuntimePageTaskSender,
        parser_boundary: tokio::sync::mpsc::UnboundedSender<PageTask>,
        resource_completion: RendererResourceCompletionSender,
        loader_bootstrap: crate::network::context::DocumentResourceLoaderBootstrap,
        env: &crate::runtime::PageVmEnvConfig,
        bootstrap: RendererDocumentIsolateBootstrap,
    ) -> Result<()> {
        let document = bootstrap_document
            .dom()
            .document()
            .ok_or_else(|| anyhow!("new main Document is missing"))?;
        let document_url = bootstrap_document
            .document_url()
            .cloned()
            .ok_or_else(|| anyhow!("new main Document URL is missing"))?;
        let transition = self
            ._context_host
            .borrow_mut()
            .prepare_initial_main_document_owner_transition()
            .ok_or_else(|| anyhow!("initial main Document has no current owner"))?;
        self.commit_main_document_with_replaced_inspector(
            transition,
            bootstrap,
            env,
            &document_url,
            |vm, inherited| {
                let mut dom_host = vm.document_runtime.dom_host().clone();
                let document_handle = dom_host.replace_main_document(document);
                if let Some(base) = inherited
                    .as_ref()
                    .and_then(|environment| environment.fallback_base_url.clone())
                {
                    dom_host.set_document_fallback_base_url_for_handle(document_handle, Some(base));
                }
                let origin = inherited
                    .as_ref()
                    .map(|environment| environment.origin.clone())
                    .unwrap_or_else(|| moli_url::origin_ascii_serialization(&document_url));
                let base_url = dom_host
                    .document_base_url_for_handle(document_handle)
                    .unwrap_or_else(|| document_url.clone());
                vm._context_host
                    .borrow_mut()
                    .commit_initial_main_document_in_current_window(
                        transition,
                        document_handle,
                        document_url.clone(),
                    )
                    .ok_or_else(|| anyhow!("initial main Document changed during commit"))?;
                let author_styles_disabled = loader_bootstrap.author_styles_disabled();
                let mut runtime = DocumentRuntime::from_main_frame_dom_host(
                    dom_host,
                    transition.current_owner(),
                    Some(page_task_tx.page_task_sender()),
                    parser_boundary,
                    page_task_tx.stylesheet_task_sender(),
                    page_task_tx.main_parser_continuation_sender(),
                );
                runtime.set_author_styles_disabled(author_styles_disabled);
                runtime.set_bypass_content_security_policy(
                    env.document_settings.bypass_content_security_policy,
                );
                runtime.retain_event_targets_from_initial_document(&mut vm.document_runtime);
                // JsContextHost and retained native wrappers point into this allocation.
                // Replace its contents while keeping the allocation's address stable.
                *vm.document_runtime = runtime;
                vm.post_domcontentloaded_page_task_tx = page_task_tx.page_task_sender();
                vm.page_runtime_wake_tx = page_task_tx.page_runtime_wake_sender();
                {
                    let mut host = vm._context_host.borrow_mut();
                    if let Some(environment) = inherited.as_ref() {
                        host.inherit_document_domain_override(environment.document_domain_override.clone());
                    }
                    host.rebind_initial_main_document_senders(
                        resource_completion,
                        page_task_tx.top_level_navigation_handoff_sender(),
                        page_task_tx.service_worker_task_sender(),
                    );
                    host.adopt_initial_document_service_worker_client(
                        transition,
                        env.reserved_service_worker_client_id,
                    );
                    host.retire_document_resource_loader(
                        crate::native_bridge::WindowDocumentOwner::Frame(
                            transition.retired_owner(),
                        ),
                    );
                    let loader = loader_bootstrap.commit(
                        crate::network::context::DocumentFetchContext::new(
                            crate::native_bridge::WindowDocumentOwner::Frame(
                                transition.current_owner(),
                            ),
                            document_url.clone(),
                            base_url,
                            origin,
                        ),
                    );
                    host.register_main_document_resource_loader(&loader);
                    vm.document_runtime
                        .set_cookie_store(loader.request_client().cookie_store());
                    vm.document_runtime
                        .set_service_worker_connected_link_context(
                            host.browser_context_runtime(),
                            host.service_worker_client_id(),
                        );
                }
                let context = &vm.page_default_runtime.context;
                vm.renderer_document_isolate
                    .with_entered_renderer_document_isolate(|isolate| {
                        let scope = pin!(v8::HandleScope::new(isolate));
                        let scope = &mut scope.init();
                        let context = v8::Local::new(scope, context);
                        let scope = &mut v8::ContextScope::new(scope, context);
                        crate::context_bootstrap::sync_global_location_runtime_state(
                            scope,
                            document_url.as_str(),
                        );
                        let global = context.global(scope);
                        if let Some(document) = global
                            .get(scope, crate::util::v8str(scope, "document").into())
                            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                        {
                            crate::context_bootstrap::sync_document_location_runtime_state_from_window(
                                scope, document, global,
                            );
                        }
                        Ok(())
                    })?;
                Ok(())
            },
        )
    }
}
