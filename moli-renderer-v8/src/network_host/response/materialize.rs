use super::super::fetch_surface::{
    RESPONSE_BODY_USED_SLOT, RESPONSE_INTERNAL_HEADERS_SLOT, RESPONSE_INTERNAL_STATUS_SLOT,
    RESPONSE_INTERNAL_STATUS_TEXT_SLOT, body_already_used, mark_response_object,
    response_slot_number,
};
use super::metadata::{response_metadata, set_response_metadata};
use super::*;
use crate::network_host::headers::headers_list;
use crate::types::NetworkBodySourceId;
use crate::web_api_interfaces;
use moli_fetch::RequestMode;
use moli_fetch_types::{FetchResponseMetadata, FetchResponseType as FetchResponseFilter};
use moli_url::WebOrigin;
use moli_webapi_declare::WebApiObject;

#[derive(Clone, Copy)]
pub(crate) struct FetchResponseRequest<'a> {
    pub(crate) method: &'a str,
    pub(crate) mode: RequestMode,
}

impl FetchResponseRequest<'_> {
    fn has_null_body(self, status: u16) -> bool {
        matches!(self.method, "HEAD" | "CONNECT") || matches!(status, 101 | 103 | 204 | 205 | 304)
    }
}

#[derive(WebApiObject)]
#[webapi(plain, prototype = "Response")]
struct FetchResponseHeadDeclaration {
    #[webapi(slot = RESPONSE_STATUS_SLOT)]
    status: f64,
    #[webapi(slot = RESPONSE_OK_SLOT)]
    ok: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct FetchResponseBodyDeclaration<'scope> {
    #[webapi(slot = RESPONSE_STATUS_TEXT_SLOT)]
    status_text: String,
    #[webapi(slot = RESPONSE_BODY_SLOT)]
    body: v8::Local<'scope, v8::Value>,
    #[webapi(slot = RESPONSE_BODY_USED_SLOT, init = false)]
    body_used: (),
}

/// Classifies a network response using the whole redirect chain. A no-cors
/// response stays opaque even when a cross-origin hop redirects back home.
/// Service worker responses must retain their own filter instead: their URL
/// does not determine whether the worker returned a readable response.
pub(crate) fn network_response_filter(
    request_origin: impl Into<WebOrigin>,
    head: &moli_fetch::ResponseHead,
    request_mode: RequestMode,
) -> Option<crate::types::AsyncSubresourceFetchResponseFilter> {
    let kind = head.fetch_response_type(&request_origin.into(), request_mode);
    matches!(
        kind,
        FetchResponseFilter::Opaque | FetchResponseFilter::OpaqueRedirect
    )
    .then(|| kind.into())
}

fn filtered_response_status(head: &moli_fetch::ResponseHead, filter: FetchResponseFilter) -> u16 {
    if filter.is_readable() { head.status } else { 0 }
}

fn filtered_response_exposes_body(filter: FetchResponseFilter) -> bool {
    filter.is_readable()
}

fn filtered_response_exposes_headers(filter: FetchResponseFilter) -> bool {
    filter.is_readable()
}

fn filtered_response_status_text(
    head: &moli_fetch::ResponseHead,
    filter: FetchResponseFilter,
) -> &str {
    if filter.is_readable() {
        head.status_text()
    } else {
        ""
    }
}

pub(crate) fn build_fetch_response_object_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request_origin: impl Into<WebOrigin>,
    request: FetchResponseRequest<'_>,
    response: Response,
) -> v8::Local<'s, v8::Object> {
    let request_origin = request_origin.into();
    let (head, body) = response.into_body();
    build_fetch_response_object_from_body_source_for_request_mode(
        scope,
        &request_origin,
        request,
        head,
        body,
    )
}

pub(crate) fn build_fetch_response_object_from_body_source_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request_origin: impl Into<WebOrigin>,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
) -> v8::Local<'s, v8::Object> {
    let metadata = head.fetch_metadata(&request_origin.into(), request.mode);
    build_fetch_response_object_from_body_source_for_request_mode_with_metadata(
        scope, request, head, body, metadata,
    )
}

pub(crate) fn build_fetch_response_object_from_body_source_for_request_mode_with_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
    metadata: FetchResponseMetadata,
) -> v8::Local<'s, v8::Object> {
    let filter = metadata.response_type;
    let obj = build_fetch_response_object_head(scope, &head, &metadata, None);
    let body_stream = if request.has_null_body(head.status) {
        None
    } else if filtered_response_exposes_body(filter) {
        network_body_stream_from_response_body(scope, obj, body)
    } else {
        set_filtered_response_internal_body_from_response_body(scope, obj, body);
        None
    };
    finish_fetch_response_object_with_body_stream(scope, obj, &head, body_stream, filter)
}

pub(crate) fn build_fetch_response_object_from_subresource_body_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body: crate::protocol_types::SubresourceResponseBody,
    metadata: FetchResponseMetadata,
) -> v8::Local<'s, v8::Object> {
    let filter = metadata.response_type;
    let obj = build_fetch_response_object_head(scope, &head, &metadata, None);
    let body_stream = if request.has_null_body(head.status) {
        None
    } else if filtered_response_exposes_body(filter) {
        Some(network_body_stream_from_subresource_body(scope, obj, body))
    } else {
        set_filtered_response_internal_body_from_subresource_body(scope, obj, body);
        None
    };
    finish_fetch_response_object_with_body_stream(scope, obj, &head, body_stream, filter)
}

#[cfg(test)]
pub(crate) fn build_fetch_response_object_from_stream_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request_origin: impl Into<WebOrigin>,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body_source_id: NetworkBodySourceId,
) -> v8::Local<'s, v8::Object> {
    let metadata = head.fetch_metadata(&request_origin.into(), request.mode);
    build_fetch_response_object_from_stream_for_request_mode_with_metadata(
        scope,
        request,
        head,
        body_source_id,
        metadata,
    )
}

pub(crate) fn build_fetch_response_object_from_stream_for_request_mode_with_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body_source_id: NetworkBodySourceId,
    metadata: FetchResponseMetadata,
) -> v8::Local<'s, v8::Object> {
    build_fetch_response_object_from_stream_for_request_mode_with_surface_url(
        scope,
        request,
        head,
        body_source_id,
        None,
        metadata,
    )
}

pub(crate) fn build_navigation_preload_response_object_from_stream_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request_url: &url::Url,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body_source_id: NetworkBodySourceId,
    metadata: FetchResponseMetadata,
) -> v8::Local<'s, v8::Object> {
    build_fetch_response_object_from_stream_for_request_mode_with_surface_url(
        scope,
        request,
        head,
        body_source_id,
        Some(request_url.as_str()),
        metadata,
    )
}

fn build_fetch_response_object_from_stream_for_request_mode_with_surface_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body_source_id: NetworkBodySourceId,
    filtered_surface_url: Option<&str>,
    metadata: FetchResponseMetadata,
) -> v8::Local<'s, v8::Object> {
    let filter = metadata.response_type;
    let filtered_surface_url = (filter == FetchResponseFilter::OpaqueRedirect)
        .then_some(filtered_surface_url)
        .flatten();
    let obj = build_fetch_response_object_head(scope, &head, &metadata, filtered_surface_url);
    if request.has_null_body(head.status) {
        // Fetch nulls the internal body, including for filtered responses. Do
        // not register a body source: subsequent transport chunks and terminal
        // events then have no stream to enqueue into or retain bytes for.
        return finish_fetch_response_object_with_body_stream(scope, obj, &head, None, filter);
    }
    if !filtered_response_exposes_body(filter) {
        set_filtered_response_internal_body_from_pending_stream(scope, obj, body_source_id);
        return finish_fetch_response_object_with_body_stream(scope, obj, &head, None, filter);
    }
    let stream = pending_network_body_stream(scope, obj, body_source_id);
    finish_fetch_response_object_with_body_stream(scope, obj, &head, Some(stream), filter)
}

fn build_fetch_response_object_head<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    head: &moli_fetch::ResponseHead,
    metadata: &FetchResponseMetadata,
    filtered_surface_url: Option<&str>,
) -> v8::Local<'s, v8::Object> {
    let filter = metadata.response_type;
    let status = filtered_response_status(head, filter);
    let obj = FetchResponseHeadDeclaration::new(status as f64, (200..300).contains(&status))
        .bind(scope)
        .expect("Fetch Response head declaration should bind");
    set_response_metadata(scope, obj, metadata);
    if let Some(url) = filtered_surface_url {
        set_response_slot_string(scope, obj, RESPONSE_URL_SLOT, url);
    }
    if !filter.is_readable() {
        set_response_slot_value(
            scope,
            obj,
            RESPONSE_INTERNAL_STATUS_SLOT,
            v8::Number::new(scope, head.status as f64).into(),
        );
        set_response_slot_string(
            scope,
            obj,
            RESPONSE_INTERNAL_STATUS_TEXT_SLOT,
            head.status_text(),
        );
        let internal_headers = filter_headers_for_guard(
            &moli_fetch::headers_to_byte_strings(&head.headers),
            HeadersGuard::Response,
        );
        let internal_headers_obj =
            build_headers_object_with_state(scope, &internal_headers, HeadersGuard::Response, true);
        set_response_slot_value(
            scope,
            obj,
            RESPONSE_INTERNAL_HEADERS_SLOT,
            internal_headers_obj.into(),
        );
    }
    mark_response_object(scope, obj);
    obj
}

fn finish_fetch_response_object_with_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    head: &moli_fetch::ResponseHead,
    body_stream: Option<v8::Local<'s, v8::Object>>,
    filter: FetchResponseFilter,
) -> v8::Local<'s, v8::Object> {
    let header_entries = if filtered_response_exposes_headers(filter) {
        head.headers.as_slice()
    } else {
        &[][..]
    };
    let headers = filter_headers_for_guard(
        &moli_fetch::headers_to_byte_strings(header_entries),
        HeadersGuard::Response,
    );
    let headers_obj =
        build_headers_object_with_state(scope, &headers, HeadersGuard::Response, true);
    set_response_slot_value(scope, obj, RESPONSE_HEADERS_SLOT, headers_obj.into());

    let body_value = if !filtered_response_exposes_body(filter) {
        v8::null(scope).into()
    } else if let Some(stream) = body_stream {
        stream.into()
    } else {
        v8::null(scope).into()
    };
    FetchResponseBodyDeclaration::new(
        filtered_response_status_text(head, filter).to_owned(),
        body_value,
    )
    .initialize(scope, obj)
    .expect("Fetch Response body declaration should initialize");
    obj
}

pub(crate) fn build_filtered_cached_response_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    metadata: FetchResponseMetadata,
    body: Vec<u8>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !matches!(
        metadata.response_type,
        FetchResponseFilter::Opaque | FetchResponseFilter::OpaqueRedirect
    ) {
        return None;
    }
    let obj = FetchResponseHeadDeclaration::new(0.0, false)
        .bind(scope)
        .ok()?;
    set_response_metadata(scope, obj, &metadata);
    mark_response_object(scope, obj);
    let headers = filter_headers_for_guard(&[], HeadersGuard::Response);
    let headers_obj =
        build_headers_object_with_state(scope, &headers, HeadersGuard::Response, true);
    set_response_slot_value(scope, obj, RESPONSE_HEADERS_SLOT, headers_obj.into());
    set_filtered_response_internal_body_from_bytes(scope, obj, body);
    FetchResponseBodyDeclaration::new(String::new(), v8::null(scope).into())
        .initialize(scope, obj)
        .ok()?;
    Some(obj)
}

#[cfg(test)]
use moli_fetch_types::FetchResponse as MaterializedResponseObject;
pub(crate) use moli_fetch_types::FetchResponseHead as MaterializedResponseHead;

pub(crate) enum MaterializedResponseBody<'s> {
    Ready(Vec<u8>),
    Pending(v8::Local<'s, v8::Promise>),
    Failure(String),
}

#[cfg(test)]
pub(crate) fn materialize_response_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: &str,
) -> Result<MaterializedResponseObject, String> {
    let (head, response) = materialize_response_object_head(scope, value, context)?;
    let body_is_null = response_slot_value(scope, response, RESPONSE_BODY_SLOT)
        .is_some_and(|body| body.is_null_or_undefined());
    let body = match try_network_body_bytes_from_object(scope, response) {
        Ok(Some(body)) => body,
        Ok(None) if body_is_null => Vec::new(),
        Ok(None) => {
            return Err(format!("{context} requires a materialized Response body."));
        }
        Err(error) => return Err(error),
    };
    set_response_slot_bool(scope, response, RESPONSE_BODY_USED_SLOT, true);
    Ok(head.with_body(body))
}

pub(crate) fn materialize_response_object_head<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: &str,
) -> Result<(MaterializedResponseHead, v8::Local<'s, v8::Object>), String> {
    let response = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| format!("{context} requires a Response."))?;
    if !is_branded_response_object(scope, response) {
        return Err(format!("{context} requires a Response."));
    }
    let metadata = response_metadata(scope, response)
        .ok_or_else(|| format!("{context} requires a Response with native metadata."))?;
    if metadata.response_type == FetchResponseFilter::Error {
        return Err(format!("{context} rejected an error Response."));
    }
    if body_already_used(scope, response, RESPONSE_BODY_USED_SLOT) {
        return Err(format!(
            "{context} rejected a Response whose body is already used."
        ));
    }
    if response_body_locked(scope, response) {
        return Err(format!(
            "{context} rejected a Response whose body is locked."
        ));
    }
    let status = response_slot_value(scope, response, RESPONSE_STATUS_SLOT)
        .and_then(|value| value.number_value(scope))
        .ok_or_else(|| format!("{context} requires a Response."))?;
    let filtered_response_type = matches!(
        metadata.response_type,
        FetchResponseFilter::Opaque | FetchResponseFilter::OpaqueRedirect
    );
    let status_allowed =
        (200.0..=599.0).contains(&status) || filtered_response_type && status == 0.0;
    if !status_allowed {
        return Err(format!(
            "{context} requires a Response with status 200-599."
        ));
    }
    let status_text =
        response_slot_string(scope, response, RESPONSE_STATUS_TEXT_SLOT).unwrap_or_default();
    let headers = response_slot_object(scope, response, RESPONSE_HEADERS_SLOT)
        .map(|headers| {
            moli_fetch::headers_from_byte_strings(&headers_list(scope, headers))
                .expect("Headers contain ByteStrings")
        })
        .unwrap_or_default();

    Ok((
        MaterializedResponseHead {
            metadata,
            status: status as u16,
            status_text,
            headers,
        },
        response,
    ))
}

pub(crate) fn materialize_response_object_head_for_service_worker_respond_with<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: &str,
) -> Result<(MaterializedResponseHead, v8::Local<'s, v8::Object>), String> {
    let (mut head, response) = materialize_response_object_head(scope, value, context)?;
    if head.metadata.response_type == FetchResponseFilter::OpaqueRedirect
        && let Some(internal_status) =
            response_slot_number(scope, response, RESPONSE_INTERNAL_STATUS_SLOT)
    {
        head.status = internal_status as u16;
        head.status_text =
            response_slot_string(scope, response, RESPONSE_INTERNAL_STATUS_TEXT_SLOT)
                .unwrap_or_default();
        head.headers = response_slot_object(scope, response, RESPONSE_INTERNAL_HEADERS_SLOT)
            .map(|headers| {
                moli_fetch::headers_from_byte_strings(&headers_list(scope, headers))
                    .expect("Headers contain ByteStrings")
            })
            .unwrap_or_default();
    }
    Ok((head, response))
}

pub(crate) fn materialize_response_object_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    context: &str,
) -> MaterializedResponseBody<'s> {
    match materialize_response_object_body_preserving_error(scope, response, context) {
        Ok(body) => body,
        Err(error) => MaterializedResponseBody::Failure(format!(
            "{context} failed to materialize Response body: {}",
            js_value_string(scope, error)
        )),
    }
}

/// Cache's ReadAllBytes rejection preserves the stream's original reason.
/// In particular an AbortError or author sentinel must not be stringified.
pub(crate) fn materialize_response_object_body_preserving_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    context: &str,
) -> Result<MaterializedResponseBody<'s>, v8::Local<'s, v8::Value>> {
    set_response_slot_bool(scope, response, RESPONSE_BODY_USED_SLOT, true);
    let consumption = consume_filtered_response_internal_body_value_from_object(
        scope,
        response,
        NetworkBodyConsumptionKind::Bytes,
    )
    .unwrap_or_else(|| {
        consume_network_body_value_from_object(scope, response, NetworkBodyConsumptionKind::Bytes)
    });
    Ok(match consumption {
        NetworkBodyConsumption::Ready(value) => {
            match materialized_body_bytes_from_value(scope, value) {
                Ok(bytes) => MaterializedResponseBody::Ready(bytes),
                Err(error) => MaterializedResponseBody::Failure(format!("{context} {error}")),
            }
        }
        NetworkBodyConsumption::Rejected(error) => return Err(error),
        NetworkBodyConsumption::Pending(promise) => MaterializedResponseBody::Pending(promise),
        NetworkBodyConsumption::Failed => MaterializedResponseBody::Failure(format!(
            "{context} requires a materialized Response body."
        )),
    })
}

pub(crate) fn materialize_response_object_body_with_chunk_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    context: &str,
    chunk_callback: v8::Local<'s, v8::Function>,
) -> (MaterializedResponseBody<'s>, Option<v8::Global<v8::Object>>) {
    set_response_slot_bool(scope, response, RESPONSE_BODY_USED_SLOT, true);
    let (consumption, stream_cancel_handle) = if let Some(consumption) =
        consume_filtered_response_internal_body_value_from_object_with_chunk_callback(
            scope,
            response,
            NetworkBodyConsumptionKind::Bytes,
            chunk_callback,
        ) {
        consumption
    } else {
        consume_network_body_value_from_object_with_chunk_callback(
            scope,
            response,
            NetworkBodyConsumptionKind::Bytes,
            chunk_callback,
        )
    };
    let body = match consumption {
        NetworkBodyConsumption::Ready(value) => {
            match materialized_body_bytes_from_value(scope, value) {
                Ok(bytes) => MaterializedResponseBody::Ready(bytes),
                Err(error) => MaterializedResponseBody::Failure(format!("{context} {error}")),
            }
        }
        NetworkBodyConsumption::Rejected(error) => MaterializedResponseBody::Failure(format!(
            "{context} failed to materialize Response body: {}",
            js_value_string(scope, error)
        )),
        NetworkBodyConsumption::Pending(promise) => MaterializedResponseBody::Pending(promise),
        NetworkBodyConsumption::Failed => MaterializedResponseBody::Failure(format!(
            "{context} requires a materialized Response body."
        )),
    };
    (body, stream_cancel_handle)
}

pub(crate) fn materialized_body_bytes_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<u8>, String> {
    crate::blob::buffer_source_bytes_from_value(scope, value)
        .ok_or_else(|| "failed to materialize Response body bytes.".to_owned())
}

fn js_value_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> String {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown rejection".to_owned())
}

fn response_body_locked<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(body) = response_slot_value(scope, response, RESPONSE_BODY_SLOT) else {
        return false;
    };
    if body.is_null_or_undefined() {
        return false;
    }
    let Ok(stream) = v8::Local::<v8::Object>::try_from(body) else {
        return false;
    };
    if !web_api_interfaces::ReadableStream::is_instance(scope, stream) {
        return false;
    }
    stream
        .get(scope, crate::util::v8str(scope, "locked").into())
        .is_some_and(move |value| value.boolean_value(scope))
}
