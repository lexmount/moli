use super::*;
use crate::network_host::ResolveContextUrlError;
use crate::network_host::controlled_resource::{ControlledXhrContext, start_buffered_xhr};
use crate::network_host::{CapturedBlobUrl, blob_url_entry, local_url_response_with_blob_entry};
use crossbeam_channel::{after, never, select};
use moli_webapi_declare::WebApiObject;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(in crate::worker) struct PreparedWorkerXhrSendRequest {
    pub(in crate::worker) use_cors_preflight: bool,
    document_url: Url,
    resolved_url: Url,
    blob_url_entry: Option<CapturedBlobUrl>,
    method: String,
    request_headers: moli_fetch::RequestHeaders,
    send_body: Option<bytes::Bytes>,
    credentials_mode: RequestCredentialsMode,
}

#[derive(Debug)]
pub(in crate::worker) enum WorkerXhrSendPrepareError {
    ScriptUrlUnavailable,
    Url(ResolveContextUrlError),
}

impl std::fmt::Display for WorkerXhrSendPrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScriptUrlUnavailable => {
                f.write_str("worker xhr: worker script url is unavailable")
            }
            Self::Url(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for WorkerXhrSendPrepareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Url(error) => Some(error),
            Self::ScriptUrlUnavailable => None,
        }
    }
}

pub(in crate::worker) const WORKER_XHR_TIMEOUT_DATA_XHR: &str = "xhr";
pub(in crate::worker) const WORKER_XHR_TIMEOUT_DATA_XHR_ID: &str = "xhrId";

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct WorkerXhrTimeoutDataDeclaration<'scope> {
    xhr: v8::Local<'scope, v8::Object>,
    xhr_id: f64,
}

pub(in crate::worker) fn schedule_worker_xhr_timeout<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    xhr: v8::Local<'s, v8::Object>,
    xhr_id: u32,
) {
    let Some(delay_ms) = worker_xhr_timeout_remaining_delay_ms(scope, xhr) else {
        return;
    };
    if xhr_id == 0 {
        return;
    }

    cancel_worker_xhr_timeout(scope, xhr);

    let timer_id = {
        let mut state = state.borrow_mut();
        state.next_timer_id += 1;
        state.next_timer_id
    };
    let data = WorkerXhrTimeoutDataDeclaration {
        xhr,
        xhr_id: xhr_id as f64,
    }
    .bind(scope)
    .expect("worker XHR timeout data declaration should bind");
    let callback = v8::FunctionTemplate::builder(worker_xhr_timeout_callback)
        .data(data.into())
        .build(scope)
        .get_function(scope);
    let Some(callback) = callback else {
        return;
    };
    let timer = TimerInfo {
        id: timer_id,
        callback: super::super::timer_callback::WorkerTimerCallback::browser_function(
            scope, callback,
        ),
        delay_ms,
        is_interval: false,
        extra_args: Vec::new(),
    };
    if let Some(timers) = worker_isolate_timer_queues(scope) {
        timers.push_pending(timer);
        set_xhr_state_number(scope, xhr, XHR_TIMEOUT_TIMER_SLOT, timer_id as f64);
    }
}

pub(in crate::worker) fn cancel_worker_xhr_timeout(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    let timer_id = xhr_state_number_property(scope, xhr, XHR_TIMEOUT_TIMER_SLOT)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value as u32)
        .unwrap_or(0);
    if timer_id == 0 {
        return;
    }
    set_xhr_state_number(scope, xhr, XHR_TIMEOUT_TIMER_SLOT, 0.0);
    if let Some(timers) = worker_isolate_timer_queues(scope) {
        timers.cancel_active(timer_id);
    }
}

pub(crate) fn try_worker_xhr_reschedule_timeout_after_timeout_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let active_xhr_id =
        xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT).unwrap_or(0.0) as u32;
    let send_flag = xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false);
    cancel_worker_xhr_timeout(scope, xhr);
    if send_flag && active_xhr_id != 0 {
        if worker_xhr_timeout_start_ms(scope, xhr).is_none() {
            mark_worker_xhr_timeout_start(scope, xhr);
        }
        schedule_worker_xhr_timeout(scope, &state, xhr, active_xhr_id);
    }
    true
}

fn mark_worker_xhr_timeout_start(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    set_xhr_state_number(
        scope,
        xhr,
        XHR_TIMEOUT_START_MS_SLOT,
        worker_current_time_ms(),
    );
}

fn clear_worker_xhr_timeout_start(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    set_xhr_state_number(scope, xhr, XHR_TIMEOUT_START_MS_SLOT, 0.0);
}

fn worker_xhr_timeout_remaining_delay_ms(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<u64> {
    let timeout_ms = worker_xhr_configured_timeout_ms(scope, xhr)? as f64;
    let elapsed_ms = worker_xhr_timeout_start_ms(scope, xhr)
        .map(|started_at_ms| (worker_current_time_ms() - started_at_ms).max(0.0))
        .unwrap_or(0.0);
    let remaining_ms = (timeout_ms - elapsed_ms).max(0.0).ceil();
    Some(remaining_ms.min(u64::MAX as f64) as u64)
}

fn worker_xhr_configured_timeout_ms(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<u64> {
    let timeout_ms = xhr_state_number_property(scope, xhr, XHR_TIMEOUT_SLOT)?;
    if !timeout_ms.is_finite() || timeout_ms <= 0.0 {
        return None;
    }
    Some(timeout_ms.min(u64::MAX as f64) as u64)
}

fn worker_xhr_timeout_start_ms(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<f64> {
    xhr_state_number_property(scope, xhr, XHR_TIMEOUT_START_MS_SLOT)
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn worker_current_time_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

pub(in crate::worker) fn worker_xhr_timeout_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(data) = args.data().to_object(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(xhr) = data
        .get(scope, v8str(scope, WORKER_XHR_TIMEOUT_DATA_XHR).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let scheduled_xhr_id = data
        .get(scope, v8str(scope, WORKER_XHR_TIMEOUT_DATA_XHR_ID).into())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value as u32)
        .unwrap_or(0);
    let active_xhr_id =
        xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT).unwrap_or(0.0) as u32;
    if scheduled_xhr_id == 0 || scheduled_xhr_id != active_xhr_id {
        rv.set_undefined();
        return;
    }

    let pending = state.borrow_mut().pending_xhrs.remove(&scheduled_xhr_id);
    if let Some(pending) = pending {
        pending.load.cancel();
    }
    apply_xhr_timeout(scope, xhr);
    rv.set_undefined();
}

pub(crate) fn try_worker_xhr_send_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    xhr: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };

    let body = match convert_xhr_send_body_from_args(scope, args) {
        Ok(body) => body,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return true;
        }
    };

    if !xhr_ensure_send_allowed(scope, xhr) {
        return true;
    }

    let async_request = xhr_state_bool_property(scope, xhr, XHR_ASYNC_SLOT).unwrap_or(true);
    let method =
        xhr_state_string_property(scope, xhr, XHR_METHOD_SLOT).unwrap_or_else(|| "GET".to_owned());
    let prepared_body = match body.prepare(scope, &method) {
        Ok(body) => body,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return true;
        }
    };

    cancel_worker_xhr_timeout(scope, xhr);
    begin_xhr_send(scope, xhr);

    let prepared = match prepare_worker_xhr_send_request(scope, &state, xhr, method, prepared_body)
    {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::debug!("Worker XHR request preparation error: {error}");
            if async_request {
                apply_xhr_failure(scope, xhr);
            } else {
                let request_url =
                    xhr_state_string_property(scope, xhr, XHR_URL_SLOT).unwrap_or_default();
                throw_synchronous_xhr_failure(scope, xhr, &request_url, "NetworkError");
            }
            return true;
        }
    };

    mark_worker_xhr_timeout_start(scope, xhr);
    let open_generation =
        xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT).unwrap_or(0.0);
    let extra_http_headers = state.borrow().extra_http_headers.clone();
    let blocked_url_patterns = state.borrow().blocked_url_patterns.clone();
    let network_offline = state.borrow().network_offline;
    let mut prepared = prepared;
    prepared.request_headers =
        merge_worker_request_headers(&extra_http_headers, &prepared.request_headers);
    let url_policy = moli_url_policy::route_xml_http_request_url(&prepared.resolved_url);
    let request_url = prepared.resolved_url.to_string();

    if async_request && !dispatch_xhr_loadstart(scope, xhr, prepared.send_body.as_deref()) {
        return true;
    }

    dispatch_worker_content_security_policy_report_only_violation_for_state(
        scope,
        &state,
        &prepared.document_url,
        &prepared.resolved_url,
        crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
    );
    let csp_violation = {
        let state_ref = state.borrow();
        worker_content_security_policy_violation(
            &state_ref,
            &prepared.document_url,
            &prepared.resolved_url,
            crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
        )
    };
    let request_error = if let Some(violation) = csp_violation {
        dispatch_worker_content_security_policy_violation_event_for_state(
            scope, &state, &violation,
        );
        Some(worker_content_security_policy_error_message(
            &violation, "xhr",
        ))
    } else if let Err(error) = url_policy {
        Some(error.to_string())
    } else if should_request_be_blocked_due_to_bad_port(&prepared.resolved_url) {
        Some(format!(
            "xhr: blocked bad port for `{}`",
            prepared.resolved_url
        ))
    } else if worker_url_blocked(&blocked_url_patterns, &prepared.resolved_url) {
        Some(BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned())
    } else if network_offline {
        Some("Network emulation offline".to_owned())
    } else {
        None
    };
    if xhr_state_bool_property(scope, xhr, XHR_ABORTED_SLOT).unwrap_or(false)
        || worker_xhr_open_generation_changed(scope, xhr, open_generation)
    {
        return true;
    }

    // A rejected fetch still completes in a networking task. Keep the pending
    // XHR and its load lease until delivery so abort/open can cancel that task.
    let local_response = request_error.map(Err).or_else(|| {
        local_url_response_with_blob_entry(
            &prepared.resolved_url,
            &prepared.method,
            prepared.blob_url_entry.as_ref(),
        )
        .map(|response| response.map_err(|error| error.to_string()))
    });
    if !async_request && let Some(result) = local_response {
        match result {
            Ok(response) => apply_xhr_response(scope, xhr, response),
            Err(error) => {
                record_worker_subresource_failure(
                    &state.borrow(),
                    prepared.document_url,
                    prepared.resolved_url,
                    prepared.method,
                    prepared.request_headers,
                    request_body_text(&prepared.send_body),
                    SubresourceResourceType::Xhr,
                    error,
                );
                throw_synchronous_xhr_failure(scope, xhr, &request_url, "NetworkError");
            }
        }
        return true;
    }

    let loader = state.borrow().loader.clone();

    let cancel_handle = state.borrow().network_cancel.child_for_subrequest();
    let control = state.borrow().resource_load_control();

    if !async_request {
        send_synchronous_worker_xhr(scope, &state, xhr, prepared, loader, control);
        return true;
    }

    // A locally rejected fetch has not sent its upload body.
    if local_response.as_ref().is_none_or(Result::is_ok) {
        dispatch_xhr_upload_complete(scope, xhr, prepared.send_body.as_deref());
        if xhr_state_bool_property(scope, xhr, XHR_ABORTED_SLOT).unwrap_or(false)
            || worker_xhr_open_generation_changed(scope, xhr, open_generation)
        {
            return true;
        }
    }

    let Some(load) = loader.register_load(
        ResourceLoadKind::Xhr,
        ResourceLoadDisposition::Ordinary,
        Some(cancel_handle.clone()),
    ) else {
        apply_xhr_failure(scope, xhr);
        return true;
    };

    let xhr_id = {
        let mut state = state.borrow_mut();
        let xhr_id = next_xhr_id(&mut state);
        state.pending_xhrs.insert(
            xhr_id,
            PendingWorkerXhr {
                xhr: v8::Global::new(scope, xhr),
                credentials_mode: prepared.credentials_mode,

                load: load.clone(),
            },
        );
        xhr_id
    };
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, xhr_id as f64);
    schedule_worker_xhr_timeout(scope, &state, xhr, xhr_id);

    if let Some(result) = local_response {
        // Local responses and network errors still complete asynchronously, so
        // abort(), open() and timeout processing use the ordinary pending XHR.
        let _ = state.borrow().xhr_completion_tx.send(WorkerXhrCompletion {
            xhr_id,
            result,
            local_request: Some(Box::new(worker_xhr_info(&prepared))),
            csp_violations: Vec::new(),
        });
        return true;
    }

    let completion_tx = state.borrow().xhr_completion_tx.clone();
    let info = worker_xhr_info(&prepared);
    let request = worker_xhr_request(&state.borrow(), &prepared);
    load.task_runner().spawn(async move {
        let completed = control
            .load(
                request,
                info,
                load,
                cancel_handle,
                prepared.request_headers.to_byte_strings(),
            )
            .run()
            .await;
        let _ = completion_tx.send(WorkerXhrCompletion {
            xhr_id,
            local_request: None,
            csp_violations: completed.csp_violations,
            result: completed.result,
        });
    });
    true
}

fn worker_xhr_request(
    state: &WorkerGlobalState,
    prepared: &PreparedWorkerXhrSendRequest,
) -> Request {
    let mut request = Request::new_bytes(
        &prepared.method,
        prepared.resolved_url.as_str(),
        prepared.send_body.clone(),
        prepared.request_headers.clone(),
    )
    .expect("worker XHR URL was already resolved")
    .with_page_network_policy()
    .with_initiator_url(&prepared.document_url)
    .with_request_origin(state.request_origin())
    .with_credentials_mode(prepared.credentials_mode)
    .with_network_partition_key(state.network_partition_key.clone())
    .with_request_mode(RequestMode::Cors)
    .with_browser_request_metadata(BrowserRequestMetadata::Xhr)
    .with_use_cors_preflight(prepared.use_cors_preflight);
    if let Some(referrer_policy) = state.referrer_policy.clone() {
        request = request.with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
            document_referrer_policy: Some(referrer_policy),
            ..Default::default()
        });
    }
    request
}

fn worker_xhr_info(prepared: &PreparedWorkerXhrSendRequest) -> PendingSubresourceFetchInfo {
    PendingSubresourceFetchInfo {
        internal_id: crate::network::interception_control::next_subresource_request_id(),
        network_request_handle: Some(
            crate::network::interception_control::next_subresource_network_handle(),
        ),
        frame_id: None,
        document_url: prepared.document_url.clone(),
        url: prepared.resolved_url.clone(),
        websocket_socket_id: None,
        method: prepared.method.clone(),
        request_headers: prepared.request_headers.clone(),
        request_body: request_body_text(&prepared.send_body),
        request_body_bytes: prepared.send_body.clone(),
        resource_type: SubresourceResourceType::Xhr,
        request_cookie_report: None,
    }
}

fn send_synchronous_worker_xhr(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedWorkerXhrSendRequest,
    loader: crate::network::context::WorkerResourceLoader,
    control: ControlledXhrContext,
) {
    let request_url_text = prepared.resolved_url.to_string();
    let request = worker_xhr_request(&state.borrow(), &prepared);
    let cancel = state.borrow().network_cancel.child_for_subrequest();
    let Some(load) = loader.register_load(
        ResourceLoadKind::Xhr,
        ResourceLoadDisposition::Ordinary,
        Some(cancel.clone()),
    ) else {
        throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
        return;
    };
    let xhr_timeout = synchronous_worker_xhr_timeout(scope, xhr);
    let timeout_rx = xhr_timeout
        .as_ref()
        .map(|timeout| after(timeout.wait_delay))
        .unwrap_or_else(never);
    let response_rx = start_buffered_xhr(
        request,
        worker_xhr_info(&prepared),
        load.clone(),
        cancel,
        prepared.request_headers.to_byte_strings(),
        Some(control),
    );
    let result = select! {
        recv(response_rx) -> result => result.unwrap_or_else(|_| Err("worker sync XHR resource task dropped response channel".into())),
        recv(timeout_rx) -> _ => {
            load.cancel();
            throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "TimeoutError");
            return;
        }
    };
    if state.borrow().network_cancel.is_cancelled() {
        return;
    }
    // The loader has already published the network result independently of JS.
    if let Ok(completed) = &result {
        for violation in &completed.csp_violations {
            dispatch_worker_content_security_policy_violation_event_for_state(
                scope, state, violation,
            );
        }
    }
    match result.map(|completion| completion.result) {
        Ok(Ok(mut response)) => {
            response.headers = filter_cors_exposed_response_headers(
                &worker_global_origin(scope).unwrap_or(moli_url::WebOrigin::Opaque),
                &response.head(),
                prepared.credentials_mode,
            );
            apply_xhr_response(scope, xhr, response);
        }
        Ok(Err(_)) | Err(_) => {
            throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError")
        }
    }
}

struct SynchronousWorkerXhrTimeout {
    wait_delay: Duration,
}

fn synchronous_worker_xhr_timeout(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<SynchronousWorkerXhrTimeout> {
    Some(SynchronousWorkerXhrTimeout {
        wait_delay: Duration::from_millis(worker_xhr_timeout_remaining_delay_ms(scope, xhr)?),
    })
}

fn worker_xhr_open_generation_changed(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    expected: f64,
) -> bool {
    xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT)
        .is_some_and(|current| current != expected)
}

pub(crate) fn try_worker_xhr_abort_callback(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };

    cancel_worker_xhr_timeout(scope, xhr);
    clear_worker_xhr_timeout_start(scope, xhr);
    let internal_id =
        xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT).unwrap_or(0.0) as u32;
    let pending = if internal_id != 0 {
        state.borrow_mut().pending_xhrs.remove(&internal_id)
    } else {
        None
    };
    if let Some(pending) = pending {
        pending.load.cancel();
    }

    crate::network_host::finish_xhr_abort(scope, xhr);
    true
}

pub(in crate::worker) fn drain_worker_xhr_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerXhrCompletion,
) {
    let xhr = {
        let state = state.borrow();
        let Some(pending) = state.pending_xhrs.get(&completion.xhr_id) else {
            return;
        };
        v8::Local::new(scope, &pending.xhr)
    };
    if xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT)
        != Some(f64::from(completion.xhr_id))
        || !xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false)
        || xhr_state_bool_property(scope, xhr, XHR_ABORTED_SLOT).unwrap_or(false)
    {
        if let Some(pending) = state.borrow_mut().pending_xhrs.remove(&completion.xhr_id) {
            pending.load.cancel();
        }
        return;
    }
    let Some(pending) = state.borrow_mut().pending_xhrs.remove(&completion.xhr_id) else {
        return;
    };
    pending.load.finish();
    cancel_worker_xhr_timeout(scope, xhr);
    if let (Some(info), Err(error)) = (&completion.local_request, &completion.result) {
        record_worker_subresource_failure_with_handle(
            &state.borrow(),
            info.network_request_handle,
            info.document_url.clone(),
            info.url.clone(),
            info.method.clone(),
            info.request_headers.clone(),
            info.request_body_bytes.clone(),
            info.resource_type,
            error.clone(),
        );
    }
    for violation in completion.csp_violations {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
    match completion.result {
        Ok(response) => {
            let (mut head, body) = response.into_body();
            head.headers = filter_cors_exposed_response_headers(
                state.borrow().request_origin(),
                &head,
                pending.credentials_mode,
                pending.load.request_client().web_security_disabled(),
            );
            apply_xhr_response_body_source(scope, xhr, head, body);
        }
        Err(_) => apply_xhr_failure(scope, xhr),
    }
}

pub(in crate::worker) fn prepare_worker_xhr_send_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    xhr: v8::Local<'s, v8::Object>,
    method: String,
    prepared_body: PreparedXhrSendBody,
) -> Result<PreparedWorkerXhrSendRequest, WorkerXhrSendPrepareError> {
    let url_str = xhr_state_string_property(scope, xhr, XHR_URL_SLOT).unwrap_or_default();

    let document_url = state
        .borrow()
        .current_script_url
        .clone()
        .ok_or(WorkerXhrSendPrepareError::ScriptUrlUnavailable)?;
    let resolved_url = resolve_context_url(&document_url, &url_str, None)
        .map_err(WorkerXhrSendPrepareError::Url)?;
    let request_headers = xhr_author_request_headers(scope, xhr, &prepared_body);
    let credentials_mode =
        if xhr_state_bool_property(scope, xhr, XHR_WITH_CREDENTIALS_SLOT).unwrap_or(false) {
            RequestCredentialsMode::Include
        } else {
            RequestCredentialsMode::SameOrigin
        };

    Ok(PreparedWorkerXhrSendRequest {
        use_cors_preflight: capture_xhr_upload_listener_flag(scope, xhr),
        document_url,
        resolved_url,
        blob_url_entry: blob_url_entry(scope, xhr),
        method,
        request_headers: moli_fetch::RequestHeaders::from_byte_strings(&request_headers)
            .expect("validated worker XHR headers are ByteStrings"),
        send_body: prepared_body.body,
        credentials_mode,
    })
}

// ─── postMessage ────────────────────────────────────────────────────────────
