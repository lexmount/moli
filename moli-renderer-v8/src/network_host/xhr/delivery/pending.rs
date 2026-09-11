use super::super::events::xhr_is_aborted;
use super::super::*;
use super::{apply_xhr_failure, apply_xhr_response_pending_body};

pub(in crate::network_host::xhr) fn queue_xhr_response_delivery(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    response: Response,
) {
    let (head, body) = response.into_body();
    let response_type = xhr_state_string_property(scope, xhr, XHR_RESPONSE_TYPE_SLOT)
        .as_deref()
        .and_then(XmlHttpRequestResponseType::parse)
        .unwrap_or(XmlHttpRequestResponseType::Default);
    set_xhr_state_string(scope, xhr, XHR_PENDING_KIND_SLOT, "response");
    set_xhr_state_number(scope, xhr, XHR_PENDING_STATUS_SLOT, head.status as f64);
    set_xhr_state_string(scope, xhr, XHR_PENDING_STATUS_TEXT_SLOT, head.status_text());
    set_xhr_state_string(scope, xhr, XHR_PENDING_URL_SLOT, head.final_url.as_str());
    match response_type {
        XmlHttpRequestResponseType::ArrayBuffer | XmlHttpRequestResponseType::Blob => {
            let body_bytes = body
                .try_into_materialized_bytes()
                .expect("XHR response body should remain materialized at the delivery boundary");
            set_xhr_state_number(
                scope,
                xhr,
                XHR_PENDING_BODY_LENGTH_SLOT,
                body_bytes.len() as f64,
            );
            // Preserve exact bytes behind the shared renderer body-source
            // carrier until the queued XHR completion materializes response.
            let body_source =
                crate::network_host::network_body_source_object_from_bytes(scope, None, body_bytes);
            set_xhr_state_value(scope, xhr, XHR_PENDING_BODY_BYTES_SLOT, body_source.into());
            set_xhr_state_string(scope, xhr, XHR_PENDING_BODY_SLOT, "");
        }
        XmlHttpRequestResponseType::Json
        | XmlHttpRequestResponseType::Document
        | XmlHttpRequestResponseType::Default
        | XmlHttpRequestResponseType::Text => {
            let (body_text, body_bytes) = body
                .try_into_lossy_materialized_text()
                .expect("XHR response body should remain materialized at the delivery boundary");
            set_xhr_state_number(
                scope,
                xhr,
                XHR_PENDING_BODY_LENGTH_SLOT,
                body_bytes.len() as f64,
            );
            set_xhr_state_value(
                scope,
                xhr,
                XHR_PENDING_BODY_BYTES_SLOT,
                v8::undefined(scope).into(),
            );
            set_xhr_state_string(scope, xhr, XHR_PENDING_BODY_SLOT, &body_text);
        }
    }
    let headers_json = serde_json::to_string(&head.headers).unwrap_or_else(|_| "[]".to_owned());
    set_xhr_state_string(scope, xhr, XHR_PENDING_HEADERS_SLOT, &headers_json);
    schedule_xhr_delivery(scope, host, xhr);
}

pub(in crate::network_host::xhr) fn queue_xhr_failure_delivery(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
) {
    set_xhr_state_string(scope, xhr, XHR_PENDING_KIND_SLOT, "failure");
    schedule_xhr_delivery(scope, host, xhr);
}

fn schedule_xhr_delivery(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
) {
    let Some(callback) = v8::Function::builder(xhr_complete_callback)
        .data(xhr.into())
        .build(scope)
    else {
        return;
    };
    let _ = host;
    enqueue_host_microtask(scope, callback);
}

fn xhr_complete_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.data().to_object(scope) else {
        return;
    };
    let kind = xhr_state_string_property(scope, xhr, XHR_PENDING_KIND_SLOT).unwrap_or_default();
    let pending_status =
        xhr_state_number_property(scope, xhr, XHR_PENDING_STATUS_SLOT).unwrap_or(0.0) as u16;
    let pending_status_text =
        xhr_state_string_property(scope, xhr, XHR_PENDING_STATUS_TEXT_SLOT).unwrap_or_default();
    let pending_url =
        xhr_state_string_property(scope, xhr, XHR_PENDING_URL_SLOT).unwrap_or_default();
    let pending_body_value = xhr_state_value(scope, xhr, XHR_PENDING_BODY_BYTES_SLOT)
        .filter(|value| !value.is_null_or_undefined());
    let pending_body_text =
        xhr_state_string_property(scope, xhr, XHR_PENDING_BODY_SLOT).unwrap_or_default();
    let pending_body_len =
        xhr_state_number_property(scope, xhr, XHR_PENDING_BODY_LENGTH_SLOT).map(|value| {
            if value.is_finite() && value >= 0.0 {
                value as usize
            } else {
                0
            }
        });
    let pending_headers_json = xhr_state_string_property(scope, xhr, XHR_PENDING_HEADERS_SLOT)
        .unwrap_or_else(|| "[]".to_owned());
    xhr_clear_pending(scope, xhr);
    if xhr_is_aborted(scope, xhr) {
        return;
    }
    match kind.as_str() {
        "response" => {
            let Some(final_url) = url::Url::parse(&pending_url).ok().or_else(|| {
                let host_ptr = context_host_ptr_from_global_bridge(scope)?;
                Some(unsafe { &*host_ptr }.document_url().clone())
            }) else {
                apply_xhr_failure(scope, xhr);
                return;
            };
            let headers = serde_json::from_str::<Vec<(String, String)>>(&pending_headers_json)
                .unwrap_or_default();
            apply_xhr_response_pending_body(
                scope,
                xhr,
                moli_fetch::ResponseHead {
                    status_text: Some(pending_status_text),
                    final_url,
                    status: pending_status,
                    headers,
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                pending_body_value,
                pending_body_text,
                pending_body_len,
            );
        }
        "failure" => apply_xhr_failure(scope, xhr),
        _ => {}
    }
}

fn xhr_clear_pending(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    set_xhr_state_string(scope, xhr, XHR_PENDING_KIND_SLOT, "");
    set_xhr_state_number(scope, xhr, XHR_PENDING_STATUS_SLOT, 0.0);
    set_xhr_state_string(scope, xhr, XHR_PENDING_STATUS_TEXT_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_PENDING_URL_SLOT, "");
    set_xhr_state_string(scope, xhr, XHR_PENDING_BODY_SLOT, "");
    set_xhr_state_number(scope, xhr, XHR_PENDING_BODY_LENGTH_SLOT, 0.0);
    set_xhr_state_value(
        scope,
        xhr,
        XHR_PENDING_BODY_BYTES_SLOT,
        v8::undefined(scope).into(),
    );
    set_xhr_state_string(scope, xhr, XHR_PENDING_HEADERS_SLOT, "[]");
}
