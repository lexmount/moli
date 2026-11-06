//! EventSource uses the Worker's native fetch transport and networking turns.

use super::fetch::WorkerFetchKind;
use super::*;
use crate::network_host::{
    EventSourceParser, EventSourceTerminalMode, event_source_active_request_id,
    event_source_connection_url, event_source_last_event_id, event_source_ready_state,
    event_source_reconnect_delay_ms, event_source_with_credentials, fail_event_source_connection,
    set_event_source_active_request_id,
};

pub(super) fn parser_for_request(
    kind: WorkerFetchKind,
    headers: &moli_fetch::RequestHeaders,
) -> Option<EventSourceParser> {
    let WorkerFetchKind::EventSource { reconnect_delay_ms } = kind else {
        return None;
    };
    let last_event_id = headers
        .to_byte_strings()
        .into_iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("Last-Event-ID"))
        .map(|(_, value)| {
            let bytes = value.chars().map(|ch| ch as u8).collect::<Vec<_>>();
            String::from_utf8_lossy(&bytes).into_owned()
        })
        .unwrap_or_default();
    Some(EventSourceParser::new(last_event_id, reconnect_delay_ms))
}

pub(super) fn send_stream_chunk(
    completion: &mpsc::UnboundedSender<WorkerFetchEvent>,
    parser: &mut Option<EventSourceParser>,
    fetch_id: u32,
    body_source_id: NetworkBodySourceId,
    bytes: Vec<u8>,
) {
    if let Some(parser) = parser {
        let messages = parser.push(&bytes);
        let _ = completion.send(WorkerFetchEvent::EventSourceState {
            fetch_id,
            body_source_id,
            last_event_id: parser.last_event_id().to_owned(),
            reconnect_delay_ms: parser.reconnect_delay_ms(),
        });
        for message in messages {
            let _ = completion.send(WorkerFetchEvent::EventSourceMessage {
                fetch_id,
                body_source_id,
                message,
            });
        }
    } else {
        let _ = completion.send(WorkerFetchEvent::StreamingChunk(
            WorkerFetchStreamingChunk {
                fetch_id,
                body_source_id,
                bytes,
            },
        ));
    }
}

fn current_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    fetch_id: u32,
    body_source_id: Option<NetworkBodySourceId>,
) -> Option<v8::Local<'s, v8::Object>> {
    let source = {
        let state = state.borrow();
        let pending = state.pending_fetches.get(&fetch_id)?;
        if pending.load.is_cancelled()
            || body_source_id.is_some_and(|id| pending.streaming_body_source_id != Some(id))
        {
            return None;
        }
        v8::Local::new(scope, pending.client.event_source()?)
    };
    (event_source_active_request_id(scope, source) == Some(u64::from(fetch_id))
        && event_source_ready_state(scope, source) != crate::network_host::EVENT_SOURCE_CLOSED)
        .then_some(source)
}

pub(super) fn apply_state(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    fetch_id: u32,
    body_source_id: NetworkBodySourceId,
    last_event_id: &str,
    reconnect_delay_ms: u64,
) {
    if let Some(source) = current_source(scope, state, fetch_id, Some(body_source_id)) {
        crate::network_host::update_event_source_stream_state(
            scope,
            source,
            last_event_id,
            reconnect_delay_ms,
        );
    }
}

pub(super) fn apply_message(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    fetch_id: u32,
    body_source_id: NetworkBodySourceId,
    message: &crate::network_host::EventSourceMessage,
) {
    if let Some(source) = current_source(scope, state, fetch_id, Some(body_source_id))
        && event_source_ready_state(scope, source) == crate::network_host::EVENT_SOURCE_OPEN
    {
        crate::network_host::dispatch_event_source_message(scope, source, message);
    }
}

pub(crate) fn start_worker_event_source_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let Some(document_url) = state.borrow().current_script_url.clone() else {
        return false;
    };
    let Some(url) =
        event_source_connection_url(scope, source).and_then(|url| Url::parse(&url).ok())
    else {
        return false;
    };
    let (
        loader,
        policy_context,
        extra_headers,
        offline,
        blocked,
        intercept,
        intercept_type,
        referrer_policy,
        partition,
        completion,
    ) = {
        let state = state.borrow();
        (
            state.loader.clone(),
            state.policy_context,
            state.extra_http_headers.clone(),
            state.network_offline,
            state.blocked_url_patterns.clone(),
            state.fetch_subresource_interception_enabled,
            state.fetch_subresource_interception_resource_type,
            state.referrer_policy.clone(),
            state.network_partition_key.clone(),
            state.fetch_completion_tx.clone(),
        )
    };
    let kind = WorkerFetchKind::EventSource {
        reconnect_delay_ms: event_source_reconnect_delay_ms(scope, source),
    };
    let credentials = if event_source_with_credentials(scope, source) {
        RequestCredentialsMode::Include
    } else {
        RequestCredentialsMode::SameOrigin
    };
    let credentials = policy_context.network_credentials_mode(RequestMode::Cors, credentials);
    let origin = worker_global_origin(scope).unwrap_or(moli_url::WebOrigin::Opaque);
    let timing = WorkerResourceTiming::new(
        scope,
        url.clone(),
        origin,
        WorkerTimingRequest::EventSource {
            credentials,
            policy: policy_context,
        },
    );
    let mut fields = vec![("Accept".to_owned(), "text/event-stream".to_owned())];
    let last_id = event_source_last_event_id(scope, source);
    if !last_id.is_empty() {
        fields.push((
            "Last-Event-ID".to_owned(),
            last_id.bytes().map(char::from).collect(),
        ));
    }
    let headers = merge_worker_request_headers(
        &extra_headers,
        &moli_fetch::RequestHeaders::from_byte_strings(&fields)
            .expect("EventSource native headers are ByteStrings"),
    );
    let cancel = FetchCancelHandle::new();
    let Some(load) = loader.register_load(
        crate::network::loads::ResourceLoadKind::EventSource,
        ResourceLoadDisposition::Ordinary,
        Some(cancel.clone()),
    ) else {
        return false;
    };
    let connect_policy = worker_request_policy_snapshot(&state.borrow(), kind.csp_resource_kind());
    let metadata = crate::service_worker_runtime::ServiceWorkerFetchRequestMetadata {
        cache: "no-store".to_owned(),
        mixed_content_policy: state.borrow().mixed_content_policy(),
        ..Default::default()
    };
    let environment = crate::network_host::BlobUrlFetchEnvironment::for_worker(scope);
    let blob = crate::network_host::CapturedBlobUrl::capture(&url)
        .map(|entry| entry.for_environment(environment.as_ref()));
    let fetch_id = {
        let mut state = state.borrow_mut();
        let id = next_fetch_id(&mut state);
        state.pending_fetches.insert(
            id,
            PendingWorkerFetch {
                upload: None,
                timing: timing.clone(),
                kind,
                client: WorkerFetchClient::EventSource(v8::Global::new(scope, source)),
                redirect_csp_state: crate::network_host::FetchCspRedirectState::new(
                    &connect_policy,
                ),
                connect_policy,
                document_url: document_url.clone(),
                credentials_mode: credentials,
                request_mode: RequestMode::Cors,
                redirect_mode: RequestRedirectMode::Follow,
                request_priority: None,
                request_metadata: metadata.clone(),
                policy_context,
                signal_id: None,
                load: load.clone(),
                request_url: url.clone(),
                blob_url_entry: blob.clone(),
                request_method: "GET".to_owned(),
                request_headers: headers.clone(),
                request_body: None,
                network_request_handle: None,
                network_record: None,
                paused_response: None,
                streaming_body_source_id: None,
                streaming_needs_orb_body_validation: false,
            },
        );
        id
    };
    set_event_source_active_request_id(scope, source, Some(u64::from(fetch_id)));
    let csp_error = check_worker_request_csp(
        scope,
        &state,
        &document_url,
        &url,
        kind.csp_resource_kind(),
        "EventSource",
        crate::content_security_policy::ContentSecurityPolicyDisposition::Enforce,
    );
    let error = csp_error
        .or_else(|| {
            moli_url_policy::route_fetch_url(&url)
                .err()
                .map(|e| e.to_string())
        })
        .or_else(|| {
            should_request_be_blocked_due_to_bad_port(&url)
                .then(|| "EventSource URL has a blocked port".to_owned())
        })
        .or_else(|| {
            worker_url_blocked(&blocked, &url).then(|| BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
        });
    if let Some(error) = error {
        let _ = completion.send(WorkerFetchEvent::EventSourceFailure {
            fetch_id,
            error,
            mode: EventSourceTerminalMode::Close,
        });
        return true;
    }
    if intercept
        && intercept_type.is_none_or(|t| {
            t.has_same_cdp_fetch_interception_type(SubresourceResourceType::EventSource)
        })
        && !matches!(url.scheme(), "blob" | "data")
    {
        let info = PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: None,
            document_url,
            url,
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: headers,
            request_body_bytes: None,
            request_body: None,
            resource_type: SubresourceResourceType::EventSource,
            request_cookie_report: None,
        };
        let _ = state
            .borrow()
            .parent_tx
            .send(WorkerToParentMessage::PendingSubresourceFetch(
                WorkerPendingSubresourceFetch {
                    fetch_id,
                    load,
                    credentials_mode: credentials,
                    request_mode: RequestMode::Cors,
                    network_partition_key: partition,
                    info,
                },
            ));
        return true;
    }
    if offline {
        let _ = completion.send(WorkerFetchEvent::EventSourceFailure {
            fetch_id,
            error: "Network emulation offline".to_owned(),
            mode: EventSourceTerminalMode::Reconnect,
        });
        return true;
    }
    let redirect = super::fetch::worker_fetch_redirect_check(&state.borrow(), fetch_id);
    if let Some((runtime, client_id)) =
        super::fetch::worker_service_worker_controller(&state.borrow(), &url)
    {
        let request_body_owner = crate::context_bootstrap::request_body_owner(scope, |_| {});
        super::fetch::spawn_worker_fetch_service_worker(
            kind,
            runtime,
            client_id,
            load,
            completion,
            fetch_id,
            cancel,
            timing,
            document_url,
            referrer_policy,
            partition,
            policy_context,
            url,
            "GET".to_owned(),
            None,
            None,
            request_body_owner,
            headers,
            RequestMode::Cors,
            credentials,
            RequestRedirectMode::Follow,
            None,
            metadata,
            redirect,
        );
    } else {
        super::fetch::spawn_worker_fetch_network(
            kind,
            load,
            completion,
            fetch_id,
            cancel,
            timing,
            document_url,
            referrer_policy,
            partition,
            url,
            blob,
            "GET".to_owned(),
            None,
            headers,
            RequestMode::Cors,
            credentials,
            RequestRedirectMode::Follow,
            None,
            metadata,
            None,
            true,
            None,
            Vec::new(),
            redirect,
            None,
        );
    }
    true
}

pub(super) fn fail_request(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    fetch_id: u32,
    error: String,
    mode: EventSourceTerminalMode,
) {
    let pending = state.borrow_mut().pending_fetches.remove(&fetch_id);
    let Some(pending) = pending else { return };
    pending.load.cancel();
    pending.timing.reject(scope);
    super::fetch::record_worker_fetch_failure(&state.borrow(), &pending, error);
    if let Some(source) = pending.client.event_source() {
        let source = v8::Local::new(scope, source);
        if event_source_active_request_id(scope, source) == Some(u64::from(fetch_id)) {
            fail_event_source_connection(scope, source, mode);
        }
    }
}

pub(crate) fn abort_worker_event_source(scope: &mut v8::PinScope<'_, '_>, id: u64) {
    let (Some(state), Ok(id)) = (get_worker_state(scope), u32::try_from(id)) else {
        return;
    };
    let pending = state.borrow_mut().pending_fetches.remove(&id);
    if let Some(pending) = pending {
        pending.timing.abort(scope);
        pending.load.cancel();
        if let Some(runtime) = state.borrow().service_worker_runtime.clone() {
            runtime.abort_controlled_fetch(u64::from(id));
        }
        super::fetch::record_worker_fetch_failure(
            &state.borrow(),
            &pending,
            ABORTED_ERROR_TEXT.to_owned(),
        );
    }
}

pub(super) fn start_response(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    started: &WorkerFetchStreamingStarted,
) -> bool {
    let is_source = state
        .borrow()
        .pending_fetches
        .get(&started.fetch_id)
        .is_some_and(|pending| pending.client.event_source().is_some());
    if !is_source {
        return false;
    }
    let (report_only, enforced) = {
        let state = state.borrow();
        let pending = &state.pending_fetches[&started.fetch_id];
        if started.skip_fetch_security_validation {
            (None, None)
        } else {
            super::fetch::worker_fetch_response_csp_violations(&state, pending, &started.head)
        }
    };
    if let Some(violation) = report_only {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
    if let Some(violation) = enforced {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
        fail_request(
            scope,
            state,
            started.fetch_id,
            worker_content_security_policy_error_message(&violation, "EventSource"),
            EventSourceTerminalMode::Close,
        );
        return true;
    }
    let error = {
        let state = state.borrow();
        // Reporting a policy violation can run script that closes the source.
        let Some(pending) = state.pending_fetches.get(&started.fetch_id) else {
            return true;
        };
        validate_response(
            pending,
            &started.head,
            started.response_filter.as_ref(),
            started.skip_fetch_security_validation,
        )
        .err()
        .or_else(|| crate::network_host::event_source_response_error(&started.head))
    };
    if let Some(error) = error {
        fail_request(
            scope,
            state,
            started.fetch_id,
            error,
            EventSourceTerminalMode::Close,
        );
        return true;
    }
    {
        let mut state = state.borrow_mut();
        let pending = state
            .pending_fetches
            .get_mut(&started.fetch_id)
            .expect("current SSE response");
        pending.streaming_body_source_id = Some(started.body_source_id);
        if let Some(headers) = started.network_request_headers.clone()
            && let Some(record) = pending.network_record.as_mut()
        {
            record
                .initial_network_request_headers
                .get_or_insert(headers);
        }
    }
    if let Some(source) =
        current_source(scope, state, started.fetch_id, Some(started.body_source_id))
    {
        crate::network_host::open_event_source_connection(scope, source, &started.head.final_url);
    }
    true
}

fn validate_response(
    pending: &PendingWorkerFetch,
    head: &ResponseHead,
    filter: Option<&crate::types::AsyncSubresourceFetchResponseFilter>,
    skip: bool,
) -> Result<(), String> {
    if filter.is_some_and(|filter| !filter.is_readable()) {
        return Err("EventSource cannot read an opaque response".to_owned());
    }
    if !skip {
        crate::network_host::validate_fetch_response_headers(
            pending.timing.origin(),
            head,
            pending.request_mode,
            pending.credentials_mode,
            pending.policy_context,
        )?;
    }
    Ok(())
}

pub(super) fn complete_response(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    pending: PendingWorkerFetch,
    completion: WorkerFetchCompletion,
) {
    match completion.result {
        Err(error) => {
            let id = completion.fetch_id;
            state.borrow_mut().pending_fetches.insert(id, pending);
            let mode = if crate::network_host::is_cors_policy_failure_message(&error) {
                EventSourceTerminalMode::Close
            } else {
                EventSourceTerminalMode::Reconnect
            };
            fail_request(scope, state, id, error, mode);
        }
        Ok(response) => {
            let head = response.head();
            if let Err(error) = validate_response(
                &pending,
                &head,
                completion.response_filter.as_ref(),
                completion.skip_fetch_security_validation,
            ) {
                state
                    .borrow_mut()
                    .pending_fetches
                    .insert(completion.fetch_id, pending);
                fail_request(
                    scope,
                    state,
                    completion.fetch_id,
                    error,
                    EventSourceTerminalMode::Close,
                );
                return;
            }
            let body = response.subresource_response_body();
            let bytes = match body.try_bytes() {
                Ok(bytes) => bytes.into_owned(),
                Err(error) => {
                    state
                        .borrow_mut()
                        .pending_fetches
                        .insert(completion.fetch_id, pending);
                    fail_request(
                        scope,
                        state,
                        completion.fetch_id,
                        error.to_string(),
                        EventSourceTerminalMode::Reconnect,
                    );
                    return;
                }
            };
            let mut parser = parser_for_request(pending.kind, &pending.request_headers);
            state
                .borrow_mut()
                .pending_fetches
                .insert(completion.fetch_id, pending);
            let tx = state.borrow().fetch_completion_tx.clone();
            let body_source_id = crate::network_host::new_network_body_source_id();
            let _ = tx.send(WorkerFetchEvent::StreamingStarted(
                WorkerFetchStreamingStarted {
                    fetch_id: completion.fetch_id,
                    body_source_id,
                    head: head.clone(),
                    network_request_headers: completion.network_request_headers,
                    response_filter: completion.response_filter,
                    skip_fetch_security_validation: true,
                },
            ));
            send_stream_chunk(&tx, &mut parser, completion.fetch_id, body_source_id, bytes);
            let _ = tx.send(WorkerFetchEvent::StreamingFinished(
                WorkerFetchStreamingFinished {
                    fetch_id: completion.fetch_id,
                    body_source_id,
                    head,
                    result: Ok(body),
                },
            ));
        }
    }
}

pub(super) fn finish_response(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    pending: PendingWorkerFetch,
    finished: WorkerFetchStreamingFinished,
) {
    pending.load.finish();
    pending.timing.publish(scope);
    let source = pending
        .client
        .event_source()
        .map(|source| v8::Local::new(scope, source));
    match finished.result {
        Ok(body) => {
            if let Some(record) = pending.network_record {
                let id = record.internal_id;
                super::fetch::record_worker_subresource_success_with_handle(
                    &state.borrow(),
                    pending.network_request_handle,
                    pending.document_url,
                    record.url,
                    record.method,
                    record.request_headers,
                    record.request_body_bytes,
                    SubresourceResourceType::EventSource,
                    record.initial_network_request_headers,
                    finished.head,
                    body,
                );
                let _ = state
                    .borrow()
                    .parent_tx
                    .send(WorkerToParentMessage::SubresourceContinue(
                        PendingSubresourceContinueEvent::Completed { internal_id: id },
                    ));
            } else {
                super::fetch::record_worker_subresource_success_with_handle(
                    &state.borrow(),
                    pending.network_request_handle,
                    pending.document_url,
                    pending.request_url,
                    pending.request_method,
                    pending.request_headers,
                    None,
                    SubresourceResourceType::EventSource,
                    None,
                    finished.head,
                    body,
                );
            }
        }
        Err(error) => super::fetch::record_worker_fetch_failure(&state.borrow(), &pending, error),
    }
    if let Some(source) = source
        && event_source_active_request_id(scope, source) == Some(u64::from(finished.fetch_id))
        && event_source_ready_state(scope, source) != crate::network_host::EVENT_SOURCE_CLOSED
    {
        fail_event_source_connection(scope, source, EventSourceTerminalMode::Reconnect);
    }
}
