use super::*;

impl ScriptVm {
    pub(crate) fn refresh_browser_storage_event_registration(&self) {
        self._context_host
            .borrow_mut()
            .refresh_browser_storage_event_registration();
    }

    pub(crate) fn navigate_child_browsing_context_frame_to_url(
        &mut self,
        frame_id: &str,
        url: &str,
    ) -> Result<bool> {
        let Some(child_handle) = self
            ._context_host
            .borrow()
            .child_browsing_context_handle_by_frame_id(frame_id)
        else {
            return Ok(false);
        };
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(
                unsafe { &mut *host_ptr }.navigate_child_browsing_context_to_url(
                    scope,
                    child_handle,
                    url,
                ),
            )
        })
    }

    pub(crate) fn navigate_top_level_same_document_from_browser(
        &mut self,
        url: &str,
        replace_current: bool,
    ) -> Result<bool> {
        let url = url.to_owned();
        self.with_default_context_scope(|scope, _host_ptr| {
            Ok(
                crate::context_bootstrap::navigate_top_level_same_document_from_browser(
                    scope,
                    url,
                    replace_current,
                ),
            )
        })
    }

    pub(crate) fn set_document_ready_state(
        &mut self,
        state: crate::dom::native::DocumentReadyState,
    ) -> Result<()> {
        self.document_runtime.set_document_ready_state(state);
        Ok(())
    }

    pub(crate) fn snapshot_live_document(&self) -> NativeDom {
        self.document_runtime.snapshot_document()
    }

    /// Runs late custom-element upgrades at parser/runtime checkpoints.
    ///
    /// Parser-created custom elements whose definitions are known at token creation time are
    /// constructed through the step-scoped `ParserElementCreationConsumer` callback from
    /// `TreeSink::create_element`. This checkpoint walk is only for elements that become
    /// upgradable later, for example because a
    /// parser-blocking script registered their definition after the parser had already created the
    /// element. Fixtures like `connected_from_parser.html` and `legacy/html/slot.html` rely on
    /// those late-definition upgrades being visible before the next parser-connected script runs.
    ///
    /// This compatibility walk happens just after the parser's normal checkpoint. Because the
    /// delayed upgrade may itself run constructors or lifecycle callbacks, finish those reactions
    /// here before the following parser script. This is a named parser-algorithm checkpoint, not
    /// an implicit property of entering the default V8 context.
    pub(crate) fn upgrade_late_defined_custom_elements_after_parser_checkpoint(
        &mut self,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            let document_handle = unsafe { &*host_ptr }.dom_host().document_handle();
            let _ = custom_elements::upgrade_late_defined_connected_tree_after_parser_sync(
                scope,
                host_ptr,
                document_handle,
            );
            perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
            Ok(())
        })
    }

    // Parser operations reach ScriptVm only when they must enter the page default
    // V8 context. Runtime DOM identity stays inside DocumentRuntime's active
    // parser step.
    pub(crate) fn create_and_construct_parser_custom_element_direct_in_default_context(
        &mut self,
        construction: &moli_dom::native::ParserConstruction,
        document_handle: DomHandle,
        local_name: &str,
        namespace: &str,
        prefix: Option<&str>,
        token_attributes: &[Attribute],
        intended_parent: Option<DomHandle>,
    ) -> Result<Option<DomHandle>> {
        let context_host = self._context_host.clone();
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                // SAFETY: `context_ptr` points to `self.page_default_context`, which is kept
                // alive for the duration of this non-escaping closure while the document
                // isolate is exclusively borrowed.
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                Ok(
                    custom_elements::create_and_construct_parser_custom_element_direct_for_document(
                        scope,
                        host_ptr,
                        document_handle,
                        local_name,
                        namespace,
                        prefix,
                        token_attributes,
                        intended_parent,
                        |document_handle, local_name, namespace, prefix| {
                            document_runtime.create_parser_element_for_document_without_attributes_in_live_dom_host(
                                construction,
                                document_handle,
                                local_name,
                                namespace,
                                prefix,
                            )
                        },
                    ),
                )
            })
    }

    pub(crate) fn run_pending_parser_post_step_runtime_work_in_default_context(
        &mut self,
    ) -> Result<()> {
        // Entering V8 allocates per-thread isolate state. Most character tokens
        // have no post-step work, so check before constructing the scope.
        if !self
            .document_runtime
            .has_pending_parser_post_step_runtime_work()
        {
            return Ok(());
        }
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }.run_pending_parser_post_step_runtime_work(scope, host_ptr);
            Ok(())
        })
    }

    #[cfg(test)]
    pub(crate) fn queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(
        &mut self,
        work: crate::document_runtime::ParserPostStepRuntimeWorkForTest,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            let runtime = unsafe { &mut *host_ptr };
            runtime.queue_pending_parser_post_step_runtime_work_for_test(work);
            runtime.run_pending_parser_post_step_runtime_work(scope, host_ptr);
            Ok(())
        })
    }

    /// Rebinds the request-client view of the current committed Document.
    ///
    /// The Document authority is installed before realm bootstrap and remains
    /// stable here. Existing leases keep their captured request client; only
    /// subsequently registered loads observe this replacement transport.
    #[cfg(test)]
    pub(crate) fn replace_document_resource_runtime(
        &mut self,
        request_client: &ResourceRequestClient,
    ) -> DocumentResourceLoader {
        let navigator_identity = request_client.browser_identity().clone();
        self.replace_document_resource_runtime_with_navigator_identity(
            request_client,
            &navigator_identity,
        )
    }

    pub(crate) fn replace_document_resource_runtime_with_navigator_identity(
        &mut self,
        request_client: &ResourceRequestClient,
        navigator_identity: &moli_browser_profile::BrowserIdentityProfile,
    ) -> DocumentResourceLoader {
        let current = self
            .current_main_document_resource_loader()
            .expect("committed Document must install its resource authority before rebinding");
        let document_loader = current.with_replacement_transport(request_client.clone());
        self._context_host
            .borrow_mut()
            .replace_main_document_resource_transport(&document_loader);
        self.document_runtime
            .set_cookie_store(document_loader.request_client().cookie_store());
        let _ = self.with_default_context_scope(|scope, _| {
            set_window_navigator_identity(scope, navigator_identity)
        });
        document_loader
    }

    pub(crate) fn set_document_navigator_identity(
        &mut self,
        navigator_identity: &moli_browser_profile::BrowserIdentityProfile,
    ) {
        let _ = self.with_default_context_scope(|scope, _| {
            set_window_navigator_identity(scope, navigator_identity)
        });
    }

    pub(crate) fn set_web_storage_handles(&mut self, handles: &crate::RendererWebStorageHandles) {
        self._context_host
            .borrow_mut()
            .set_web_storage_handles(handles);
    }

    pub(crate) fn web_storage_handles(&self) -> crate::RendererWebStorageHandles {
        let host = self._context_host.borrow();
        crate::RendererWebStorageHandles::new(
            host.web_storage_store(),
            host.session_storage_store(),
        )
    }

    pub(crate) fn set_wpt_extensions_enabled(&mut self, enabled: bool) -> Result<()> {
        self._context_host
            .borrow_mut()
            .set_wpt_extensions_enabled(enabled);
        if !enabled {
            return Ok(());
        }

        #[cfg(not(feature = "wpt-extensions"))]
        {
            Ok(())
        }

        #[cfg(feature = "wpt-extensions")]
        self.with_default_context_scope(|scope, _| {
            let global = scope.get_current_context().global(scope);
            crate::context_bootstrap::install_wpt_webdriver_runtime_state(scope, global)
        })
    }

    pub(crate) fn restore_top_level_location_runtime_state(&mut self, url: &Url) {
        let href = url.as_str().to_owned();
        self.sync_top_level_location_runtime_state(&href);
        if let Some((previous_target, next_target)) =
            self.document_runtime.set_document_url(url.clone())
        {
            self._context_host
                .borrow_mut()
                .note_target_style_activity(previous_target, next_target);
        }
    }

    pub(super) fn same_document_fragment_url(current: &Url, candidate: &Url) -> bool {
        let mut current_without_fragment = current.clone();
        current_without_fragment.set_fragment(None);
        let mut candidate_without_fragment = candidate.clone();
        candidate_without_fragment.set_fragment(None);
        current_without_fragment == candidate_without_fragment
    }

    pub(super) fn top_level_context_ptrs(&self) -> Vec<*const v8::Global<v8::Context>> {
        let mut contexts = vec![&self.page_default_context as *const _];
        contexts.extend(
            self.page_isolated_world_contexts
                .contexts()
                .filter(|world| world.child_handle.is_none())
                .map(|world| &world.context as *const _),
        );
        contexts
    }

    pub(super) fn runtime_binding_replay_context_ptrs(
        &self,
    ) -> Vec<*const v8::Global<v8::Context>> {
        let mut contexts = Vec::with_capacity(
            1 + self.page_isolated_world_contexts.len() + self.child_frame_realm_store.len(),
        );
        contexts.push(&self.page_default_context as *const _);
        contexts.extend(
            self.page_isolated_world_contexts
                .contexts()
                .map(|world| &world.context as *const _),
        );
        contexts.extend(
            self.child_frame_realm_store
                .values()
                .map(|world| &world.context as *const _),
        );
        contexts
    }

    pub(crate) fn refresh_top_level_document_url_from_world_locations(&mut self) {
        let current = self.document_runtime.document_url().clone();
        let mut candidates = Vec::new();
        for context_ptr in self.top_level_context_ptrs() {
            // Internal location reconciliation, not an owner-visible script turn.
            let Ok(raw_href) = self.eval_string_in_context_ptr_internal_snapshot(
                context_ptr,
                "(() => String((globalThis.location && globalThis.location.href) || ''))()",
            ) else {
                continue;
            };
            let Ok(candidate) = Url::parse(&raw_href) else {
                continue;
            };
            candidates.push(candidate);
        }

        if candidates
            .iter()
            .any(|candidate| candidate.as_str() == current.as_str())
        {
            self.sync_top_level_location_runtime_state(current.as_str());
            return;
        }

        for candidate in candidates {
            if candidate.as_str() == current.as_str()
                || !Self::same_document_fragment_url(&current, &candidate)
            {
                continue;
            }
            if let Some((previous_target, next_target)) =
                self.document_runtime.set_document_url(candidate.clone())
            {
                self._context_host
                    .borrow_mut()
                    .note_target_style_activity(previous_target, next_target);
            }
            self.sync_top_level_location_runtime_state(candidate.as_str());
            break;
        }
    }

    pub(super) fn sync_top_level_location_runtime_state(&mut self, href: &str) {
        let href = href.to_owned();
        for context_ptr in self.top_level_context_ptrs() {
            let href = href.clone();
            let _ = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    sync_global_location_runtime_state(scope, &href);
                    Ok(())
                });
        }
    }

    pub(crate) fn retire_document_resource_authorities(&mut self) {
        self._context_host
            .borrow_mut()
            .retire_all_document_resource_loaders();
        self.document_runtime.clear_cookie_store();
    }
}
