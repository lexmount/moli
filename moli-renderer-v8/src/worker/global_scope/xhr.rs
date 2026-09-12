use super::*;
use crate::network_host::{CapturedBlobUrl, blob_url_entry, local_url_response_with_blob_entry};
use crossbeam_channel::{after, bounded, never, select};
use moli_url::WebOrigin;
use moli_webapi_declare::WebApiObject;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(in crate::worker) struct PreparedWorkerXhrSendRequest {
    pub(in crate::worker) use_cors_preflight: bool,
    document_url: Url,
    resolved_url: Url,
    blob_url_entry: Option<CapturedBlobUrl>,
    method: String,
    request_headers: Vec<(String, String)>,
    send_body: Option<Vec<u8>>,
    credentials_mode: RequestCredentialsMode,
}

pub(in crate::worker) enum WorkerXhrSendPrepareError {
    Request(String),
}

pub(in crate::worker) const WORKER_XHR_TIMEOUT_ERROR_TEXT: &str = "XMLHttpRequest timeout";
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
        if let Some(record) = pending.network_record {
            let state = state.borrow();
            record_worker_subresource_failure_with_handle(
                &state,
                pending.network_request_handle,
                pending.document_url,
                record.url,
                record.method,
                record.request_headers,
                record.request_body,
                SubresourceResourceType::Xhr,
                WORKER_XHR_TIMEOUT_ERROR_TEXT.to_owned(),
            );
            let _ = state
                .parent_tx
                .send(WorkerToParentMessage::SubresourceContinue(
                    PendingSubresourceContinueEvent::Completed {
                        internal_id: record.internal_id,
                    },
                ));
        } else if pending.request_paused {
            let _ = state.borrow().parent_tx.send(
                WorkerToParentMessage::PendingSubresourceFetchCanceled {
                    fetch_id: scheduled_xhr_id,
                    error_text: WORKER_XHR_TIMEOUT_ERROR_TEXT.to_owned(),
                },
            );
        }
    }
    apply_xhr_timeout(scope, xhr);
    rv.set_undefined();
}

pub(crate) fn try_worker_xhr_send_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };

    let xhr = args.this();
    if !xhr_ensure_send_allowed(scope, xhr) {
        return true;
    }

    let async_request = xhr_state_bool_property(scope, xhr, XHR_ASYNC_SLOT).unwrap_or(true);

    let method =
        xhr_state_string_property(scope, xhr, XHR_METHOD_SLOT).unwrap_or_else(|| "GET".to_owned());
    let prepared_body = match prepare_xhr_send_body_from_args(scope, args, &method) {
        Ok(body) => body,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return true;
        }
    };

    cancel_worker_xhr_timeout(scope, xhr);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, true);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);

    let prepared = match prepare_worker_xhr_send_request(scope, &state, xhr, method, prepared_body)
    {
        Ok(prepared) => prepared,
        Err(WorkerXhrSendPrepareError::Request(message)) => {
            tracing::debug!("Worker XHR request preparation error: {message}");
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
    let fetch_subresource_interception_enabled =
        state.borrow().fetch_subresource_interception_enabled;
    let fetch_subresource_interception_resource_type =
        state.borrow().fetch_subresource_interception_resource_type;
    let mut prepared = prepared;
    prepared.request_headers =
        merge_worker_request_headers(&extra_http_headers, &prepared.request_headers);
    let url_policy = moli_url_policy::route_xml_http_request_url(&prepared.resolved_url);
    let request_url = prepared.resolved_url.to_string();

    if async_request {
        if !dispatch_xhr_loadstart(scope, xhr, prepared.send_body.as_deref()) {
            return true;
        }
        if xhr_state_bool_property(scope, xhr, XHR_ABORTED_SLOT).unwrap_or(false)
            || worker_xhr_open_generation_changed(scope, xhr, open_generation)
        {
            return true;
        }
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
    if let Some(violation) = csp_violation {
        dispatch_worker_content_security_policy_violation_event_for_state(
            scope, &state, &violation,
        );
        let message = worker_content_security_policy_error_message(&violation, "xhr");
        record_worker_subresource_failure(
            &state.borrow(),
            prepared.document_url,
            prepared.resolved_url,
            prepared.method,
            prepared.request_headers,
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            message,
        );
        apply_worker_xhr_request_failure(scope, xhr, async_request, &request_url);
        return true;
    }

    if let Err(error) = url_policy {
        record_worker_subresource_failure(
            &state.borrow(),
            prepared.document_url,
            prepared.resolved_url,
            prepared.method,
            prepared.request_headers,
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            error.to_string(),
        );
        apply_worker_xhr_request_failure(scope, xhr, async_request, &request_url);
        return true;
    }

    if should_request_be_blocked_due_to_bad_port(&prepared.resolved_url) {
        record_worker_subresource_failure(
            &state.borrow(),
            prepared.document_url,
            prepared.resolved_url.clone(),
            prepared.method,
            prepared.request_headers,
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            format!("xhr: blocked bad port for `{}`", prepared.resolved_url),
        );
        apply_worker_xhr_request_failure(scope, xhr, async_request, &request_url);
        return true;
    }

    if worker_url_blocked(&blocked_url_patterns, &prepared.resolved_url) {
        record_worker_subresource_failure(
            &state.borrow(),
            prepared.document_url,
            prepared.resolved_url,
            prepared.method,
            prepared.request_headers,
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
        );
        apply_worker_xhr_request_failure(scope, xhr, async_request, &request_url);
        return true;
    }

    if network_offline {
        record_worker_subresource_failure(
            &state.borrow(),
            prepared.document_url,
            prepared.resolved_url,
            prepared.method,
            prepared.request_headers,
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            "Network emulation offline".to_owned(),
        );
        apply_worker_xhr_request_failure(scope, xhr, async_request, &request_url);
        return true;
    }

    let local_response = local_url_response_with_blob_entry(
        &prepared.resolved_url,
        &prepared.method,
        prepared.blob_url_entry.as_ref(),
    );
    if !async_request && let Some(result) = local_response {
        match result {
            Ok(response) => apply_xhr_response(scope, xhr, response),
            Err(message) => {
                record_worker_subresource_failure(
                    &state.borrow(),
                    prepared.document_url,
                    prepared.resolved_url,
                    prepared.method,
                    prepared.request_headers,
                    request_body_text(&prepared.send_body),
                    SubresourceResourceType::Xhr,
                    message,
                );
                throw_synchronous_xhr_failure(scope, xhr, &request_url, "NetworkError");
            }
        }
        return true;
    }

    let loader = state.borrow().loader.clone();

    let cancel_handle = FetchCancelHandle::new();
    let intercept_request_stage = local_response.is_none()
        && fetch_subresource_interception_enabled
        && fetch_subresource_interception_resource_type.is_none_or(|expected| {
            expected.has_same_cdp_fetch_interception_type(SubresourceResourceType::Xhr)
        });
    if intercept_request_stage && !async_request {
        record_worker_subresource_failure(
            &state.borrow(),
            prepared.document_url,
            prepared.resolved_url,
            prepared.method,
            prepared.request_headers,
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            "Synchronous XMLHttpRequest interception is not supported".to_owned(),
        );
        apply_worker_xhr_request_failure(scope, xhr, async_request, &request_url);
        return true;
    }

    if !async_request {
        send_synchronous_worker_xhr(scope, &state, xhr, prepared, loader);
        return true;
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
        let request_body = request_body_text(&prepared.send_body);
        state.pending_xhrs.insert(
            xhr_id,
            PendingWorkerXhr {
                xhr: v8::Global::new(scope, xhr),
                document_url: prepared.document_url.clone(),
                credentials_mode: prepared.credentials_mode,
                use_cors_preflight: prepared.use_cors_preflight,
                load: load.clone(),
                request_paused: intercept_request_stage,
                request_url: prepared.resolved_url.clone(),
                request_method: prepared.method.clone(),
                request_headers: prepared.request_headers.clone(),
                request_body,
                network_request_handle: None,
                network_record: None,
                paused_response: None,
            },
        );
        xhr_id
    };
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, xhr_id as f64);
    schedule_worker_xhr_timeout(scope, &state, xhr, xhr_id);

    if let Some(result) = local_response {
        // Local responses and network errors still complete asynchronously, so
        // abort(), open() and timeout processing use the ordinary pending XHR.
        let _ = state
            .borrow()
            .xhr_completion_tx
            .send(WorkerXhrEvent::Completion(Box::new(WorkerXhrCompletion {
                xhr_id,
                network_request_headers: None,
                result: result.map(|response| WorkerXhrResponse::Materialized(Box::new(response))),
            })));
        return true;
    }

    if intercept_request_stage {
        let info = PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: None,
            document_url: prepared.document_url,
            url: prepared.resolved_url,
            websocket_socket_id: None,
            method: prepared.method,
            request_headers: prepared.request_headers,
            request_body: request_body_text(&prepared.send_body),
            request_body_bytes: prepared.send_body.clone(),
            resource_type: SubresourceResourceType::Xhr,
            request_cookie_report: None,
        };
        let _ = state
            .borrow()
            .parent_tx
            .send(WorkerToParentMessage::PendingSubresourceFetch(
                WorkerPendingSubresourceFetch {
                    fetch_id: xhr_id,
                    load: load.clone(),
                    credentials_mode: prepared.credentials_mode,
                    request_mode: RequestMode::Cors,
                    network_partition_key: state.borrow().network_partition_key.clone(),
                    info,
                },
            ));
        return true;
    }

    let completion_tx = state.borrow().xhr_completion_tx.clone();
    spawn_worker_xhr_network(
        load,
        completion_tx,
        xhr_id,
        cancel_handle,
        prepared.document_url,
        state.borrow().referrer_policy.clone(),
        state.borrow().network_partition_key.clone(),
        prepared.resolved_url,
        prepared.method,
        prepared.send_body,
        prepared.request_headers,
        prepared.credentials_mode,
        prepared.use_cors_preflight,
        None,
    );

    true
}

fn send_synchronous_worker_xhr(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedWorkerXhrSendRequest,
    loader: crate::network::context::WorkerResourceLoader,
) {
    let request_url_text = prepared.resolved_url.to_string();
    let request = match Request::new_bytes(
        &prepared.method,
        prepared.resolved_url.as_str(),
        prepared.send_body.clone(),
        prepared.request_headers.clone(),
    ) {
        Ok(request) => {
            let mut request = request
                .with_page_network_policy()
                .with_initiator_url(&prepared.document_url)
                .with_credentials_mode(prepared.credentials_mode)
                .with_network_partition_key(state.borrow().network_partition_key.clone())
                .with_browser_request_metadata(BrowserRequestMetadata::Xhr)
                .with_use_cors_preflight(prepared.use_cors_preflight);
            if let Some(referrer_policy) = state.borrow().referrer_policy.clone() {
                request =
                    request.with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
                        document_referrer_policy: Some(referrer_policy),
                        ..moli_fetch::ScriptFetchRequestMetadata::default()
                    });
            }
            request
        }
        Err(error) => {
            record_worker_subresource_failure(
                &state.borrow(),
                prepared.document_url,
                prepared.resolved_url,
                prepared.method,
                prepared.request_headers,
                request_body_text(&prepared.send_body),
                SubresourceResourceType::Xhr,
                error.to_string(),
            );
            throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
            return;
        }
    };
    let request_url = prepared.resolved_url.clone();
    let request_method = prepared.method.clone();
    let request_headers = prepared.request_headers.clone();
    let preflight_headers = prepared.request_headers.clone();
    let request_body = request_body_text(&prepared.send_body);
    let timeout_document_url = prepared.document_url.clone();
    let timeout_request_url = request_url.clone();
    let timeout_request_method = request_method.clone();
    let timeout_request_headers = request_headers.clone();
    let timeout_request_body = request_body.clone();

    let cancel_handle = FetchCancelHandle::new();
    let Some(load) = loader.register_load(
        ResourceLoadKind::Xhr,
        ResourceLoadDisposition::Ordinary,
        Some(cancel_handle.clone()),
    ) else {
        throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
        return;
    };
    let worker_cancel_handle = cancel_handle.clone();
    let (response_tx, response_rx) = bounded(1);
    let xhr_timeout = synchronous_worker_xhr_timeout(scope, xhr);
    let timeout_rx = xhr_timeout
        .as_ref()
        .map(|timeout| after(timeout.wait_delay))
        .unwrap_or_else(never);
    let spawn_result = thread::Builder::new()
        .name("lm-worker-sync-xhr-fetch".to_owned())
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("failed to build worker sync XHR fetch runtime: {error}"))
                .and_then(|runtime| {
                    runtime
                        .block_on(
                            fetch_browser_subresource_with_preflight_headers_and_network_metadata(
                                loader.request_client().clone(),
                                request,
                                Some(worker_cancel_handle),
                                preflight_headers,
                            ),
                        )
                        .map(moli_fetch::NetworkFetchResult::into_response)
                });
            let _ = response_tx.send(result);
        });

    let result = match spawn_result {
        Ok(_) => select! {
            recv(response_rx) -> result => result.unwrap_or_else(|_| {
                Err("worker sync XHR fetch thread dropped response channel".to_owned())
            }),
            recv(timeout_rx) -> _ => {
                let timeout = xhr_timeout.as_ref().expect("never channel should not fire without xhr timeout");
                load.cancel();
                record_worker_subresource_failure(
                    &state.borrow(),
                    timeout_document_url,
                    timeout_request_url,
                    timeout_request_method,
                    timeout_request_headers,
                    timeout_request_body,
                    SubresourceResourceType::Xhr,
                    format!(
                        "Synchronous XMLHttpRequest timed out after {} ms",
                        timeout.configured_timeout.as_millis()
                    ),
                );
                throw_synchronous_xhr_failure(
                    scope,
                    xhr,
                    &request_url_text,
                    "TimeoutError",
                );
                return;
            }
        },
        Err(error) => Err(format!(
            "failed to spawn worker sync XHR fetch thread: {error}"
        )),
    };

    match result {
        Ok(mut response) => {
            let response_head = response.head();
            let redirect_status = if response_head.redirect_chain.is_empty() {
                crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
            } else {
                crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
            };
            if redirect_status
                == crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
            {
                dispatch_worker_content_security_policy_report_only_violation_for_checked_url_with_redirect_status_for_state(
                    scope,
                    state,
                    &prepared.document_url,
                    &response_head.final_url,
                    &request_url,
                    crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
                    redirect_status,
                );
            }
            let csp_violation = {
                let state_ref = state.borrow();
                worker_content_security_policy_violation_for_checked_url_with_redirect_status(
                    &state_ref,
                    &prepared.document_url,
                    &response_head.final_url,
                    &request_url,
                    crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
                    if response_head.redirect_chain.is_empty() {
                        crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
                    } else {
                        crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
                    },
                )
            };
            if let Some(violation) = csp_violation {
                dispatch_worker_content_security_policy_violation_event_for_state(
                    scope, state, &violation,
                );
                let message = worker_content_security_policy_error_message(&violation, "xhr");
                record_worker_subresource_failure(
                    &state.borrow(),
                    prepared.document_url,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    SubresourceResourceType::Xhr,
                    message,
                );
                throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
                return;
            }
            let request_origin = cors_request_origin_after_redirects(
                &WebOrigin::from_url(&prepared.document_url),
                response_head
                    .redirect_chain
                    .iter()
                    .map(|redirect| (&redirect.from_url, &redirect.to_url)),
            );
            if let Err(message) = validate_cors_response_for_origin(
                &request_origin,
                &response_head.final_url,
                &response_head.headers,
                prepared.credentials_mode,
            ) {
                record_worker_subresource_failure(
                    &state.borrow(),
                    prepared.document_url,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    SubresourceResourceType::Xhr,
                    message,
                );
                throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
                return;
            }
            record_worker_subresource_success(
                &state.borrow(),
                prepared.document_url,
                request_url,
                request_method,
                request_headers,
                request_body,
                SubresourceResourceType::Xhr,
                response_head,
                SubresourceResponseBody::from_fetch_response(&response),
            );
            response.headers = filter_cors_exposed_response_headers_for_origin(
                &request_origin,
                &response.final_url,
                &response.headers,
                prepared.credentials_mode,
            );
            apply_xhr_response(scope, xhr, response);
        }
        Err(error) => {
            record_worker_subresource_failure(
                &state.borrow(),
                prepared.document_url,
                request_url,
                request_method,
                request_headers,
                request_body,
                SubresourceResourceType::Xhr,
                error.to_string(),
            );
            throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
        }
    }
}

fn apply_worker_xhr_request_failure(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    async_request: bool,
    request_url: &str,
) {
    if async_request {
        apply_xhr_failure(scope, xhr);
    } else {
        throw_synchronous_xhr_failure(scope, xhr, request_url, "NetworkError");
    }
}

struct SynchronousWorkerXhrTimeout {
    wait_delay: Duration,
    configured_timeout: Duration,
}

fn synchronous_worker_xhr_timeout(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<SynchronousWorkerXhrTimeout> {
    Some(SynchronousWorkerXhrTimeout {
        wait_delay: Duration::from_millis(worker_xhr_timeout_remaining_delay_ms(scope, xhr)?),
        configured_timeout: Duration::from_millis(worker_xhr_configured_timeout_ms(scope, xhr)?),
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
    args: &v8::FunctionCallbackArguments<'_>,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };

    let xhr = args.this();
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
        if let Some(record) = pending.network_record {
            let state = state.borrow();
            record_worker_subresource_failure_with_handle(
                &state,
                pending.network_request_handle,
                pending.document_url,
                record.url,
                record.method,
                record.request_headers,
                record.request_body,
                SubresourceResourceType::Xhr,
                ABORTED_ERROR_TEXT.to_owned(),
            );
            let _ = state
                .parent_tx
                .send(WorkerToParentMessage::SubresourceContinue(
                    PendingSubresourceContinueEvent::Completed {
                        internal_id: record.internal_id,
                    },
                ));
        } else if pending.request_paused {
            let _ = state.borrow().parent_tx.send(
                WorkerToParentMessage::PendingSubresourceFetchCanceled {
                    fetch_id: internal_id,
                    error_text: ABORTED_ERROR_TEXT.to_owned(),
                },
            );
        }
    }

    crate::network_host::finish_xhr_abort(scope, xhr);
    true
}

pub(in crate::worker) fn record_worker_xhr_failure(
    state: &WorkerGlobalState,
    pending: &PendingWorkerXhr,
    message: String,
) {
    let network_error_text = if is_cors_policy_failure_message(&message) {
        FAILED_ERROR_TEXT.to_owned()
    } else {
        message
    };
    if let Some(record) = pending.network_record.as_ref() {
        record_worker_subresource_failure_with_handle(
            state,
            pending.network_request_handle,
            pending.document_url.clone(),
            record.url.clone(),
            record.method.clone(),
            record.request_headers.clone(),
            record.request_body.clone(),
            SubresourceResourceType::Xhr,
            network_error_text,
        );
        let _ = state
            .parent_tx
            .send(WorkerToParentMessage::SubresourceContinue(
                PendingSubresourceContinueEvent::Completed {
                    internal_id: record.internal_id,
                },
            ));
        return;
    }

    record_worker_subresource_failure_with_handle(
        state,
        pending.network_request_handle,
        pending.document_url.clone(),
        pending.request_url.clone(),
        pending.request_method.clone(),
        pending.request_headers.clone(),
        pending.request_body.clone(),
        SubresourceResourceType::Xhr,
        network_error_text,
    );
}

pub(in crate::worker) fn drain_worker_xhr_event(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    event: WorkerXhrEvent,
) {
    match event {
        WorkerXhrEvent::Upload { xhr_id, event } => {
            let xhr = {
                let state = state.borrow();
                let Some(pending) = state.pending_xhrs.get(&xhr_id) else {
                    return;
                };
                v8::Local::new(scope, &pending.xhr)
            };
            if !apply_xhr_upload_event(scope, xhr, u64::from(xhr_id), event)
                && let Some(pending) = state.borrow_mut().pending_xhrs.remove(&xhr_id)
            {
                pending.load.cancel();
            }
        }
        WorkerXhrEvent::Completion(completion) => {
            drain_worker_xhr_completion(scope, state, *completion)
        }
    }
}

pub(in crate::worker) fn drain_worker_xhr_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerXhrCompletion,
) {
    let parent_tx = state.borrow().parent_tx.clone();
    if let Some(network_request_headers) = completion.network_request_headers.as_ref()
        && let Some(record) = state
            .borrow_mut()
            .pending_xhrs
            .get_mut(&completion.xhr_id)
            .and_then(|pending| pending.network_record.as_mut())
    {
        record
            .initial_network_request_headers
            .get_or_insert_with(|| network_request_headers.clone());
    }
    if let Ok(response) = &completion.result {
        let response_head = response.head();
        let redirect_status = if response_head.redirect_chain.is_empty() {
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
        } else {
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
        };
        if redirect_status
            == crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
        {
            let (document_url, request_url) = {
                let state_ref = state.borrow();
                let Some(pending) = state_ref.pending_xhrs.get(&completion.xhr_id) else {
                    return;
                };
                (pending.document_url.clone(), pending.request_url.clone())
            };
            dispatch_worker_content_security_policy_report_only_violation_for_checked_url_with_redirect_status_for_state(
                scope,
                state,
                &document_url,
                &response_head.final_url,
                &request_url,
                crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
                redirect_status,
            );
        }
        let csp_failure = {
            let state_ref = state.borrow();
            let Some(pending) = state_ref.pending_xhrs.get(&completion.xhr_id) else {
                return;
            };
            worker_content_security_policy_violation_for_checked_url_with_redirect_status(
                &state_ref,
                &pending.document_url,
                &response_head.final_url,
                &pending.request_url,
                crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerConnect,
                if response_head.redirect_chain.is_empty() {
                    crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
                } else {
                    crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
                },
            )
        };
        if let Some(violation) = csp_failure {
            dispatch_worker_content_security_policy_violation_event_for_state(
                scope, state, &violation,
            );
            let message = worker_content_security_policy_error_message(&violation, "xhr");
            let Some(pending) = state.borrow_mut().pending_xhrs.remove(&completion.xhr_id) else {
                return;
            };
            pending.load.finish();
            let xhr = v8::Local::new(scope, &pending.xhr);
            cancel_worker_xhr_timeout(scope, xhr);
            record_worker_xhr_failure(&state.borrow(), &pending, message);
            apply_xhr_failure(scope, xhr);
            return;
        }
        let auth_required = {
            let mut state_ref = state.borrow_mut();
            let Some(pending) = state_ref.pending_xhrs.get_mut(&completion.xhr_id) else {
                return;
            };
            let response_head = response.head();
            pending.network_record.clone().and_then(|record| {
                if record.handle_auth_requests
                    && matches!(response_head.status, 401 | 407)
                    && let Some(challenge) =
                        extract_subresource_auth_challenge(&response_head.headers)
                {
                    let response_body = response.subresource_response_body();
                    pending.paused_response = Some(PausedWorkerSubresourceResponse {
                        head: response_head.clone(),
                        body: response_body.clone(),
                    });
                    Some(PendingSubresourceAuthInfo {
                        internal_id: record.internal_id,
                        url: record.url.clone(),
                        method: record.method.clone(),
                        request_headers: record.request_headers.clone(),
                        request_body: record.request_body.clone(),
                        resource_type: SubresourceResourceType::Xhr,
                        request_cookie_report: response_head.request_cookie_report.clone(),
                        network_request_headers: record.initial_network_request_headers.clone(),
                        challenge,
                        intercept_response: record.intercept_response,
                        response_final_url: response_head.final_url.clone(),
                        response_status: response_head.status,
                        response_headers: response_head.headers.clone(),
                        response_body,
                        response_from_cache: response_head.from_cache,
                    })
                } else {
                    None
                }
            })
        };
        if let Some(info) = auth_required {
            let _ = parent_tx.send(WorkerToParentMessage::SubresourceContinue(
                PendingSubresourceContinueEvent::AuthRequired(info),
            ));
            return;
        }
        let response_paused = {
            let mut state = state.borrow_mut();
            let Some(pending) = state.pending_xhrs.get_mut(&completion.xhr_id) else {
                return;
            };
            let response_head = response.head();
            match pending.network_record.as_ref() {
                Some(record) if record.intercept_response => {
                    let response_body = response.subresource_response_body();
                    let info = PendingSubresourceResponseInfo {
                        internal_id: record.internal_id,
                        url: record.url.clone(),
                        final_url: response_head.final_url.clone(),
                        method: record.method.clone(),
                        request_headers: record.request_headers.clone(),
                        request_body: record.request_body.clone(),
                        resource_type: SubresourceResourceType::Xhr,
                        request_cookie_report: response_head.request_cookie_report.clone(),
                        network_request_headers: record.initial_network_request_headers.clone(),
                        response_status: response_head.status,
                        response_headers: response_head.headers.clone(),
                        response_body: response_body.clone(),
                        from_cache: response_head.from_cache,
                    };
                    pending.paused_response = Some(PausedWorkerSubresourceResponse {
                        head: response_head,
                        body: response_body,
                    });
                    Some(info)
                }
                _ => None,
            }
        };
        if let Some(info) = response_paused {
            let _ = parent_tx.send(WorkerToParentMessage::SubresourceContinue(
                PendingSubresourceContinueEvent::ResponsePaused(info),
            ));
            return;
        }
    }

    let Some(pending) = state.borrow_mut().pending_xhrs.remove(&completion.xhr_id) else {
        return;
    };
    pending.load.finish();
    let xhr = v8::Local::new(scope, &pending.xhr);
    cancel_worker_xhr_timeout(scope, xhr);
    match completion.result {
        Ok(response) => {
            let response_head = response.head();
            let request_origin = cors_request_origin_after_redirects(
                &WebOrigin::from_url(&pending.document_url),
                response_head
                    .redirect_chain
                    .iter()
                    .map(|redirect| (&redirect.from_url, &redirect.to_url)),
            );
            match validate_cors_response_for_origin(
                &request_origin,
                &response_head.final_url,
                &response_head.headers,
                pending.credentials_mode,
            ) {
                Ok(()) => {
                    let response_body = pending
                        .network_record
                        .as_ref()
                        .map(|_| response.subresource_response_body());
                    match response.into_body_source() {
                        Ok((mut response_head, body)) => {
                            if let Some(record) = pending.network_record.as_ref()
                                && let Some(response_body) = response_body
                            {
                                record_worker_subresource_success_with_handle(
                                    &state.borrow(),
                                    pending.network_request_handle,
                                    pending.document_url.clone(),
                                    record.url.clone(),
                                    record.method.clone(),
                                    record.request_headers.clone(),
                                    record.request_body.clone(),
                                    SubresourceResourceType::Xhr,
                                    record.initial_network_request_headers.clone(),
                                    response_head.clone(),
                                    response_body,
                                );
                                let _ = parent_tx.send(WorkerToParentMessage::SubresourceContinue(
                                    PendingSubresourceContinueEvent::Completed {
                                        internal_id: record.internal_id,
                                    },
                                ));
                            }
                            response_head.headers = filter_cors_exposed_response_headers_for_origin(
                                &request_origin,
                                &response_head.final_url,
                                &response_head.headers,
                                pending.credentials_mode,
                            );
                            apply_xhr_response_body_source(scope, xhr, response_head, body);
                        }
                        Err(message) => {
                            record_worker_xhr_failure(&state.borrow(), &pending, message);
                            apply_xhr_failure(scope, xhr);
                        }
                    }
                }
                Err(message) => {
                    record_worker_xhr_failure(&state.borrow(), &pending, message);
                    apply_xhr_failure(scope, xhr);
                }
            }
        }
        Err(message) => {
            record_worker_xhr_failure(&state.borrow(), &pending, message);
            apply_xhr_failure(scope, xhr);
        }
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

    let document_url = state.borrow().current_script_url.clone().ok_or_else(|| {
        WorkerXhrSendPrepareError::Request(
            "worker xhr: worker script url is unavailable".to_owned(),
        )
    })?;
    let resolved_url = resolve_context_url(&document_url, &url_str, None)
        .map_err(WorkerXhrSendPrepareError::Request)?;
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
        request_headers,
        send_body: prepared_body.body,
        credentials_mode,
    })
}

// ─── postMessage ────────────────────────────────────────────────────────────
