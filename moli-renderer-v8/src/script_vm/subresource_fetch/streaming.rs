use super::*;

impl ScriptVm {
    pub(in crate::script_vm) fn spawn_running_subresource_fetch(
        &mut self,
        request_client: ResourceRequestClient,
        mut request: moli_fetch::Request,
        state: RunningSubresourceFetchState,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) {
        let task_runner = state.pending.load.task_runner();
        let internal_id = state.pending.info.internal_id;
        let request_url = state.request_url.clone();
        let request_method = state.request_method.clone();
        let request_headers = state.request_headers.clone();
        let request_body = state.request_body.clone();
        let completion_tx = self._context_host.borrow().resource_completion_sender();
        let local_response = crate::network_host::local_url_response_with_blob_entry(
            &request.url,
            &request.method,
            &request.request_headers.to_byte_strings(),
            state.pending.blob_url_entry.as_ref(),
        )
        .map(|result| result.map_err(|error| error.into_message()));
        {
            let mut host = self._context_host.borrow_mut();
            host.begin_active_subresource_request();
            host.record_running_subresource_fetch(state);
            if let Some(check) = host.window_fetch_redirect_check(internal_id) {
                request = request.with_redirect_check(check);
            }
        }
        let request =
            crate::network_host::observe_async_xhr_upload(request, &completion_tx, internal_id);
        task_runner.spawn(async move {
            let result = if let Some(result) = local_response {
                result.map(crate::protocol_types::NavigationResponse::from)
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
            };
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
    pub(super) fn complete_running_subresource_fetch_body(
        &mut self,
        running: RunningSubresourceFetchState,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        response_filter: Option<AsyncSubresourceFetchResponseFilter>,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let RunningSubresourceFetchState {
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
            intercept_response,
            handle_auth_requests,
            initial_auth_network_request_headers,
        } = running;
        let internal_id = pending.info.internal_id;
        let resource_type = pending.info.resource_type;
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields =
            async_subresource_trace_fields_for_pending("completion", internal_id, &pending);
        trace_async_subresource_stage(
            "async_subresource_complete_running_start",
            trace_fields,
            trace_started,
        );

        let activity = match result {
            Ok(response) => {
                let response = if let Some(headers) = initial_auth_network_request_headers.clone() {
                    response.with_network_request_headers(Some(headers))
                } else {
                    response
                };
                if handle_auth_requests
                    && matches!(response.status, 401 | 407)
                    && let Some(challenge) =
                        crate::network_host::extract_subresource_auth_challenge(&response.headers)
                {
                    // Keep the original request for Network event replay; only the
                    // challenged hop is used by Fetch.authRequired and auth retries.
                    let mut challenged_request = moli_fetch::Request::new(
                        &request_method,
                        request_url.as_str(),
                        request_body.clone(),
                        request_headers.clone(),
                    )?
                    .with_redirect_headers(pending.redirect_headers.clone());
                    for redirect in &response.redirect_chain {
                        challenged_request.apply_redirect_status(redirect.status);
                    }
                    let challenged_body =
                        challenged_request.body.as_ref().and(request_body.clone());
                    let info = PendingSubresourceAuthInfo {
                        internal_id,
                        url: response.final_url.clone(),
                        method: challenged_request.method,
                        request_headers: challenged_request.request_headers,
                        request_body: challenged_body,
                        resource_type,
                        request_cookie_report: response.request_cookie_report.clone(),
                        network_request_headers: response
                            .network_request_headers()
                            .map(|headers| headers.to_vec()),
                        challenge,
                        intercept_response,
                        response_final_url: response.final_url.clone(),
                        response_status: response.status,
                        response_headers: response.headers.clone(),
                        response_body: SubresourceResponseBody::from_navigation_response(&response),
                        response_from_cache: response.from_cache,
                        response_cache_state: response.cache_state,
                        response_preload_state: response.preload_state.clone(),
                    };
                    trace_async_subresource_stage(
                        "async_subresource_complete_running_auth_required",
                        trace_fields,
                        trace_started,
                    );
                    self._context_host
                        .borrow_mut()
                        .record_pending_subresource_auth(PendingSubresourceAuthState {
                            pending,
                            request_url,
                            request_method,
                            request_headers,
                            request_body,
                            intercept_response,
                            initial_network_request_headers: initial_auth_network_request_headers
                                .or_else(|| {
                                    response
                                        .network_request_headers()
                                        .map(|headers| headers.to_vec())
                                }),
                            response,
                        });
                    self._context_host
                        .borrow_mut()
                        .record_pending_subresource_continue_event(
                            PendingSubresourceContinueEvent::AuthRequired(info),
                        );
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }

                if intercept_response {
                    trace_async_subresource_stage(
                        "async_subresource_complete_running_response_paused",
                        trace_fields,
                        trace_started,
                    );
                    let info = PendingSubresourceResponseInfo {
                        internal_id,
                        url: request_url.clone(),
                        final_url: response.final_url.clone(),
                        method: request_method.clone(),
                        request_headers: request_headers.clone(),
                        request_body: request_body.clone(),
                        resource_type,
                        request_cookie_report: response.request_cookie_report.clone(),
                        network_request_headers: response
                            .network_request_headers()
                            .map(|headers| headers.to_vec()),
                        response_status: response.status,
                        response_headers: response.headers.clone(),
                        response_body: SubresourceResponseBody::from_navigation_response(&response),
                        from_cache: response.from_cache,
                        cache_state: response.cache_state,
                        preload_state: response.preload_state.clone(),
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
                    self._context_host
                        .borrow_mut()
                        .record_pending_subresource_continue_event(
                            PendingSubresourceContinueEvent::ResponsePaused(info),
                        );
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }

                let activity = self.resolve_pending_subresource_fetch_body(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    response_status_text.clone(),
                    skip_fetch_security_validation,
                    response_filter,
                    network_error_text.clone(),
                    Ok(response),
                );
                trace_async_subresource_stage(
                    "async_subresource_complete_running_done",
                    trace_fields,
                    trace_started,
                );
                activity?
            }
            Err(error) => {
                let activity = self.resolve_pending_subresource_fetch_body(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    response_status_text,
                    skip_fetch_security_validation,
                    response_filter,
                    network_error_text,
                    Err(error),
                );
                trace_async_subresource_stage(
                    "async_subresource_complete_running_done",
                    trace_fields,
                    trace_started,
                );
                activity?
            }
        };
        self._context_host
            .borrow_mut()
            .record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
        Ok(activity)
    }
    /// Standalone ScriptVm test turn: apply one terminal and immediately submit
    /// the checkpoint that production owns in the selected Networking task.
    #[cfg(test)]
    pub(crate) fn complete_async_subresource_fetch(
        &mut self,
        completion: AsyncSubresourceFetchCompletion,
    ) -> Result<()> {
        let activity = self.complete_async_subresource_fetch_body(completion)?;
        self.finish_async_subresource_body_checkpoint_for_test(activity)
    }
    pub(super) fn complete_async_subresource_fetch_body(
        &mut self,
        completion: AsyncSubresourceFetchCompletion,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let internal_id = completion.internal_id;
        trace_async_subresource_stage(
            "async_subresource_complete_start",
            AsyncSubresourceTraceFields {
                event_kind: Some("completion"),
                internal_id: Some(internal_id),
                ..AsyncSubresourceTraceFields::default()
            },
            trace_started,
        );
        let running = {
            self._context_host
                .borrow_mut()
                .take_running_subresource_fetch(completion.internal_id)
        };
        if let Some(running) = running {
            let trace_fields = async_subresource_trace_fields_for_pending(
                "completion",
                internal_id,
                &running.pending,
            );
            trace_async_subresource_stage(
                "async_subresource_complete_running",
                trace_fields,
                trace_started,
            );
            self._context_host
                .borrow_mut()
                .finish_active_subresource_request();
            let result = self.complete_running_subresource_fetch_body(
                running,
                completion.response_status_text,
                completion.skip_fetch_security_validation,
                completion.response_filter,
                completion.network_error_text,
                completion.result.into_result(),
            );
            trace_async_subresource_stage(
                "async_subresource_complete_done",
                trace_fields,
                trace_started,
            );
            return result;
        }
        let Some(pending) = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(completion.internal_id)
        else {
            trace_async_subresource_stage(
                "async_subresource_complete_missing",
                AsyncSubresourceTraceFields {
                    event_kind: Some("completion"),
                    internal_id: Some(internal_id),
                    ..AsyncSubresourceTraceFields::default()
                },
                trace_started,
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let trace_fields =
            async_subresource_trace_fields_for_pending("completion", internal_id, &pending);
        trace_async_subresource_stage(
            "async_subresource_complete_pending",
            trace_fields,
            trace_started,
        );
        let activity = self.resolve_pending_subresource_fetch_completion(
            pending,
            completion.request_url,
            completion.request_method,
            completion.request_headers,
            completion.request_body,
            completion.response_status_text,
            completion.skip_fetch_security_validation,
            completion.response_filter,
            completion.network_error_text,
            completion.result,
        )?;
        trace_async_subresource_stage(
            "async_subresource_complete_done",
            trace_fields,
            trace_started,
        );
        Ok(activity)
    }
    pub(crate) fn complete_async_subresource_fetch_event_body(
        &mut self,
        event: AsyncSubresourceFetchEvent,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields = async_subresource_trace_fields_for_event(&event);
        trace_async_subresource_stage("async_subresource_event_start", trace_fields, trace_started);
        let result = match event {
            AsyncSubresourceFetchEvent::DocumentAbort { internal_id } => {
                self.complete_window_request_document_abort(internal_id)
            }
            AsyncSubresourceFetchEvent::ContentSecurityPolicyViolation {
                report_context,
                violation,
            } => self.report_async_fetch_csp_violation(&report_context, &violation),
            AsyncSubresourceFetchEvent::Upload { internal_id, event } => {
                self.apply_async_xhr_upload_event(internal_id, event)
            }
            AsyncSubresourceFetchEvent::Completion(completion) => {
                self.complete_async_subresource_fetch_body(*completion)
            }
            AsyncSubresourceFetchEvent::ObservedNetworkRecord(record) => {
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(*record);
                Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered)
            }
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                self.start_streaming_async_subresource_fetch_body(*started)
            }
            AsyncSubresourceFetchEvent::StreamingChunk(chunk) => Ok(self
                .append_streaming_async_subresource_fetch_chunk_body(
                    chunk.body_source_id,
                    chunk.bytes,
                )),
            AsyncSubresourceFetchEvent::StreamingFinished(finished) => self
                .finish_streaming_async_subresource_fetch_body(
                    finished.internal_id,
                    finished.body_source_id,
                    finished.result,
                ),
        };
        trace_async_subresource_stage("async_subresource_event_done", trace_fields, trace_started);
        result
    }
    fn complete_window_request_document_abort(
        &mut self,
        internal_id: u64,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let Some(pending) = self
            ._context_host
            .borrow_mut()
            .take_pending_window_request_abort(internal_id)
        else {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let context_host = self._context_host.clone();
        self.renderer_document_isolate.with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let host_ptr = context_host.as_ptr();
            let context = v8::Local::new(scope, pending.binding.context_global());
            let scope = &mut v8::ContextScope::new(scope, context);
            let entered = pending.binding.with_current_scope(scope, host_ptr, |scope, owner| {
                let previous = enter_subresource_owner_async_scope(&context_host, scope, owner);
                match pending.continuation {
                    crate::types::WindowRequestAbortContinuation::Fetch { resolver, body_source } => {
                        let message = v8_string(scope, "Failed to fetch").expect("static abort message");
                        let reason = v8::Exception::type_error(scope, message);
                        if let Some(body_source) = body_source {
                            crate::network_host::error_pending_network_body_stream_with_reason(scope, body_source, "The document load was aborted.".to_owned(), reason);
                        }
                        if let Some(resolver) = resolver {
                            let resolver = v8::Local::new(scope, &resolver);
                            let _ = resolver.reject(scope, reason);
                        }
                    }
                    crate::types::WindowRequestAbortContinuation::Xhr(xhr) => {
                        let xhr = v8::Local::new(scope, &xhr);
                        if crate::network_host::xhr_state_number_property(scope, xhr, crate::network_host::XHR_ACTIVE_INTERNAL_ID_SLOT) == Some(internal_id as f64) {
                            crate::network_host::apply_xhr_document_abort(scope, xhr);
                        }
                    }
                }
                defer_subresource_owner_async_scope(&context_host, scope, owner, previous);
            }).is_some();
            Ok(if entered { AsyncSubresourceFetchBodyActivity::WindowRealmEntered } else { AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered })
        })
    }

    fn report_async_fetch_csp_violation(
        &mut self,
        report_context: &crate::network_host::WindowCspReportRequestContext,
        violation: &crate::content_security_policy::ContentSecurityPolicyUrlViolation,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        crate::network_host::send_content_security_policy_violation_report_from_window_context(
            &mut self._context_host.borrow_mut(),
            report_context,
            violation,
        );
        let identity = report_context.identity();
        if !self
            ._context_host
            .borrow()
            .window_document_owner_is_current_for_dispatch_scope(
                identity.owner(),
                identity.dispatch_scope(),
            )
        {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        self.with_default_context_scope(|scope, host_ptr| {
            // SAFETY: the default context scope keeps this ScriptVm's host alive.
            unsafe { &mut *host_ptr }.dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(
                scope, host_ptr, identity, violation,
            );
            Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
        })
    }

    fn apply_async_xhr_upload_event(
        &mut self,
        internal_id: u64,
        event: moli_fetch::UploadEvent,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let context_host = self._context_host.clone();
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let Some((xhr, execution)) = context_host
                    .borrow()
                    .xhr_upload_delivery(scope, internal_id)
                else {
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                };
                let Some(context) = execution.context_global() else {
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                };
                let context = v8::Local::new(scope, context);
                let scope = &mut v8::ContextScope::new(scope, context);
                if execution.window_realm_binding().is_some_and(|binding| {
                    crate::native_bridge::current_runtime_observable_context_token(scope)
                        != Some(binding.realm_token())
                        || !binding.is_current(&context_host.borrow())
                }) {
                    let _ = context_host
                        .borrow_mut()
                        .abort_subresource_fetch(internal_id);
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }
                let dispatch_scope = execution.dispatch_scope();
                let previous =
                    enter_subresource_owner_async_scope(&context_host, scope, dispatch_scope);
                let current =
                    crate::network_host::apply_xhr_upload_event(scope, xhr, internal_id, event);
                defer_subresource_owner_async_scope(&context_host, scope, dispatch_scope, previous);
                if !current {
                    let _ = context_host
                        .borrow_mut()
                        .abort_subresource_fetch(internal_id);
                }
                Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
            })
    }

    pub(crate) fn async_subresource_fetch_event_target_is_current(
        &self,
        target: crate::types::AsyncSubresourceFetchEventTarget,
    ) -> bool {
        self._context_host
            .borrow()
            .async_subresource_fetch_event_target_is_current(target)
    }
    pub(super) fn start_network_only_subresource_stream(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        started: crate::types::AsyncSubresourceStreamingStarted,
    ) -> Result<()> {
        let detached_window_fetch = pending.continuation.is_detached_window_fetch();
        debug_assert!(
            detached_window_fetch || pending.execution_context.is_window_network_only(),
            "network-only stream must be an accepted fire-and-forget request or detached Fetch"
        );
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);

        let security_error = detached_window_fetch
            .then(|| {
                if !started.head.redirect_chain.is_empty() {
                    detached_window_fetch_csp_redirect_failure_message(
                        &self._context_host,
                        &pending,
                        &started.head.final_url,
                    )
                } else {
                    None
                }
                .or_else(|| {
                    if started.skip_fetch_security_validation {
                        return None;
                    }
                    crate::network_host::validate_fetch_response_headers(
                        &pending.request_origin,
                        &started.head,
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    )
                    .err()
                })
            })
            .flatten();

        if let Some(error_text) = security_error {
            pending.load.cancel();
            let mut network_record = crate::types::SubresourceNetworkRecord::failure(
                pending.info.frame_id.clone(),
                pending.info.document_url.clone(),
                started.request_url,
                started.request_method,
                started.request_headers,
                started.request_body,
                pending.info.resource_type,
                error_text,
            )
            .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
            .with_request_body_bytes(pending.info.request_body_bytes.clone());
            if let Some(handle) = pending.info.network_request_handle {
                network_record = network_record.with_request_handle(handle);
            }
            self._context_host
                .borrow_mut()
                .record_subresource_network(network_record);
            self._context_host
                .borrow_mut()
                .record_pending_subresource_continue_event(
                    PendingSubresourceContinueEvent::Completed {
                        internal_id: started.internal_id,
                    },
                );
            return Ok(());
        }

        let detached_identity = pending.execution_context.detached_window_fetch_identity();
        let accepted_context = pending.execution_context.window_network_only_identity();
        let accepted_document = pending
            .execution_context
            .window_document_network_only_identity();
        let disk_pool = pending.load.request_client().disk_pool();
        self._context_host
            .borrow_mut()
            .record_streaming_subresource_fetch(StreamingSubresourceFetchState {
                response_filter: started.response_filter,
                skip_fetch_security_validation: started.skip_fetch_security_validation,
                pending,
                request_url: started.request_url,
                request_method: started.request_method,
                request_headers: started.request_headers,
                request_body: started.request_body,
                body_source_id: started.body_source_id,
                head: started.head,
                network_request_headers: started.network_request_headers,
                body_writer: SubresourceResponseBodyWriter::with_disk_pool(disk_pool),
                event_source_parser: None,
                xhr_response: None,
            });
        tracing::debug!(
            internal_id = started.internal_id,
            ?detached_identity,
            ?accepted_context,
            ?accepted_document,
            "continued network-only subresource streaming without a V8 body source"
        );
        Ok(())
    }
    /// Standalone ScriptVm test turn for a streaming-start terminal.
    #[cfg(test)]
    pub(in crate::script_vm) fn start_streaming_async_subresource_fetch(
        &mut self,
        started: crate::types::AsyncSubresourceStreamingStarted,
    ) -> Result<()> {
        let activity = self.start_streaming_async_subresource_fetch_body(started)?;
        self.finish_async_subresource_body_checkpoint_for_test(activity)
    }
    pub(super) fn start_streaming_async_subresource_fetch_body(
        &mut self,
        started: crate::types::AsyncSubresourceStreamingStarted,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields = AsyncSubresourceTraceFields {
            event_kind: Some("streaming_started"),
            internal_id: Some(started.internal_id),
            body_source_id: Some(started.body_source_id),
            ..AsyncSubresourceTraceFields::default()
        };
        trace_async_subresource_stage(
            "async_subresource_streaming_start_begin",
            trace_fields,
            trace_started,
        );
        let Some(mut pending) = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(started.internal_id)
        else {
            trace_async_subresource_stage(
                "async_subresource_streaming_start_missing",
                trace_fields,
                trace_started,
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let trace_fields = async_subresource_trace_fields_for_pending_with_body(
            "streaming_started",
            started.internal_id,
            Some(started.body_source_id),
            &pending,
        );
        if pending.continuation.is_detached_window_fetch()
            || pending.execution_context.is_window_network_only()
        {
            return self
                .start_network_only_subresource_stream(pending, started)
                .map(|()| AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if !window_subresource_owner_is_current(&self._context_host, &pending) {
            pending.load.cancel();
            self._context_host
                .borrow_mut()
                .record_pending_subresource_continue_event(
                    PendingSubresourceContinueEvent::Completed {
                        internal_id: started.internal_id,
                    },
                );
            tracing::debug!(
                internal_id = started.internal_id,
                owner = ?pending.execution_context.window_request_target().map(crate::native_bridge::WindowTaskTarget::owner),
                "discarded streaming subresource start for retired Window execution context"
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);

        let pending_context_ptr: *const v8::Global<v8::Context> = pending
            .execution_context
            .context_global()
            .expect("active streaming subresource must retain its V8 context");
        let result = self.renderer_document_isolate.with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            // SAFETY: `pending_context_ptr` points into `pending.execution_context`, which stays
            // alive until the streaming state is recorded after this non-escaping closure returns.
            let context = unsafe { v8::Local::new(scope, &*pending_context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            if !window_subresource_realm_is_current(&self._context_host, scope, &pending) {
                pending.load.cancel();
                self._context_host
                    .borrow_mut()
                    .record_pending_subresource_continue_event(
                        PendingSubresourceContinueEvent::Completed {
                            internal_id: started.internal_id,
                        },
                    );
                tracing::debug!(
                    internal_id = started.internal_id,
                    expected_realm = ?pending.execution_context.realm_token(),
                    "discarded streaming subresource start for retired V8 realm"
                );
                return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
            }
            trace_async_subresource_stage(
                "async_subresource_streaming_context_entered",
                trace_fields,
                trace_started,
            );
            let owner_async_scope =
                enter_subresource_owner_async_scope(
                    &self._context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                );

            let streaming_start_error = if !started.head.redirect_chain.is_empty() {
                document_connect_csp_redirect_failure_message(
                    scope,
                    &self._context_host,
                    &pending,
                    &started.head.final_url,
                )
            } else {
                None
            }
            .or_else(|| {
                (!started.skip_fetch_security_validation && matches!(
                    pending.info.resource_type,
                    SubresourceResourceType::EventSource
                        | SubresourceResourceType::Fetch
                        | SubresourceResourceType::Xhr
                ))
                .then(|| {
                    crate::network_host::validate_fetch_response_headers(
                        &pending.request_origin,
                        &started.head,
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    )
                    .err()
                })
                .flatten()
            });
            if let Some(error_text) = streaming_start_error {
                pending.load.cancel();
                let network_error_text =
                    if crate::network_host::is_cors_policy_failure_message(&error_text) {
                        crate::network_host::FAILED_ERROR_TEXT.to_owned()
                    } else {
                        error_text.clone()
                    };
                let mut network_record = crate::types::SubresourceNetworkRecord::failure(
                    pending.info.frame_id.clone(),
                    pending.info.document_url.clone(),
                    started.request_url.clone(),
                    started.request_method.clone(),
                    started.request_headers.clone(),
                    started.request_body.clone(),
                    pending.info.resource_type,
                    network_error_text,
                )
                .with_request_body_bytes(pending.info.request_body_bytes.clone());
                if let Some(handle) = pending.info.network_request_handle {
                    network_record = network_record.with_request_handle(handle);
                }
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                match &pending.continuation {
                    PendingSubresourceContinuation::Fetch(fetch) => {
                        let resolver = fetch
                            .resolver()
                            .expect("detached keepalive stream is handled before V8 entry");
                        let resolver = v8::Local::new(scope, resolver);
                        let exception = v8_string(scope, &error_text)
                            .map(|message| v8::Exception::type_error(scope, message))
                            .unwrap_or_else(|| v8::undefined(scope).into());
                        resolver.reject(scope, exception);
                    }
                    PendingSubresourceContinuation::Xhr { xhr, .. } => {
                        let xhr = v8::Local::new(scope, xhr);
                        crate::network_host::apply_xhr_failure(scope, xhr);
                    }
                    PendingSubresourceContinuation::EventSource(event_source) => {
                        let event_source = v8::Local::new(scope, event_source);
                        crate::network_host::fail_event_source_connection(
                            scope,
                            event_source,
                            crate::network_host::EventSourceTerminalMode::Close,
                        );
                    }
                    PendingSubresourceContinuation::Image {
                        image_handle,
                        sequence,
                        ..
                    } => apply_image_subresource_terminal(
                        scope,
                        &self._context_host,
                        *image_handle,
                        *sequence,
                        started.internal_id,
                        &started.request_url,
                        ImageSubresourceTerminal::Failure,
                    ),
                    PendingSubresourceContinuation::Media {
                        media_handle,
                        sequence,
                    } => apply_media_subresource_terminal(
                        scope,
                        &self._context_host,
                        *media_handle,
                        *sequence,
                        started.internal_id,
                        false,
                    ),
                    PendingSubresourceContinuation::TextTrack {
                        track_handle,
                        sequence,
                    } => apply_text_track_subresource_terminal(
                        scope,
                        &self._context_host,
                        *track_handle,
                        *sequence,
                        started.internal_id,
                        Err(error_text.clone()),
                    ),
                    PendingSubresourceContinuation::StylesheetSubresource { binding, .. } => {
                        apply_stylesheet_subresource_terminal(&self._context_host, *binding);
                    }
                    PendingSubresourceContinuation::Beacon
                    | PendingSubresourceContinuation::CspReport { .. }
                    | PendingSubresourceContinuation::WebSocket(_)
                    | PendingSubresourceContinuation::WorkerFetch { .. }
                    | PendingSubresourceContinuation::WorkerXhr { .. }
                    | PendingSubresourceContinuation::WorkerCspReport { .. }
                    | PendingSubresourceContinuation::SharedWorkerFetch { .. }
                    | PendingSubresourceContinuation::SharedWorkerXhr { .. }
                    | PendingSubresourceContinuation::SharedWorkerCspReport { .. } => {}
                }
                defer_subresource_owner_async_scope(
                    &self._context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                    owner_async_scope,
                );
                self._context_host
                    .borrow_mut()
                    .record_pending_subresource_continue_event(
                        PendingSubresourceContinueEvent::Completed {
                            internal_id: started.internal_id,
                        },
                    );
                return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
            }

            if pending.continuation.is_window_event_source()
                && let Some(error_text) =
                    crate::network_host::event_source_response_error(&started.head)
            {
                pending.load.cancel();
                if let Some(handle) = pending.info.network_request_handle {
                    self._context_host
                        .borrow_mut()
                        .record_subresource_response_started(
                            crate::types::SubresourceResponseStarted::new(
                                handle,
                                started
                                    .head
                                    .redirect_chain
                                    .clone()
                                    .into_iter()
                                    .map(Into::into)
                                    .collect(),
                                started.head.final_url.clone(),
                                started.head.status,
                                started.head.headers.clone(),
                                started.head.cookie_set_reports.clone(),
                            )
                            .with_from_cache(started.head.from_cache)
                            .with_negotiated_http_version(
                                started.head.negotiated_http_version,
                            )
                            .with_network_request_headers(
                                started.network_request_headers.clone(),
                            ),
                        );
                    self._context_host
                        .borrow_mut()
                        .record_subresource_body_finished(
                            crate::types::SubresourceBodyFinished::failed(handle, error_text),
                        );
                }
                if let PendingSubresourceContinuation::EventSource(event_source) =
                    &pending.continuation
                {
                    let event_source = v8::Local::new(scope, event_source);
                    crate::network_host::fail_event_source_connection(
                        scope,
                        event_source,
                        crate::network_host::EventSourceTerminalMode::Close,
                    );
                }
                defer_subresource_owner_async_scope(
                    &self._context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                    owner_async_scope,
                );
                self._context_host
                    .borrow_mut()
                    .record_pending_subresource_continue_event(
                        PendingSubresourceContinueEvent::Completed {
                            internal_id: started.internal_id,
                        },
                    );
                return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
            }

            let mut observable_head = started.head.clone();
            if pending.info.resource_type == SubresourceResourceType::Xhr && !started.response_filter.as_ref().is_some_and(|filter| filter.is_readable()) {
                observable_head.headers = crate::network_host::filter_cors_exposed_response_headers(
                    &pending.request_origin,
                    &observable_head,
                    pending.credentials_mode,
                );
            }

            if matches!(&pending.continuation, PendingSubresourceContinuation::Xhr { .. })
                && let Some(handle) = pending.info.network_request_handle
            {
                self._context_host
                    .borrow_mut()
                    .record_subresource_response_started(
                        crate::types::SubresourceResponseStarted::new(
                            handle,
                            started
                                .head
                                .redirect_chain
                                .clone()
                                .into_iter()
                                .map(Into::into)
                                .collect(),
                            started.head.final_url.clone(),
                            started.head.status,
                            started.head.headers.clone(),
                            started.head.cookie_set_reports.clone(),
                        )
                        .with_from_cache(started.head.from_cache)
                        .with_negotiated_http_version(started.head.negotiated_http_version)
                        .with_network_request_headers(started.network_request_headers.clone()),
                    );
            }

            if let PendingSubresourceContinuation::Xhr { xhr, .. } = &pending.continuation {
                let xhr = v8::Local::new(scope, xhr);
                let pending_owner = pending.execution_context.dispatch_scope();
                let disk_pool = pending.load.request_client().disk_pool();
                let xhr_response = crate::types::XhrStreamingResponseState::new(
                    &observable_head.headers,
                );
                self._context_host
                    .borrow_mut()
                    .record_streaming_subresource_fetch(StreamingSubresourceFetchState {
                        response_filter: started.response_filter,
                        skip_fetch_security_validation: started.skip_fetch_security_validation,
                        pending,
                        request_url: started.request_url.clone(),
                        request_method: started.request_method.clone(),
                        request_headers: started.request_headers.clone(),
                        request_body: started.request_body.clone(),
                        body_source_id: started.body_source_id,
                        head: started.head.clone(),
                        network_request_headers: started.network_request_headers.clone(),
                        body_writer: SubresourceResponseBodyWriter::with_disk_pool(disk_pool),
                        event_source_parser: None,
                        xhr_response: Some(xhr_response),
                    });
                let keep_stream =
                    crate::network_host::apply_xhr_streaming_response_head(
                        scope,
                        xhr,
                        &observable_head,
                        started.internal_id,
                    );
                if !keep_stream {
                    // Registering the stream before dispatching state 2 lets abort()
                    // retire the transport synchronously. open() and isolate
                    // termination also invalidate the old XHR generation; retire
                    // any state that remains after those callbacks return.
                    let _ = self
                        ._context_host
                        .borrow_mut()
                        .abort_subresource_fetch(started.internal_id);
                }
                defer_subresource_owner_async_scope(
                    &self._context_host,
                    scope,
                    pending_owner,
                    owner_async_scope,
                );
                return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
            }

            let mut event_source_parser = None;
            let mut event_source_to_open = None;
            match &pending.continuation {
                PendingSubresourceContinuation::Fetch(fetch) => {
                    let resolver = fetch
                        .resolver()
                        .expect("detached keepalive stream is handled before V8 entry");
                    let resolver = v8::Local::new(scope, resolver);
                    let response_request = crate::network_host::FetchResponseRequest {
                        method: &started.request_method,
                        mode: pending.request_mode,
                        redirect_mode: fetch.redirect_mode(),
                    };
                    let response_filter = started.response_filter.clone().or_else(|| Some(response_request.network_response_filter(
                        &pending.request_origin,
                        &observable_head,
                        pending.credentials_mode,
                    )));
                    let response_obj = crate::network_host::build_fetch_response_object_from_stream_for_request_mode_with_filter(
                        scope,
                        &pending.request_origin,
                        response_request,
                        observable_head,
                        started.body_source_id,
                        response_filter,
                    );
                    resolver.resolve(scope, response_obj.into());
                }
                PendingSubresourceContinuation::Image {
                    image_handle,
                    sequence,
                    ..
                } => apply_image_subresource_terminal(
                    scope,
                    &self._context_host,
                    *image_handle,
                    *sequence,
                    started.internal_id,
                    &started.request_url,
                    // Image requests require the complete body for MIME sniffing and decode.
                    // Production image transports are buffered, so a streaming head is an
                    // invalid terminal rather than a successful image response.
                    ImageSubresourceTerminal::Failure,
                ),
                PendingSubresourceContinuation::Media {
                    media_handle,
                    sequence,
                } => apply_media_subresource_terminal(
                    scope,
                    &self._context_host,
                    *media_handle,
                    *sequence,
                    started.internal_id,
                    crate::network_host::media_response_status_is_successful(started.head.status),
                ),
                PendingSubresourceContinuation::TextTrack {
                    track_handle,
                    sequence,
                } => apply_text_track_subresource_terminal(
                    scope,
                    &self._context_host,
                    *track_handle,
                    *sequence,
                    started.internal_id,
                    Err("text-track response unexpectedly used streaming transport".to_owned()),
                ),
                PendingSubresourceContinuation::StylesheetSubresource { binding, .. } => {
                    apply_stylesheet_subresource_terminal(&self._context_host, *binding);
                }
                PendingSubresourceContinuation::EventSource(event_source) => {
                    if let Some(handle) = pending.info.network_request_handle {
                        self._context_host
                            .borrow_mut()
                            .record_subresource_response_started(
                                crate::types::SubresourceResponseStarted::new(
                                    handle,
                                    started
                                        .head
                                        .redirect_chain
                                        .clone()
                                        .into_iter()
                                        .map(Into::into)
                                        .collect(),
                                    started.head.final_url.clone(),
                                    started.head.status,
                                    started.head.headers.clone(),
                                    started.head.cookie_set_reports.clone(),
                                )
                                .with_from_cache(started.head.from_cache)
                                .with_negotiated_http_version(
                                    started.head.negotiated_http_version,
                                )
                                .with_network_request_headers(
                                    started.network_request_headers.clone(),
                                ),
                            );
                    }
                    let event_source = v8::Local::new(scope, event_source);
                    let last_event_id =
                        crate::network_host::event_source_last_event_id(scope, event_source);
                    let reconnect_delay_ms =
                        crate::network_host::event_source_reconnect_delay_ms(scope, event_source);
                    event_source_parser = Some(crate::network_host::EventSourceParser::new(
                        last_event_id,
                        reconnect_delay_ms,
                    ));
                    event_source_to_open = Some(event_source);
                }
                PendingSubresourceContinuation::Beacon
                | PendingSubresourceContinuation::CspReport { .. }
                | PendingSubresourceContinuation::Xhr { .. }
                | PendingSubresourceContinuation::WebSocket(_)
                | PendingSubresourceContinuation::WorkerFetch { .. }
                | PendingSubresourceContinuation::WorkerXhr { .. }
                | PendingSubresourceContinuation::WorkerCspReport { .. }
                | PendingSubresourceContinuation::SharedWorkerFetch { .. }
                | PendingSubresourceContinuation::SharedWorkerXhr { .. }
                | PendingSubresourceContinuation::SharedWorkerCspReport { .. } => {}
            }

            let pending_owner = pending.execution_context.dispatch_scope();
            let disk_pool = pending.load.request_client().disk_pool();
            self._context_host.borrow_mut().record_streaming_subresource_fetch(
                StreamingSubresourceFetchState {
                        response_filter: started.response_filter,
                    skip_fetch_security_validation: started.skip_fetch_security_validation,
                    pending,
                    request_url: started.request_url.clone(),
                    request_method: started.request_method.clone(),
                    request_headers: started.request_headers.clone(),
                    request_body: started.request_body.clone(),
                    body_source_id: started.body_source_id,
                    head: started.head.clone(),
                    network_request_headers: started.network_request_headers.clone(),
                    body_writer: SubresourceResponseBodyWriter::with_disk_pool(disk_pool),
                    event_source_parser,
                    xhr_response: None,
                },
            );
            if let Some(event_source) = event_source_to_open {
                crate::network_host::open_event_source_connection(
                    scope,
                    event_source,
                    &started.head.final_url,
                );
            }

            defer_subresource_owner_async_scope(
                &self._context_host,
                scope,
                pending_owner,
                owner_async_scope,
            );
            Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
        });
        trace_async_subresource_stage(
            "async_subresource_streaming_start_isolate_done",
            trace_fields,
            trace_started,
        );
        result
    }
    /// Standalone ScriptVm test turn for one streaming body chunk.
    #[cfg(test)]
    pub(in crate::script_vm) fn append_streaming_async_subresource_fetch_chunk(
        &mut self,
        body_source_id: NetworkBodySourceId,
        bytes: Vec<u8>,
    ) {
        let activity =
            self.append_streaming_async_subresource_fetch_chunk_body(body_source_id, bytes);
        self.finish_async_subresource_body_checkpoint_for_test(activity)
            .expect("streaming chunk test task checkpoint should complete");
    }
    pub(super) fn append_streaming_async_subresource_fetch_chunk_body(
        &mut self,
        body_source_id: NetworkBodySourceId,
        bytes: Vec<u8>,
    ) -> AsyncSubresourceFetchBodyActivity {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let bytes_len = bytes.len();
        let trace_fields = AsyncSubresourceTraceFields {
            event_kind: Some("streaming_chunk"),
            body_source_id: Some(body_source_id),
            bytes: Some(bytes_len),
            ..AsyncSubresourceTraceFields::default()
        };
        trace_async_subresource_stage(
            "async_subresource_streaming_chunk_start",
            trace_fields,
            trace_started,
        );
        let is_xhr = self
            ._context_host
            .borrow()
            .streaming_subresource_is_xhr(body_source_id);
        if is_xhr {
            let context_host = self._context_host.clone();
            let entered = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let Some(delivery) = context_host.borrow_mut().append_streaming_xhr_chunk(
                        scope,
                        body_source_id,
                        &bytes,
                    ) else {
                        return Ok(false);
                    };
                    if let Some(handle) = delivery.request_handle {
                        context_host.borrow_mut().record_subresource_data_received(
                            crate::types::SubresourceDataReceived::new(
                                handle, bytes_len, bytes_len,
                            ),
                        );
                    }
                    let context = delivery.context;
                    let xhr = delivery.xhr;
                    let dispatch_scope = delivery.dispatch_scope;
                    let realm_token = delivery.realm_token;
                    let internal_id = delivery.internal_id;
                    let scope = &mut v8::ContextScope::new(scope, context);
                    if realm_token.is_some_and(|expected| {
                        crate::native_bridge::current_runtime_observable_context_token(scope)
                            != Some(expected)
                    }) {
                        let _ = context_host
                            .borrow_mut()
                            .abort_subresource_fetch(internal_id);
                        return Ok(false);
                    }
                    let previous =
                        enter_subresource_owner_async_scope(&context_host, scope, dispatch_scope);
                    let remains_current = crate::network_host::apply_xhr_streaming_response_chunk(
                        scope,
                        xhr,
                        internal_id,
                        &delivery.decoded_text,
                        delivery.loaded,
                        delivery.total,
                    );
                    defer_subresource_owner_async_scope(
                        &context_host,
                        scope,
                        dispatch_scope,
                        previous,
                    );
                    if !remains_current {
                        let _ = context_host
                            .borrow_mut()
                            .abort_subresource_fetch(internal_id);
                    }
                    Ok(true)
                })
                .unwrap_or(false);
            trace_async_subresource_stage(
                "async_subresource_streaming_chunk_done",
                trace_fields,
                trace_started,
            );
            return if entered {
                AsyncSubresourceFetchBodyActivity::WindowRealmEntered
            } else {
                AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered
            };
        }
        let is_event_source = self
            ._context_host
            .borrow()
            .streaming_subresource_is_event_source(body_source_id);
        if is_event_source {
            let entered = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let Some(delivery) = self
                        ._context_host
                        .borrow_mut()
                        .append_streaming_event_source_chunk(scope, body_source_id, &bytes)
                    else {
                        return Ok(false);
                    };
                    if let Some(handle) = delivery.request_handle {
                        self._context_host
                            .borrow_mut()
                            .record_subresource_data_received(
                                crate::types::SubresourceDataReceived::new(
                                    handle, bytes_len, bytes_len,
                                ),
                            );
                    }
                    let context = delivery.context;
                    let event_source = delivery.event_source;
                    let scope = &mut v8::ContextScope::new(scope, context);
                    dispatch_streaming_event_source_messages(
                        &self._context_host,
                        scope,
                        event_source,
                        delivery.request_handle,
                        &delivery.messages,
                    );
                    Ok(true)
                })
                .unwrap_or(false);
            trace_async_subresource_stage(
                "async_subresource_streaming_chunk_done",
                trace_fields,
                trace_started,
            );
            return if entered {
                AsyncSubresourceFetchBodyActivity::WindowRealmEntered
            } else {
                AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered
            };
        }
        let body_binding = self
            ._context_host
            .borrow()
            .streaming_subresource_body_binding_by_body_source_id(body_source_id);
        let append_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        self._context_host
            .borrow_mut()
            .append_streaming_subresource_body(body_source_id, &bytes);
        trace_async_subresource_stage(
            "async_subresource_streaming_chunk_appended",
            trace_fields,
            append_started,
        );
        let mut activity = AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered;
        if let Some((context_ptr, dispatch_scope, realm_token)) = body_binding {
            let enqueue_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
            let context_host = self._context_host.clone();
            let entered = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    if realm_token.is_some_and(|expected| {
                        crate::native_bridge::current_runtime_observable_context_token(scope)
                            != Some(expected)
                    }) {
                        return Ok(false);
                    }
                    let previous =
                        enter_subresource_owner_async_scope(&context_host, scope, dispatch_scope);
                    // The CDP capture already borrowed this chunk into its
                    // body-writer above, so the original Vec can move into the
                    // Web-visible stream without cloning the full chunk.
                    crate::network_host::enqueue_pending_network_body_chunk(
                        scope,
                        body_source_id,
                        bytes,
                    );
                    defer_subresource_owner_async_scope(
                        &context_host,
                        scope,
                        dispatch_scope,
                        previous,
                    );
                    Ok(true)
                })
                .unwrap_or(false);
            if entered {
                activity = AsyncSubresourceFetchBodyActivity::WindowRealmEntered;
            }
            trace_async_subresource_stage(
                "async_subresource_streaming_chunk_enqueued",
                trace_fields,
                enqueue_started,
            );
        }
        trace_async_subresource_stage(
            "async_subresource_streaming_chunk_done",
            trace_fields,
            trace_started,
        );
        activity
    }
    pub(super) fn finish_network_only_subresource_stream(
        &mut self,
        streaming: StreamingSubresourceFetchState,
        internal_id: u64,
        result: std::result::Result<(), String>,
    ) -> Result<()> {
        debug_assert!(
            streaming.pending.continuation.is_detached_window_fetch()
                || streaming.pending.execution_context.is_window_network_only(),
            "network-only stream must be an accepted fire-and-forget request or detached Fetch"
        );
        let detached_identity = streaming
            .pending
            .execution_context
            .detached_window_fetch_identity();
        let accepted_context = streaming
            .pending
            .execution_context
            .window_network_only_identity();
        let accepted_document = streaming
            .pending
            .execution_context
            .window_document_network_only_identity();
        let needs_orb_body_validation = streaming.needs_orb_body_validation();
        let response_body = streaming.body_writer.finish();
        let result = result.and_then(|()| {
            if needs_orb_body_validation {
                crate::network_host::validated_opaque_response_body(
                    &streaming.head.headers,
                    &response_body,
                )
                .map(|_| ())
                .map_err(crate::network_host::FetchResponseSecurityViolation::into_message)
            } else {
                Ok(())
            }
        });
        match result {
            Ok(()) => {
                let request_cookie_report = streaming
                    .head
                    .request_cookie_report
                    .clone()
                    .or_else(|| streaming.pending.info.request_cookie_report.clone());
                let mut network_record = crate::types::SubresourceNetworkRecord::success_with_body(
                    streaming.pending.info.frame_id.clone(),
                    streaming.pending.info.document_url.clone(),
                    streaming.request_url,
                    streaming.request_method,
                    streaming.request_headers,
                    streaming.request_body,
                    streaming.pending.info.resource_type,
                    request_cookie_report,
                    streaming
                        .head
                        .redirect_chain
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    streaming.head.final_url,
                    streaming.head.status,
                    streaming.head.headers,
                    response_body,
                    streaming.head.cookie_set_reports,
                )
                .with_from_cache(streaming.head.from_cache)
                .with_negotiated_http_version(streaming.head.negotiated_http_version)
                .with_network_request_headers(streaming.network_request_headers)
                .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
                .with_request_body_bytes(streaming.pending.info.request_body_bytes.clone());
                if let Some(handle) = streaming.pending.info.network_request_handle {
                    network_record = network_record.with_request_handle(handle);
                }
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
            Err(_) => {
                let network_error_text = crate::network_host::ABORTED_ERROR_TEXT.to_owned();
                if let Some(handle) = streaming.pending.info.network_request_handle {
                    let partial_body = response_body;
                    self._context_host
                        .borrow_mut()
                        .record_subresource_response_started(
                            crate::types::SubresourceResponseStarted::new(
                                handle,
                                streaming
                                    .head
                                    .redirect_chain
                                    .into_iter()
                                    .map(Into::into)
                                    .collect(),
                                streaming.head.final_url,
                                streaming.head.status,
                                streaming.head.headers,
                                streaming.head.cookie_set_reports,
                            )
                            .with_from_cache(streaming.head.from_cache)
                            .with_negotiated_http_version(streaming.head.negotiated_http_version)
                            .with_network_request_headers(streaming.network_request_headers),
                        );
                    self._context_host
                        .borrow_mut()
                        .record_subresource_body_finished(
                            crate::types::SubresourceBodyFinished::failed_with_partial_body(
                                handle,
                                network_error_text,
                                partial_body,
                            ),
                        );
                } else {
                    self._context_host.borrow_mut().record_subresource_network(
                        crate::types::SubresourceNetworkRecord::failure(
                            streaming.pending.info.frame_id.clone(),
                            streaming.pending.info.document_url.clone(),
                            streaming.request_url,
                            streaming.request_method,
                            streaming.request_headers,
                            streaming.request_body,
                            streaming.pending.info.resource_type,
                            network_error_text,
                        )
                        .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
                        .with_request_body_bytes(streaming.pending.info.request_body_bytes.clone()),
                    );
                }
            }
        }
        self._context_host
            .borrow_mut()
            .record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
        tracing::debug!(
            internal_id,
            ?detached_identity,
            ?accepted_context,
            ?accepted_document,
            "finished network-only subresource without entering V8"
        );
        Ok(())
    }
    /// Standalone ScriptVm test turn for a streaming-finish terminal.
    #[cfg(test)]
    pub(in crate::script_vm) fn finish_streaming_async_subresource_fetch(
        &mut self,
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
        result: std::result::Result<(), String>,
    ) -> Result<()> {
        let activity = self.finish_streaming_async_subresource_fetch_body(
            internal_id,
            body_source_id,
            result,
        )?;
        self.finish_async_subresource_body_checkpoint_for_test(activity)
    }
    pub(super) fn finish_streaming_async_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
        result: std::result::Result<(), String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields = AsyncSubresourceTraceFields {
            event_kind: Some("streaming_finished"),
            internal_id: Some(internal_id),
            body_source_id: Some(body_source_id),
            ..AsyncSubresourceTraceFields::default()
        };
        trace_async_subresource_stage(
            "async_subresource_streaming_finish_start",
            trace_fields,
            trace_started,
        );
        let Some(mut streaming) = self
            ._context_host
            .borrow_mut()
            .take_streaming_subresource_fetch(internal_id)
        else {
            trace_async_subresource_stage(
                "async_subresource_streaming_finish_missing",
                trace_fields,
                trace_started,
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let trace_fields = async_subresource_trace_fields_for_pending_with_body(
            "streaming_finished",
            internal_id,
            Some(body_source_id),
            &streaming.pending,
        );
        if streaming.pending.continuation.is_detached_window_fetch()
            || streaming.pending.execution_context.is_window_network_only()
        {
            return self
                .finish_network_only_subresource_stream(streaming, internal_id, result)
                .map(|()| AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if !window_subresource_owner_is_current(&self._context_host, &streaming.pending) {
            self._context_host
                .borrow_mut()
                .record_pending_subresource_continue_event(
                    PendingSubresourceContinueEvent::Completed { internal_id },
                );
            tracing::debug!(
                internal_id,
                owner = ?streaming.pending.execution_context.window_request_target().map(crate::native_bridge::WindowTaskTarget::owner),
                "discarded streaming subresource finish for retired Window execution context"
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }

        let context_host = self._context_host.clone();
        let result = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(
                    scope,
                    streaming
                        .pending
                        .execution_context
                        .context_global()
                        .expect("active streaming finish must retain its V8 context"),
                );
                let scope = &mut v8::ContextScope::new(scope, context);
                if !window_subresource_realm_is_current(&context_host, scope, &streaming.pending) {
                    context_host
                        .borrow_mut()
                        .record_pending_subresource_continue_event(
                            PendingSubresourceContinueEvent::Completed { internal_id },
                        );
                    tracing::debug!(
                        internal_id,
                        expected_realm = ?streaming.pending.execution_context.realm_token(),
                        "discarded streaming subresource finish for retired V8 realm"
                    );
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }
                trace_async_subresource_stage(
                    "async_subresource_streaming_finish_context_entered",
                    trace_fields,
                    trace_started,
                );
                let pending_owner = streaming.pending.execution_context.dispatch_scope();
                let owner_async_scope =
                    enter_subresource_owner_async_scope(&context_host, scope, pending_owner);
                if streaming.pending.continuation.is_window_event_source() {
                    let event_source = match &streaming.pending.continuation {
                        PendingSubresourceContinuation::EventSource(event_source) => {
                            v8::Local::new(scope, event_source)
                        }
                        _ => unreachable!("EventSource continuation was checked above"),
                    };
                    if let Some(parser) = streaming.event_source_parser.take() {
                        crate::network_host::update_event_source_stream_state(
                            scope,
                            event_source,
                            parser.last_event_id(),
                            parser.reconnect_delay_ms(),
                        );
                    }
                    let response_body = streaming.body_writer.finish();
                    if let Some(handle) = streaming.pending.info.network_request_handle {
                        let body = match result {
                            Ok(()) => crate::types::SubresourceBodyFinished::ready_after_streaming(
                                handle,
                                response_body,
                            ),
                            Err(error_text) => {
                                crate::types::SubresourceBodyFinished::failed_with_partial_body(
                                    handle,
                                    error_text,
                                    response_body,
                                )
                            }
                        };
                        context_host
                            .borrow_mut()
                            .record_subresource_body_finished(body);
                    }
                    if crate::network_host::event_source_ready_state(scope, event_source)
                        != crate::network_host::EVENT_SOURCE_CLOSED
                    {
                        crate::network_host::fail_event_source_connection(
                            scope,
                            event_source,
                            crate::network_host::EventSourceTerminalMode::Reconnect,
                        );
                    }
                    defer_subresource_owner_async_scope(
                        &context_host,
                        scope,
                        pending_owner,
                        owner_async_scope,
                    );
                    context_host
                        .borrow_mut()
                        .record_pending_subresource_continue_event(
                            PendingSubresourceContinueEvent::Completed { internal_id },
                        );
                    return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
                }
                match result {
                    Ok(()) => {
                        let needs_orb_body_validation = streaming.needs_orb_body_validation();
                        let request_cookie_report = streaming
                            .head
                            .request_cookie_report
                            .clone()
                            .or_else(|| streaming.pending.info.request_cookie_report.clone());
                        let finish_body_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        let response_body = streaming.body_writer.finish();
                        if needs_orb_body_validation
                            && let Err(error_text) =
                                crate::network_host::release_pending_opaque_response_body(
                                    scope,
                                    body_source_id,
                                    &streaming.head.headers,
                                    &response_body,
                                )
                        {
                            let mut record = crate::types::SubresourceNetworkRecord::failure(
                                streaming.pending.info.frame_id.clone(),
                                streaming.pending.info.document_url.clone(),
                                streaming.request_url,
                                streaming.request_method,
                                streaming.request_headers,
                                streaming.request_body,
                                streaming.pending.info.resource_type,
                                error_text,
                            )
                            .with_request_body_bytes(streaming.pending.info.request_body_bytes.clone());
                            if let Some(handle) = streaming.pending.info.network_request_handle {
                                record = record.with_request_handle(handle);
                            }
                            context_host.borrow_mut().record_subresource_network(record);
                            context_host.borrow_mut().record_pending_subresource_continue_event(
                                PendingSubresourceContinueEvent::Completed { internal_id },
                            );
                            defer_subresource_owner_async_scope(
                                &context_host, scope, pending_owner, owner_async_scope,
                            );
                            return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
                        }
                        let response_body_size = response_body.len();
                        trace_async_subresource_stage(
                            "async_subresource_streaming_body_finished",
                            trace_fields,
                            finish_body_started,
                        );
                        let xhr_delivery_body = if matches!(
                            &streaming.pending.continuation,
                            PendingSubresourceContinuation::Xhr { .. }
                        ) {
                            let materialize_started =
                                moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                            match response_body.materialize_bytes() {
                                Ok(bytes) => {
                                    trace_async_subresource_stage(
                                        "async_subresource_streaming_xhr_body_materialized",
                                        trace_fields,
                                        materialize_started,
                                    );
                                    Some(moli_fetch::ResponseBody::materialized_bytes(bytes))
                                }
                                Err(error) => {
                                    let error_text = format!(
                                        "failed to materialize streaming XHR body: {error}"
                                    );
                                    crate::network_host::error_pending_network_body_stream(
                                        scope,
                                        body_source_id,
                                        error_text.clone(),
                                    );
                                    let request_handle =
                                        streaming.pending.info.network_request_handle;
                                    let mut network_record =
                                        crate::types::SubresourceNetworkRecord::failure(
                                            streaming.pending.info.frame_id.clone(),
                                            streaming.pending.info.document_url.clone(),
                                            streaming.request_url,
                                            streaming.request_method,
                                            streaming.request_headers,
                                            streaming.request_body,
                                            streaming.pending.info.resource_type,
                                            error_text,
                                        )
                                        .with_request_body_bytes(
                                            streaming.pending.info.request_body_bytes.clone(),
                                        );
                                    if let Some(handle) = request_handle {
                                        network_record = network_record.with_request_handle(handle);
                                    }
                                    context_host
                                        .borrow_mut()
                                        .record_subresource_network(network_record);
                                    if let PendingSubresourceContinuation::Xhr { xhr, .. } =
                                        streaming.pending.continuation
                                    {
                                        let xhr = v8::Local::new(scope, &xhr);
                                        crate::network_host::apply_xhr_streaming_failure(
                                            scope, xhr, internal_id,
                                        );
                                    }
                                    context_host
                                        .borrow_mut()
                                        .record_pending_subresource_continue_event(
                                            PendingSubresourceContinueEvent::Completed {
                                                internal_id,
                                            },
                                        );
                                    defer_subresource_owner_async_scope(
                                        &context_host,
                                        scope,
                                        pending_owner,
                                        owner_async_scope,
                                    );
                                    return Ok(
                                        AsyncSubresourceFetchBodyActivity::WindowRealmEntered,
                                    );
                                }
                            }
                        } else {
                            None
                        };
                        let close_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        crate::network_host::close_pending_network_body_stream(
                            scope,
                            body_source_id,
                        );
                        trace_async_subresource_stage(
                            "async_subresource_stream_closed",
                            trace_fields,
                            close_started,
                        );
                        let record_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        let request_handle = streaming.pending.info.network_request_handle;
                        if matches!(
                            &streaming.pending.continuation,
                            PendingSubresourceContinuation::Xhr { .. }
                        ) && let Some(handle) = request_handle
                        {
                            context_host.borrow_mut().record_subresource_body_finished(
                                crate::types::SubresourceBodyFinished::ready_after_streaming(
                                    handle,
                                    response_body,
                                ),
                            );
                        } else {
                            let mut network_record =
                                crate::types::SubresourceNetworkRecord::success_with_body(
                                    streaming.pending.info.frame_id.clone(),
                                    streaming.pending.info.document_url.clone(),
                                    streaming.request_url,
                                    streaming.request_method,
                                    streaming.request_headers,
                                    streaming.request_body,
                                    streaming.pending.info.resource_type,
                                    request_cookie_report,
                                    streaming
                                        .head
                                        .redirect_chain
                                        .clone()
                                        .into_iter()
                                        .map(Into::into)
                                        .collect(),
                                    streaming.head.final_url.clone(),
                                    streaming.head.status,
                                    streaming.head.headers.clone(),
                                    response_body,
                                    streaming.head.cookie_set_reports.clone(),
                                )
                                .with_from_cache(streaming.head.from_cache)
                                .with_negotiated_http_version(
                                    streaming.head.negotiated_http_version,
                                )
                                .with_network_request_headers(
                                    streaming.network_request_headers.clone(),
                                )
                                .with_request_body_bytes(
                                    streaming.pending.info.request_body_bytes.clone(),
                                );
                            if let Some(handle) = request_handle {
                                network_record = network_record.with_request_handle(handle);
                            }
                            context_host
                                .borrow_mut()
                                .record_subresource_network(network_record);
                        }
                        trace_async_subresource_stage(
                            "async_subresource_streaming_network_recorded",
                            trace_fields,
                            record_started,
                        );
                        let request_origin = streaming.pending.request_origin();
                        if let PendingSubresourceContinuation::Xhr { xhr, .. } =
                            streaming.pending.continuation
                            && let Some(response_body) = xhr_delivery_body
                        {
                            let xhr_started =
                                moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                            crate::context_bootstrap::record_resource_performance_entry(
                                scope,
                                crate::context_bootstrap::ResourcePerformanceEntry::from_streaming_network_response(
                                    streaming.pending.info.url.as_str(),
                                    "xmlhttprequest",
                                    None,
                                    &streaming.head,
                                    response_body_size,
                                ),
                            );
                            let xhr = v8::Local::new(scope, &xhr);
                            let mut observable_head = streaming.head;
                            if !streaming
                                .response_filter
                                .is_some_and(|filter| filter.is_readable())
                            {
                                observable_head.headers =
                                    crate::network_host::filter_cors_exposed_response_headers(
                                        &request_origin,
                                        &observable_head,
                                        streaming.pending.credentials_mode,
                                    );
                            }
                            // XHR still exposes a complete response at DONE. Keep the
                            // network transfer and CDP record streaming/spooled, and
                            // materialize only at this Web-visible completion boundary.
                            crate::network_host::apply_xhr_streaming_response_body_source(
                                scope,
                                xhr,
                                observable_head,
                                response_body,
                                internal_id,
                            );
                            trace_async_subresource_stage(
                                "async_subresource_streaming_xhr_delivered",
                                trace_fields,
                                xhr_started,
                            );
                        }
                    }
                    Err(error_text) => {
                        let error_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        crate::network_host::error_pending_network_body_stream(
                            scope,
                            body_source_id,
                            error_text.clone(),
                        );
                        let network_error_text = crate::network_host::ABORTED_ERROR_TEXT.to_owned();
                        let request_handle = streaming.pending.info.network_request_handle;
                        if let Some(handle) = request_handle {
                            let partial_body = streaming.body_writer.finish();
                            if !matches!(
                                &streaming.pending.continuation,
                                PendingSubresourceContinuation::Xhr { .. }
                            ) {
                                context_host
                                    .borrow_mut()
                                    .record_subresource_response_started(
                                        crate::types::SubresourceResponseStarted::new(
                                            handle,
                                            streaming
                                                .head
                                                .redirect_chain
                                                .clone()
                                                .into_iter()
                                                .map(Into::into)
                                                .collect(),
                                            streaming.head.final_url.clone(),
                                            streaming.head.status,
                                            streaming.head.headers.clone(),
                                            streaming.head.cookie_set_reports.clone(),
                                        )
                                        .with_from_cache(streaming.head.from_cache)
                                        .with_negotiated_http_version(
                                            streaming.head.negotiated_http_version,
                                        )
                                        .with_network_request_headers(
                                            streaming.network_request_headers.clone(),
                                        ),
                                    );
                            }
                            context_host.borrow_mut().record_subresource_body_finished(
                                crate::types::SubresourceBodyFinished::failed_with_partial_body(
                                    handle,
                                    network_error_text,
                                    partial_body,
                                ),
                            );
                        } else {
                            context_host.borrow_mut().record_subresource_network(
                                crate::types::SubresourceNetworkRecord::failure(
                                    streaming.pending.info.frame_id.clone(),
                                    streaming.pending.info.document_url.clone(),
                                    streaming.request_url,
                                    streaming.request_method,
                                    streaming.request_headers,
                                    streaming.request_body,
                                    streaming.pending.info.resource_type,
                                    network_error_text,
                                )
                                .with_request_body_bytes(
                                    streaming.pending.info.request_body_bytes.clone(),
                                ),
                            );
                        }
                        trace_async_subresource_stage(
                            "async_subresource_streaming_error_recorded",
                            trace_fields,
                            error_started,
                        );
                        if let PendingSubresourceContinuation::Xhr { xhr, .. } =
                            &streaming.pending.continuation
                        {
                            let xhr = v8::Local::new(scope, xhr);
                            crate::network_host::apply_xhr_streaming_failure(
                                scope, xhr, internal_id,
                            );
                        }
                    }
                }
                context_host
                    .borrow_mut()
                    .record_pending_subresource_continue_event(
                        PendingSubresourceContinueEvent::Completed { internal_id },
                    );
                defer_subresource_owner_async_scope(
                    &context_host,
                    scope,
                    pending_owner,
                    owner_async_scope,
                );
                Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
            });
        trace_async_subresource_stage(
            "async_subresource_streaming_finish_done",
            trace_fields,
            trace_started,
        );
        result
    }
}
