use super::*;

pub(crate) struct OpenedRendererWindow<'s> {
    pub(crate) window_proxy: v8::Local<'s, v8::Object>,
    pub(crate) window: Option<crate::runtime::RendererAuxiliaryWindow>,
    pub(crate) name: crate::runtime::RendererBrowsingContextName,
    pub(crate) pending_page: Option<crate::runtime::RendererPendingAuxiliaryPage>,
    pub(crate) session_storage: Option<SharedWebStorageStore>,
    pub(crate) initial_storage_key: Option<MoliStorageKey>,
    pub(crate) initial_document_environment:
        Option<crate::runtime::RendererCapturedDocumentEnvironment>,
}

impl JsContextHost {
    pub(crate) fn has_browser_owned_auxiliary_page_factory(&self) -> bool {
        self.browser_context_runtime
            .browser_owns_auxiliary_document_responses()
            && self
                .page_script_environment()
                .and_then(|environment| environment.auxiliary_allocator())
                .is_some()
    }

    pub(crate) fn open_renderer_owned_auxiliary_window<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        creator: v8::Local<'s, v8::Object>,
        exposes_opener: bool,
        creator_child_handle: Option<DomHandle>,
        target_name: &str,
        href: &str,
        creator_base_url: Url,
        mut policy: DocumentPolicyContainer,
        update_existing_opener: bool,
    ) -> Result<OpenedRendererWindow<'s>> {
        let environment = self
            .page_script_environment()
            .ok_or_else(|| anyhow::anyhow!("auxiliary creator Page is retired"))?;
        if exposes_opener
            && let Some(name) = trackable_lightweight_popup_window_name(target_name)
            && let Some(existing) =
                environment.named_related_window(&name, self.browsing_context_name())
        {
            if update_existing_opener {
                existing.set_opener(Some(v8::Global::new(scope, creator)));
            }
            let (window, name) = existing
                .window_identity()
                .expect("named related lookup retains its browsing context identity");
            let window_proxy = existing.window_proxy_in_scope(scope)?;
            let initial_document_environment = if !href.is_empty()
                && moli_url::is_about_blank(&Url::parse(href)?)
            {
                // Combine the initiator's policy with the target frame's fixed
                // restrictions, excluding CSP from the target's old response.
                if let Some(window) = window.as_ref() {
                    policy.sandbox = policy
                        .sandbox
                        .with_response_content_security_policy(window.frame_sandbox());
                }
                let storage_scope = self.lightweight_popup_storage_scope_for_initiated_navigation(
                    scope,
                    Some(creator),
                    creator_child_handle,
                    &about_blank_url(),
                    policy.sandbox.forces_opaque_origin,
                );
                policy.document_referrer = Url::parse(&policy.document_referrer)
                    .ok()
                    .and_then(|source| {
                        moli_fetch::referrer_value(
                            &source,
                            &about_blank_url(),
                            None,
                            policy.referrer_policy.as_deref(),
                        )
                    })
                    .unwrap_or_default();
                let inherited =
                    crate::script_vm::ScriptVmInitialDocumentEnvironment::inherited_in_scope(
                        scope,
                        creator,
                        storage_scope.origin().to_owned(),
                        storage_scope.storage_key().clone(),
                        creator_base_url,
                        policy,
                    )?;
                Some(
                    environment
                        .auxiliary_allocator()
                        .ok_or_else(|| anyhow::anyhow!("navigation initiator has no allocator"))?
                        .capture_document_environment(
                            &environment,
                            inherited,
                            crate::context_bootstrap::window_realm_secure_context_available(
                                scope, creator,
                            ),
                        )?,
                )
            } else {
                None
            };
            return Ok(OpenedRendererWindow {
                window_proxy,
                window,
                name,
                pending_page: None,
                session_storage: None,
                initial_storage_key: None,
                initial_document_environment,
            });
        }
        let allocator = environment
            .auxiliary_allocator()
            .ok_or_else(|| anyhow::anyhow!("auxiliary creator Page has no allocator"))?;
        let requested_url = if href.is_empty() {
            about_blank_url()
        } else {
            Url::parse(href)?
        };
        let initial_url = if moli_url::is_about_blank(&requested_url) {
            requested_url
        } else {
            about_blank_url()
        };
        let base_url = if exposes_opener {
            lightweight_popup_initial_base_url(&initial_url, creator_base_url)
        } else {
            initial_url.clone()
        };
        let sandbox = creator_child_handle
            .and_then(|handle| self.child_browsing_context_popup_opener_sandbox_policy(handle));
        if policy.sandbox.allows_popups_to_escape {
            policy.sandbox = DocumentSandboxPolicy::default();
        }
        inherit_lightweight_popup_opener_sandbox(&mut policy.sandbox, sandbox);
        let storage_scope = self.lightweight_popup_storage_scope_for_initiated_navigation(
            scope,
            Some(creator),
            creator_child_handle,
            &initial_url,
            policy.sandbox.forces_opaque_origin,
        );
        let session_storage = if exposes_opener {
            self.cloned_lightweight_popup_session_storage_store(
                scope,
                creator,
                creator_child_handle,
                &storage_scope,
            )
        } else {
            new_shared_web_storage_store()
        };
        let source_loader = self
            .document_resource_loader_for_dispatch_scope(
                creator_child_handle.map_or(OwnerDispatchScope::Top, OwnerDispatchScope::Child),
            )
            .ok_or_else(|| anyhow::anyhow!("auxiliary creator Document has no resource authority"))?
            .clone();
        let loader = source_loader
            .request_client()
            .fork_with_isolated_page_network_policy();
        let env = crate::runtime::PageVmEnvConfig {
            root_frame_id: None,
            main_document_commit: None,
            top_level_storage_key: Some(storage_scope.storage_key().clone()),
            web_storage: crate::RendererWebStorageHandles::new(
                self.web_storage_store(),
                session_storage.clone(),
            ),
            document_start_scripts: Vec::new(),
            runtime_bindings: Vec::new(),
            runtime_inspector_session_restore_snapshots: Vec::new(),
            runtime_isolated_worlds: Vec::new(),
            permission_overrides: Vec::new(),
            extra_http_headers: Default::default(),
            navigator_identity: loader.browser_identity().clone(),
            document_policy_container: policy.clone(),
            document_default_language: None,
            document_last_modified: None,
            script_execution_disabled: !policy.sandbox.allows_scripts,
            bypass_content_security_policy: false,
            emulated_media: Default::default(),
            idle_override: None,
            navigator_overrides: self.navigator_overrides().clone(),
            viewport_surface: self.viewport_surface(),
            document_activity: Default::default(),
            network_offline: false,
            blocked_url_patterns: Vec::new(),
            indexed_db_manager: self.indexed_db_manager(),
            storage_bucket_store: Some(self.storage_bucket_store()),
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
            layout_policy: Default::default(),
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        };
        let mut dom_host = crate::dom::native::DomHost::from_dom(
            crate::parser::HtmlParser::with_scripting_enabled(policy.sandbox.allows_scripts).parse(
                initial_url,
                "<!doctype html><html><head></head><body></body></html>".to_owned(),
            ),
        );
        let document = dom_host.document_handle();
        dom_host.set_document_fallback_base_url_for_handle(document, Some(base_url));
        let window = self
            .browser_context_runtime
            .new_auxiliary_window_with_sandbox(policy.sandbox);
        let name = if exposes_opener {
            crate::runtime::RendererBrowsingContextName::new_auxiliary(self.browsing_context_name())
        } else {
            crate::runtime::RendererBrowsingContextName::default()
        };
        name.set(trackable_lightweight_popup_window_name(target_name).unwrap_or_default());
        let creator_context = creator
            .get_creation_context(scope)
            .ok_or_else(|| anyhow::anyhow!("auxiliary creator has no live creation context"))?;
        let inherited_security_token =
            v8::Global::new(scope, creator_context.get_security_token(scope));
        let init = crate::runtime::RendererRelatedInitialEmptyPageInit {
            dom_host,
            loader,
            env,
            inherited_origin: storage_scope.origin().to_owned(),
            inherited_security_token,
            opener: exposes_opener.then(|| v8::Global::new(scope, creator)),
            window: window.clone(),
            name: name.clone(),
        };
        let (pending, window_proxy) =
            allocator.stage_in_scope(scope, &environment, &self.bridge.bindings, init)?;
        Ok(OpenedRendererWindow {
            window_proxy,
            window: Some(window),
            name,
            pending_page: Some(pending),
            session_storage: Some(session_storage),
            initial_storage_key: Some(storage_scope.storage_key().clone()),
            initial_document_environment: None,
        })
    }
}
