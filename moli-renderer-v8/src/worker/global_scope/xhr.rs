use super::fetch::{
    publish_worker_network_item, publish_worker_request_failure, record_worker_fetch_response,
    worker_request_started,
};
use super::*;
use crate::network_host::ResolveContextUrlError;
use crossbeam_channel::{after, bounded, never, select};
use moli_page_types::{ScriptNetworkOutputItem, SubresourceBodyFinished};
use moli_webapi_declare::WebApiObject;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) struct PreparedWorkerXhrSendRequest {
    pub(super) document_url: Url,
    pub(super) resolved_url: Url,
    pub(super) method: String,
    pub(super) request_headers: moli_fetch::RequestHeaders,
    pub(super) send_body: Option<Vec<u8>>,
    pub(super) credentials_mode: RequestCredentialsMode,
}

impl PreparedWorkerXhrSendRequest {
    pub(super) fn network_request(&self, state: &WorkerGlobalState) -> Result<Request, String> {
        let mut request = Request::new_bytes(
            &self.method,
            self.resolved_url.as_str(),
            self.send_body.clone(),
            self.request_headers.clone(),
        )
        .map_err(|error| format!("xhr: failed to build request: {error}"))?
        .with_initiator_url(&self.document_url)
        .with_request_origin(moli_url::WebOrigin::from_url(&self.document_url))
        .with_credentials_mode(self.credentials_mode)
        .with_network_partition_key(state.network_partition_key.clone())
        .with_browser_request_metadata(BrowserRequestMetadata::Xhr);
        if let Some(referrer_policy) = state.referrer_policy.clone() {
            request = request.with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
                document_referrer_policy: Some(referrer_policy),
                ..Default::default()
            });
        }
        Ok(request)
    }
}

fn record_worker_xhr_started(
    state: &WorkerGlobalState,
    network: &crate::runtime::RendererWorkerNetworkRequest,
    prepared: &PreparedWorkerXhrSendRequest,
) {
    publish_worker_network_item(
        &state.parent_tx.network_observer(),
        network,
        ScriptNetworkOutputItem::SubresourceRequestStarted(Arc::new(worker_request_started(
            network,
            &prepared.document_url,
            &prepared.resolved_url,
            &prepared.method,
            &prepared.request_headers,
            &prepared.send_body,
            SubresourceResourceType::Xhr,
        ))),
    );
}

pub(super) fn update_worker_xhr_request(
    pending: &mut PendingWorkerXhr,
    record: PendingWorkerFetchNetworkRecord,
    observer: &crate::worker::WorkerNetworkObserver,
) {
    let previous = pending.network_record.as_ref();
    let changed = previous.map_or(&pending.request_url, |previous| &previous.url) != &record.url
        || previous.map_or(&pending.request_method, |previous| &previous.method) != &record.method
        || previous.map_or(&pending.request_headers, |previous| {
            &previous.request_headers
        }) != &record.request_headers
        || previous.map_or(&pending.request_body, |previous| &previous.request_body)
            != &record.request_body;
    if changed {
        publish_worker_network_item(
            observer,
            &pending.network,
            ScriptNetworkOutputItem::SubresourceRequestUpdated(Arc::new(worker_request_started(
                &pending.network,
                &pending.document_url,
                &record.url,
                &record.method,
                &record.request_headers,
                &record.request_body,
                SubresourceResourceType::Xhr,
            ))),
        );
    }
    pending.network_record = Some(record);
}

fn record_worker_xhr_response(
    state: &WorkerGlobalState,
    network: &crate::runtime::RendererWorkerNetworkRequest,
    head: Option<ResponseHead>,
    body: SubresourceBodyFinished,
    network_request_headers: Option<Vec<(String, String)>>,
) {
    let observer = state.parent_tx.network_observer();
    if let Some(head) = head {
        record_worker_fetch_response(&observer, network, head, network_request_headers);
    }
    publish_worker_network_item(
        &observer,
        network,
        ScriptNetworkOutputItem::SubresourceBodyFinished(Arc::new(body)),
    );
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
        record_worker_xhr_failure(
            &state.borrow(),
            &pending,
            WORKER_XHR_TIMEOUT_ERROR_TEXT.to_owned(),
        );
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
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, true);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);

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
        dispatch_xhr_upload_complete(scope, xhr, prepared.send_body.as_deref());
        if xhr_state_bool_property(scope, xhr, XHR_ABORTED_SLOT).unwrap_or(false)
            || worker_xhr_open_generation_changed(scope, xhr, open_generation)
        {
            return true;
        }
        xhr_dispatch_progress_event(scope, xhr, "loadstart", 0.0, 0.0);
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

    let local_response = local_url_response_result(&prepared.resolved_url, &prepared.method);
    if !async_request && let Some(result) = local_response {
        let Some(network) = state.borrow().global_kind.network().start_request() else {
            apply_worker_xhr_request_failure(scope, xhr, async_request, &request_url);
            return true;
        };
        record_worker_xhr_started(&state.borrow(), &network, &prepared);
        match result {
            Ok(response) => {
                record_worker_xhr_response(
                    &state.borrow(),
                    &network,
                    Some(response.head()),
                    SubresourceBodyFinished::ready(
                        network.handle(),
                        SubresourceResponseBody::from_fetch_response(&response),
                    ),
                    None,
                );
                apply_xhr_response(scope, xhr, response);
            }
            Err(error) => {
                record_worker_xhr_response(
                    &state.borrow(),
                    &network,
                    None,
                    SubresourceBodyFinished::failed(network.handle(), error.to_string()),
                    None,
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

    let Some(network) = state.borrow().global_kind.network().start_request() else {
        load.cancel();
        apply_xhr_failure(scope, xhr);
        return true;
    };
    let network_request_handle = network.handle();
    record_worker_xhr_started(&state.borrow(), &network, &prepared);
    let xhr_id = {
        let mut state = state.borrow_mut();
        let xhr_id = next_xhr_id(&mut state);
        let request_body = prepared.send_body.clone();
        state.pending_xhrs.insert(
            xhr_id,
            PendingWorkerXhr {
                xhr: v8::Global::new(scope, xhr),
                document_url: prepared.document_url.clone(),
                credentials_mode: prepared.credentials_mode,
                load: load.clone(),
                request_url: prepared.resolved_url.clone(),
                request_method: prepared.method.clone(),
                request_headers: prepared.request_headers.clone(),
                request_body,
                network: network.clone(),
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
            .send(WorkerXhrCompletion::decision(
                xhr_id,
                result
                    .map(|response| WorkerResourceResponse::Materialized(Box::new(response)))
                    .map_err(|error| error.to_string().into()),
            ));
        return true;
    }

    if intercept_request_stage {
        let info = PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: Some(network_request_handle),
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
        publish_worker_fetch_pause(
            &state.borrow(),
            crate::runtime::WorkerFetchTarget::Xhr(xhr_id),
            network_request_handle,
            load,
            crate::runtime::RendererWorkerFetchStage::Request(Box::new(info)),
        );
        return true;
    }

    let completion_tx = state.borrow().xhr_completion_tx.clone();
    spawn_worker_xhr_network(
        load,
        network,
        state.borrow().parent_tx.network_observer(),
        move |delivery| {
            let _ = completion_tx.send(WorkerXhrCompletion::TransportCompletion(delivery));
        },
        xhr_id,
        cancel_handle,
        prepared.network_request(&state.borrow()),
        true,
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
    let request_url = prepared.resolved_url.clone();
    let cancel_handle = FetchCancelHandle::new();
    let Some(load) = loader.register_load(
        ResourceLoadKind::Xhr,
        ResourceLoadDisposition::Ordinary,
        Some(cancel_handle.clone()),
    ) else {
        throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
        return;
    };
    let Some(network) = state.borrow().global_kind.network().start_request() else {
        load.cancel();
        throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
        return;
    };
    let observer = state.borrow().parent_tx.network_observer();
    record_worker_xhr_started(&state.borrow(), &network, &prepared);
    let (response_tx, response_rx) = bounded(1);
    let xhr_timeout = synchronous_worker_xhr_timeout(scope, xhr);
    let timeout_rx = xhr_timeout
        .as_ref()
        .map(|timeout| after(timeout.wait_delay))
        .unwrap_or_else(never);
    spawn_worker_xhr_network(
        load.clone(),
        network.clone(),
        observer.clone(),
        move |delivery| {
            let _ = response_tx.send(delivery);
        },
        0,
        cancel_handle,
        prepared
            .network_request(&state.borrow())
            .map(Request::with_page_network_policy),
        true,
    );
    let completion = select! {
        recv(response_rx) -> result => match result {
            Ok(delivery) => delivery.claim(&network).expect("synchronous XHR owns its exact network request"),
            Err(_) => WorkerRequestCompletion {
                id: 0,
                network_request_headers: None,
                result: Err("worker sync XHR completion channel closed".to_owned().into()),
            },
        },
        recv(timeout_rx) -> _ => {
            load.cancel();
            let timeout = xhr_timeout.as_ref().expect("timeout channel requires a configured deadline");
            publish_worker_request_failure(&observer, &network,
                format!("Synchronous XMLHttpRequest timed out after {} ms", timeout.configured_timeout.as_millis()).into());
            throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "TimeoutError");
            return;
        }
    };

    match completion.result {
        Ok(response) => {
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
                publish_worker_request_failure(&observer, &network, message.into());
                throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
                return;
            }
            if let Err(message) = crate::network_host::validate_cors_response_chain(
                &prepared.document_url,
                &response_head,
                prepared.credentials_mode,
            ) {
                publish_worker_request_failure(&observer, &network, message.into());
                throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
                return;
            }
            let native_head = response.native_head();
            let native_body = response.native_body(network.handle());
            match response.into_body_source() {
                Ok((head, body)) => {
                    record_worker_xhr_response(
                        &state.borrow(),
                        &network,
                        native_head,
                        native_body,
                        completion.network_request_headers,
                    );
                    apply_xhr_response_body_source(scope, xhr, head, body);
                }
                Err(message) => {
                    publish_worker_request_failure(&observer, &network, message.into());
                    throw_synchronous_xhr_failure(scope, xhr, &request_url_text, "NetworkError");
                }
            }
        }
        Err(error) => {
            publish_worker_request_failure(&observer, &network, error);
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
    let ready_state_key = v8str(scope, "readyState");
    let ready_state = xhr
        .get(scope, ready_state_key.into())
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0) as u32;
    if ready_state == 0 || ready_state == 4 {
        return true;
    }

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
        record_worker_xhr_failure(&state.borrow(), &pending, ABORTED_ERROR_TEXT.to_owned());
    }

    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, true);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 4.0);
    reset_xhr_response_for_request_error(scope, xhr);
    dispatch_xhr_upload_abort_if_in_progress(scope, xhr);
    xhr_dispatch_progress_event(scope, xhr, "abort", 0.0, 0.0);
    xhr_dispatch_progress_event(scope, xhr, "loadend", 0.0, 0.0);
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 0.0);
    true
}

pub(in crate::worker) fn record_worker_xhr_failure(
    state: &WorkerGlobalState,
    pending: &PendingWorkerXhr,
    error: impl Into<WorkerRequestError>,
) {
    publish_worker_request_failure(
        &state.parent_tx.network_observer(),
        &pending.network,
        error.into(),
    );
}

pub(in crate::worker) fn drain_worker_xhr_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerXhrCompletion,
) {
    let completion = match completion {
        WorkerXhrCompletion::Completion(completion) => *completion,
        WorkerXhrCompletion::TransportCompletion(delivery) => {
            let completion = state
                .borrow()
                .pending_xhrs
                .get(&delivery.request_id())
                .and_then(|pending| delivery.claim(&pending.network));
            let Some(completion) = completion else { return };
            completion
        }
    };
    if let Some(network_request_headers) = completion.network_request_headers.as_ref()
        && let Some(record) = state
            .borrow_mut()
            .pending_xhrs
            .get_mut(&completion.id)
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
                let Some(pending) = state_ref.pending_xhrs.get(&completion.id) else {
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
            let Some(pending) = state_ref.pending_xhrs.get(&completion.id) else {
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
            let Some(pending) = state.borrow_mut().pending_xhrs.remove(&completion.id) else {
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
            let Some(pending) = state_ref.pending_xhrs.get_mut(&completion.id) else {
                return;
            };
            let response_head = response.head();
            pending.network_record.clone().and_then(|mut record| {
                if record.handle_auth_requests
                    && matches!(response_head.status, 401 | 407)
                    && let Some(challenge) =
                        extract_subresource_auth_challenge(&response_head.headers)
                {
                    record.follow_redirects(&response_head);
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
                        request_body: request_body_text(&record.request_body),
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
            let state = state.borrow();
            let pending = &state.pending_xhrs[&completion.id];
            publish_worker_fetch_pause(
                &state,
                crate::runtime::WorkerFetchTarget::Xhr(completion.id),
                pending.network.handle(),
                pending.load.clone(),
                crate::runtime::RendererWorkerFetchStage::Auth(Box::new(info)),
            );
            return;
        }
        let response_paused = {
            let mut state = state.borrow_mut();
            let Some(pending) = state.pending_xhrs.get_mut(&completion.id) else {
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
                        request_body: request_body_text(&record.request_body),
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
            let state = state.borrow();
            let pending = &state.pending_xhrs[&completion.id];
            publish_worker_fetch_pause(
                &state,
                crate::runtime::WorkerFetchTarget::Xhr(completion.id),
                pending.network.handle(),
                pending.load.clone(),
                crate::runtime::RendererWorkerFetchStage::Response(Box::new(info)),
            );
            return;
        }
    }

    let Some(pending) = state.borrow_mut().pending_xhrs.remove(&completion.id) else {
        return;
    };
    pending.load.finish();
    let xhr = v8::Local::new(scope, &pending.xhr);
    cancel_worker_xhr_timeout(scope, xhr);
    match completion.result {
        Ok(response) => {
            let response_head = response.head();
            match crate::network_host::validate_cors_response_chain(
                &pending.document_url,
                &response_head,
                pending.credentials_mode,
            ) {
                Ok(()) => {
                    let native_body = response.native_body(pending.network.handle());
                    let native_head = response.native_head();
                    match response.into_body_source() {
                        Ok((mut response_head, body)) => {
                            let record = pending.network_record.as_ref();
                            record_worker_xhr_response(
                                &state.borrow(),
                                &pending.network,
                                native_head,
                                native_body,
                                record
                                    .and_then(|record| {
                                        record.initial_network_request_headers.clone()
                                    })
                                    .or(completion.network_request_headers),
                            );
                            response_head.headers = filter_cors_exposed_response_headers(
                                &pending.document_url,
                                &response_head,
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

fn prepare_worker_xhr_send_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    xhr: v8::Local<'_, v8::Object>,
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
    let request_headers =
        xhr_author_request_headers(scope, xhr, prepared_body.default_content_type);
    let credentials_mode =
        if xhr_state_bool_property(scope, xhr, XHR_WITH_CREDENTIALS_SLOT).unwrap_or(false) {
            RequestCredentialsMode::Include
        } else {
            RequestCredentialsMode::SameOrigin
        };

    Ok(PreparedWorkerXhrSendRequest {
        document_url,
        resolved_url,
        method,
        request_headers: moli_fetch::RequestHeaders::from_byte_strings(&request_headers)
            .expect("validated worker XHR headers are ByteStrings"),
        send_body: prepared_body.body,
        credentials_mode,
    })
}

// ─── postMessage ────────────────────────────────────────────────────────────
