use super::*;

impl RendererCreateStreamingRawPageRequest {
    pub(in crate::runtime) fn validate_bootstrap_configuration(&self) -> Result<()> {
        ensure!(
            self.lifecycle_decider.is_none()
                || (matches!(self.reply_boundary, crate::RendererReplyBoundary::Stage)
                    && matches!(
                        self.top_level_navigation_dispatch,
                        RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter
                    )
                    && matches!(
                        self.navigation_reply_policy,
                        NavigationReplyPolicy::FollowBeforeReply
                    )),
            "a lifecycle decider requires standalone follow-before-reply page creation"
        );
        Ok(())
    }

    pub(in crate::runtime) async fn bootstrap(
        self,
        page_id: PageId,
        local_executor: JsLocalExecutor,
        runtime_hooks: PageVmRuntimeHooks,
    ) -> Result<StreamingNavigationPageCreationResult> {
        let navigation_bootstrap_entry = self.navigation_history
            .as_ref()
            .map(|history| history.resolve(&self.final_url))
            .transpose()?;
        self.bootstrap_with_navigation_seed(page_id, local_executor, runtime_hooks, navigation_bootstrap_entry)
            .await
    }

    pub(in crate::runtime) async fn bootstrap_with_navigation_seed(
        self,
        page_id: PageId,
        local_executor: JsLocalExecutor,
        runtime_hooks: PageVmRuntimeHooks,
        navigation_bootstrap_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
    ) -> Result<StreamingNavigationPageCreationResult> {
        let RendererCreateStreamingRawPageRequest {
            web_mcp_invocation,
            document_replacement: _document_replacement,
            root_frame_id,
            main_document_commit,
            navigation_history,
            final_url,
            response_status,
            response_headers,
            loader,
            navigator_identity,
            web_storage,
            raw_body,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds,
            permission_overrides,
            extra_http_headers,
            document_settings,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager,
            storage_bucket_store,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_configuration,
            wpt_extensions_enabled,
            stage,
            reply_boundary,
            reserved_service_worker_client,
            ..
        } = self;
        let runtime_hooks = runtime_hooks.with_web_mcp_navigation(web_mcp_invocation);
        let loader = loader_for_new_document(
            &loader,
            &extra_http_headers,
            network_offline,
            &blocked_url_patterns,
        );
        let document_policy_container = DocumentPolicyContainer::from_navigation_response_headers(
            &response_headers,
            &final_url,
        )
        .with_content_security_policy_bypass(document_settings.bypass_content_security_policy);
        let document_default_language =
            crate::document_language::document_default_language_from_headers(&response_headers);
        let document_last_modified =
            crate::document_last_modified::document_last_modified_from_headers(&response_headers);
        let mut env = PageVmEnvConfig {
            web_storage,
            document_start_scripts,
            runtime_bindings,
            runtime_inspector_session_restore_snapshots,
            runtime_isolated_worlds,
            permission_overrides,
            extra_http_headers,
            navigator_identity,
            document_policy_container,
            document_default_language,
            document_last_modified,
            document_settings,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager,
            storage_bucket_store,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_configuration,
            wpt_extensions_enabled,
            root_frame_id,
            main_document_commit,
            top_level_storage_key: None,
            navigation_bootstrap_entry,
            navigation_history_source: navigation_history.as_ref().map(|history| history.source_history()),
            reserved_service_worker_client_id: reserved_service_worker_client
                .map(RendererReservedServiceWorkerClient::release),
        };
        env.apply_main_document_commit_referrer();

        let bootstrap_executor = local_executor.clone();
        PageVm::run_bootstrap_future_on_fresh_local_task(
            local_executor,
            "external raw document bootstrap local task channel closed",
            Box::pin(async move {
                // Keep the provisional replacement scope alive through bootstrap.
                let _replacement_scope = _document_replacement;
                ConcurrentParseTimeRuntime::create_external_raw_document_response_at_reply_boundary(
                    page_id,
                    bootstrap_executor,
                    &loader,
                    &env,
                    runtime_hooks,
                    stage,
                    Instant::now(),
                    final_url,
                    response_status,
                    response_headers,
                    raw_body,
                    reply_boundary,
                )
                .await
            }),
        )
        .await
    }
}
