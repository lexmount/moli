use super::*;

impl ScriptVm {
    pub(crate) fn should_intercept_parser_script_source_fetch(
        &self,
        script: &crate::planning::PreparedScript,
    ) -> bool {
        script.source_kind == crate::types::ScriptSourceKind::External
            && matches!(script.url.scheme(), "http" | "https")
            && self
                ._context_host
                .borrow()
                .should_intercept_subresource(SubresourceResourceType::Script)
    }
    pub(crate) fn start_parser_script_source_fetch_interception(
        &mut self,
        script: crate::planning::PreparedScript,
        request_client: ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
        document_character_set: Option<String>,
    ) -> crate::planning::SharedScriptSourceLoad {
        // Main-parser source completion is routed by the exact parser
        // continuation registered on the returned load. The interception
        // request itself is frozen into this Page turn's concrete output
        // journal; it is never parked in browser-global state for a later
        // protocol snapshot to rediscover.
        let (load, completer) =
            crate::planning::SharedScriptSourceLoad::pending_with_owner_wake(None);
        let (info, continuation) = browser_context_runtime.prepare_detached_parser_script_fetch(
            PendingSubresourceFetchInfo {
                internal_id: 0,
                network_request_handle: None,
                frame_id: self.root_frame_id.clone(),
                document_url: script.initiator_url.clone(),
                url: script.url.clone(),
                websocket_socket_id: None,
                method: "GET".to_owned(),
                request_headers: Default::default(),
                request_body: None,
                request_body_bytes: None,
                resource_type: SubresourceResourceType::Script,
                request_cookie_report: None,
            },
            script,
            self.current_main_document_resource_loader()
                .expect("parser interception requires its Document authority")
                .fetch_context()
                .request_origin(),
            request_client,
            task_runner,
            document_character_set,
            completer,
        );
        let source_document = self
            ._context_host
            .borrow()
            .root_document_lifecycle_identity()
            .expect("parser fetch interception requires an active root Document");
        let appended = self._context_host.borrow().append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::DetachedParserScriptFetchPause {
                source_document,
                info: Box::new(info),
                continuation,
            },
        );
        assert!(
            appended,
            "parser fetch interception requires a concrete Page output journal"
        );
        load
    }
}
