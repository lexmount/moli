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
        let RendererCreateStreamingRawPageRequest {
            document_replacement: _document_replacement,
            root_frame_id,
            main_document_commit,
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
            script_execution_disabled,
            bypass_content_security_policy,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager,
            storage_bucket_store,
            emulated_media,
            idle_override,
            navigator_overrides,
            viewport_surface,
            document_activity,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_policy,
            wpt_extensions_enabled,
            stage,
            reply_boundary,
            reserved_service_worker_client,
            ..
        } = self;
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
        .with_content_security_policy_bypass(bypass_content_security_policy);
        let document_default_language =
            crate::document_language::document_default_language_from_headers(&response_headers);
        let document_last_modified =
            crate::document_last_modified::document_last_modified_from_headers(&response_headers);
        let env = PageVmEnvConfig {
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
            script_execution_disabled,
            bypass_content_security_policy,
            emulated_media,
            idle_override,
            navigator_overrides,
            viewport_surface,
            document_activity,
            network_offline,
            blocked_url_patterns,
            indexed_db_manager,
            storage_bucket_store,
            fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type,
            layout_policy,
            wpt_extensions_enabled,
            root_frame_id,
            main_document_commit,
            top_level_storage_key: None,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: reserved_service_worker_client
                .map(RendererReservedServiceWorkerClient::release),
        };

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
