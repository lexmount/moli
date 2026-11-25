use super::super::events::{
    xhr_dispatch_progress_event_with_length_computable, xhr_is_aborted, xhr_is_async,
};
use super::super::*;
use moli_encoding::{XhrResponseDecoder, XhrResponseTextKind, encoding_for_label};
use moli_web_mime::{
    effective_response_mime_essence, extract_response_mime_type, is_html_document_mime,
    mime_charset,
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
    let body_bytes = body
        .try_into_materialized_bytes()
        .expect("XHR response body should remain materialized at the V8 boundary");
    apply_xhr_response_body(scope, xhr, head, body_bytes, status_text, mode);
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

fn apply_xhr_response_body(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    head: moli_fetch::ResponseHead,
    body_bytes: Vec<u8>,
    status_text: Option<&str>,
    mode: XhrResponseDeliveryMode,
) {
    let Some(response_type) = prepare_xhr_response(scope, xhr, &head, status_text, mode) else {
        return;
    };
    let (body_text, character_set) =
        if let Some(mut decoder) = xhr_text_decoder(scope, xhr, &head.headers, response_type) {
            let mut text = decoder.push(&body_bytes);
            text.push_str(&decoder.finish());
            (text, decoder.encoding_name())
        } else {
            (String::new(), "UTF-8")
        };
    let loaded = body_bytes.len() as f64;
    let progress = xhr_response_progress(&head, loaded);
    let response_val: v8::Local<'_, v8::Value> = match response_type {
        XmlHttpRequestResponseType::Json => parse_xhr_json_response(scope, &body_bytes),
        XmlHttpRequestResponseType::Document => {
            let document = parse_xhr_response_document(
                scope,
                xhr,
                &body_text,
                &head.headers,
                response_type,
                character_set,
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
                let document = parse_xhr_response_document(
                    scope,
                    xhr,
                    &body_text,
                    &head.headers,
                    response_type,
                    character_set,
                );
                set_xhr_state_value(scope, xhr, XHR_RESPONSE_XML_SLOT, document);
            }
            v8_string(scope, &body_text)
                .map(|s| s.into())
                .unwrap_or_else(|| v8::undefined(scope).into())
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

fn parse_xhr_json_response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: &[u8],
) -> v8::Local<'s, v8::Value> {
    // XHR JSON uses UTF-8 regardless of MIME overrides and consumes exactly
    // one leading UTF-8 BOM. Parse failures produce null, not a script error.
    let text = moli_encoding::decode_utf8(bytes);
    v8::tc_scope!(let scope, scope);
    v8_string(scope, &text)
        .and_then(|text| v8::json::parse(scope, text))
        .unwrap_or_else(|| v8::null(scope).into())
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
    super::super::events::xhr_fire_readystatechange(scope, xhr, XhrReadyState::HeadersReceived);
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
    clear_xhr_active_request(scope, xhr);
    if xhr_is_aborted(scope, xhr) {
        return None;
    }
    if matches!(mode, XhrResponseDeliveryMode::Buffered) {
        set_xhr_response_head(scope, xhr, head, status_text);
        if xhr_is_async(scope, xhr) {
            let generation = xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT);
            super::super::events::xhr_fire_readystatechange(
                scope,
                xhr,
                XhrReadyState::HeadersReceived,
            );
            if scope.is_execution_terminating()
                || xhr_is_aborted(scope, xhr)
                || xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT) != generation
            {
                return None;
            }
        }
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
        status_text.unwrap_or_else(|| head.status_text()),
    );
    let response_url = head
        .redirect_chain
        .last()
        .map(|redirect| &redirect.to_url)
        .unwrap_or(&head.final_url);
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_URL_SLOT, response_url.as_str());

    // All delivery paths expose the same filtered header list, including
    // synchronous workers and responses fulfilled without HTTP transport.
    // Keep head.headers intact for MIME processing and network metadata.
    let headers = super::super::header_surface::xhr_response_headers_for_script(&head.headers);
    let headers_json = serde_json::to_string(&headers).unwrap_or_else(|_| "[]".to_owned());
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
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_TEXT_SLOT, response_text);
    set_xhr_state_value(scope, xhr, XHR_RESPONSE_SLOT, response_val);
    if dispatch_intermediate_events {
        super::super::events::xhr_fire_readystatechange(scope, xhr, XhrReadyState::Loading);
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
    super::super::events::xhr_fire_readystatechange(scope, xhr, XhrReadyState::Done);
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

fn identity_encoded_content_length(headers: &[(String, Vec<u8>)]) -> Option<u64> {
    let headers = moli_fetch::headers_to_byte_strings(headers);
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

pub(crate) fn xhr_streaming_text_decoder(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    headers: &[(String, Vec<u8>)],
) -> Option<XhrResponseDecoder> {
    let response_type = xhr_response_type(scope, xhr);
    xhr_text_decoder(scope, xhr, headers, response_type)
}

fn xhr_text_decoder(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    headers: &[(String, Vec<u8>)],
    response_type: XmlHttpRequestResponseType,
) -> Option<XhrResponseDecoder> {
    if matches!(
        response_type,
        XmlHttpRequestResponseType::Json
            | XmlHttpRequestResponseType::ArrayBuffer
            | XmlHttpRequestResponseType::Blob
    ) {
        return None;
    }

    let override_mime = xhr_state_string_property(scope, xhr, XHR_OVERRIDE_MIME_TYPE_SLOT);
    let response_mime = extract_response_mime_type(headers);
    let override_charset = override_mime
        .as_deref()
        .and_then(mime_charset)
        .filter(|label| !label.is_empty());
    let charset = override_charset
        .as_deref()
        .or_else(|| response_mime.as_ref()?.parameter("charset"));
    let mime = override_mime
        .as_deref()
        .filter(|mime| !mime.is_empty())
        .and_then(moli_web_mime::mime_essence)
        .or_else(|| response_mime.as_ref().map(|mime| mime.essence()))
        .unwrap_or_else(|| "text/xml".to_owned());
    let kind = match response_type {
        XmlHttpRequestResponseType::Document if is_html_document_mime(&mime) => {
            XhrResponseTextKind::Html
        }
        XmlHttpRequestResponseType::Default | XmlHttpRequestResponseType::Document
            if matches!(mime.as_str(), "text/xml" | "application/xml")
                || mime.ends_with("+xml") =>
        {
            XhrResponseTextKind::Xml
        }
        _ => XhrResponseTextKind::Text,
    };
    Some(XhrResponseDecoder::new(
        kind,
        charset.and_then(encoding_for_label),
    ))
}

fn xhr_response_mime_essence(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    headers: &[(String, Vec<u8>)],
) -> Option<String> {
    let override_mime = xhr_state_string_property(scope, xhr, XHR_OVERRIDE_MIME_TYPE_SLOT);
    effective_response_mime_essence(headers, override_mime.as_deref())
}

fn parse_xhr_response_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
    body_text: &str,
    headers: &[(String, Vec<u8>)],
    response_type: XmlHttpRequestResponseType,
    character_set: &str,
) -> v8::Local<'s, v8::Value> {
    let mime =
        xhr_response_mime_essence(scope, xhr, headers).unwrap_or_else(|| "text/xml".to_owned());
    let is_xml = matches!(mime.as_str(), "text/xml" | "application/xml") || mime.ends_with("+xml");
    let is_html = is_html_document_mime(&mime);
    if (!is_xml && !is_html) || (is_html && response_type != XmlHttpRequestResponseType::Document) {
        return v8::null(scope).into();
    }
    let document = if is_html {
        dom_parser::parse_detached_document_from_string(scope, body_text, &mime)
    } else {
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            return v8::null(scope).into();
        };
        let document_url = unsafe { &*host_ptr }.document_url().clone();
        dom_parser::parse_detached_xml_document_from_source(
            scope,
            document_url,
            body_text,
            &mime,
            dom_parser::XmlParseErrorBehavior::ReturnNone,
        )
    };
    let Some(document) = document else {
        return v8::null(scope).into();
    };
    let Ok((runtime_ptr, document_handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, document)
    else {
        return v8::null(scope).into();
    };
    // Response metadata belongs to this Document, independently of the
    // requesting Document and any later reuse of the XHR object.
    let dom = unsafe { &mut *runtime_ptr }.dom_host_mut();
    dom.set_document_character_set_for_handle(document_handle, character_set);
    dom.set_document_source_last_modified_for_handle(
        document_handle,
        crate::document_last_modified::document_last_modified_from_headers(headers),
    );
    document.into()
}

fn xhr_response_blob_mime_type(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    headers: &[(String, Vec<u8>)],
) -> String {
    // overrideMimeType already stores a parsed and serialized MIME record.
    // Native response Blobs use that record, not Blob constructor normalization.
    xhr_state_string_property(scope, xhr, XHR_OVERRIDE_MIME_TYPE_SLOT)
        .filter(|mime| !mime.is_empty())
        .or_else(|| extract_response_mime_type(headers).map(|mime| mime.to_string()))
        .unwrap_or_else(|| "text/xml".to_owned())
}

#[cfg(test)]
mod tests {
    use moli_web_mime::effective_response_mime_essence;

    #[test]
    fn xhr_document_response_mime_uses_shared_effective_essence() {
        let headers: Vec<(String, Vec<u8>)> = vec![(
            "Content-Type".to_owned(),
            b"Text/HTML; Charset=UTF-8".to_vec(),
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
}
