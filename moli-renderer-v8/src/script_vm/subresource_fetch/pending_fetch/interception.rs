use super::*;
use crate::{PendingSubresourceContinueOutcome, RendererSyntheticResponseBody};
use anyhow::bail;

impl PendingSubresourceFetchState {
    fn apply_request_overrides(&mut self, overrides: moli_fetch::RequestOverrides) {
        let mut request = request_for_pending_subresource(
            self,
            &self.info.url,
            &self.info.method,
            self.info.request_headers.clone(),
            self.info.request_body_bytes.clone(),
        );
        overrides.apply(&mut request);
        self.update_request_snapshot(request);
    }
}

impl ScriptVm {
    pub(crate) fn continue_pending_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<bytes::Bytes>>,
        headers: Option<moli_fetch::RequestHeaderOverride>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let mut pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let headers_overridden = headers.is_some();
        pending.apply_request_overrides(moli_fetch::RequestOverrides {
            url,
            method,
            body,
            headers,
        });
        let PendingSubresourceFetchState {
            dispatch,
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
        } = pending;
        let pending = match continuation {
            PendingSubresourceContinuation::WebSocket(connection) => {
                let request_url = info.url.clone();
                let request_headers = info.request_headers.clone();
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

            PendingSubresourceContinuation::CspReport { client_id } => {
                let request_url = info.url.clone();
                let request_method = info.method.clone();
                let request_body_bytes = info.request_body_bytes.clone();
                let request_body = info.request_body.clone();
                let request_headers = info.request_headers.clone();
                let pending = PendingSubresourceFetchState {
                    dispatch,
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
                dispatch,
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
            },
        };
        let request_url = pending.info.url.clone();
        let request_method = pending.info.method.clone();
        let request_body = pending.info.request_body.clone();
        let request_headers = pending.info.request_headers.clone();
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
        if let PendingSubresourceContinuation::CaptureResource(capture) = &pending.continuation
            && (!capture.endpoint.is_active()
                || !window_subresource_owner_is_current(&self._context_host, &pending))
        {
            bail!("capture resource Document owner changed while request was paused");
        }
        // Captured resource requests choose SW before enforcing network
        // availability, just like direct dispatch. An offline worker can still
        // serve a synthetic/cached response; only network fallback must fail.
        if pending.dispatch.is_none() && pending.load.network_offline() {
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
        if matches!(
            &pending.continuation,
            PendingSubresourceContinuation::ChildDocument(_)
        ) {
            self.spawn_running_child_document_fetch(
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
                None,
            );
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                PendingSubresourceContinueOutcome::Started,
            ));
        }
        // Request interception may resume after the initiating Document has
        // navigated. The lease retains the request-time frozen client; looking
        // up the ambient Page loader here would silently rebind policy/backend
        // to a newer Document identity.
        let loader = pending.load.request_client();
        let request = request_for_pending_subresource(
            &pending,
            &request_url,
            &request_method,
            request_headers.clone(),
            pending.info.request_body_bytes.clone(),
        );
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        pending.load.attach_cancel_handle(cancel_handle.clone());
        let state = RunningSubresourceFetchState {
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
            intercept_response,
            handle_auth_requests,
            initial_auth_network_request_headers: None,
        };
        if let Some(dispatch) = &state.pending.dispatch {
            let dispatch = dispatch.with_request(request);
            self.spawn_running_resource_request(dispatch, state, cancel_handle);
        } else {
            self.spawn_running_subresource_fetch(loader, request, state, Some(cancel_handle));
        }
        Ok(AsyncSubresourceCommandExecution::without_window_realm(
            PendingSubresourceContinueOutcome::Started,
        ))
    }

    fn spawn_running_child_document_fetch(
        &mut self,
        mut state: RunningSubresourceFetchState,
        auth: Option<crate::SubresourceAuthCredentials>,
    ) {
        let PendingSubresourceContinuation::ChildDocument(fetch) = &mut state.pending.continuation
        else {
            unreachable!("child document transfer requires its navigation owner");
        };
        fetch.apply_request_override(
            state.pending.redirect_headers.clone(),
            state.request_url.clone(),
            state.request_method.clone(),
            state.request_headers.clone(),
            state.pending.info.request_body_bytes.clone(),
            state
                .pending
                .info
                .request_body
                .as_deref()
                .map(std::sync::Arc::from),
        );
        let prepared = match auth {
            Some(auth) => fetch.set_auth(auth),
            None => Ok(()),
        };
        let response = fetch.response_future();
        self.spawn_running_subresource_response(state, async move {
            match prepared {
                Ok(()) => response.await,
                Err(error) => AsyncSubresourceFetchResult::Failure(error),
            }
        });
    }

    fn continue_csp_report_via_service_worker(
        &mut self,
        pending: PendingSubresourceFetchState,
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
        request_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
        request_body_bytes: Option<bytes::Bytes>,
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
                request.redirect_mode(),
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

    #[cfg(feature = "devtools")]
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
        if !pending.pending.load.try_restart_with_auth() {
            // Reuse the terminal auth path to settle Window and Worker consumers.
            self._context_host
                .borrow_mut()
                .record_pending_subresource_auth(pending);
            let execution = self.fail_pending_subresource_auth_body(
                internal_id,
                moli_fetch::AuthRetryBudget::ERROR_TEXT.to_owned(),
            )?;
            let ((), activity, post_checkpoint_event) = execution.into_parts();
            let mut completion = AsyncSubresourceCommandExecution::after_body(
                PendingSubresourceContinueOutcome::Started,
                activity,
            );
            if let Some(event) = post_checkpoint_event {
                completion = completion.with_post_checkpoint_event(event);
            } else {
                // Worker auth failure clears its network record, so its IPC
                // completion does not retire this parent protocol correlation.
                completion = completion.with_post_checkpoint_event(
                    PendingSubresourceContinueEvent::Completed { internal_id },
                );
            }
            return Ok(completion);
        }
        let PendingSubresourceAuthState {
            pending: pending_fetch,
            response,
        } = pending;
        let PendingSubresourceAuthResponse {
            request_url,
            request_method,
            request_headers: original_request_headers,
            request_body,
            intercept_response,
            initial_network_request_headers,
            response,
            ..
        } = response;
        if matches!(
            &pending_fetch.continuation,
            PendingSubresourceContinuation::ChildDocument(_)
        ) {
            self.spawn_running_child_document_fetch(
                RunningSubresourceFetchState {
                    pending: pending_fetch,
                    request_url,
                    request_method,
                    request_headers: original_request_headers,
                    request_body,
                    intercept_response,
                    handle_auth_requests: true,
                    initial_auth_network_request_headers: initial_network_request_headers,
                },
                Some(auth),
            );
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                PendingSubresourceContinueOutcome::Started,
            ));
        }
        // A redirect may have discarded the upload before the auth challenge.

        let loader = pending_fetch.load.request_client();
        let mut request = request_for_pending_subresource(
            &pending_fetch,
            &request_url,
            &request_method,
            original_request_headers.clone(),
            request_body
                .as_ref()
                .and_then(|_| pending_fetch.info.request_body_bytes.clone()),
        );
        for redirect in &response.redirect_chain {
            request.apply_redirect_status(redirect.status);
        }
        request.url = response.final_url.clone();
        request.set_auth(Some(auth.into()));
        request = request.with_redirect_chain(
            response
                .redirect_chain
                .into_iter()
                .map(Into::into)
                .collect(),
        );
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

    #[cfg(feature = "devtools")]
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

        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.response.request_url,
            pending.response.request_method,
            pending.response.request_headers,
            pending.response.request_body,
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
        let PendingSubresourceAuthState { pending, response } = pending;
        let PendingSubresourceAuthResponse {
            request_url,
            request_method,
            request_headers,
            request_body,
            intercept_response,
            initial_network_request_headers,
            response,
            encoded,
            response_status_text,
            skip_fetch_security_validation,
            response_filter,
        } = response;
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
            response_body: encoded.as_ref().map_or_else(
                || SubresourceResponseBody::from_navigation_response(&response),
                |image| SubresourceResponseBody::from_parkable_image(image.clone()),
            ),
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
                details: PendingSubresourceResponseDetails {
                    encoded,
                    response_status_text,
                    skip_fetch_security_validation,
                    response_filter,
                },
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
            dispatch,
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
        } = pending;
        let pending = match continuation {
            PendingSubresourceContinuation::ChildDocument(fetch) => {
                let mut host = self._context_host.borrow_mut();
                host.fail_child_document_fetch(internal_id, *fetch, error_text);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            PendingSubresourceContinuation::WebSocket(connection) => {
                self._context_host
                    .borrow_mut()
                    .fail_pending_websocket_connection(connection, error_text)
                    .map_err(|error| anyhow!(error))?;
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }

            continuation => PendingSubresourceFetchState {
                dispatch,
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
        request_overrides: Option<moli_fetch::RequestOverrides>,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let mut pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        if let Some(overrides) = request_overrides {
            pending.apply_request_overrides(overrides);
            pending.info.request_cookie_report = None;
        }
        if pending.continuation.window_fetch().is_some() {
            return self.fulfill_intercepted_window_fetch(
                pending,
                response_code,
                response_headers,
                response_body,
            );
        }

        let PendingSubresourceFetchState {
            dispatch,
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
            PendingSubresourceContinuation::ChildDocument(fetch) => {
                self._context_host
                    .borrow_mut()
                    .fulfill_child_document_fetch(
                        internal_id,
                        *fetch,
                        head,
                        response_body.bytes().to_vec(),
                    );
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
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

            continuation => PendingSubresourceFetchState {
                dispatch,
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

        let response = crate::protocol_types::NavigationResponse::with_status_headers_from(
            &pending.response,
            response_code.unwrap_or(pending.response.status),
            response_headers.unwrap_or_else(|| pending.response.headers.clone()),
        );
        let response_status_text = if response_code.is_none() {
            pending.details.response_status_text
        } else {
            None
        };
        let activity = self.resolve_pending_subresource_fetch_completion(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            response_status_text,
            pending.details.skip_fetch_security_validation,
            pending.details.response_filter,
            None,
            AsyncSubresourceFetchResult::from_image_parts(Ok(response), pending.details.encoded),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    #[cfg(feature = "devtools")]
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

    #[cfg(feature = "devtools")]
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
        let mut pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_response(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource response `{internal_id}`"))?;

        if let PendingSubresourceContinuation::ChildDocument(fetch) =
            &mut pending.pending.continuation
        {
            fetch.discard_response_body();
        }

        let network_response_extra_info = pending.response.network_response_extra_info().cloned();
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
            Ok(response_body
                .into_navigation_response(moli_fetch::ResponseHead {
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
                })
                .with_network_response_extra_info(network_response_extra_info)),
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
        body: Option<Option<bytes::Bytes>>,
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
            None,
        )?;
        self.finish_async_subresource_command_for_test(execution)
    }
}

impl ScriptVm {
    fn spawn_running_resource_request(
        &mut self,
        dispatch: crate::network_host::SubresourceRequestDispatch,
        state: RunningSubresourceFetchState,
        cancel_handle: moli_fetch::FetchCancelHandle,
    ) {
        let internal_id = state.pending.info.internal_id;
        let loader = state.pending.load.request_client();
        let task_runner = state.pending.load.task_runner();
        let network_context = crate::types::AsyncSubresourceNetworkContext {
            frame_id: state.pending.info.frame_id.clone(),
            document_url: state.pending.info.document_url.clone(),
            request_origin: state.pending.request_origin.clone(),
            resource_type: state.pending.info.resource_type,
            policy_context: state.pending.policy_context,
        };
        let cookie_report = state.pending.info.request_cookie_report.clone();
        let mut host = self._context_host.borrow_mut();
        host.begin_active_subresource_request();
        host.record_running_subresource_fetch(state);
        dispatch.dispatch(
            &host,
            internal_id,
            loader,
            task_runner,
            network_context,
            cookie_report,
            cancel_handle,
        );
    }

    pub(in crate::script_vm) fn spawn_running_subresource_fetch(
        &mut self,
        request_client: ResourceRequestClient,
        request: moli_fetch::Request,
        mut state: RunningSubresourceFetchState,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) {
        let intercepted_window = state
            .pending
            .continuation
            .window_fetch()
            .is_some_and(|fetch| fetch.intercepted_request.is_some());
        if intercepted_window {
            state.pending.update_request_snapshot(request.clone());
        }
        let request = self
            ._context_host
            .borrow()
            .resource_completion_sender()
            .observe_subresource_request(request, state.pending.info.internal_id);
        let response = async move {
            if intercepted_window {
                crate::network_host::fetch_intercepted_window_hop(
                    request_client,
                    request,
                    cancel_handle,
                )
                .await
            } else {
                crate::network_host::fetch_browser_subresource_with_preflight_and_network_metadata(
                    request_client,
                    request,
                    cancel_handle,
                )
                .await
                .map(|observed| {
                    let (response, request_observation) = observed.into_parts();
                    crate::protocol_types::NavigationResponse::from(response)
                        .with_network_request_headers(
                            request_observation.map(|observation| observation.into_headers()),
                        )
                })
            }
        };
        self.spawn_running_subresource_response(state, response);
    }

    fn spawn_running_subresource_response<R: Into<AsyncSubresourceFetchResult> + Send + 'static>(
        &mut self,
        state: RunningSubresourceFetchState,
        response: impl std::future::Future<Output = R> + Send + 'static,
    ) {
        let task_runner = state.pending.load.task_runner();
        let internal_id = state.pending.info.internal_id;
        let request_url = state.request_url.clone();
        let request_method = state.request_method.clone();
        let request_headers = state.request_headers.clone();
        let request_body = state.request_body.clone();
        let completion_tx = self._context_host.borrow().resource_completion_sender();
        {
            let mut host = self._context_host.borrow_mut();
            host.begin_active_subresource_request();
            host.record_running_subresource_fetch(state);
        }
        task_runner.spawn(async move {
            let result = response.await;
            let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: result.into(),
            });
        });
    }
}
