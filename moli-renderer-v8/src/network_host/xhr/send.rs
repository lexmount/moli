mod paths;
mod request;

use self::paths::{
    dispatch_service_worker_xhr, queue_local_xhr_response, record_bad_port_xhr_failure,
    record_blocked_xhr_failure, record_csp_xhr_failure, record_intercepted_xhr,
    record_offline_xhr_failure, record_url_policy_xhr_failure, record_xhr_response_success,
    request_body_text, spawn_network_xhr_fetch,
};
#[cfg(test)]
pub(crate) use self::request::prepare_xhr_send_body;
pub(crate) use self::request::{
    PreparedXhrSendBody, convert_xhr_send_body_from_args, xhr_author_request_headers,
};
use self::request::{
    PreparedXhrSendRequest, XhrSendPrepareError, prepare_xhr_send_request,
    xhr_dom_debugger_request_url,
};
use super::delivery::{
    apply_xhr_abort, apply_xhr_response, cancel_xhr_timeout, mark_xhr_timeout_start,
    queue_xhr_failure_delivery, schedule_xhr_timeout, throw_synchronous_xhr_failure,
};
use super::events::{
    xhr_dispatch_progress_event, xhr_dispatch_upload_progress_event, xhr_is_aborted, xhr_is_async,
};
use super::*;
use crate::runtime::RendererPageContextCancelReason;
use crossbeam_channel::{after, never, select, unbounded};
use moli_fetch::{BrowserRequestMetadata, FetchCancelHandle, RequestMode};
use std::time::Duration;

pub(super) fn xhr_send_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if crate::worker::try_worker_xhr_send_callback(scope, &args) {
        return;
    }

    let xhr = args.this();
    let body = match convert_xhr_send_body_from_args(scope, &args) {
        Ok(body) => body,
        Err(error) => {
            crate::webidl::throw_error(scope, &error);
            return;
        }
    };

    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let breakpoint_url = xhr_dom_debugger_request_url(scope, host, xhr);
    host.break_on_dom_debugger_xhr_or_fetch_network_request(&breakpoint_url);

    if !xhr_ensure_send_allowed(scope, xhr) {
        return;
    }

    let method =
        xhr_state_string_property(scope, xhr, XHR_METHOD_SLOT).unwrap_or_else(|| "GET".to_owned());
    let async_request = xhr_is_async(scope, xhr);
    let prepared_body = match body.prepare(scope, &method) {
        Ok(body) => body,
        Err(error) => {
            crate::webidl::throw_error(scope, &error);
            return;
        }
    };

    cancel_xhr_timeout(scope, xhr);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, true);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);

    let prepared = match prepare_xhr_send_request(scope, host, xhr, method, prepared_body) {
        Ok(prepared) => prepared,
        Err(XhrSendPrepareError::ExecutionContext) => {
            if async_request {
                queue_xhr_failure_delivery(scope, host, xhr);
            } else {
                let request_url =
                    xhr_state_string_property(scope, xhr, XHR_URL_SLOT).unwrap_or_default();
                throw_synchronous_xhr_failure(scope, xhr, &request_url, "NetworkError");
            }
            tracing::debug!("XHR owner execution context is unavailable");
            return;
        }
        Err(XhrSendPrepareError::Url(message)) => {
            if async_request {
                queue_xhr_failure_delivery(scope, host, xhr);
            } else {
                let request_url =
                    xhr_state_string_property(scope, xhr, XHR_URL_SLOT).unwrap_or_default();
                throw_synchronous_xhr_failure(scope, xhr, &request_url, "NetworkError");
            }
            tracing::debug!("XHR URL resolution error: {message}");
            return;
        }
    };
    let url_policy = moli_url_policy::route_xml_http_request_url(&prepared.resolved_url);

    mark_xhr_timeout_start(scope, xhr);
    let open_generation =
        xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT).unwrap_or(0.0);
    if async_request {
        dispatch_xhr_upload_complete(scope, xhr, prepared.send_body.as_deref());
        if xhr_is_aborted(scope, xhr) || xhr_open_generation_changed(scope, xhr, open_generation) {
            return;
        }
        xhr_dispatch_progress_event(scope, xhr, "loadstart", 0.0, 0.0);
        if xhr_is_aborted(scope, xhr) || xhr_open_generation_changed(scope, xhr, open_generation) {
            return;
        }
    }

    let owner = prepared.owner;
    if let Some(violation) = host
        .check_document_connect_csp_for_owner(
            scope,
            owner,
            &prepared.document_url,
            &prepared.resolved_url,
        )
        .into_blocking_violation()
    {
        let message = crate::document_runtime::document_content_security_policy_error_message(
            &violation,
            "XMLHttpRequest",
        );
        if async_request {
            record_csp_xhr_failure(scope, host, xhr, prepared, message);
        } else {
            record_synchronous_xhr_failure(scope, host, xhr, prepared, message);
        }
        return;
    }

    if let Err(error) = url_policy {
        if async_request {
            record_url_policy_xhr_failure(scope, host, xhr, prepared, error.to_string());
        } else {
            record_synchronous_xhr_failure(scope, host, xhr, prepared, error.to_string());
        }
        return;
    }

    if moli_fetch::should_request_be_blocked_due_to_bad_port(&prepared.resolved_url) {
        if async_request {
            record_bad_port_xhr_failure(scope, host, xhr, prepared);
        } else {
            let error_text = format!("xhr: blocked bad port for `{}`", prepared.resolved_url);
            record_synchronous_xhr_failure(scope, host, xhr, prepared, error_text);
        }
        return;
    }

    if host.is_url_blocked(&prepared.resolved_url) {
        if async_request {
            record_blocked_xhr_failure(scope, host, xhr, prepared);
        } else {
            record_synchronous_xhr_failure(
                scope,
                host,
                xhr,
                prepared,
                BLOCKED_BY_CLIENT_ERROR_TEXT.to_owned(),
            );
        }
        return;
    }

    // Renderer-owned URLs, including local errors, bypass network interception.
    if let Some(result) = local_url_response_result(&prepared.resolved_url, &prepared.method) {
        match result {
            Ok(response) if async_request => {
                queue_local_xhr_response(scope, host, xhr, prepared, response);
            }
            Ok(response) => {
                record_xhr_response_success(host, &prepared, &response);
                apply_xhr_response(scope, xhr, response);
            }
            Err(error) if async_request => {
                record_url_policy_xhr_failure(scope, host, xhr, prepared, error.to_string());
            }
            Err(error) => {
                record_synchronous_xhr_failure(scope, host, xhr, prepared, error.to_string());
            }
        }
        return;
    }

    if host.should_intercept_subresource(SubresourceResourceType::Xhr) {
        if !async_request {
            record_synchronous_xhr_failure(
                scope,
                host,
                xhr,
                prepared,
                "Synchronous XMLHttpRequest interception is not supported".to_owned(),
            );
            return;
        }
        let internal_id = record_intercepted_xhr(scope, host, xhr, prepared);
        set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, internal_id as f64);
        schedule_xhr_timeout(scope, host, xhr, internal_id);
        return;
    }

    if host.network_offline() {
        if async_request {
            record_offline_xhr_failure(scope, host, xhr, prepared);
        } else {
            record_synchronous_xhr_failure(
                scope,
                host,
                xhr,
                prepared,
                "Network emulation offline".to_owned(),
            );
        }
        return;
    }

    if async_request
        && let Some(internal_id) = dispatch_service_worker_xhr(scope, host, xhr, &prepared)
    {
        set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, internal_id as f64);
        schedule_xhr_timeout(scope, host, xhr, internal_id);
        return;
    }

    if async_request {
        let loader = prepared.resource_loader.request_client().clone();
        let internal_id = spawn_network_xhr_fetch(scope, host, xhr, prepared, loader);
        set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, internal_id as f64);
        schedule_xhr_timeout(scope, host, xhr, internal_id);
    } else {
        send_synchronous_network_xhr(scope, host, xhr, prepared);
    }
}

fn xhr_open_generation_changed(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    expected: f64,
) -> bool {
    xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT)
        .is_some_and(|current| current != expected)
}

pub(crate) fn dispatch_xhr_upload_complete(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    send_body: Option<&[u8]>,
) {
    let Some(send_body) = send_body else {
        return;
    };
    let total = send_body.len() as f64;
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT, true);
    for event_type in ["loadstart", "progress", "load", "loadend"] {
        if xhr_is_aborted(scope, xhr) {
            set_xhr_state_bool(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT, false);
            return;
        }
        xhr_dispatch_upload_progress_event(scope, xhr, event_type, total, total);
    }
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT, false);
}

pub(crate) fn dispatch_xhr_upload_abort_if_in_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    if !xhr_state_bool_property(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT).unwrap_or(false) {
        return;
    }
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT, false);
    xhr_dispatch_upload_progress_event(scope, xhr, "abort", 0.0, 0.0);
    xhr_dispatch_upload_progress_event(scope, xhr, "loadend", 0.0, 0.0);
}

fn send_synchronous_network_xhr(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
) {
    use crate::network::{ResourceResponseFailure, ResourceResponseStream, ResourceTransfer};
    use std::sync::Arc;

    let request = Request::new_bytes(
        &prepared.method,
        prepared.resolved_url.as_str(),
        prepared.send_body.clone(),
        prepared.request_headers.clone(),
    )
    .expect("xhr request url was already resolved")
    .with_initiator_url(&prepared.document_url)
    .with_request_origin(prepared.request_origin.clone())
    .with_credentials_mode(prepared.credentials_mode)
    .with_network_partition_key(prepared.network_partition_key.clone())
    .with_browser_request_metadata(BrowserRequestMetadata::Xhr);
    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.request_origin,
        &prepared.resolved_url,
        &prepared.method,
        prepared.credentials_mode,
    );
    let cancel_handle = FetchCancelHandle::new();
    let xhr_timeout = synchronous_xhr_timeout(scope, xhr);
    let timeout_rx = xhr_timeout.map(after).unwrap_or_else(never);
    let page_context_cancel_rx = host.page_context_cancel_receiver();
    if let Some(reason) = page_context_cancel_rx.reason() {
        let reason_text = match reason {
            RendererPageContextCancelReason::PageClosed => "page was closed",
            RendererPageContextCancelReason::ContextDropped => "context was dropped",
        };
        host.record_subresource_network(SubresourceNetworkRecord::failure(
            prepared.frame_id,
            prepared.document_url,
            prepared.resolved_url,
            prepared.method,
            prepared.request_headers,
            request_body_text(&prepared.send_body),
            SubresourceResourceType::Xhr,
            format!("Synchronous XMLHttpRequest aborted because {reason_text}"),
        ));
        apply_xhr_abort(scope, xhr);
        return;
    }
    let load = prepared
        .resource_loader
        .register_load(
            crate::network::loads::ResourceLoadKind::Xhr,
            crate::network::loads::ResourceLoadDisposition::Ordinary,
            Some(cancel_handle.clone()),
        )
        .expect("synchronous XHR retains its Document load");
    enum SynchronousXhrEvent {
        Network(crate::runtime::RendererNetworkObservation),
        Completed(Box<Result<moli_fetch::Response, ResourceResponseFailure>>),
    }
    let (response_tx, response_rx) = unbounded();
    let response_tx = Arc::new(response_tx);
    // The publisher must not keep the completion channel alive if its I/O task exits.
    let observer_tx = Arc::downgrade(&response_tx);
    let (network, started) = ResourceTransfer::start(
        host.document_network_reporter()
            .expect("a synchronous XHR has a Document source")
            .start_request()
            .expect("an active Document admits its XHR"),
        move |event| {
            if let Some(sender) = observer_tx.upgrade() {
                let _ = sender.send(SynchronousXhrEvent::Network(event));
            }
        },
        |network| {
            crate::types::SubresourceRequestStarted::new(
                network.handle(),
                prepared.frame_id.clone(),
                prepared.document_url.clone(),
                prepared.resolved_url.clone(),
                prepared.method.clone(),
                prepared.request_headers.clone(),
                request_body_text(&prepared.send_body),
                SubresourceResourceType::Xhr,
                crate::types::SubresourceRequestInitiatorType::Script,
                request_cookie_report,
            )
            .with_request_body_bytes(prepared.send_body.clone())
        },
    );
    host.record_native_resource_observation(started);
    let response = ResourceResponseStream::for_load(network, &load, SubresourceResourceType::Xhr);
    let observer = response.network.clone();
    let preflight = crate::network_host::CorsPreflightNetworkObserver {
        request: observer.request(),
        observer: Arc::new(move |event| observer.observe(event)),
        frame_id: prepared.frame_id.clone(),
        resource_type: SubresourceResourceType::Xhr,
        keepalive: false,
    };
    let preflight_headers = prepared.cors_preflight_request_headers.clone();
    let worker_response = response.clone();
    let worker_cancel = cancel_handle.clone();
    load.task_runner().spawn(async move {
        let loader = load.request_client();
        let result = async {
            let stream = crate::network_host::fetch_browser_subresource_raw_stream_with_preflight_headers_and_observer(
                &loader, request, Some(worker_cancel), preflight_headers, Some(&preflight),
            ).await?;
            let body = worker_response.collect(stream).await?;
            let bytes = body.body.materialize_bytes().map_err(|error| {
                worker_response.failure(format!("failed to materialize synchronous XHR body: {error}"))
            })?;
            Ok(moli_fetch::RawResponse::from_head_and_body(body.head, bytes)
                .into_lossy_materialized_text_response())
        }.await;
        let _ = response_tx.send(SynchronousXhrEvent::Completed(Box::new(result)));
        load.finish();
    });
    let fail = |host: &mut JsContextHost, message| {
        response
            .network
            .failed_with(&response.failure(message), |event| {
                // Terminal permission has been claimed; no later producer callback can
                // overtake the receipts already queued before this cancellation.
                while let Ok(SynchronousXhrEvent::Network(earlier)) = response_rx.try_recv() {
                    host.record_native_resource_observation(earlier);
                }
                host.record_native_resource_observation(event);
            });
    };
    host.publish_live_turn_output_prefix();
    let result = loop {
        select! {
            recv(response_rx) -> event => match event {
                Ok(SynchronousXhrEvent::Network(event)) => {
                    host.record_native_resource_observation(event);
                    host.publish_live_turn_output_prefix();
                }
                Ok(SynchronousXhrEvent::Completed(result)) => break *result,
                Err(_) => break Err(response.failure("sync XHR request dropped response channel".into())),
            },
            recv(timeout_rx) -> _ => {
                cancel_handle.cancel();
                fail(host, format!("Synchronous XMLHttpRequest timed out after {} ms",
                    xhr_timeout.expect("never channel cannot fire without a timeout").as_millis()));
                throw_synchronous_xhr_failure(scope, xhr, prepared.resolved_url.as_str(), "TimeoutError");
                return;
            },
            recv(page_context_cancel_rx.wake_receiver()) -> _ => {
                cancel_handle.cancel();
                let reason_text = match page_context_cancel_rx.reason()
                    .unwrap_or(RendererPageContextCancelReason::ContextDropped) {
                    RendererPageContextCancelReason::PageClosed => "page was closed",
                    RendererPageContextCancelReason::ContextDropped => "context was dropped",
                };
                fail(host, format!("Synchronous XMLHttpRequest aborted because {reason_text}"));
                apply_xhr_abort(scope, xhr);
                return;
            }
        }
    };
    let result = result.and_then(|fetched| {
        crate::network_host::validate_fetch_response_security_policy_with_body(
            &prepared.request_origin,
            &fetched.head(),
            fetched.body_bytes(),
            RequestMode::Cors,
            prepared.credentials_mode,
            prepared.policy_context,
        )
        .map_err(|message| response.failure(message))?;
        Ok(fetched)
    });
    match result {
        Ok(mut fetched) => {
            response.network.body_completed_with(
                response.head().as_ref().clone(),
                response
                    .finish_response()
                    .expect("synchronous XHR retains its received response")
                    .body,
                |event| host.record_native_resource_observation(event),
            );
            crate::context_bootstrap::record_resource_performance_entry(
                scope,
                crate::context_bootstrap::ResourcePerformanceEntry::from_fetch_response(
                    prepared.resolved_url.as_str(),
                    "xmlhttprequest",
                    None,
                    &fetched,
                ),
            );
            fetched.headers = crate::network_host::filter_cors_exposed_response_headers(
                &prepared.request_origin,
                &fetched.head(),
                prepared.credentials_mode,
            );
            apply_xhr_response(scope, xhr, fetched);
        }
        Err(error) => {
            response.network.failed_with(&error, |event| {
                host.record_native_resource_observation(event)
            });
            throw_synchronous_xhr_failure(
                scope,
                xhr,
                prepared.resolved_url.as_str(),
                "NetworkError",
            );
        }
    }
}

fn synchronous_xhr_timeout(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<Duration> {
    xhr_state_number_property(scope, xhr, XHR_TIMEOUT_SLOT)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| Duration::from_millis(value.min(u32::MAX as f64) as u64))
}

fn record_synchronous_xhr_failure(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
    error_text: String,
) {
    let request_url = prepared.resolved_url.to_string();
    host.record_subresource_network(SubresourceNetworkRecord::failure(
        prepared.frame_id,
        prepared.document_url,
        prepared.resolved_url,
        prepared.method,
        prepared.request_headers,
        request_body_text(&prepared.send_body),
        SubresourceResourceType::Xhr,
        error_text,
    ));
    throw_synchronous_xhr_failure(scope, xhr, &request_url, "NetworkError");
}
