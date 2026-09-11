use super::super::events::{
    xhr_dispatch_progress_event_with_length_computable, xhr_is_aborted, xhr_is_async,
};
use super::super::*;
use moli_web_mime::{
    effective_response_mime_essence, effective_response_mime_type, is_dom_parser_xml_mime,
    is_html_document_mime, normalize_response_blob_mime_type,
};

pub(crate) fn apply_xhr_response(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    response: Response,
) {
    let (head, body) = response.into_body();
    apply_xhr_response_body_source(scope, xhr, head, body);
}

pub(crate) fn apply_xhr_response_body_source(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
) {
    apply_xhr_response_body_source_with_status_text(scope, xhr, head, body, None);
}

pub(crate) fn apply_xhr_response_body_source_with_status_text(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
    status_text: Option<&str>,
) {
    apply_xhr_response_body_source_with_mode(
        scope,
        xhr,
        head,
        body,
        status_text,
        XhrResponseDeliveryMode::Buffered,
    );
}

pub(crate) fn apply_xhr_streaming_response_body_source(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
    internal_id: u64,
) {
    if !super::progress::flush_xhr_streaming_progress(scope, xhr, internal_id) {
        return;
    }
    apply_xhr_response_body_source_with_mode(
        scope,
        xhr,
        head,
        body,
        None,
        XhrResponseDeliveryMode::Streaming { internal_id },
    );
}

fn apply_xhr_response_body_source_with_mode(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
    status_text: Option<&str>,
    mode: XhrResponseDeliveryMode,
) {
    // XHR's Web-visible surface is still fully materialized. Branch by the
    // requested responseType so byte consumers do not pay for an inaccessible
    // lossy text view.
    match xhr_response_type(scope, xhr) {
        XmlHttpRequestResponseType::ArrayBuffer | XmlHttpRequestResponseType::Blob => {
            let body_bytes = body
                .try_into_materialized_bytes()
                .expect("XHR response body should remain materialized at the V8 boundary");
            apply_xhr_response_bytes(scope, xhr, head, body_bytes, status_text, mode);
        }
        XmlHttpRequestResponseType::Json
        | XmlHttpRequestResponseType::Document
        | XmlHttpRequestResponseType::Default
        | XmlHttpRequestResponseType::Text => {
            let (body_text, body_bytes) = body
                .try_into_lossy_materialized_text()
                .expect("XHR response body should remain materialized at the V8 boundary");
            apply_xhr_response_body(
                scope,
                xhr,
                head,
                Some(body_text),
                body_bytes,
                status_text,
                mode,
            );
        }
    }
}

#[derive(Clone, Copy)]
enum XhrResponseDeliveryMode {
    Buffered,
    Streaming { internal_id: u64 },
}

#[derive(Clone, Copy)]
struct XhrResponseProgress {
    loaded: f64,
    length_computable: bool,
    total: f64,
}

pub(in crate::network_host::xhr) fn apply_xhr_response_pending_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'s, v8::Object>,
    head: moli_fetch::ResponseHead,
    body_value: Option<v8::Local<'s, v8::Value>>,
    fallback_body_text: String,
    fallback_body_len: Option<usize>,
) {
    let response_type = xhr_response_type(scope, xhr);
    if response_type == XmlHttpRequestResponseType::ArrayBuffer
        && let Some(body_value) = body_value
        && let Some(loaded) = buffer_value_byte_length(body_value)
    {
        apply_xhr_response_prebuilt_value(
            scope,
            xhr,
            head,
            response_type,
            body_value,
            loaded,
            None,
            XhrResponseDeliveryMode::Buffered,
        );
        return;
    }

    if body_value.is_none()
        && matches!(
            response_type,
            XmlHttpRequestResponseType::Json
                | XmlHttpRequestResponseType::Document
                | XmlHttpRequestResponseType::Default
                | XmlHttpRequestResponseType::Text
        )
    {
        let loaded = fallback_body_len.unwrap_or(fallback_body_text.len());
        apply_xhr_response_text(
            scope,
            xhr,
            head,
            fallback_body_text,
            loaded,
            None,
            XhrResponseDeliveryMode::Buffered,
        );
        return;
    }

    let body_bytes = body_value
        .and_then(|value| {
            v8::Local::<v8::Object>::try_from(value)
                .ok()
                .and_then(|object| {
                    crate::network_host::take_network_body_bytes_from_object(scope, object)
                })
                .or_else(|| blob::buffer_source_bytes_from_value(scope, value))
        })
        .unwrap_or_else(|| fallback_body_text.into_bytes());
    apply_xhr_response_bytes(
        scope,
        xhr,
        head,
        body_bytes,
        None,
        XhrResponseDeliveryMode::Buffered,
    );
}

fn apply_xhr_response_bytes(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    body_bytes: Vec<u8>,
    status_text: Option<&str>,
    mode: XhrResponseDeliveryMode,
) {
    apply_xhr_response_body(scope, xhr, head, None, body_bytes, status_text, mode);
}

fn apply_xhr_response_body(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    body_text: Option<String>,
    body_bytes: Vec<u8>,
    status_text: Option<&str>,
    mode: XhrResponseDeliveryMode,
) {
    let Some(response_type) = prepare_xhr_response(scope, xhr, &head, status_text, mode) else {
        return;
    };
    let body_text = match response_type {
        XmlHttpRequestResponseType::Json
        | XmlHttpRequestResponseType::Document
        | XmlHttpRequestResponseType::Default
        | XmlHttpRequestResponseType::Text => {
            Some(body_text.unwrap_or_else(|| String::from_utf8_lossy(&body_bytes).into_owned()))
        }
        XmlHttpRequestResponseType::ArrayBuffer | XmlHttpRequestResponseType::Blob => None,
    };
    let loaded = body_bytes.len() as f64;
    let progress = xhr_response_progress(&head, loaded);
    let response_val: v8::Local<'_, v8::Value> = match response_type {
        XmlHttpRequestResponseType::Json => {
            v8_json_parse(scope, body_text.as_deref().unwrap_or(""))
                .unwrap_or_else(|| v8::null(scope).into())
        }
        XmlHttpRequestResponseType::Document => {
            let mime = xhr_response_mime_essence(scope, xhr, &head.headers)
                .unwrap_or_else(|| "text/html".to_owned());
            let document = parse_xhr_response_document(
                scope,
                xhr,
                &head,
                body_text.as_deref().unwrap_or(""),
                Some(&mime),
            );
            set_xhr_state_value(scope, xhr, XHR_RESPONSE_XML_SLOT, document);
            document
        }
        XmlHttpRequestResponseType::ArrayBuffer => blob::array_buffer_from_bytes(scope, body_bytes)
            .map(|value| value.into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        XmlHttpRequestResponseType::Blob => {
            let mime_type = xhr_response_blob_mime_type(scope, xhr, &head.headers);
            blob::build_blob_object(scope, body_bytes, mime_type)
                .map(|value| value.into())
                .unwrap_or_else(|| v8::undefined(scope).into())
        }
        XmlHttpRequestResponseType::Default | XmlHttpRequestResponseType::Text => {
            if response_type == XmlHttpRequestResponseType::Default {
                let document = parse_default_xhr_response_xml(
                    scope,
                    xhr,
                    &head,
                    body_text.as_deref().unwrap_or(""),
                );
                set_xhr_state_value(scope, xhr, XHR_RESPONSE_XML_SLOT, document);
            }
            v8_string(scope, body_text.as_deref().unwrap_or(""))
                .map(|s| s.into())
                .unwrap_or_else(|| v8::undefined(scope).into())
        }
    };
    finish_xhr_response(
        scope,
        xhr,
        response_type,
        response_val,
        body_text.as_deref().unwrap_or(""),
        progress,
        mode,
    );
}

fn apply_xhr_response_text(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    body_text: String,
    loaded: usize,
    status_text: Option<&str>,
    mode: XhrResponseDeliveryMode,
) {
    let Some(response_type) = prepare_xhr_response(scope, xhr, &head, status_text, mode) else {
        return;
    };
    let progress = xhr_response_progress(&head, loaded as f64);
    let response_val: v8::Local<'_, v8::Value> = match response_type {
        XmlHttpRequestResponseType::Json => {
            v8_json_parse(scope, &body_text).unwrap_or_else(|| v8::null(scope).into())
        }
        XmlHttpRequestResponseType::Document => {
            let mime = xhr_response_mime_essence(scope, xhr, &head.headers)
                .unwrap_or_else(|| "text/html".to_owned());
            let document = parse_xhr_response_document(scope, xhr, &head, &body_text, Some(&mime));
            set_xhr_state_value(scope, xhr, XHR_RESPONSE_XML_SLOT, document);
            document
        }
        XmlHttpRequestResponseType::Default | XmlHttpRequestResponseType::Text => {
            if response_type == XmlHttpRequestResponseType::Default {
                let document = parse_default_xhr_response_xml(scope, xhr, &head, &body_text);
                set_xhr_state_value(scope, xhr, XHR_RESPONSE_XML_SLOT, document);
            }
            v8_string(scope, &body_text)
                .map(|s| s.into())
                .unwrap_or_else(|| v8::undefined(scope).into())
        }
        XmlHttpRequestResponseType::ArrayBuffer | XmlHttpRequestResponseType::Blob => {
            v8::undefined(scope).into()
        }
    };
    finish_xhr_response(
        scope,
        xhr,
        response_type,
        response_val,
        &body_text,
        progress,
        mode,
    );
}

fn apply_xhr_response_prebuilt_value(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    response_type: XmlHttpRequestResponseType,
    response_val: v8::Local<'_, v8::Value>,
    loaded: usize,
    status_text: Option<&str>,
    mode: XhrResponseDeliveryMode,
) {
    let Some(_) = prepare_xhr_response(scope, xhr, &head, status_text, mode) else {
        return;
    };
    let progress = xhr_response_progress(&head, loaded as f64);
    finish_xhr_response(scope, xhr, response_type, response_val, "", progress, mode);
}

pub(crate) fn apply_xhr_streaming_response_head(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: &moli_fetch::ResponseHead,
    internal_id: u64,
) -> bool {
    if !super::progress::xhr_stream_is_current(scope, xhr, internal_id) {
        return false;
    }
    set_xhr_response_head(scope, xhr, head, None);
    super::super::events::xhr_fire_readystatechange(scope, xhr, 2);
    !scope.is_execution_terminating()
        && super::progress::xhr_stream_is_current(scope, xhr, internal_id)
}

pub(crate) fn apply_xhr_streaming_response_chunk(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
    decoded_text: &str,
    loaded: usize,
    total: Option<usize>,
) -> bool {
    if !super::progress::xhr_stream_is_current(scope, xhr, internal_id) {
        return false;
    }
    if matches!(
        xhr_response_type(scope, xhr),
        XmlHttpRequestResponseType::Default | XmlHttpRequestResponseType::Text
    ) {
        let mut response_text =
            xhr_state_string_property(scope, xhr, XHR_RESPONSE_TEXT_SLOT).unwrap_or_default();
        response_text.push_str(decoded_text);
        set_xhr_state_string(scope, xhr, XHR_RESPONSE_TEXT_SLOT, &response_text);
        let response_value: v8::Local<'_, v8::Value> = v8_string(scope, &response_text)
            .map(Into::into)
            .unwrap_or_else(|| v8::undefined(scope).into());
        set_xhr_state_value(scope, xhr, XHR_RESPONSE_SLOT, response_value);
    }

    super::progress::dispatch_or_defer_xhr_streaming_progress(
        scope,
        xhr,
        internal_id,
        loaded,
        total,
    )
}

fn prepare_xhr_response(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: &moli_fetch::ResponseHead,
    status_text: Option<&str>,
    mode: XhrResponseDeliveryMode,
) -> Option<XmlHttpRequestResponseType> {
    if let XhrResponseDeliveryMode::Streaming { internal_id } = mode
        && !super::progress::xhr_stream_is_current(scope, xhr, internal_id)
    {
        return None;
    }
    super::cancel_xhr_timeout(scope, xhr);
    super::clear_xhr_timeout_start(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    if xhr_is_aborted(scope, xhr) {
        return None;
    }
    if matches!(mode, XhrResponseDeliveryMode::Buffered) {
        set_xhr_response_head(scope, xhr, head, status_text);
    }
    Some(xhr_response_type(scope, xhr))
}

fn set_xhr_response_head(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: &moli_fetch::ResponseHead,
    status_text: Option<&str>,
) {
    set_xhr_state_number(scope, xhr, XHR_STATUS_SLOT, head.status as f64);
    set_xhr_state_string(
        scope,
        xhr,
        XHR_STATUS_TEXT_SLOT,
        status_text.unwrap_or_else(|| http_status_text(head.status)),
    );
    let response_url = xhr_response_url(head);
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_URL_SLOT, response_url.as_str());

    let headers_json = serde_json::to_string(&head.headers).unwrap_or_else(|_| "[]".to_owned());
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_HEADERS_SLOT, &headers_json);
}

fn finish_xhr_response(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    response_type: XmlHttpRequestResponseType,
    response_val: v8::Local<'_, v8::Value>,
    response_text: &str,
    progress: XhrResponseProgress,
    mode: XhrResponseDeliveryMode,
) {
    if !matches!(
        response_type,
        XmlHttpRequestResponseType::Default | XmlHttpRequestResponseType::Document
    ) {
        set_xhr_state_value(scope, xhr, XHR_RESPONSE_XML_SLOT, v8::null(scope).into());
    }

    let dispatch_intermediate_events =
        matches!(mode, XhrResponseDeliveryMode::Buffered) && xhr_is_async(scope, xhr);
    if dispatch_intermediate_events {
        super::super::events::xhr_fire_readystatechange(scope, xhr, 2);
        if scope.is_execution_terminating() {
            return;
        }
        if xhr_is_aborted(scope, xhr) {
            return;
        }
    }
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_TEXT_SLOT, response_text);
    set_xhr_state_value(scope, xhr, XHR_RESPONSE_SLOT, response_val);
    if dispatch_intermediate_events {
        super::super::events::xhr_fire_readystatechange(scope, xhr, 3);
        if scope.is_execution_terminating() {
            return;
        }
        if xhr_is_aborted(scope, xhr) {
            return;
        }
    }
    if dispatch_intermediate_events {
        xhr_dispatch_progress_event_with_length_computable(
            scope,
            xhr,
            "progress",
            progress.length_computable,
            progress.loaded,
            progress.total,
        );
        if scope.is_execution_terminating() {
            return;
        }
        if xhr_is_aborted(scope, xhr) {
            return;
        }
    }
    super::super::events::xhr_fire_readystatechange(scope, xhr, 4);
    if scope.is_execution_terminating() {
        return;
    }
    if xhr_is_aborted(scope, xhr) {
        return;
    }

    xhr_dispatch_progress_event_with_length_computable(
        scope,
        xhr,
        "load",
        progress.length_computable,
        progress.loaded,
        progress.total,
    );
    if scope.is_execution_terminating() {
        return;
    }
    xhr_dispatch_progress_event_with_length_computable(
        scope,
        xhr,
        "loadend",
        progress.length_computable,
        progress.loaded,
        progress.total,
    );
}

fn xhr_response_progress(head: &moli_fetch::ResponseHead, loaded: f64) -> XhrResponseProgress {
    if matches!(head.final_url.scheme(), "data" | "blob") {
        return XhrResponseProgress {
            loaded,
            length_computable: true,
            total: loaded,
        };
    }

    let total = identity_encoded_content_length(&head.headers).map(|value| value as f64);
    XhrResponseProgress {
        loaded,
        length_computable: total.is_some(),
        total: total.unwrap_or(0.0),
    }
}

fn identity_encoded_content_length(headers: &[(String, String)]) -> Option<u64> {
    let cannot_compare_delivered_body_length = headers.iter().any(|(name, value)| {
        (name.eq_ignore_ascii_case("content-encoding")
            && !value.trim().eq_ignore_ascii_case("identity"))
            || name.eq_ignore_ascii_case("transfer-encoding")
    });
    if cannot_compare_delivered_body_length {
        return None;
    }

    let mut values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .flat_map(|(_, value)| value.split(','))
        .map(|value| value.trim().parse::<u64>());
    let first = values.next()?.ok()?;
    values
        .all(|value| value.ok() == Some(first))
        .then_some(first)
}

fn xhr_response_type(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> XmlHttpRequestResponseType {
    xhr_state_string_property(scope, xhr, XHR_RESPONSE_TYPE_SLOT)
        .as_deref()
        .and_then(XmlHttpRequestResponseType::parse)
        .unwrap_or(XmlHttpRequestResponseType::Default)
}

fn buffer_value_byte_length(value: v8::Local<'_, v8::Value>) -> Option<usize> {
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        return Some(buffer.byte_length());
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        return Some(view.byte_length());
    }
    None
}

fn xhr_response_mime_value(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    headers: &[(String, String)],
) -> Option<String> {
    let override_mime = xhr_state_string_property(scope, xhr, XHR_OVERRIDE_MIME_TYPE_SLOT);
    effective_response_mime_type(headers, override_mime.as_deref())
}

fn xhr_response_mime_essence(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    headers: &[(String, String)],
) -> Option<String> {
    let override_mime = xhr_state_string_property(scope, xhr, XHR_OVERRIDE_MIME_TYPE_SLOT);
    effective_response_mime_essence(headers, override_mime.as_deref())
}

fn parse_default_xhr_response_xml<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: &moli_fetch::ResponseHead,
    body_text: &str,
) -> v8::Local<'s, v8::Value> {
    let mime = xhr_response_mime_essence(scope, xhr, &head.headers);
    parse_xhr_response_document(scope, xhr, head, body_text, mime.as_deref())
}

fn xhr_response_url(head: &moli_fetch::ResponseHead) -> &url::Url {
    head.redirect_chain
        .last()
        .map(|redirect| &redirect.to_url)
        .unwrap_or(&head.final_url)
}

fn parse_xhr_response_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: &moli_fetch::ResponseHead,
    body_text: &str,
    mime: Option<&str>,
) -> v8::Local<'s, v8::Value> {
    let Some(mime) =
        mime.filter(|mime| is_html_document_mime(mime) || is_dom_parser_xml_mime(mime))
    else {
        return v8::null(scope).into();
    };
    build_xhr_response_document(scope, xhr, xhr_response_url(head).clone(), body_text, mime)
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into())
}

fn build_xhr_response_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
    response_url: url::Url,
    body_text: &str,
    mime: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let xhr = v8::Global::new(scope, xhr);
    let xhr = v8::Local::new(scope, xhr);
    let context = xhr.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &*host_ptr };
    let binding = xhr_execution_context_binding(scope, host, xhr)?;
    let origin_document_handle = match binding.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Top => host.document_handle(),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_document_handle(handle)?
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            host.lightweight_popup_document_handle(popup_id)?
        }
    };
    let origin_document = unsafe { &mut *host_ptr }.native_bridge_mut().wrap_handle(
        scope,
        host_ptr,
        origin_document_handle,
    )?;
    let document = dom_parser::parse_detached_document_from_string_with_url(
        scope,
        response_url,
        body_text,
        mime,
    )?;
    // The response URL controls relative URL resolution; its origin does not
    // replace the XHR environment's origin (including for CORS responses).
    crate::native_bridge::document::inherit_detached_document_origin(
        scope,
        document,
        origin_document,
    );
    Some(document)
}

fn xhr_response_blob_mime_type(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    headers: &[(String, String)],
) -> String {
    normalize_response_blob_mime_type(xhr_response_mime_value(scope, xhr, headers).as_deref())
}

#[cfg(test)]
mod tests {
    use moli_web_mime::{effective_response_mime_essence, response_blob_mime_type};

    #[test]
    fn xhr_document_response_mime_uses_shared_effective_essence() {
        let headers = vec![(
            "Content-Type".to_owned(),
            "Text/HTML; Charset=UTF-8".to_owned(),
        )];

        assert_eq!(
            effective_response_mime_essence(&headers, None),
            Some("text/html".to_owned())
        );
        assert_eq!(
            effective_response_mime_essence(&headers, Some("Application/XML")),
            Some("application/xml".to_owned())
        );
    }

    #[test]
    fn xhr_blob_response_mime_uses_shared_blob_normalization() {
        let headers = vec![(
            "Content-Type".to_owned(),
            "Text/Plain; Charset=UTF-8".to_owned(),
        )];

        assert_eq!(
            response_blob_mime_type(&headers),
            "text/plain; charset=utf-8"
        );
    }
}
