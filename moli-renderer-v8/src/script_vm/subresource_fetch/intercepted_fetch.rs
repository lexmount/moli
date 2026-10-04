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
                .script_fetch_origin(),
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
    pub(super) fn continue_worker_owned_fetch(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_fetch(worker_id, request),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_fetch(instance_id, request),
        }
    }
    pub(super) fn continue_worker_owned_xhr(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_xhr(worker_id, request),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_xhr(instance_id, request),
        }
    }
    pub(super) fn continue_worker_owned_csp_report(
        &self,
        target: WorkerOwnedCspReportTarget,
        request: crate::worker::WorkerPendingFetchContinue,
    ) -> bool {
        match target {
            WorkerOwnedCspReportTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_csp_report(worker_id, request),
            WorkerOwnedCspReportTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_csp_report(instance_id, request),
        }
    }
    pub(super) fn fail_worker_owned_fetch(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_fetch(worker_id, request, error_text),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_fetch(instance_id, request, error_text),
        }
    }
    pub(super) fn fail_worker_owned_xhr(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_xhr(worker_id, request, error_text),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_xhr(instance_id, request, error_text),
        }
    }
    pub(super) fn fail_worker_owned_csp_report(
        &self,
        target: WorkerOwnedCspReportTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedCspReportTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_csp_report(worker_id, request, error_text),
            WorkerOwnedCspReportTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_csp_report(instance_id, request, error_text),
        }
    }
    pub(super) fn fail_worker_owned_fetch_auth(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_fetch_auth(worker_id, request, error_text),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_fetch_auth(instance_id, request, error_text),
        }
    }
    pub(super) fn fail_worker_owned_xhr_auth(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_xhr_auth(worker_id, request, error_text),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_xhr_auth(instance_id, request, error_text),
        }
    }
    pub(super) fn continue_worker_owned_fetch_response(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_fetch_response(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                ),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_fetch_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                ),
        }
    }
    pub(super) fn continue_worker_owned_xhr_response(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_xhr_response(worker_id, request, response_code, response_headers),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_xhr_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                ),
        }
    }
    pub(super) fn fail_worker_owned_fetch_response(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_fetch_response(worker_id, request, error_text),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_fetch_response(instance_id, request, error_text),
        }
    }
    pub(super) fn fail_worker_owned_xhr_response(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_xhr_response(worker_id, request, error_text),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_xhr_response(instance_id, request, error_text),
        }
    }
    pub(super) fn fulfill_worker_owned_fetch(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => {
                self._context_host.borrow_mut().fulfill_worker_fetch(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => {
                self._context_host.borrow_mut().fulfill_shared_worker_fetch(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
        }
    }
    pub(super) fn fulfill_worker_owned_xhr(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => {
                self._context_host.borrow_mut().fulfill_worker_xhr(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => {
                self._context_host.borrow_mut().fulfill_shared_worker_xhr(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
        }
    }
    pub(super) fn fulfill_worker_owned_csp_report(
        &self,
        target: WorkerOwnedCspReportTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedCspReportTarget::Dedicated { worker_id, .. } => {
                self._context_host.borrow_mut().fulfill_worker_csp_report(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
            WorkerOwnedCspReportTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fulfill_shared_worker_csp_report(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                ),
        }
    }
    pub(super) fn fulfill_worker_owned_fetch_response(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fulfill_worker_fetch_response(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                ),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fulfill_shared_worker_fetch_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                ),
        }
    }
    pub(super) fn fulfill_worker_owned_xhr_response(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => {
                self._context_host.borrow_mut().fulfill_worker_xhr_response(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fulfill_shared_worker_xhr_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                ),
        }
    }
}
