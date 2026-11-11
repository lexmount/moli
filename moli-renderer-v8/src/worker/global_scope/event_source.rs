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
        policy_context,
        extra_headers,
        blocked,
        referrer_policy,
        partition,
    ) = {
        let state = state.borrow();
        (
            state.policy_context,
            state.extra_http_headers.clone(),
            state.blocked_url_patterns.clone(),
            state.referrer_policy.clone(),
            state.network_partition_key.clone(),
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
    let metadata = crate::service_worker_runtime::ServiceWorkerFetchRequestMetadata {
        cache: "no-store".to_owned(),
        mixed_content_policy: state.borrow().mixed_content_policy(),
        ..Default::default()
    };
    let environment = crate::network_host::BlobUrlFetchEnvironment::for_worker(scope);
    let blob = crate::network_host::CapturedBlobUrl::capture(&url)
        .map(|entry| entry.for_environment(environment.as_ref()));
    let error = check_worker_request_csp(
        scope, &state, &document_url, &url, kind.csp_resource_kind(), "EventSource",
        crate::content_security_policy::ContentSecurityPolicyDisposition::Enforce,
    ).or_else(|| moli_url_policy::route_fetch_url(&url).err().map(|error| error.to_string()))
    .or_else(|| should_request_be_blocked_due_to_bad_port(&url).then(|| "EventSource URL has a blocked port".to_owned()))
    .or_else(|| worker_url_blocked(&blocked, &url).then(|| BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned()));
    let mut request = Request::new_browser("GET", url, None, headers, timing.origin().clone())
        .with_initiator_url(&document_url)
        .with_mixed_content_policy(metadata.mixed_content_policy)
        .with_request_mode(RequestMode::Cors)
        .with_credentials_mode(credentials)
        .with_cache_mode(moli_fetch::RequestCacheMode::NoStore)
        .with_network_partition_key(partition)
        .with_browser_request_metadata(BrowserRequestMetadata::EventSource);
    if let Some(policy) = referrer_policy {
        request = request.with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata { referrer_policy: Some(policy), ..Default::default() });
    }
    super::fetch::start_native(scope, &state, super::fetch::NativeWorkerFetchRequest {
        request, blob_url_entry: blob, kind, font_face: None, timing,
        body_stream: None, body_stream_locked: false, signal: None,
        event_source: Some(source), admission_error: error,
    }, document_url, None, metadata, policy_context).is_some()
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
    tracing::debug!(%error, "Worker EventSource request failed");
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
        if let Some(runtime) = state.borrow().service_worker_runtime.clone()
            && let Some(handle) = pending.load.fetch_cancel_handle()
        {
            runtime.abort_controlled_fetch_with_handle_and_reason(&handle, None);
        }
        pending.load.cancel();

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
    let response_filter = completion
        .result
        .as_ref()
        .ok()
        .and_then(WorkerFetchResponse::response_filter);
    let skip_fetch_security_validation = completion
        .result
        .as_ref()
        .is_ok_and(WorkerFetchResponse::skips_fetch_security_validation);
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
                response_filter.as_ref(),
                skip_fetch_security_validation,
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
                    response_filter,
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
    if let Some(source) = source
        && event_source_active_request_id(scope, source) == Some(u64::from(finished.fetch_id))
        && event_source_ready_state(scope, source) != crate::network_host::EVENT_SOURCE_CLOSED
    {
        fail_event_source_connection(scope, source, EventSourceTerminalMode::Reconnect);
    }
}
