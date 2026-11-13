mod paths;
mod request;

use self::paths::{
    dispatch_service_worker_xhr, queue_local_xhr_response, record_bad_port_xhr_failure,
    record_blocked_xhr_failure, record_compatibility_intercepted_xhr, record_csp_xhr_failure,
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
use crate::network::synchronous_load::{SynchronousLoadOutcome, wait_for_synchronous_load};
use crate::network_host::controlled_xhr::{
    ControlledXhrContext, XhrInterceptionRoute, start_buffered_xhr,
};
use crate::runtime::RendererPageContextCancelReason;
use moli_fetch::{FetchCancelHandle, RequestMode};
use std::time::Duration;

pub(super) fn xhr_send_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let xhr = xhr_receiver_target(scope, args.this());
    if crate::worker::try_worker_xhr_send_callback(scope, &args, xhr) {
        return;
    }

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
    begin_xhr_send(scope, xhr);

    let prepared = match prepare_xhr_send_request(scope, host, xhr, method, prepared_body) {
        Ok(prepared) => prepared,
        Err(XhrSendPrepareError::ExecutionContext) => {
            if async_request {
                queue_xhr_failure_delivery(scope, xhr);
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
                queue_xhr_failure_delivery(scope, xhr);
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
        if !dispatch_xhr_loadstart(scope, xhr, prepared.send_body.as_deref()) {
            return;
        }
        dispatch_xhr_upload_complete(scope, xhr, prepared.send_body.as_deref());
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

    let route = match host.xhr_interception_route(prepared.owner) {
        Ok(route) => route,
        Err(message) => {
            if async_request {
                record_url_policy_xhr_failure(scope, host, xhr, prepared, message.into());
            } else {
                record_synchronous_xhr_failure(scope, host, xhr, prepared, message.into());
            }
            return;
        }
    };
    if async_request && !matches!(route, XhrInterceptionRoute::Ordinary) {
        let internal_id = match route {
            XhrInterceptionRoute::Native(control) => {
                let loader = prepared.resource_loader.request_client().clone();
                spawn_network_xhr_fetch(scope, host, xhr, prepared, loader, Some(control))
            }
            XhrInterceptionRoute::Compatibility => {
                record_compatibility_intercepted_xhr(scope, host, xhr, prepared)
            }
            XhrInterceptionRoute::Ordinary => unreachable!(),
        };
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

    let loader = prepared.resource_loader.request_client().clone();

    if async_request {
        let internal_id = spawn_network_xhr_fetch(scope, host, xhr, prepared, loader, None);
        set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, internal_id as f64);
        schedule_xhr_timeout(scope, host, xhr, internal_id);
    } else {
        let control = match route.synchronous() {
            Ok(control) => control,
            Err(message) => {
                record_synchronous_xhr_failure(scope, host, xhr, prepared, message.into());
                return;
            }
        };
        send_synchronous_network_xhr(scope, host, xhr, prepared, control);
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

pub(crate) fn dispatch_xhr_loadstart(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    send_body: Option<&[u8]>,
) -> bool {
    let open_generation =
        xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT).unwrap_or(0.0);
    // The XHR loadstart listener can abort before upload.loadstart runs.
    let has_upload_listeners = xhr_upload_object(scope, xhr).is_some_and(|upload| {
        crate::context_bootstrap::simple_object_has_event_listeners(scope, upload)
    });
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_LISTENER_SLOT, has_upload_listeners);
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT, send_body.is_some());
    xhr_dispatch_progress_event(scope, xhr, "loadstart", 0.0, 0.0);
    if xhr_is_aborted(scope, xhr) || xhr_open_generation_changed(scope, xhr, open_generation) {
        return false;
    }
    if let Some(send_body) = send_body {
        xhr_dispatch_upload_progress_event(scope, xhr, "loadstart", 0.0, send_body.len() as f64);
    }
    !xhr_is_aborted(scope, xhr) && !xhr_open_generation_changed(scope, xhr, open_generation)
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
    for event_type in ["progress", "load", "loadend"] {
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
    host: &mut PageScriptHost,
    xhr: v8::Local<'_, v8::Object>,
    prepared: PreparedXhrSendRequest,
    control: Option<ControlledXhrContext>,
) {
    let request = prepared.request().with_request_mode(RequestMode::Cors);

    let request_cookie_report = observe_subresource_request_cookie_report(
        prepared.resource_loader.request_client(),
        &prepared.document_url,
        &prepared.request_origin,
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
    let xhr_timeout = synchronous_xhr_timeout(scope, xhr);
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

    let Some(load) = prepared.resource_loader.register_load(
        crate::network::loads::ResourceLoadKind::Xhr,
        crate::network::loads::ResourceLoadDisposition::Ordinary,
        Some(cancel_handle.clone()),
    ) else {
        record_synchronous_xhr_failure(
            scope,
            host,
            xhr,
            prepared,
            "XHR Document is no longer active".into(),
        );
        return;
    };
    let mut info = PendingSubresourceFetchInfo {
        internal_id: 0,
        network_request_handle: None,
        frame_id: prepared.frame_id.clone(),
        document_url: prepared.document_url.clone(),
        url: prepared.resolved_url.clone(),
        websocket_socket_id: None,
        method: prepared.method.clone(),
        request_headers: prepared.request_headers.clone(),
        request_body: request_body.clone(),
        request_body_bytes: prepared.send_body.clone(),
        resource_type: SubresourceResourceType::Xhr,
        request_cookie_report: request_cookie_report.clone(),
    };
    host.assign_pending_subresource_fetch_identity(&mut info);
    let native_handle = control.as_ref().and(info.network_request_handle);
    let output_scope = host.begin_synchronous_network_wait();
    let response_rx = start_buffered_xhr(
        request,
        info,
        load,
        cancel_handle.clone(),
        preflight_headers,
        control,
    );
    let result = match wait_for_synchronous_load(
        response_rx,
        page_context_cancel_rx.wake_receiver(),
        xhr_timeout,
    ) {
        SynchronousLoadOutcome::Completed(result) => result,
        SynchronousLoadOutcome::TimedOut => {
            cancel_handle.cancel();
            drop(output_scope);
            let mut record = SubresourceNetworkRecord::failure(
                prepared.frame_id,
                prepared.document_url,
                request_url.clone(),
                request_method,
                request_headers,
                request_body,
                SubresourceResourceType::Xhr,
                format!(
                    "Synchronous XMLHttpRequest timed out after {} ms",
                    xhr_timeout.unwrap().as_millis()
                ),
            );
            if let Some(handle) = native_handle {
                record = record.with_request_handle(handle);
            }
            host.record_subresource_network(record);
            throw_synchronous_xhr_failure(scope, xhr, request_url.as_str(), "TimeoutError");
            return;
        }
        SynchronousLoadOutcome::OwnerCancelled => {
            cancel_handle.cancel();
            drop(output_scope);
            let reason = page_context_cancel_rx
                .reason()
                .unwrap_or(RendererPageContextCancelReason::ContextDropped);
            let reason_text = match reason {
                RendererPageContextCancelReason::PageClosed => "page was closed",
                RendererPageContextCancelReason::ContextDropped => "context was dropped",
            };
            let mut record = SubresourceNetworkRecord::failure(
                prepared.frame_id,
                prepared.document_url,
                request_url,
                request_method,
                request_headers,
                request_body,
                SubresourceResourceType::Xhr,
                format!("Synchronous XMLHttpRequest aborted because {reason_text}"),
            );
            if let Some(handle) = native_handle {
                record = record.with_request_handle(handle);
            }
            host.record_subresource_network(record);
            apply_xhr_abort(scope, xhr);
            return;
        }
    };
    // XHR's own completion events may now run on the original JS stack; the
    // native network publication privilege ends before entering those callbacks.
    drop(output_scope);
    let completed = match result {
        Ok(completed) => completed,
        Err(error) => {
            record_synchronous_xhr_failure(scope, host, xhr, prepared, error);
            return;
        }
    };
    // Window's synchronous report preserves the concrete rejection reason;
    // live protocol/Worker output retains its existing normalized error text.
    let record = match &completed.result {
        Err(error) => completed.network_record.with_error_text(error.clone()),
        Ok(_) => completed.network_record,
    };
    host.record_subresource_network(record);

    match completed.result {
        Ok(response) => {
            crate::context_bootstrap::record_resource_performance_entry(
                scope,
                crate::context_bootstrap::ResourcePerformanceEntry::from_fetch_response(
                    prepared.resolved_url.as_str(),
                    "xmlhttprequest",
                    None,
                    &response,
                )
                .with_initiator_url(crate::resource_initiator::response_url(
                    prepared.initiator_url.clone(),
                    &prepared.request_origin,
                    &response.final_url,
                    response.cache_state,
                )),
            );
            let observable_headers = crate::network_host::filter_cors_exposed_response_headers(
                &prepared.request_origin,
                &response.head(),
                prepared.credentials_mode,
            );
            let mut response = response;
            response.headers = observable_headers;
            apply_xhr_response(scope, xhr, response);
        }
        Err(_) => {
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
    host: &mut PageScriptHost,
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
