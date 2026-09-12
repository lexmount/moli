use super::*;

impl ScriptVm {
    pub(super) fn resolve_network_only_subresource_fetch(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<()> {
        let detached_window_fetch = pending.continuation.is_detached_window_fetch();
        debug_assert!(
            detached_window_fetch || pending.execution_context.is_window_network_only(),
            "network-only completion must be an accepted fire-and-forget request or detached Fetch"
        );
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);

        let result = if detached_window_fetch {
            result.and_then(|response| {
                if !response.redirect_chain.is_empty()
                    && let Some(message) = detached_window_fetch_csp_redirect_failure_message(
                        &self._context_host,
                        &pending,
                        &response.final_url,
                    )
                {
                    return Err(message);
                }
                if !skip_fetch_security_validation {
                    crate::network_host::validate_fetch_response_security_policy_with_body(
                        &pending.request_origin,
                        &response.head(),
                        response.body_bytes(),
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    )?;
                }
                Ok(response)
            })
        } else {
            result
        };

        match result {
            Ok(response) => {
                let network_request_headers = response
                    .network_request_headers()
                    .map(|headers| headers.to_vec());
                let request_cookie_report = response
                    .request_cookie_report
                    .clone()
                    .or_else(|| pending.info.request_cookie_report.clone());
                let response_body = SubresourceResponseBody::from_navigation_response(&response);
                let mut network_record = crate::types::SubresourceNetworkRecord::success_with_body(
                    pending.info.frame_id.clone(),
                    pending.info.document_url.clone(),
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    pending.info.resource_type,
                    request_cookie_report,
                    response.redirect_chain,
                    response.final_url,
                    response.status,
                    response.headers,
                    response_body,
                    response.cookie_set_reports,
                )
                .with_from_cache(response.from_cache)
                .with_negotiated_http_version(response.negotiated_http_version)
                .with_network_request_headers(network_request_headers)
                .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
                .with_request_body_bytes(pending.info.request_body_bytes.clone());
                if let Some(status_text) = response_status_text.as_deref() {
                    network_record = network_record.with_response_status_text(status_text);
                }
                if let Some(handle) = pending.info.network_request_handle {
                    network_record = network_record.with_request_handle(handle);
                }
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
            Err(error_text) => {
                let error_text = network_error_text.as_deref().unwrap_or(&error_text);
                let mut network_record = crate::types::SubresourceNetworkRecord::failure(
                    pending.info.frame_id.clone(),
                    pending.info.document_url.clone(),
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    pending.info.resource_type,
                    error_text.to_owned(),
                )
                .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
                .with_request_body_bytes(pending.info.request_body_bytes.clone());
                if let Some(handle) = pending.info.network_request_handle {
                    network_record = network_record.with_request_handle(handle);
                }
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
        }
        tracing::debug!(
            internal_id = pending.info.internal_id,
            detached_owner = ?pending.execution_context.detached_window_fetch_identity(),
            accepted_context = ?pending.execution_context.window_network_only_identity(),
            accepted_document = ?pending.execution_context.window_document_network_only_identity(),
            "completed network-only subresource without entering V8"
        );
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_pending_event_source_fetch(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);
        let context_host = self._context_host.clone();
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(
                    scope,
                    pending
                        .execution_context
                        .context_global()
                        .expect("EventSource completion must retain its V8 context"),
                );
                let scope = &mut v8::ContextScope::new(scope, context);
                if !window_subresource_realm_is_current(&context_host, scope, &pending) {
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }
                let owner_async_scope = enter_subresource_owner_async_scope(
                    &context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                );
                let event_source = match &pending.continuation {
                    PendingSubresourceContinuation::EventSource(event_source) => {
                        v8::Local::new(scope, event_source)
                    }
                    _ => unreachable!("EventSource completion requires EventSource continuation"),
                };
                let request_handle = pending.info.network_request_handle;

                match result {
                    Err(error_text) => {
                        let error_text = network_error_text.unwrap_or(error_text);
                        let mut record = crate::types::SubresourceNetworkRecord::failure(
                            pending.info.frame_id.clone(),
                            pending.info.document_url.clone(),
                            request_url,
                            request_method,
                            request_headers,
                            request_body,
                            SubresourceResourceType::EventSource,
                            error_text,
                        );
                        if let Some(handle) = request_handle {
                            record = record.with_request_handle(handle);
                        }
                        context_host
                            .borrow_mut()
                            .record_subresource_network(record);
                        crate::network_host::fail_event_source_connection(
                            scope,
                            event_source,
                            crate::network_host::EventSourceTerminalMode::Reconnect,
                        );
                    }
                    Ok(response) => {
                        let security_error = if !response.redirect_chain.is_empty() {
                            document_connect_csp_redirect_failure_message(
                                scope,
                                &context_host,
                                &pending,
                                &response.final_url,
                            )
                        } else {
                            None
                        }
                        .or_else(|| {
                            (!skip_fetch_security_validation)
                                .then(|| {
                                    crate::network_host::validate_fetch_response_security_policy_with_body(
                                        &pending.request_origin,
                                        &response.head(),
                                        response.body_bytes(),
                                        pending.request_mode,
                                        pending.credentials_mode,
                                        pending.policy_context,
                                    )
                                    .err()
                                })
                                .flatten()
                        });
                        if let Some(error_text) = security_error {
                            let mut record = crate::types::SubresourceNetworkRecord::failure(
                                pending.info.frame_id.clone(),
                                pending.info.document_url.clone(),
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                SubresourceResourceType::EventSource,
                                error_text,
                            );
                            if let Some(handle) = request_handle {
                                record = record.with_request_handle(handle);
                            }
                            context_host
                                .borrow_mut()
                                .record_subresource_network(record);
                            crate::network_host::fail_event_source_connection(
                                scope,
                                event_source,
                                crate::network_host::EventSourceTerminalMode::Close,
                            );
                        } else {
                            let head = response.head();
                            if let Some(handle) = request_handle {
                                context_host
                                    .borrow_mut()
                                    .record_subresource_response_started(
                                        crate::types::SubresourceResponseStarted::new(
                                            handle,
                                            response.redirect_chain.clone(),
                                            response.final_url.clone(),
                                            response.status,
                                            response.headers.clone(),
                                            response.cookie_set_reports.clone(),
                                        )
                                        .with_status_text(response_status_text)
                                        .with_from_cache(response.from_cache)
                                        .with_negotiated_http_version(
                                            response.negotiated_http_version,
                                        )
                                        .with_network_request_headers(
                                            response
                                                .network_request_headers()
                                                .map(|headers| headers.to_vec()),
                                        ),
                                    );
                            }
                            if let Some(error_text) =
                                crate::network_host::event_source_response_error(&head)
                            {
                                if let Some(handle) = request_handle {
                                    context_host
                                        .borrow_mut()
                                        .record_subresource_body_finished(
                                            crate::types::SubresourceBodyFinished::failed(
                                                handle, error_text,
                                            ),
                                        );
                                }
                                crate::network_host::fail_event_source_connection(
                                    scope,
                                    event_source,
                                    crate::network_host::EventSourceTerminalMode::Close,
                                );
                            } else {
                                crate::network_host::open_event_source_connection(
                                    scope,
                                    event_source,
                                    &response.final_url,
                                );
                                let bytes = response.body_bytes();
                                if let Some(handle) = request_handle
                                    && !bytes.is_empty()
                                {
                                    context_host.borrow_mut().record_subresource_data_received(
                                        crate::types::SubresourceDataReceived::new(
                                            handle,
                                            bytes.len(),
                                            bytes.len(),
                                        ),
                                    );
                                }
                                let mut parser = crate::network_host::EventSourceParser::new(
                                    crate::network_host::event_source_last_event_id(
                                        scope,
                                        event_source,
                                    ),
                                    crate::network_host::event_source_reconnect_delay_ms(
                                        scope,
                                        event_source,
                                    ),
                                );
                                if crate::network_host::event_source_ready_state(scope, event_source)
                                    != crate::network_host::EVENT_SOURCE_CLOSED
                                {
                                    let messages = parser.push(bytes);
                                    dispatch_streaming_event_source_messages(
                                        &context_host,
                                        scope,
                                        event_source,
                                        request_handle,
                                        &messages,
                                    );
                                }
                                if crate::network_host::event_source_ready_state(scope, event_source)
                                    != crate::network_host::EVENT_SOURCE_CLOSED
                                {
                                    crate::network_host::update_event_source_stream_state(
                                        scope,
                                        event_source,
                                        parser.last_event_id(),
                                        parser.reconnect_delay_ms(),
                                    );
                                }
                                if let Some(handle) = request_handle {
                                    context_host.borrow_mut().record_subresource_body_finished(
                                        crate::types::SubresourceBodyFinished::ready_after_streaming(
                                            handle,
                                            SubresourceResponseBody::from_navigation_response(
                                                &response,
                                            ),
                                        ),
                                    );
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
                            }
                        }
                    }
                }
                defer_subresource_owner_async_scope(
                    &context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                    owner_async_scope,
                );
                Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
            })
    }
    pub(super) fn resolve_pending_subresource_fetch_body(
        &mut self,
        pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        response_filter: Option<AsyncSubresourceFetchResponseFilter>,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        self.resolve_pending_subresource_fetch_completion(
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
            response_status_text,
            skip_fetch_security_validation,
            response_filter,
            network_error_text,
            result.into(),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_pending_subresource_fetch_completion(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        response_filter: Option<AsyncSubresourceFetchResponseFilter>,
        network_error_text: Option<String>,
        completion_result: AsyncSubresourceFetchResult,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let (supplied_parkable_image, result) = match completion_result {
            AsyncSubresourceFetchResult::Response(response) => (None, Ok(response)),
            AsyncSubresourceFetchResult::Image { response, encoded } => {
                (Some(encoded), Ok(response))
            }
            AsyncSubresourceFetchResult::Failure(error) => (None, Err(error)),
        };
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields = async_subresource_trace_fields_for_pending(
            "completion",
            pending.info.internal_id,
            &pending,
        );
        trace_async_subresource_stage(
            "async_subresource_resolve_pending_start",
            trace_fields,
            trace_started,
        );
        if pending.continuation.is_detached_window_fetch()
            || pending.execution_context.is_window_network_only()
        {
            return self
                .resolve_network_only_subresource_fetch(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    response_status_text,
                    skip_fetch_security_validation,
                    network_error_text,
                    result,
                )
                .map(|()| AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if !window_subresource_owner_is_current(&self._context_host, &pending) {
            tracing::debug!(
                internal_id = pending.info.internal_id,
                owner = ?pending.execution_context.window_request_target().map(crate::native_bridge::WindowTaskTarget::owner),
                "discarded subresource completion for retired Window execution context"
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if pending.continuation.is_window_event_source() {
            return self.resolve_pending_event_source_fetch(
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text,
                skip_fetch_security_validation,
                network_error_text,
                result,
            );
        }
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);
        let context_host = self._context_host.clone();
        let mut completed_web_font = None;
        let result = self.renderer_document_isolate.with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(
                scope,
                pending
                    .execution_context
                    .context_global()
                    .expect("active subresource completion must retain its V8 context"),
            );
            let scope = &mut v8::ContextScope::new(scope, context);
            if !window_subresource_realm_is_current(&context_host, scope, &pending) {
                tracing::debug!(
                    internal_id = pending.info.internal_id,
                    expected_realm = ?pending.execution_context.realm_token(),
                    "discarded subresource completion for retired V8 realm"
                );
                return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
            }
            trace_async_subresource_stage(
                "async_subresource_context_entered",
                trace_fields,
                trace_started,
            );
            let owner_async_scope =
                enter_subresource_owner_async_scope(
                    &context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                );

            let security_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
            let mut opaque_response_blocked = false;
            let result = result.and_then(|response| {
                if !response.redirect_chain.is_empty()
                    && let Some(message) = document_connect_csp_redirect_failure_message(
                        scope,
                        &context_host,
                        &pending,
                        &response.final_url,
                    )
                {
                    return Err(message);
                }
                if !skip_fetch_security_validation
                    && matches!(
                    pending.info.resource_type,
                    SubresourceResourceType::Audio
                        | SubresourceResourceType::Fetch
                        | SubresourceResourceType::Font
                        | SubresourceResourceType::Image
                        | SubresourceResourceType::Media
                        | SubresourceResourceType::TextTrack
                        | SubresourceResourceType::Video
                        | SubresourceResourceType::Xhr
                ) {
                    let supplied_snapshot = supplied_parkable_image
                        .as_ref()
                        .map(|image| {
                            image.snapshot().map_err(|error| {
                                format!("failed to read image response bytes: {error}")
                            })
                        })
                        .transpose()?;
                    let response_body = supplied_snapshot
                        .as_ref()
                        .map_or_else(|| response.body_bytes(), AsRef::as_ref);
                    let validation = crate::network_host::validate_fetch_response_security_policy_with_body_classified(
                        &pending.request_origin,
                        &response.head(),
                        response_body,
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    );
                    match validation {
                        Ok(()) => {}
                        Err(crate::network_host::FetchResponseSecurityViolation::OpaqueResponseBlocked(_))
                            if pending.continuation.is_window_fetch() =>
                        {
                            opaque_response_blocked = true;
                        }
                        Err(violation) => return Err(violation.into_message()),
                    }
                }
                Ok(response)
            });
            trace_async_subresource_stage(
                "async_subresource_security_checked",
                trace_fields,
                security_started,
            );

            let request_initiator_type = pending.continuation.request_initiator_type();
            match result {
                Ok(mut response) => {
                    let response_status = response.status;
                    let response_request_method = request_method.clone();
                    let parkable_image = (!opaque_response_blocked
                        && pending.info.resource_type == SubresourceResourceType::Image)
                        .then(|| {
                            supplied_parkable_image.clone().unwrap_or_else(|| {
                                let runner = pending.load.task_runner();
                                pending
                                    .load
                                    .request_client()
                                    .parkable_image_manager(&runner)
                                    .from_frozen_bytes(response.take_body_bytes())
                            })
                        });
                    let request_cookie_report = response
                        .request_cookie_report
                        .clone()
                        .or_else(|| pending.info.request_cookie_report.clone());
                    let record_started =
                        moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    let mut network_record = if opaque_response_blocked {
                        crate::types::SubresourceNetworkRecord::failure(
                            pending.info.frame_id.clone(),
                            pending.info.document_url.clone(),
                            request_url.clone(),
                            request_method,
                            request_headers,
                            request_body,
                            pending.info.resource_type,
                            crate::network_host::ABORTED_ERROR_TEXT.to_owned(),
                        )
                        .with_request_initiator_type(request_initiator_type)
                        .with_request_body_bytes(pending.info.request_body_bytes.clone())
                    } else {
                        crate::types::SubresourceNetworkRecord::success_with_body(
                            pending.info.frame_id.clone(),
                            pending.info.document_url.clone(),
                            request_url.clone(),
                            request_method,
                            request_headers,
                            request_body,
                            pending.info.resource_type,
                            request_cookie_report,
                            response.redirect_chain.clone().into_iter().collect(),
                            response.final_url.clone(),
                            response.status,
                            response.headers.clone(),
                            parkable_image.as_ref().map_or_else(
                                || SubresourceResponseBody::from_navigation_response(&response),
                                |image| {
                                    SubresourceResponseBody::from_parkable_image(image.clone())
                                },
                            ),
                            response.cookie_set_reports.clone(),
                        )
                        .with_from_cache(response.from_cache)
                        .with_negotiated_http_version(response.negotiated_http_version)
                        .with_network_request_headers(
                            response.network_request_headers().map(|headers| headers.to_vec()),
                        )
                        .with_request_initiator_type(request_initiator_type)
                        .with_request_body_bytes(pending.info.request_body_bytes.clone())
                    };
                    if !opaque_response_blocked
                        && let Some(status_text) = response_status_text.as_deref()
                    {
                        network_record =
                            network_record.with_response_status_text(status_text);
                    }
                    if let Some(handle) = pending.info.network_request_handle {
                        network_record = network_record.with_request_handle(handle);
                    }
                    context_host
                        .borrow_mut()
                        .record_subresource_network(network_record);
                    trace_async_subresource_stage(
                        "async_subresource_network_recorded",
                        trace_fields,
                        record_started,
                    );
                    let mut observable_response = response;
                    if pending.info.resource_type == SubresourceResourceType::Xhr && !response_filter.as_ref().is_some_and(|filter| filter.is_readable()) {
                        observable_response.headers =
                            crate::network_host::filter_cors_exposed_response_headers(
                                &pending.request_origin,
                                &observable_response.head(),
                                pending.credentials_mode,
                            );
                    }
                    let continuation_started =
                        moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    match pending.continuation {
                        PendingSubresourceContinuation::Fetch(fetch) => {
                            let redirect_mode = fetch.redirect_mode();
                            let resolver = fetch
                                .into_resolver()
                                .expect("detached keepalive completion is handled before V8 entry");
                            let resolver = v8::Local::new(scope, &resolver);
                            let (mut head, body) = observable_response.into_body();
                            let response_request = crate::network_host::FetchResponseRequest {
                                method: &response_request_method,
                                mode: pending.request_mode,
                                redirect_mode,
                            };
                            let response_filter = response_filter.or_else(|| Some(response_request.network_response_filter(
                                &pending.request_origin,
                                &head,
                                pending.credentials_mode,
                            )));
                            if let Some(status_text) = response_status_text {
                                head.status_text = Some(status_text);
                            }
                            let body = if opaque_response_blocked {
                                moli_fetch::ResponseBody::materialized_bytes(Vec::new())
                            } else {
                                body
                            };
                            // ORB discards the internal body; request policy
                            // still selects opaque versus opaqueredirect.
                            // Preserve explicit service-worker filter overrides.
                            let response_obj =
                                crate::network_host::build_fetch_response_object_from_body_source_for_request_mode_with_filter(
                                    scope,
                                    &pending.request_origin,
                                    response_request,
                                    head,
                                    body,
                                    response_filter,
                                );
                            resolver.resolve(scope, response_obj.into());
                        }
                        PendingSubresourceContinuation::Xhr { xhr, .. } => {
                            crate::context_bootstrap::record_resource_performance_entry(
                                scope,
                                crate::context_bootstrap::ResourcePerformanceEntry::from_network_response(
                                    pending.info.url.as_str(),
                                    "xmlhttprequest",
                                    None,
                                    &observable_response,
                                ),
                            );
                            let xhr = v8::Local::new(scope, &xhr);
                            let (head, body) = observable_response.into_body();
                            crate::network_host::apply_xhr_response_body_source_with_status_text(
                                scope,
                                xhr,
                                head,
                                body,
                                response_status_text.as_deref(),
                            );
                        }
                        PendingSubresourceContinuation::Image {
                            image_handle,
                            sequence,
                            ..
                        } => {
                            let encoded = parkable_image
                                .as_ref()
                                .expect("an accepted image response must be parkable");
                            apply_image_subresource_terminal(
                                scope,
                                &context_host,
                                image_handle,
                                sequence,
                                pending.info.internal_id,
                                &pending.info.url,
                                ImageSubresourceTerminal::Response {
                                    response: &observable_response,
                                    encoded,
                                },
                            )
                        }
                        PendingSubresourceContinuation::Media {
                            media_handle,
                            sequence,
                        } => apply_media_subresource_terminal(
                            scope,
                            &context_host,
                            media_handle,
                            sequence,
                            pending.info.internal_id,
                            crate::network_host::media_response_status_is_successful(
                                response_status,
                            ),
                        ),
                        PendingSubresourceContinuation::TextTrack {
                            track_handle,
                            sequence,
                        } => apply_text_track_subresource_terminal(
                            scope,
                            &context_host,
                            track_handle,
                            sequence,
                            pending.info.internal_id,
                            crate::network_host::text_track_response_result(
                                response_status,
                                observable_response.body_text(),
                            ),
                        ),
                        PendingSubresourceContinuation::StylesheetSubresource {
                            binding,
                            web_font,
                            css_image,
                        } => {
                            if let Some(identity) = css_image.as_ref() {
                                let descriptor = crate::network_host::image_response_descriptor_from_parkable(
                                    &observable_response,
                                    parkable_image
                                        .as_ref()
                                        .expect("a CSS image response must be parkable"),
                                );
                                let _ = context_host
                                    .borrow_mut()
                                    .complete_stylesheet_css_image_response(
                                        identity,
                                        descriptor,
                                        parkable_image
                                            .as_ref()
                                            .expect("a CSS image response must be parkable")
                                            .clone(),
                                    );
                            }
                            if binding.child_handle().is_none() {
                                completed_web_font = web_font.map(|font| {
                                    if (200..=299).contains(&response_status)
                                        && !opaque_response_blocked
                                    {
                                        crate::css_resource_urls::CompletedStylesheetWebFont::response(
                                            font,
                                            observable_response.clone_body_bytes(),
                                        )
                                    } else {
                                        crate::css_resource_urls::CompletedStylesheetWebFont::failure(
                                            font,
                                        )
                                    }
                                });
                            }
                            apply_stylesheet_subresource_terminal(&context_host, binding);
                        }
                        PendingSubresourceContinuation::Beacon
                        | PendingSubresourceContinuation::CspReport { .. }
                        | PendingSubresourceContinuation::EventSource(_)
                        | PendingSubresourceContinuation::WebSocket(_)
                        | PendingSubresourceContinuation::WorkerFetch { .. }
                        | PendingSubresourceContinuation::WorkerXhr { .. }
                        | PendingSubresourceContinuation::WorkerCspReport { .. }
                        | PendingSubresourceContinuation::SharedWorkerFetch { .. }
                        | PendingSubresourceContinuation::SharedWorkerXhr { .. }
                        | PendingSubresourceContinuation::SharedWorkerCspReport { .. } => {}
                    }
                    trace_async_subresource_stage(
                        "async_subresource_continuation_delivered",
                        trace_fields,
                        continuation_started,
                    );
                }
                Err(error_text) => {
                    let network_error_text = network_error_text
                        .as_deref()
                        .or_else(|| {
                            crate::network_host::is_cors_policy_failure_message(&error_text)
                                .then_some(crate::network_host::FAILED_ERROR_TEXT)
                        })
                        .unwrap_or(&error_text);
                    let record_started =
                        moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    let mut network_record = crate::types::SubresourceNetworkRecord::failure(
                        pending.info.frame_id.clone(),
                        pending.info.document_url.clone(),
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                        pending.info.resource_type,
                        network_error_text.to_owned(),
                    )
                    .with_request_initiator_type(request_initiator_type);
                    if let Some(handle) = pending.info.network_request_handle {
                        network_record = network_record.with_request_handle(handle);
                    }
                    context_host
                        .borrow_mut()
                        .record_subresource_network(network_record);
                    trace_async_subresource_stage(
                        "async_subresource_network_recorded",
                        trace_fields,
                        record_started,
                    );
                    let continuation_started =
                        moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    match pending.continuation {
                        PendingSubresourceContinuation::Fetch(fetch) => {
                            let resolver = fetch
                                .into_resolver()
                                .expect("detached keepalive failure is handled before V8 entry");
                            let resolver = v8::Local::new(scope, &resolver);
                            let exception = v8_string(scope, &error_text)
                                .map(|message| v8::Exception::type_error(scope, message))
                                .unwrap_or_else(|| v8::undefined(scope).into());
                            resolver.reject(scope, exception);
                        }
                        PendingSubresourceContinuation::Xhr { xhr, .. } => {
                            let xhr = v8::Local::new(scope, &xhr);
                            crate::network_host::apply_xhr_failure(scope, xhr);
                        }
                        PendingSubresourceContinuation::Image {
                            image_handle,
                            sequence,
                            ..
                        } => apply_image_subresource_terminal(
                            scope,
                            &context_host,
                            image_handle,
                            sequence,
                            pending.info.internal_id,
                            &pending.info.url,
                            ImageSubresourceTerminal::Failure,
                        ),
                        PendingSubresourceContinuation::Media {
                            media_handle,
                            sequence,
                        } => apply_media_subresource_terminal(
                            scope,
                            &context_host,
                            media_handle,
                            sequence,
                            pending.info.internal_id,
                            false,
                        ),
                        PendingSubresourceContinuation::TextTrack {
                            track_handle,
                            sequence,
                        } => apply_text_track_subresource_terminal(
                            scope,
                            &context_host,
                            track_handle,
                            sequence,
                            pending.info.internal_id,
                            Err(error_text.clone()),
                        ),
                        PendingSubresourceContinuation::StylesheetSubresource {
                            binding,
                            web_font,
                            css_image,
                        } => {
                            if let Some(identity) = css_image.as_ref() {
                                let _ = context_host
                                    .borrow_mut()
                                    .fail_stylesheet_css_image(identity);
                            }
                            if binding.child_handle().is_none() {
                                completed_web_font = web_font.map(
                                    crate::css_resource_urls::CompletedStylesheetWebFont::failure,
                                );
                            }
                            apply_stylesheet_subresource_terminal(&context_host, binding);
                        }
                        PendingSubresourceContinuation::Beacon
                        | PendingSubresourceContinuation::CspReport { .. }
                        | PendingSubresourceContinuation::EventSource(_)
                        | PendingSubresourceContinuation::WebSocket(_)
                        | PendingSubresourceContinuation::WorkerFetch { .. }
                        | PendingSubresourceContinuation::WorkerXhr { .. }
                        | PendingSubresourceContinuation::WorkerCspReport { .. }
                        | PendingSubresourceContinuation::SharedWorkerFetch { .. }
                        | PendingSubresourceContinuation::SharedWorkerXhr { .. }
                        | PendingSubresourceContinuation::SharedWorkerCspReport { .. } => {}
                    }
                    trace_async_subresource_stage(
                        "async_subresource_continuation_delivered",
                        trace_fields,
                        continuation_started,
                    );
                }
            }

            defer_subresource_owner_async_scope(
                &context_host,
                scope,
                pending.execution_context.dispatch_scope(),
                owner_async_scope,
            );
            Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
        });
        if let Some(web_font) = completed_web_font {
            self.complete_document_web_font(web_font);
        }
        trace_async_subresource_stage(
            "async_subresource_resolve_pending_done",
            trace_fields,
            trace_started,
        );
        result
    }
}
