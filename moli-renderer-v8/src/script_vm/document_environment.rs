use super::*;

impl ScriptVm {
    pub(crate) fn set_extra_http_headers(&mut self, headers: &moli_fetch::RequestHeaders) {
        self._context_host
            .borrow_mut()
            .set_extra_http_headers(headers);
    }

    pub(crate) fn set_main_navigation_policy_container(
        &mut self,
        policy: crate::document_runtime::DocumentPolicyContainer,
    ) {
        self.document_runtime
            .set_main_navigation_policy_container(policy);
    }

    pub(crate) fn document_content_security_policies(&self) -> Vec<String> {
        self._context_host
            .borrow()
            .document_content_security_policies()
            .to_vec()
    }

    pub(crate) fn set_stored_document_start_scripts(&mut self, scripts: &[DocumentStartScript]) {
        self._context_host
            .borrow_mut()
            .set_stored_document_start_scripts(scripts);
    }

    pub(crate) fn set_stored_runtime_bindings(
        &mut self,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self._context_host
            .borrow_mut()
            .set_stored_runtime_bindings(bindings);
    }

    pub(crate) fn set_inspector_session_runtime_bindings(
        &mut self,
        inspector_session_id: Option<&str>,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self.page_inspector
            .set_runtime_bindings_for_session(inspector_session_id, bindings);
    }

    pub(crate) fn inspector_session_runtime_bindings(
        &self,
        inspector_session_id: Option<&str>,
    ) -> Vec<crate::protocol_types::RuntimeBindingRegistration> {
        self.page_inspector
            .runtime_bindings_for_session(inspector_session_id)
    }

    pub(crate) fn detach_runtime_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .remove_dom_debugger_session(inspector_session_id);
        let devtools_session = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let detached = self.page_inspector.detach_session(inspector_session_id);
        let retired_worlds = self.retire_isolated_worlds_for_devtools_session(&devtools_session);
        detached || retired_worlds != 0
    }

    pub(crate) fn set_permission_overrides(
        &mut self,
        overrides: &[crate::protocol_types::PermissionOverrideRegistration],
    ) {
        self._context_host
            .borrow_mut()
            .set_permission_overrides(overrides);
    }

    pub(crate) fn set_emulated_media(
        &mut self,
        overrides: &crate::protocol_types::EmulatedMediaOverrides,
    ) {
        {
            let mut host = self._context_host.borrow_mut();
            if host.emulated_media() == overrides {
                return;
            }
            host.queue_environment_change();
            host.set_emulated_media(overrides);
        }
        self.sync_document_fonts_for_environment();
    }

    pub(crate) fn set_emulated_media_for_bootstrap(
        &mut self,
        overrides: &crate::protocol_types::EmulatedMediaOverrides,
    ) {
        self._context_host
            .borrow_mut()
            .set_emulated_media(overrides);
    }

    pub(crate) fn set_idle_override(
        &mut self,
        idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    ) {
        self._context_host
            .borrow_mut()
            .set_idle_override(idle_override);
    }

    pub(crate) fn set_idle_override_and_sync_surface(
        &mut self,
        idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    ) -> Result<()> {
        self.set_idle_override(idle_override);
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            crate::context_bootstrap::apply_idle_override_to_current_context(scope, idle_override);
            Ok(())
        })
    }

    pub(crate) fn stylesheet_preload_media_matches(&self, media: Option<&str>) -> bool {
        let Some(media) = media.map(str::trim).filter(|media| !media.is_empty()) else {
            return true;
        };
        let host = self._context_host.borrow();
        crate::style_engine::media_list::evaluate_media_query_list(
            media,
            Some(host.emulated_media()),
            host.style_viewport(),
        )
    }

    pub(crate) fn fetch_subresource_interception_matches(
        &self,
        resource_type: crate::types::SubresourceResourceType,
    ) -> bool {
        let host = self._context_host.borrow();
        host.fetch_subresource_interception_enabled()
            && host
                .fetch_subresource_interception_resource_type()
                .is_none_or(|intercepted| intercepted == resource_type)
    }

    pub(crate) fn set_viewport_surface(
        &mut self,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    ) -> Result<()> {
        {
            let mut host = self._context_host.borrow_mut();
            if host.viewport_surface() == viewport_surface {
                return Ok(());
            }
            host.queue_environment_change();
            host.set_viewport_surface(viewport_surface);
        }
        self.sync_document_fonts_for_environment();
        Ok(())
    }

    /// Seeds the initial viewport before document-start scripts can materialize
    /// Window surfaces or register media-query listeners.
    pub(crate) fn set_viewport_surface_for_bootstrap(
        &mut self,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    ) {
        self._context_host
            .borrow_mut()
            .set_viewport_surface(viewport_surface);
    }

    pub(crate) fn set_document_activity(
        &mut self,
        activity: moli_page_types::DocumentActivity,
    ) -> Result<()> {
        let mut host = self._context_host.borrow_mut();
        if host.document_activity() != activity {
            host.queue_environment_change();
            host.set_document_activity(activity);
        }
        Ok(())
    }

    pub(crate) fn set_document_activity_for_bootstrap(
        &mut self,
        activity: moli_page_types::DocumentActivity,
    ) {
        self._context_host
            .borrow_mut()
            .set_document_activity(activity);
    }

    pub(crate) fn set_layout_policy(&mut self, policy: moli_page_types::LayoutPolicy) {
        self._context_host.borrow_mut().set_layout_policy(policy);
    }

    pub(super) fn sync_document_fonts_for_environment(&mut self) {
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(scope, &self.page_default_context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let host = self._context_host.borrow();
                for document in host.documents_with_adopted_style_sheets() {
                    crate::native_bridge::document::sync_document_fonts_for_handle(
                        scope, &host, document,
                    );
                }
            });
    }

    pub(crate) fn set_network_offline(&mut self, offline: bool) {
        self._context_host.borrow_mut().set_network_offline(offline);
    }

    pub(crate) fn set_navigator_overrides(
        &mut self,
        overrides: &moli_page_types::NavigatorOverrides,
    ) {
        self._context_host
            .borrow_mut()
            .set_navigator_overrides(overrides);
    }

    pub(crate) fn set_navigator_overrides_and_sync_surface(
        &mut self,
        overrides: &moli_page_types::NavigatorOverrides,
    ) -> Result<()> {
        let changed = self
            ._context_host
            .borrow_mut()
            .set_navigator_overrides(overrides);
        if changed {
            let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
            self.with_context_scope_by_ptr(context_ptr, |scope, _| {
                crate::context_bootstrap::notify_geolocation_override_changed(scope);
                Ok(())
            })?;
        }
        Ok(())
    }

    pub(crate) fn set_bypass_service_worker(&mut self, bypass: bool) {
        self._context_host
            .borrow_mut()
            .set_bypass_service_worker(bypass);
    }

    pub(crate) fn set_cache_disabled(&mut self, disabled: bool) {
        self._context_host.borrow_mut().set_cache_disabled(disabled);
    }

    pub(crate) fn set_blocked_url_patterns(&mut self, patterns: &[String]) {
        self._context_host
            .borrow_mut()
            .set_blocked_url_patterns(patterns);
    }

    pub(crate) fn set_fetch_subresource_interception(
        &mut self,
        enabled: bool,
        resource_type: Option<SubresourceResourceType>,
    ) {
        self._context_host
            .borrow_mut()
            .set_fetch_subresource_interception(enabled, resource_type);
    }
}

impl ScriptVm {
    pub(crate) fn capture_about_blank_reload_environment(
        &mut self,
        preserve_navigation_referrer: bool,
    ) -> Result<ScriptVmInitialDocumentEnvironment> {
        self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let origin = host
                .current_main_document_resource_loader()
                .expect("a live Document has a resource authority")
                .fetch_context()
                .origin()
                .to_owned();
            let fallback_base_url = if preserve_navigation_referrer {
                host.dom_host()
                    .node(host.document_handle())
                    .and_then(|node| node.as_document())
                    .map(|document| document.fallback_base_url().clone())
            } else {
                Some(host.document_base_url_for_handle(host.document_handle()))
            };
            let mut policy_container = host.document_policy_container().clone();
            let source = if preserve_navigation_referrer {
                url::Url::parse(&policy_container.document_referrer).ok()
            } else {
                Some(host.document_url().clone())
            };
            policy_container.document_referrer = source
                .as_ref()
                .and_then(|source| {
                    moli_fetch::referrer_value(
                        source,
                        host.document_url(),
                        None,
                        policy_container.referrer_policy.as_deref(),
                    )
                })
                .unwrap_or_default();
            let token = scope.get_current_context().get_security_token(scope);
            Ok(ScriptVmInitialDocumentEnvironment {
                security_token: Some(v8::Global::new(scope, token)),
                origin,
                policy_container,
                fallback_base_url,
                storage_key: Some(host.top_web_storage_scope().storage_key().clone()),
            })
        })
    }
}
