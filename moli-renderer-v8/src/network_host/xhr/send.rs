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
    PreparedXhrSendBody, prepare_xhr_send_body_from_args, xhr_author_request_headers,
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
use crossbeam_channel::{after, bounded, never, select};
use moli_fetch::{BrowserRequestMetadata, FetchCancelHandle, RequestMode};
use std::{thread, time::Duration};

pub(super) fn xhr_send_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if crate::worker::try_worker_xhr_send_callback(scope, &args) {
        return;
    }

    let xhr = args.this();
    let method =
        xhr_state_string_property(scope, xhr, XHR_METHOD_SLOT).unwrap_or_else(|| "GET".to_owned());
    let prepared_body = match prepare_xhr_send_body_from_args(scope, &args, &method) {
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

    let async_request = xhr_is_async(scope, xhr);

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
    if !async_request
        && !host
            .document_permissions_policy_for_owner(owner)
            .is_some_and(
                crate::permissions_policy::DocumentPermissionsPolicy::synchronous_xhr_enabled,
            )
    {
        set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
        throw_synchronous_xhr_failure(scope, xhr, prepared.resolved_url.as_str(), "NetworkError");
        return;
    }
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

    if let Some(response) = local_url_response(&prepared.resolved_url) {
        if !xhr_is_async(scope, xhr) {
            record_xhr_response_success(host, &prepared, &response);
            apply_xhr_response(scope, xhr, response);
            return;
        }
        queue_local_xhr_response(scope, host, xhr, prepared, response);
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

    let loader = prepared.resource_loader.request_client().clone();

    if async_request {
        let internal_id = spawn_network_xhr_fetch(scope, host, xhr, prepared, loader);
        set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, internal_id as f64);
        schedule_xhr_timeout(scope, host, xhr, internal_id);
    } else {
        if !host.allow_synchronous_xhr_request(&prepared.resolved_url) {
            set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
            throw_type_error(scope, "Synchronous XMLHttpRequest request limit exceeded");
            return;
        }
        send_synchronous_network_xhr(scope, host, xhr, prepared, loader);
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
    loader: ResourceRequestClient,
) {
    let request = Request::new_bytes(
        &prepared.method,
        prepared.resolved_url.as_str(),
        prepared.send_body.clone(),
        prepared.request_headers.clone(),
    )
    .expect("xhr request url was already resolved")
    .with_initiator_url(&prepared.document_url)
    .with_credentials_mode(prepared.credentials_mode)
    .with_network_partition_key(prepared.network_partition_key.clone())
    .with_browser_request_metadata(BrowserRequestMetadata::Xhr);

    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.resolved_url,
        &prepared.method,
        prepared.credentials_mode,
    );
    let request_url = prepared.resolved_url.clone();
    let request_method = prepared.method.clone();
    let request_headers = prepared.request_headers.clone();
    let request_body = request_body_text(&prepared.send_body);
    let preflight_headers = prepared.cors_preflight_request_headers.clone();
    let cancel_handle = FetchCancelHandle::new();
    let worker_cancel_handle = cancel_handle.clone();
    let (response_tx, response_rx) = bounded(1);
    let xhr_timeout = synchronous_xhr_timeout(scope, xhr);
    let timeout_rx = xhr_timeout.map(after).unwrap_or_else(never);
    let page_context_cancel_rx = host.page_context_cancel_receiver();
    if let Some(reason) = page_context_cancel_rx.reason() {
        cancel_handle.cancel();
        let reason_text = match reason {
            RendererPageContextCancelReason::PageClosed => "page was closed",
            RendererPageContextCancelReason::ContextDropped => "context was dropped",
        };
        host.record_subresource_network(SubresourceNetworkRecord::failure(
            prepared.frame_id,
            prepared.document_url,
            request_url,
            request_method,
            request_headers,
            request_body,
            SubresourceResourceType::Xhr,
            format!("Synchronous XMLHttpRequest aborted because {reason_text}"),
        ));
        apply_xhr_abort(scope, xhr);
        return;
    }

    let spawn_result = thread::Builder::new()
        .name("lm-sync-xhr-fetch".to_owned())
        .spawn(move || {
            let runtime_result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("failed to build sync XHR fetch runtime: {error}"));
            let result = match runtime_result {
                Ok(runtime) => runtime.block_on(fetch_browser_subresource_with_preflight_headers(
                    loader,
                    request,
                    Some(worker_cancel_handle),
                    preflight_headers,
                )),
                Err(error) => Err(error),
            };
            let _ = response_tx.send(result);
        });

    let result = match spawn_result {
        Ok(_) => select! {
            recv(response_rx) -> result => result.unwrap_or_else(|_| {
                Err("sync XHR fetch thread dropped response channel".to_owned())
            }),
            recv(timeout_rx) -> _ => {
                let timeout = xhr_timeout.expect("never channel should not fire without xhr timeout");
                cancel_handle.cancel();
                host.record_subresource_network(SubresourceNetworkRecord::failure(
                    prepared.frame_id,
                    prepared.document_url,
                    request_url.clone(),
                    request_method,
                    request_headers,
                    request_body,
                    SubresourceResourceType::Xhr,
                    format!(
                        "Synchronous XMLHttpRequest timed out after {} ms",
                        timeout.as_millis()
                    ),
                ));
                throw_synchronous_xhr_failure(
                    scope,
                    xhr,
                    request_url.as_str(),
                    "TimeoutError",
                );
                return;
            },
            recv(page_context_cancel_rx.wake_receiver()) -> _ => {
                let reason = page_context_cancel_rx
                    .reason()
                    .unwrap_or(RendererPageContextCancelReason::ContextDropped);
                cancel_handle.cancel();
                let reason_text = match reason {
                    RendererPageContextCancelReason::PageClosed => "page was closed",
                    RendererPageContextCancelReason::ContextDropped => "context was dropped",
                };
                host.record_subresource_network(SubresourceNetworkRecord::failure(
                    prepared.frame_id,
                    prepared.document_url,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    SubresourceResourceType::Xhr,
                    format!("Synchronous XMLHttpRequest aborted because {reason_text}"),
                ));
                apply_xhr_abort(scope, xhr);
                return;
            }
        },
        Err(error) => Err(format!("failed to spawn sync XHR fetch thread: {error}")),
    };

    let result = result.and_then(|response| {
        crate::network_host::validate_fetch_response_security_policy_with_body(
            &prepared.document_url,
            &response.final_url,
            &response.headers,
            response.body_bytes(),
            RequestMode::Cors,
            prepared.credentials_mode,
            prepared.policy_context,
        )?;
        Ok(response)
    });

    match result {
        Ok(response) => {
            crate::context_bootstrap::record_resource_performance_entry(
                scope,
                crate::context_bootstrap::ResourcePerformanceEntry::from_fetch_response(
                    prepared.resolved_url.as_str(),
                    "xmlhttprequest",
                    None,
                    &response,
                ),
            );
            let observable_headers = crate::network_host::filter_cors_exposed_response_headers(
                &prepared.document_url,
                &response.final_url,
                &response.headers,
                prepared.credentials_mode,
            );
            host.record_subresource_network(
                SubresourceNetworkRecord::success_with_body(
                    prepared.frame_id,
                    prepared.document_url,
                    prepared.resolved_url,
                    prepared.method,
                    prepared.request_headers,
                    request_body,
                    SubresourceResourceType::Xhr,
                    response
                        .request_cookie_report
                        .clone()
                        .or(request_cookie_report),
                    response
                        .redirect_chain
                        .clone()
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    response.final_url.clone(),
                    response.status,
                    response.headers.clone(),
                    crate::protocol_types::SubresourceResponseBody::from_fetch_response(&response),
                    response.cookie_set_reports.clone(),
                )
                .with_from_cache(response.from_cache)
                .with_negotiated_http_version(response.negotiated_http_version),
            );
            let mut response = response;
            response.headers = observable_headers;
            apply_xhr_response(scope, xhr, response);
        }
        Err(error_text) => {
            host.record_subresource_network(SubresourceNetworkRecord::failure(
                prepared.frame_id,
                prepared.document_url,
                request_url.clone(),
                request_method,
                request_headers,
                request_body,
                SubresourceResourceType::Xhr,
                error_text,
            ));
            throw_synchronous_xhr_failure(scope, xhr, request_url.as_str(), "NetworkError");
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
