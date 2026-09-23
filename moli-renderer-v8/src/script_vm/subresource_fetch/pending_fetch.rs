use super::*;

impl ScriptVm {
    pub(crate) fn continue_pending_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<moli_fetch::RequestHeaderOverride>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let mut pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let headers = headers.map(|override_headers| match override_headers {
            moli_fetch::RequestHeaderOverride::CurrentRequest {
                headers,
                redirect_headers,
            } => {
                pending.redirect_headers =
                    Some(redirect_headers.unwrap_or_else(|| pending.info.request_headers.clone()));
                headers
            }
            moli_fetch::RequestHeaderOverride::RedirectChain(headers) => {
                pending.redirect_headers = None;
                headers
            }
        });
        let PendingSubresourceFetchState {
            redirect_headers,
            request_origin,
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            deferred_request_started,
            blob_url_entry,
        } = pending;
        let pending = match continuation {
            PendingSubresourceContinuation::WebSocket(connection) => {
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let headers_overridden = headers.is_some();
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                self._context_host
                    .borrow_mut()
                    .start_pending_websocket_connection(
                        connection,
                        request_url,
                        request_headers,
                        headers_overridden,
                        intercept_response,
                    )
                    .map_err(|error| anyhow!(error))?;
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                    PendingSubresourceContinueOutcome::Started,
                ));
            }
            continuation @ (PendingSubresourceContinuation::WorkerFetch { .. }
            | PendingSubresourceContinuation::SharedWorkerFetch { .. }) => {
                let target = WorkerOwnedFetchTarget::from_continuation(&continuation)
                    .expect("fetch continuation target");
                let fetch_id = target.fetch_id();
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let continued = self.continue_worker_owned_fetch(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        redirect_headers: redirect_headers.clone(),
                        fetch_id,
                        internal_id,
                        network_request_handle: info.network_request_handle,
                        url: request_url.clone(),
                        method: request_method.clone(),
                        body: request_body.clone(),
                        headers: request_headers.clone(),
                        intercept_response,
                        handle_auth_requests,
                        auth: None,
                    },
                );
                if !continued {
                    bail!(target.unavailable_message());
                }
                if intercept_response || handle_auth_requests {
                    self._context_host
                        .borrow_mut()
                        .record_in_flight_worker_subresource_fetch(
                            crate::types::InFlightWorkerSubresourceFetchState {
                                pending: PendingSubresourceFetchState {
                                    redirect_headers: redirect_headers.clone(),
                                    request_origin,
                                    info,
                                    load,
                                    execution_context,
                                    credentials_mode,
                                    request_mode,
                                    network_partition_key: network_partition_key.clone(),
                                    policy_context,
                                    continuation: target.continuation(),
                                    deferred_request_started,
                                    blob_url_entry,
                                },
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                            },
                        );
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                    PendingSubresourceContinueOutcome::Started,
                ));
            }
            continuation @ (PendingSubresourceContinuation::WorkerXhr { .. }
            | PendingSubresourceContinuation::SharedWorkerXhr { .. }) => {
                let target = WorkerOwnedXhrTarget::from_continuation(&continuation)
                    .expect("xhr continuation target");
                let xhr_id = target.xhr_id();
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let continued = self.continue_worker_owned_xhr(
                    target,
                    crate::worker::WorkerPendingXhrContinue {
                        redirect_headers: redirect_headers.clone(),
                        xhr_id,
                        internal_id,
                        network_request_handle: info.network_request_handle,
                        url: request_url.clone(),
                        method: request_method.clone(),
                        body: request_body.clone(),
                        headers: request_headers.clone(),
                        intercept_response,
                        handle_auth_requests,
                        auth: None,
                    },
                );
                if !continued {
                    bail!(target.unavailable_message());
                }
                if intercept_response || handle_auth_requests {
                    self._context_host
                        .borrow_mut()
                        .record_in_flight_worker_subresource_fetch(
                            crate::types::InFlightWorkerSubresourceFetchState {
                                pending: PendingSubresourceFetchState {
                                    redirect_headers: redirect_headers.clone(),
                                    request_origin,
                                    info,
                                    load,
                                    execution_context,
                                    credentials_mode,
                                    request_mode,
                                    network_partition_key: network_partition_key.clone(),
                                    policy_context,
                                    continuation: target.continuation(),
                                    deferred_request_started,
                                    blob_url_entry,
                                },
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                            },
                        );
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                    PendingSubresourceContinueOutcome::Started,
                ));
            }
            continuation @ (PendingSubresourceContinuation::WorkerCspReport { .. }
            | PendingSubresourceContinuation::SharedWorkerCspReport { .. }) => {
                let target = WorkerOwnedCspReportTarget::from_continuation(&continuation)
                    .expect("CSP report continuation target");
                let report_id = target.report_id();
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let continued = self.continue_worker_owned_csp_report(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        redirect_headers: redirect_headers.clone(),
                        fetch_id: report_id,
                        internal_id,
                        network_request_handle: info.network_request_handle,
                        url: request_url,
                        method: request_method,
                        body: request_body,
                        headers: request_headers,
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                );
                if !continued {
                    bail!(target.unavailable_message());
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                    PendingSubresourceContinueOutcome::Started,
                ));
            }
            PendingSubresourceContinuation::CspReport { client_id } => {
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body_bytes = match &body {
                    Some(Some(body)) => Some(body.as_bytes().to_vec()),
                    Some(None) => None,
                    None => info.request_body_bytes.clone(),
                };
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let pending = PendingSubresourceFetchState {
                    redirect_headers: redirect_headers.clone(),
                    request_origin,
                    info,
                    load,
                    execution_context,
                    credentials_mode,
                    request_mode,
                    network_partition_key,
                    policy_context,
                    continuation: PendingSubresourceContinuation::CspReport { client_id },
                    deferred_request_started,
                    blob_url_entry,
                };
                if !self._context_host.borrow().network_offline() {
                    let maybe_pending = self.continue_csp_report_via_service_worker(
                        pending,
                        client_id,
                        request_url.clone(),
                        request_method.clone(),
                        request_headers.clone(),
                        request_body.clone(),
                        request_body_bytes,
                    )?;
                    let Some(pending) = maybe_pending else {
                        return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                            PendingSubresourceContinueOutcome::Started,
                        ));
                    };
                    return self.continue_pending_subresource_fetch_via_loader(
                        pending,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                        intercept_response,
                        handle_auth_requests,
                    );
                }
                return self.continue_pending_subresource_fetch_via_loader(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    intercept_response,
                    handle_auth_requests,
                );
            }
            continuation => PendingSubresourceFetchState {
                redirect_headers: redirect_headers.clone(),
                request_origin,
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                deferred_request_started,
                blob_url_entry,
            },
        };
        let request_url = url.unwrap_or_else(|| pending.info.url.clone());
        let request_method = method.unwrap_or_else(|| pending.info.method.clone());
        let request_body = body.unwrap_or_else(|| pending.info.request_body.clone());
        let request_headers = headers.unwrap_or_else(|| pending.info.request_headers.clone());
        self.continue_pending_subresource_fetch_via_loader(
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
            intercept_response,
            handle_auth_requests,
        )
    }
    pub(super) fn continue_pending_subresource_fetch_via_loader(
        &mut self,
        pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let internal_id = pending.info.internal_id;
        if pending.load.network_offline() {
            let activity = self.resolve_pending_subresource_fetch_body(
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                None,
                false,
                None,
                None,
                Err("Network emulation offline".to_owned()),
            )?;
            return Ok(AsyncSubresourceCommandExecution::after_body(
                PendingSubresourceContinueOutcome::Started,
                activity,
            )
            .with_post_checkpoint_event(PendingSubresourceContinueEvent::Completed {
                internal_id,
            }));
        }
        // Request interception may resume after the initiating Document has
        // navigated. The lease retains the request-time frozen client; looking
        // up the ambient Page loader here would silently rebind policy/backend
        // to a newer Document identity.
        let loader = pending.load.request_client();
        let mut request = moli_fetch::Request::new(
            &request_method,
            request_url.as_str(),
            request_body.clone(),
            request_headers.clone(),
        )?
        .with_redirect_headers(pending.redirect_headers.clone())
        .with_initiator_url(&pending.info.document_url)
        .with_request_origin(pending.request_origin.clone())
        .with_request_mode(pending.request_mode)
        .with_use_cors_preflight(pending.continuation.use_cors_preflight())
        .with_redirect_mode(
            pending
                .continuation
                .window_fetch()
                .map_or(moli_fetch::RequestRedirectMode::Follow, |fetch| {
                    fetch.redirect_mode()
                }),
        )
        .with_credentials_mode(pending.credentials_mode)
        .with_network_partition_key(pending.network_partition_key.clone())
        .with_subframe_context(pending.info.frame_id.is_some());
        request = match pending.info.resource_type {
            SubresourceResourceType::Script
            | SubresourceResourceType::Stylesheet
            | SubresourceResourceType::Image
            | SubresourceResourceType::Font
            | SubresourceResourceType::Audio
            | SubresourceResourceType::Video
            | SubresourceResourceType::Media
            | SubresourceResourceType::TextTrack
            | SubresourceResourceType::Ping
            | SubresourceResourceType::CspReport
            | SubresourceResourceType::Dictionary => {
                match crate::network::request_resource_type_for_subresource(
                    pending.info.resource_type,
                ) {
                    Some(resource_type) => request.with_resource_type(resource_type),
                    None => request,
                }
            }
            SubresourceResourceType::Fetch => {
                request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Fetch)
            }
            SubresourceResourceType::Manifest => request
                .with_resource_type(moli_fetch::RequestResourceType::Manifest)
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Manifest),
            SubresourceResourceType::EventSource => request
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::EventSource)
                .with_cache_mode(moli_fetch::RequestCacheMode::NoStore)
                .without_request_timeout(),
            SubresourceResourceType::Xhr => {
                request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Xhr)
            }
            SubresourceResourceType::WebSocket => request,
        };
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        pending.load.attach_cancel_handle(cancel_handle.clone());
        self.spawn_running_subresource_fetch(
            loader,
            request,
            RunningSubresourceFetchState {
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                intercept_response,
                handle_auth_requests,
                initial_auth_network_request_headers: None,
            },
            Some(cancel_handle),
        );
        Ok(AsyncSubresourceCommandExecution::without_window_realm(
            PendingSubresourceContinueOutcome::Started,
        ))
    }
    pub(super) fn continue_csp_report_via_service_worker(
        &mut self,
        pending: PendingSubresourceFetchState,
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
        request_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
        request_body_bytes: Option<Vec<u8>>,
    ) -> Result<Option<PendingSubresourceFetchState>> {
        if self
            ._context_host
            .borrow()
            .service_worker_controller_for_fetch(
                client_id,
                &pending.info.document_url,
                &request_url,
            )
            .is_none()
        {
            return Ok(Some(pending));
        }

        let mut request = moli_fetch::Request::new_bytes(
            &request_method,
            request_url.as_str(),
            request_body_bytes,
            request_headers.clone(),
        )?
        .with_initiator_url(&pending.info.document_url)
        .with_request_origin(pending.request_origin.clone())
        .with_resource_type(moli_fetch::RequestResourceType::CspReport)
        .with_request_mode(pending.request_mode)
        .with_credentials_mode(pending.credentials_mode)
        .with_network_partition_key(pending.network_partition_key.clone())
        .with_redirect_mode(moli_fetch::RequestRedirectMode::Error)
        .with_subframe_context(pending.info.frame_id.is_some());
        request.priority_hints.fetch_priority = None;

        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        pending.load.attach_cancel_handle(cancel_handle.clone());
        let internal_id = pending.info.internal_id;
        let policy_context = pending.policy_context;
        let request_cookie_report = pending.info.request_cookie_report.clone();
        let document_url = pending.info.document_url.clone();
        let frame_id = pending.info.frame_id.clone();
        let completion_tx = self._context_host.borrow().resource_completion_sender();
        let request_client = pending.load.request_client();
        let resource_task_runner = pending.load.task_runner();
        let dispatch = crate::service_worker_runtime::ServiceWorkerFetchDispatch {
            redirect_check: None,
            internal_id,
            request: self._context_host.borrow().service_worker_fetch_request(
                client_id,
                request.url.clone(),
                request.method.clone(),
                request.request_headers.clone(),
                request.body.clone(),
                crate::service_worker_runtime::ServiceWorkerRequestDestination::Report,
                request.request_mode,
                request.credentials_mode,
                request.redirect_mode,
                request.priority_hints.fetch_priority,
                crate::service_worker_runtime::service_worker_fetch_request_metadata(&request),
            ),
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context: crate::types::AsyncSubresourceNetworkContext {
                frame_id,
                request_origin: pending.request_origin.clone(),
                document_url,
                resource_type: SubresourceResourceType::CspReport,
                policy_context,
            },
            completion_tx,
            request_client,
            resource_task_runner,
            cancel_handle,
            direct_completion_tx: None,
        };

        self._context_host
            .borrow_mut()
            .restore_pending_subresource_fetch(pending);
        if self
            ._context_host
            .borrow()
            .dispatch_service_worker_fetch(dispatch)
        {
            return Ok(None);
        }

        let _ = self
            ._context_host
            .borrow()
            .resource_completion_sender()
            .send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text: None,
                skip_fetch_security_validation: true,
                response_filter: Default::default(),
                network_error_text: None,
                result: Err("service worker csp report fetch dispatch failed".to_owned()).into(),
            });
        Ok(None)
    }
    pub(crate) fn continue_pending_subresource_auth_body(
        &mut self,
        internal_id: u64,
        auth: crate::SubresourceAuthCredentials,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_auth(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource auth `{internal_id}`"))?;
        let PendingSubresourceAuthState {
            pending: pending_fetch,
            request_url,
            request_method,
            request_headers: original_request_headers,
            request_body,
            intercept_response,
            initial_network_request_headers,
            response,
        } = pending;
        if let Some(target) = WorkerOwnedFetchTarget::from_continuation(&pending_fetch.continuation)
        {
            let request_headers = original_request_headers;
            let continued = self.continue_worker_owned_fetch(
                target,
                crate::worker::WorkerPendingFetchContinue {
                    redirect_headers: pending_fetch.redirect_headers.clone(),
                    fetch_id: target.fetch_id(),
                    internal_id,
                    network_request_handle: pending_fetch.info.network_request_handle,
                    url: request_url.clone(),
                    method: request_method.clone(),
                    body: request_body.clone(),
                    headers: request_headers.clone(),
                    intercept_response,
                    handle_auth_requests: true,
                    auth: Some(auth),
                },
            );
            if !continued {
                bail!(target.unavailable_message());
            }
            self._context_host
                .borrow_mut()
                .record_in_flight_worker_subresource_fetch(
                    crate::types::InFlightWorkerSubresourceFetchState {
                        pending: pending_fetch,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                    },
                );
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                PendingSubresourceContinueOutcome::Started,
            ));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending_fetch.continuation) {
            let request_headers = original_request_headers;
            let continued = self.continue_worker_owned_xhr(
                target,
                crate::worker::WorkerPendingXhrContinue {
                    redirect_headers: pending_fetch.redirect_headers.clone(),
                    xhr_id: target.xhr_id(),
                    internal_id,
                    network_request_handle: pending_fetch.info.network_request_handle,
                    url: request_url.clone(),
                    method: request_method.clone(),
                    body: request_body.clone(),
                    headers: request_headers.clone(),
                    intercept_response,
                    handle_auth_requests: true,
                    auth: Some(auth),
                },
            );
            if !continued {
                bail!(target.unavailable_message());
            }
            self._context_host
                .borrow_mut()
                .record_in_flight_worker_subresource_fetch(
                    crate::types::InFlightWorkerSubresourceFetchState {
                        pending: pending_fetch,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                    },
                );
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                PendingSubresourceContinueOutcome::Started,
            ));
        }
        let loader = pending_fetch.load.request_client();
        let mut request = moli_fetch::Request::new(
            &request_method,
            request_url.as_str(),
            request_body.clone(),
            original_request_headers.clone(),
        )?
        .with_redirect_headers(pending_fetch.redirect_headers.clone())
        .with_initiator_url(&pending_fetch.info.document_url)
        .with_request_origin(pending_fetch.request_origin.clone())
        .with_request_mode(pending_fetch.request_mode)
        .with_use_cors_preflight(pending_fetch.continuation.use_cors_preflight())
        .with_redirect_mode(
            pending_fetch
                .continuation
                .window_fetch()
                .map_or(moli_fetch::RequestRedirectMode::Follow, |fetch| {
                    fetch.redirect_mode()
                }),
        )
        .with_credentials_mode(pending_fetch.credentials_mode)
        .with_auth(auth.into())
        .with_subframe_context(pending_fetch.info.frame_id.is_some());
        for redirect in &response.redirect_chain {
            request.apply_redirect_status(redirect.status);
        }
        request.url = response.final_url.clone();
        request = request.with_redirect_chain(
            response
                .redirect_chain
                .into_iter()
                .map(Into::into)
                .collect(),
        );
        request = match pending_fetch.info.resource_type {
            SubresourceResourceType::Script
            | SubresourceResourceType::Stylesheet
            | SubresourceResourceType::Image
            | SubresourceResourceType::Font
            | SubresourceResourceType::Audio
            | SubresourceResourceType::Video
            | SubresourceResourceType::Media
            | SubresourceResourceType::TextTrack
            | SubresourceResourceType::Ping
            | SubresourceResourceType::CspReport
            | SubresourceResourceType::Dictionary => {
                match crate::network::request_resource_type_for_subresource(
                    pending_fetch.info.resource_type,
                ) {
                    Some(resource_type) => request.with_resource_type(resource_type),
                    None => request,
                }
            }
            SubresourceResourceType::Fetch => {
                request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Fetch)
            }
            SubresourceResourceType::Manifest => request
                .with_resource_type(moli_fetch::RequestResourceType::Manifest)
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Manifest),
            SubresourceResourceType::EventSource => request
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::EventSource)
                .with_cache_mode(moli_fetch::RequestCacheMode::NoStore)
                .without_request_timeout(),
            SubresourceResourceType::Xhr => {
                request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Xhr)
            }
            SubresourceResourceType::WebSocket => request,
        };
        let request_headers = original_request_headers;
        if self._context_host.borrow().network_offline() {
            let activity = self.resolve_pending_subresource_fetch_body(
                pending_fetch,
                request_url,
                request_method,
                request_headers,
                request_body,
                None,
                false,
                None,
                None,
                Err("Network emulation offline".to_owned()),
            )?;
            return Ok(AsyncSubresourceCommandExecution::after_body(
                PendingSubresourceContinueOutcome::Started,
                activity,
            )
            .with_post_checkpoint_event(PendingSubresourceContinueEvent::Completed {
                internal_id,
            }));
        }
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        pending_fetch
            .load
            .attach_cancel_handle(cancel_handle.clone());
        self.spawn_running_subresource_fetch(
            loader,
            request,
            RunningSubresourceFetchState {
                pending: pending_fetch,
                request_url,
                request_method,
                request_headers,
                request_body,
                intercept_response,
                handle_auth_requests: true,
                initial_auth_network_request_headers: initial_network_request_headers,
            },
            Some(cancel_handle),
        );
        Ok(AsyncSubresourceCommandExecution::without_window_realm(
            PendingSubresourceContinueOutcome::Started,
        ))
    }
    pub(crate) fn fail_pending_subresource_auth_body(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_auth(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource auth `{internal_id}`"))?;
        if let Some(target) =
            WorkerOwnedFetchTarget::from_continuation(&pending.pending.continuation)
        {
            let result = Err(error_text.clone());
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let failed = self.fail_worker_owned_fetch_auth(
                target,
                crate::worker::WorkerPendingFetchContinue {
                    redirect_headers: None,
                    fetch_id: target.fetch_id(),
                    internal_id,
                    network_request_handle: pending.pending.info.network_request_handle,
                    url: pending.request_url,
                    method: pending.request_method,
                    body: pending.request_body,
                    headers: pending.request_headers,
                    intercept_response: false,
                    handle_auth_requests: false,
                    auth: None,
                },
                error_text,
            );
            if !failed {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending.pending.continuation)
        {
            let result = Err(error_text.clone());
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let failed = self.fail_worker_owned_xhr_auth(
                target,
                crate::worker::WorkerPendingXhrContinue {
                    redirect_headers: None,
                    xhr_id: target.xhr_id(),
                    internal_id,
                    network_request_handle: pending.pending.info.network_request_handle,
                    url: pending.request_url,
                    method: pending.request_method,
                    body: pending.request_body,
                    headers: pending.request_headers,
                    intercept_response: false,
                    handle_auth_requests: false,
                    auth: None,
                },
                error_text,
            );
            if !failed {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            None,
            false,
            None,
            None,
            Err(error_text),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }
    pub(crate) fn cancel_pending_subresource_auth_body(
        &mut self,
        internal_id: u64,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_auth(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource auth `{internal_id}`"))?;
        let PendingSubresourceAuthState {
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
            intercept_response,
            initial_network_request_headers,
            response,
        } = pending;
        let response = match initial_network_request_headers {
            Some(headers) => response.with_network_request_headers(Some(headers)),
            None => response,
        };
        let response_info = PendingSubresourceResponseInfo {
            internal_id,
            url: request_url.clone(),
            final_url: response.final_url.clone(),
            method: request_method.clone(),
            request_headers: request_headers.clone(),
            request_body: request_body.clone(),
            resource_type: pending.info.resource_type,
            request_cookie_report: response.request_cookie_report.clone(),
            network_request_headers: response
                .network_request_headers()
                .map(|headers| headers.to_vec()),
            response_status: response.status,
            response_headers: response.headers.clone(),
            response_body: SubresourceResponseBody::from_navigation_response(&response),
            from_cache: response.from_cache,
            cache_state: response.cache_state,
        };
        self._context_host
            .borrow_mut()
            .record_pending_subresource_response(PendingSubresourceResponseState {
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                response,
            });
        if intercept_response {
            self._context_host
                .borrow_mut()
                .record_pending_subresource_continue_event(
                    PendingSubresourceContinueEvent::ResponsePaused(response_info),
                );
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        self.continue_pending_subresource_response_body(internal_id, None, None)
    }
    pub(crate) fn fail_pending_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let PendingSubresourceFetchState {
            redirect_headers: _,
            request_origin,
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            deferred_request_started,
            blob_url_entry,
        } = pending;
        let pending = match continuation {
            PendingSubresourceContinuation::WebSocket(connection) => {
                self._context_host
                    .borrow_mut()
                    .fail_pending_websocket_connection(connection, error_text)
                    .map_err(|error| anyhow!(error))?;
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerFetch { .. }
            | PendingSubresourceContinuation::SharedWorkerFetch { .. }) => {
                let target = WorkerOwnedFetchTarget::from_continuation(&continuation)
                    .expect("fetch continuation target");
                let failed = self.fail_worker_owned_fetch(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        redirect_headers: None,
                        fetch_id: target.fetch_id(),
                        internal_id: 0,
                        network_request_handle: info.network_request_handle,
                        url: info.url.clone(),
                        method: info.method.clone(),
                        body: info.request_body.clone(),
                        headers: info.request_headers.clone(),
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                    error_text.clone(),
                );
                if !failed {
                    bail!(target.unavailable_message());
                }
                let request_body_bytes = info.request_body_bytes.clone();
                let request_handle = info.network_request_handle;
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::failure(
                        info.frame_id,
                        info.document_url,
                        info.url,
                        info.method,
                        info.request_headers,
                        info.request_body,
                        info.resource_type,
                        error_text,
                    ),
                    request_body_bytes,
                    request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerXhr { .. }
            | PendingSubresourceContinuation::SharedWorkerXhr { .. }) => {
                let target = WorkerOwnedXhrTarget::from_continuation(&continuation)
                    .expect("xhr continuation target");
                let failed = self.fail_worker_owned_xhr(
                    target,
                    crate::worker::WorkerPendingXhrContinue {
                        redirect_headers: None,
                        xhr_id: target.xhr_id(),
                        internal_id: 0,
                        network_request_handle: info.network_request_handle,
                        url: info.url.clone(),
                        method: info.method.clone(),
                        body: info.request_body.clone(),
                        headers: info.request_headers.clone(),
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                    error_text.clone(),
                );
                if !failed {
                    bail!(target.unavailable_message());
                }
                let request_body_bytes = info.request_body_bytes.clone();
                let request_handle = info.network_request_handle;
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::failure(
                        info.frame_id,
                        info.document_url,
                        info.url,
                        info.method,
                        info.request_headers,
                        info.request_body,
                        info.resource_type,
                        error_text,
                    ),
                    request_body_bytes,
                    request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerCspReport { .. }
            | PendingSubresourceContinuation::SharedWorkerCspReport { .. }) => {
                let target = WorkerOwnedCspReportTarget::from_continuation(&continuation)
                    .expect("CSP report continuation target");
                let failed = self.fail_worker_owned_csp_report(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        redirect_headers: None,
                        fetch_id: target.report_id(),
                        internal_id: 0,
                        network_request_handle: info.network_request_handle,
                        url: info.url.clone(),
                        method: info.method.clone(),
                        body: info.request_body.clone(),
                        headers: info.request_headers.clone(),
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                    error_text.clone(),
                );
                if !failed {
                    bail!(target.unavailable_message());
                }
                let request_body_bytes = info.request_body_bytes.clone();
                let request_handle = info.network_request_handle;
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::failure(
                        info.frame_id,
                        info.document_url,
                        info.url,
                        info.method,
                        info.request_headers,
                        info.request_body,
                        info.resource_type,
                        error_text,
                    ),
                    request_body_bytes,
                    request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation => PendingSubresourceFetchState {
                redirect_headers: None,
                request_origin,
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                deferred_request_started,
                blob_url_entry,
            },
        };
        let info = pending.info.clone();
        let activity = self.resolve_pending_subresource_fetch_body(
            pending,
            info.url,
            info.method,
            info.request_headers,
            info.request_body,
            None,
            false,
            None,
            None,
            Err(error_text),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }
    pub(crate) fn fulfill_pending_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let PendingSubresourceFetchState {
            redirect_headers: _,
            request_origin,
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            deferred_request_started,
            blob_url_entry,
        } = pending;
        // Request-stage fulfillment has no followed redirects yet. Reuse this
        // complete head for validation and response materialization.
        let head = moli_fetch::ResponseHead {
            status_text: None,
            final_url: info.url.clone(),
            status: response_code,
            headers: response_headers.clone(),
            request_cookie_report: info.request_cookie_report.clone(),
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            cache_state: Default::default(),
            negotiated_http_version: None,
        };
        let pending = match continuation {
            PendingSubresourceContinuation::WebSocket(connection) => {
                if response_code != 101 {
                    self._context_host
                        .borrow_mut()
                        .fail_pending_websocket_connection(
                            connection,
                            format!(
                                "Fetch.fulfillRequest WebSocket response must use status 101, got {response_code}"
                            ),
                        )
                        .map_err(|error| anyhow!(error))?;
                    return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
                }
                let request_url = info.url.clone();
                let request_headers = info.request_headers.clone();
                self._context_host
                    .borrow_mut()
                    .fulfill_pending_websocket_connection(
                        connection,
                        request_url,
                        request_headers,
                        response_code,
                        response_headers.clone(),
                    )
                    .map_err(|error| anyhow!(error))?;
                let _ = response_body;
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerFetch { .. }
            | PendingSubresourceContinuation::SharedWorkerFetch { .. }) => {
                let target = WorkerOwnedFetchTarget::from_continuation(&continuation)
                    .expect("fetch continuation target");
                let request_url = info.url.clone();
                let request_method = info.method.clone();
                let request_headers = info.request_headers.clone();
                let request_body = info.request_body.clone();
                let validation = if request_mode == moli_fetch::RequestMode::NoCors {
                    Ok(())
                } else {
                    crate::network_host::validate_cors_response_chain(
                        &info.document_url,
                        &head,
                        credentials_mode,
                    )
                };
                match validation {
                    Ok(()) => {
                        let fulfilled = self.fulfill_worker_owned_fetch(
                            target,
                            crate::worker::WorkerPendingFetchContinue {
                                redirect_headers: None,
                                fetch_id: target.fetch_id(),
                                internal_id: 0,
                                network_request_handle: info.network_request_handle,
                                url: request_url.clone(),
                                method: request_method.clone(),
                                body: request_body.clone(),
                                headers: request_headers.clone(),
                                intercept_response: false,
                                handle_auth_requests: false,
                                auth: None,
                            },
                            response_code,
                            response_headers.clone(),
                            response_body.clone(),
                        );
                        if !fulfilled {
                            bail!(target.unavailable_message());
                        }
                        let request_body_bytes = info.request_body_bytes.clone();
                        let request_handle = info.network_request_handle;
                        let network_record = with_pending_subresource_record_identity(
                            crate::types::SubresourceNetworkRecord::success_with_body(
                                info.frame_id,
                                info.document_url,
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                info.resource_type,
                                info.request_cookie_report,
                                Vec::new(),
                                info.url,
                                response_code,
                                response_headers,
                                response_body.into_subresource_response_body(),
                                Vec::new(),
                            ),
                            request_body_bytes,
                            request_handle,
                        );
                        self._context_host
                            .borrow_mut()
                            .record_subresource_network(network_record);
                    }
                    Err(message) => {
                        let failed = self.fail_worker_owned_fetch(
                            target,
                            crate::worker::WorkerPendingFetchContinue {
                                redirect_headers: None,
                                fetch_id: target.fetch_id(),
                                internal_id: 0,
                                network_request_handle: info.network_request_handle,
                                url: request_url.clone(),
                                method: request_method.clone(),
                                body: request_body.clone(),
                                headers: request_headers.clone(),
                                intercept_response: false,
                                handle_auth_requests: false,
                                auth: None,
                            },
                            message.clone(),
                        );
                        if !failed {
                            bail!(target.unavailable_message());
                        }
                        let request_body_bytes = info.request_body_bytes.clone();
                        let request_handle = info.network_request_handle;
                        let network_record = with_pending_subresource_record_identity(
                            crate::types::SubresourceNetworkRecord::failure(
                                info.frame_id,
                                info.document_url,
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                info.resource_type,
                                message,
                            ),
                            request_body_bytes,
                            request_handle,
                        );
                        self._context_host
                            .borrow_mut()
                            .record_subresource_network(network_record);
                    }
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerXhr { .. }
            | PendingSubresourceContinuation::SharedWorkerXhr { .. }) => {
                let target = WorkerOwnedXhrTarget::from_continuation(&continuation)
                    .expect("xhr continuation target");
                let request_url = info.url.clone();
                let request_method = info.method.clone();
                let request_headers = info.request_headers.clone();
                let request_body = info.request_body.clone();
                let validation = crate::network_host::validate_cors_response_chain(
                    &info.document_url,
                    &head,
                    credentials_mode,
                );
                match validation {
                    Ok(()) => {
                        let fulfilled = self.fulfill_worker_owned_xhr(
                            target,
                            crate::worker::WorkerPendingXhrContinue {
                                redirect_headers: None,
                                xhr_id: target.xhr_id(),
                                internal_id: 0,
                                network_request_handle: info.network_request_handle,
                                url: request_url.clone(),
                                method: request_method.clone(),
                                body: request_body.clone(),
                                headers: request_headers.clone(),
                                intercept_response: false,
                                handle_auth_requests: false,
                                auth: None,
                            },
                            response_code,
                            response_headers.clone(),
                            response_body.clone(),
                        );
                        if !fulfilled {
                            bail!(target.unavailable_message());
                        }
                        let request_body_bytes = info.request_body_bytes.clone();
                        let request_handle = info.network_request_handle;
                        let network_record = with_pending_subresource_record_identity(
                            crate::types::SubresourceNetworkRecord::success_with_body(
                                info.frame_id,
                                info.document_url,
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                info.resource_type,
                                info.request_cookie_report,
                                Vec::new(),
                                info.url,
                                response_code,
                                response_headers,
                                response_body.into_subresource_response_body(),
                                Vec::new(),
                            ),
                            request_body_bytes,
                            request_handle,
                        );
                        self._context_host
                            .borrow_mut()
                            .record_subresource_network(network_record);
                    }
                    Err(message) => {
                        let failed = self.fail_worker_owned_xhr(
                            target,
                            crate::worker::WorkerPendingXhrContinue {
                                redirect_headers: None,
                                xhr_id: target.xhr_id(),
                                internal_id: 0,
                                network_request_handle: info.network_request_handle,
                                url: request_url.clone(),
                                method: request_method.clone(),
                                body: request_body.clone(),
                                headers: request_headers.clone(),
                                intercept_response: false,
                                handle_auth_requests: false,
                                auth: None,
                            },
                            message.clone(),
                        );
                        if !failed {
                            bail!(target.unavailable_message());
                        }
                        let request_body_bytes = info.request_body_bytes.clone();
                        let request_handle = info.network_request_handle;
                        let network_record = with_pending_subresource_record_identity(
                            crate::types::SubresourceNetworkRecord::failure(
                                info.frame_id,
                                info.document_url,
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                info.resource_type,
                                message,
                            ),
                            request_body_bytes,
                            request_handle,
                        );
                        self._context_host
                            .borrow_mut()
                            .record_subresource_network(network_record);
                    }
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerCspReport { .. }
            | PendingSubresourceContinuation::SharedWorkerCspReport { .. }) => {
                let target = WorkerOwnedCspReportTarget::from_continuation(&continuation)
                    .expect("CSP report continuation target");
                let request_url = info.url.clone();
                let request_method = info.method.clone();
                let request_headers = info.request_headers.clone();
                let request_body = info.request_body.clone();
                let fulfilled = self.fulfill_worker_owned_csp_report(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        redirect_headers: None,
                        fetch_id: target.report_id(),
                        internal_id: 0,
                        network_request_handle: info.network_request_handle,
                        url: request_url.clone(),
                        method: request_method.clone(),
                        body: request_body.clone(),
                        headers: request_headers.clone(),
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                    response_code,
                    response_headers.clone(),
                    response_body.clone(),
                );
                if !fulfilled {
                    bail!(target.unavailable_message());
                }
                let request_body_bytes = info.request_body_bytes.clone();
                let request_handle = info.network_request_handle;
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::success_with_body(
                        info.frame_id,
                        info.document_url,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                        info.resource_type,
                        info.request_cookie_report,
                        Vec::new(),
                        info.url,
                        response_code,
                        response_headers,
                        response_body.into_subresource_response_body(),
                        Vec::new(),
                    ),
                    request_body_bytes,
                    request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation => PendingSubresourceFetchState {
                redirect_headers: None,
                request_origin,
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                deferred_request_started,
                blob_url_entry,
            },
        };
        let info = pending.info.clone();
        let activity = self.resolve_pending_subresource_fetch_body(
            pending,
            info.url.clone(),
            info.method,
            info.request_headers,
            info.request_body,
            None,
            false,
            None,
            None,
            Ok(response_body.into_navigation_response(head)),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }
    pub(in crate::script_vm) fn record_subresource_fetch_network_result(
        &mut self,
        pending: &PendingSubresourceFetchState,
        request_url: &Url,
        request_method: &str,
        request_headers: &moli_fetch::RequestHeaders,
        request_body: &Option<String>,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        match result {
            Ok(response) => {
                let request_cookie_report = response
                    .request_cookie_report
                    .clone()
                    .or_else(|| pending.info.request_cookie_report.clone());
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::success_with_body(
                        pending.info.frame_id.clone(),
                        pending.info.document_url.clone(),
                        request_url.clone(),
                        request_method.to_owned(),
                        request_headers.clone(),
                        request_body.clone(),
                        pending.info.resource_type,
                        request_cookie_report,
                        response.redirect_chain.clone().into_iter().collect(),
                        response.final_url.clone(),
                        response.status,
                        response.headers.clone(),
                        SubresourceResponseBody::from_navigation_response(response),
                        response.cookie_set_reports.clone(),
                    )
                    .with_from_cache(response.from_cache)
                    .with_negotiated_http_version(response.negotiated_http_version)
                    .with_network_request_headers(
                        response
                            .network_request_headers()
                            .map(|headers| headers.to_vec()),
                    ),
                    pending.info.request_body_bytes.clone(),
                    pending.info.network_request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
            Err(error_text) => {
                let network_error_text =
                    if crate::network_host::is_cors_policy_failure_message(error_text) {
                        crate::network_host::FAILED_ERROR_TEXT.to_owned()
                    } else {
                        error_text.clone()
                    };
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::failure(
                        pending.info.frame_id.clone(),
                        pending.info.document_url.clone(),
                        request_url.clone(),
                        request_method.to_owned(),
                        request_headers.clone(),
                        request_body.clone(),
                        pending.info.resource_type,
                        network_error_text,
                    ),
                    pending.info.request_body_bytes.clone(),
                    pending.info.network_request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
        }
    }
    pub(crate) fn continue_pending_subresource_response_body(
        &mut self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending_websocket_response = {
            self._context_host
                .borrow_mut()
                .take_pending_websocket_response(internal_id)
        };
        if let Some(pending) = pending_websocket_response {
            if response_code.is_some_and(|status| status != 101) {
                self._context_host
                    .borrow_mut()
                    .fail_websocket_handshake_response(
                        pending,
                        format!(
                            "Fetch.continueResponse WebSocket response must use status 101, got {}",
                            response_code.unwrap()
                        ),
                    )
                    .map_err(|error| anyhow!(error))?;
                bail!(
                    "Fetch.continueResponse WebSocket response must use status 101, got {}",
                    response_code.unwrap()
                );
            }
            self._context_host
                .borrow_mut()
                .continue_websocket_handshake_response(pending, response_code, response_headers)
                .map_err(|error| anyhow!(error))?;
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_response(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource response `{internal_id}`"))?;
        if let Some(target) =
            WorkerOwnedFetchTarget::from_continuation(&pending.pending.continuation)
        {
            let response = crate::protocol_types::NavigationResponse::with_status_headers_from(
                &pending.response,
                response_code.unwrap_or(pending.response.status),
                response_headers
                    .clone()
                    .unwrap_or_else(|| pending.response.headers.clone()),
            );
            let result = if pending.pending.request_mode == moli_fetch::RequestMode::NoCors {
                Ok(response)
            } else {
                crate::network_host::validate_cors_response_chain(
                    &pending.pending.info.document_url,
                    &response.head(),
                    pending.pending.credentials_mode,
                )
                .map(|()| response)
            };
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingFetchContinue {
                redirect_headers: None,
                fetch_id: target.fetch_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            match result {
                Ok(_) => {
                    let continued = self.continue_worker_owned_fetch_response(
                        target,
                        request,
                        response_code,
                        response_headers,
                    );
                    if !continued {
                        bail!(target.unavailable_message());
                    }
                }
                Err(message) => {
                    let failed = self.fail_worker_owned_fetch_response(target, request, message);
                    if !failed {
                        bail!(target.unavailable_message());
                    }
                }
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending.pending.continuation)
        {
            let response = crate::protocol_types::NavigationResponse::with_status_headers_from(
                &pending.response,
                response_code.unwrap_or(pending.response.status),
                response_headers
                    .clone()
                    .unwrap_or_else(|| pending.response.headers.clone()),
            );
            let result = if pending.pending.request_mode == moli_fetch::RequestMode::NoCors {
                Ok(response)
            } else {
                crate::network_host::validate_cors_response_chain(
                    &pending.pending.info.document_url,
                    &response.head(),
                    pending.pending.credentials_mode,
                )
                .map(|()| response)
            };
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingXhrContinue {
                redirect_headers: None,
                xhr_id: target.xhr_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            match result {
                Ok(_) => {
                    let continued = self.continue_worker_owned_xhr_response(
                        target,
                        request,
                        response_code,
                        response_headers,
                    );
                    if !continued {
                        bail!(target.unavailable_message());
                    }
                }
                Err(message) => {
                    let failed = self.fail_worker_owned_xhr_response(target, request, message);
                    if !failed {
                        bail!(target.unavailable_message());
                    }
                }
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let response = crate::protocol_types::NavigationResponse::with_status_headers_from(
            &pending.response,
            response_code.unwrap_or(pending.response.status),
            response_headers.unwrap_or_else(|| pending.response.headers.clone()),
        );
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            None,
            false,
            None,
            None,
            Ok(response),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }
    pub(crate) fn fail_pending_subresource_response_body(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending_websocket_response = {
            self._context_host
                .borrow_mut()
                .take_pending_websocket_response(internal_id)
        };
        if let Some(pending) = pending_websocket_response {
            self._context_host
                .borrow_mut()
                .fail_websocket_handshake_response(pending, error_text)
                .map_err(|error| anyhow!(error))?;
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_response(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource response `{internal_id}`"))?;
        if let Some(target) =
            WorkerOwnedFetchTarget::from_continuation(&pending.pending.continuation)
        {
            let result = Err(error_text.clone());
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let failed = self.fail_worker_owned_fetch_response(
                target,
                crate::worker::WorkerPendingFetchContinue {
                    redirect_headers: None,
                    fetch_id: target.fetch_id(),
                    internal_id,
                    network_request_handle: pending.pending.info.network_request_handle,
                    url: pending.request_url,
                    method: pending.request_method,
                    body: pending.request_body,
                    headers: pending.request_headers,
                    intercept_response: false,
                    handle_auth_requests: false,
                    auth: None,
                },
                error_text,
            );
            if !failed {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending.pending.continuation)
        {
            let result = Err(error_text.clone());
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingXhrContinue {
                redirect_headers: None,
                xhr_id: target.xhr_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            let failed = self.fail_worker_owned_xhr_response(target, request, error_text);
            if !failed {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            None,
            false,
            None,
            None,
            Err(error_text),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }
    pub(crate) fn fulfill_pending_subresource_response_body(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending_websocket_response = {
            self._context_host
                .borrow_mut()
                .take_pending_websocket_response(internal_id)
        };
        if let Some(pending) = pending_websocket_response {
            if response_code != 101 {
                self._context_host
                    .borrow_mut()
                    .fail_websocket_handshake_response(
                        pending,
                        format!(
                            "Fetch.fulfillRequest WebSocket response must use status 101, got {response_code}"
                        ),
                    )
                    .map_err(|error| anyhow!(error))?;
                bail!(
                    "Fetch.fulfillRequest WebSocket response must use status 101, got {response_code}"
                );
            }
            let _ = response_body;
            self._context_host
                .borrow_mut()
                .continue_websocket_handshake_response(
                    pending,
                    Some(response_code),
                    Some(response_headers),
                )
                .map_err(|error| anyhow!(error))?;
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_response(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource response `{internal_id}`"))?;
        if let Some(target) =
            WorkerOwnedFetchTarget::from_continuation(&pending.pending.continuation)
        {
            let response = response_body.clone_as_navigation_response(moli_fetch::ResponseHead {
                status_text: None,
                final_url: pending.response.final_url.clone(),
                status: response_code,
                headers: response_headers.clone(),
                request_cookie_report: pending.response.request_cookie_report.clone(),
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: pending.response.from_cache,
                cache_state: pending.response.cache_state,
                negotiated_http_version: pending.response.negotiated_http_version,
            });
            let result = if pending.pending.request_mode == moli_fetch::RequestMode::NoCors {
                Ok(response)
            } else {
                crate::network_host::validate_cors_response_chain(
                    &pending.pending.info.document_url,
                    &response.head(),
                    pending.pending.credentials_mode,
                )
                .map(|()| response)
            };
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingFetchContinue {
                redirect_headers: None,
                fetch_id: target.fetch_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            if let Err(message) = result {
                let failed = self.fail_worker_owned_fetch_response(target, request, message);
                if !failed {
                    bail!(target.unavailable_message());
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            let fulfilled = self.fulfill_worker_owned_fetch_response(
                target,
                request,
                response_code,
                response_headers,
                response_body,
            );
            if !fulfilled {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending.pending.continuation)
        {
            let response = response_body.clone_as_navigation_response(moli_fetch::ResponseHead {
                status_text: None,
                final_url: pending.response.final_url.clone(),
                status: response_code,
                headers: response_headers.clone(),
                request_cookie_report: pending.response.request_cookie_report.clone(),
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: pending.response.from_cache,
                cache_state: pending.response.cache_state,
                negotiated_http_version: pending.response.negotiated_http_version,
            });
            let result = if pending.pending.request_mode == moli_fetch::RequestMode::NoCors {
                Ok(response)
            } else {
                crate::network_host::validate_cors_response_chain(
                    &pending.pending.info.document_url,
                    &response.head(),
                    pending.pending.credentials_mode,
                )
                .map(|()| response)
            };
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingXhrContinue {
                redirect_headers: None,
                xhr_id: target.xhr_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            if let Err(message) = result {
                let failed = self.fail_worker_owned_xhr_response(target, request, message);
                if !failed {
                    bail!(target.unavailable_message());
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            let fulfilled = self.fulfill_worker_owned_xhr_response(
                target,
                request,
                response_code,
                response_headers,
                response_body,
            );
            if !fulfilled {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            None,
            false,
            None,
            None,
            Ok(
                response_body.into_navigation_response(moli_fetch::ResponseHead {
                    status_text: None,
                    final_url: pending.response.final_url,
                    status: response_code,
                    headers: response_headers,
                    request_cookie_report: pending.response.request_cookie_report,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: pending.response.from_cache,
                    cache_state: pending.response.cache_state,
                    negotiated_http_version: pending.response.negotiated_http_version,
                }),
            ),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }
    /// Standalone ScriptVm compatibility turn for low-level producer tests.
    /// Page behavior must use `PageVm`'s command coordinator instead.
    #[cfg(test)]
    pub(crate) fn continue_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<moli_fetch::RequestHeaderOverride>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingSubresourceContinueOutcome> {
        let execution = self.continue_pending_subresource_fetch_body(
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        )?;
        self.finish_async_subresource_command_for_test(execution)
    }
    /// Standalone ScriptVm compatibility turn for low-level producer tests.
    /// Page behavior must use `PageVm`'s command coordinator instead.
    #[cfg(test)]
    pub(crate) fn fulfill_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<()> {
        let execution = self.fulfill_pending_subresource_fetch_body(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )?;
        self.finish_async_subresource_command_for_test(execution)
    }
    #[cfg(test)]
    pub(in crate::script_vm) fn eval_in_isolated_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<String> {
        let (context_ptr, sync_child_records): (*const v8::Global<v8::Context>, bool) = {
            let world = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            (&world.context as *const _, world.child_handle.is_some())
        };
        self.eval_string_in_context_ptr_runtime_turn(context_ptr, source, sync_child_records)
    }
    pub(in crate::script_vm) fn exec_in_isolated_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<()> {
        let (context_ptr, sync_child_records): (*const v8::Global<v8::Context>, bool) = {
            let world = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            (&world.context as *const _, world.child_handle.is_some())
        };
        self.exec_in_context_ptr_runtime_turn(
            context_ptr,
            source,
            None,
            0,
            true,
            sync_child_records,
        )
    }
}
