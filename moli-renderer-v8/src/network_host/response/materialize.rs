use super::super::fetch_surface::{
    RESPONSE_BODY_USED_SLOT, RESPONSE_INTERNAL_HEADERS_SLOT, RESPONSE_INTERNAL_STATUS_SLOT,
    RESPONSE_INTERNAL_STATUS_TEXT_SLOT, RESPONSE_INTERNAL_URL_SLOT, mark_response_object,
    response_slot_number,
};
use super::*;
use crate::types::NetworkBodySourceId;
use crate::web_api_interfaces;
use moli_fetch::RequestMode;
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

fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FetchResponseFilter {
    None,
    Opaque,
    OpaqueRedirect,
}

impl From<crate::types::AsyncSubresourceFetchResponseFilter> for FetchResponseFilter {
    fn from(value: crate::types::AsyncSubresourceFetchResponseFilter) -> Self {
        match value {
            crate::types::AsyncSubresourceFetchResponseFilter::Opaque => Self::Opaque,
            crate::types::AsyncSubresourceFetchResponseFilter::OpaqueRedirect => {
                Self::OpaqueRedirect
            }
        }
    }
}

#[derive(WebApiObject)]
#[webapi(plain, prototype = "Response")]
struct FetchResponseHeadDeclaration {
    #[webapi(slot = RESPONSE_STATUS_SLOT)]
    status: f64,
    #[webapi(slot = RESPONSE_OK_SLOT)]
    ok: bool,
    #[webapi(slot = RESPONSE_URL_SLOT)]
    url: String,
    #[webapi(slot = RESPONSE_REDIRECTED_SLOT)]
    redirected: bool,
    #[webapi(slot = RESPONSE_TYPE_SLOT)]
    r#type: &'static str,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct FetchResponseInternalUrlDeclaration {
    #[webapi(slot = RESPONSE_INTERNAL_URL_SLOT)]
    internal_url: String,
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

fn response_filter(
    document_url: &url::Url,
    head: &moli_fetch::ResponseHead,
    request_mode: RequestMode,
) -> FetchResponseFilter {
    if is_redirect_status(head.status) {
        FetchResponseFilter::OpaqueRedirect
    } else if request_mode == RequestMode::NoCors && response_crosses_origin(document_url, head) {
        FetchResponseFilter::Opaque
    } else {
        FetchResponseFilter::None
    }
}

fn response_crosses_origin(document_url: &url::Url, head: &moli_fetch::ResponseHead) -> bool {
    !moli_url::same_origin(document_url, &head.final_url)
        || head.redirect_chain.iter().any(|redirect| {
            !moli_url::same_origin(document_url, &redirect.from_url)
                || !moli_url::same_origin(document_url, &redirect.to_url)
        })
}

fn compute_fetch_response_type(
    document_url: &url::Url,
    head: &moli_fetch::ResponseHead,
    filter: FetchResponseFilter,
) -> &'static str {
    // Crossing an origin taints the response even if a later redirect returns
    // to the client's origin. The final URL alone cannot restore a basic response.
    match filter {
        FetchResponseFilter::Opaque => "opaque",
        FetchResponseFilter::OpaqueRedirect => "opaqueredirect",
        FetchResponseFilter::None if !response_crosses_origin(document_url, head) => "basic",
        FetchResponseFilter::None => "cors",
    }
}

fn filtered_response_status(head: &moli_fetch::ResponseHead, filter: FetchResponseFilter) -> u16 {
    if filter == FetchResponseFilter::None {
        head.status
    } else {
        0
    }
}

fn filtered_response_url(head: &moli_fetch::ResponseHead, filter: FetchResponseFilter) -> &str {
    match filter {
        FetchResponseFilter::Opaque | FetchResponseFilter::OpaqueRedirect => "",
        FetchResponseFilter::None => head.final_url.as_str(),
    }
}

fn filtered_response_exposes_body(filter: FetchResponseFilter) -> bool {
    filter == FetchResponseFilter::None
}

fn filtered_response_exposes_redirected(filter: FetchResponseFilter) -> bool {
    filter == FetchResponseFilter::None
}

fn filtered_response_exposes_headers(filter: FetchResponseFilter) -> bool {
    filter == FetchResponseFilter::None
}

fn filtered_response_status_text(
    head: &moli_fetch::ResponseHead,
    filter: FetchResponseFilter,
) -> &'static str {
    if filter == FetchResponseFilter::None {
        http_status_text(head.status)
    } else {
        ""
    }
}

fn legacy_fetch_response_type(document_url: &url::Url, response_url: &url::Url) -> &'static str {
    if moli_url::same_origin(document_url, response_url) {
        "basic"
    } else {
        "cors"
    }
}

pub(crate) fn build_fetch_response_object_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: &url::Url,
    request: FetchResponseRequest<'_>,
    response: Response,
) -> v8::Local<'s, v8::Object> {
    let (head, body) = response.into_body();
    build_fetch_response_object_from_body_source_for_request_mode(
        scope,
        document_url,
        request,
        head,
        body,
    )
}

pub(crate) fn build_fetch_response_object_from_body_source_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: &url::Url,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
) -> v8::Local<'s, v8::Object> {
    build_fetch_response_object_from_body_source_for_request_mode_with_filter(
        scope,
        document_url,
        request,
        head,
        body,
        None,
    )
}

pub(crate) fn build_fetch_response_object_from_body_source_for_request_mode_with_filter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: &url::Url,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body: moli_fetch::ResponseBody,
    filter_override: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
) -> v8::Local<'s, v8::Object> {
    let filter = filter_override
        .map(FetchResponseFilter::from)
        .unwrap_or_else(|| response_filter(document_url, &head, request.mode));
    let obj = build_fetch_response_object_head(scope, document_url, &head, filter, None);
    let body_stream = if request.has_null_body(head.status) {
        None
    } else if filtered_response_exposes_body(filter) {
        network_body_stream_from_response_body(scope, obj, body)
    } else {
        set_filtered_response_internal_body_from_response_body(scope, obj, body);
        None
    };
    finish_fetch_response_object_with_body_stream(scope, obj, &head, body_stream)
}

pub(crate) fn build_fetch_response_object_from_subresource_body_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: &url::Url,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body: crate::protocol_types::SubresourceResponseBody,
) -> v8::Local<'s, v8::Object> {
    let filter = response_filter(document_url, &head, request.mode);
    let obj = build_fetch_response_object_head(scope, document_url, &head, filter, None);
    let body_stream = if request.has_null_body(head.status) {
        None
    } else if filtered_response_exposes_body(filter) {
        Some(network_body_stream_from_subresource_body(scope, obj, body))
    } else {
        set_filtered_response_internal_body_from_subresource_body(scope, obj, body);
        None
    };
    finish_fetch_response_object_with_body_stream(scope, obj, &head, body_stream)
}

pub(crate) fn build_fetch_response_object_from_stream_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: &url::Url,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body_source_id: NetworkBodySourceId,
) -> v8::Local<'s, v8::Object> {
    build_fetch_response_object_from_stream_for_request_mode_with_surface_url(
        scope,
        document_url,
        request,
        head,
        body_source_id,
        None,
    )
}

pub(crate) fn build_navigation_preload_response_object_from_stream_for_request_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request_url: &url::Url,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body_source_id: NetworkBodySourceId,
) -> v8::Local<'s, v8::Object> {
    build_fetch_response_object_from_stream_for_request_mode_with_surface_url(
        scope,
        request_url,
        request,
        head,
        body_source_id,
        Some(request_url.as_str()),
    )
}

fn build_fetch_response_object_from_stream_for_request_mode_with_surface_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: &url::Url,
    request: FetchResponseRequest<'_>,
    head: moli_fetch::ResponseHead,
    body_source_id: NetworkBodySourceId,
    filtered_surface_url: Option<&str>,
) -> v8::Local<'s, v8::Object> {
    let filter = response_filter(document_url, &head, request.mode);
    let filtered_surface_url = (filter == FetchResponseFilter::OpaqueRedirect)
        .then_some(filtered_surface_url)
        .flatten();
    let obj =
        build_fetch_response_object_head(scope, document_url, &head, filter, filtered_surface_url);
    if request.has_null_body(head.status) {
        // Fetch nulls the internal body, including for filtered responses. Do
        // not register a body source: subsequent transport chunks and terminal
        // events then have no stream to enqueue into or retain bytes for.
        return finish_fetch_response_object_with_body_stream(scope, obj, &head, None);
    }
    if !filtered_response_exposes_body(filter) {
        set_filtered_response_internal_body_from_pending_stream(scope, obj, body_source_id);
        return finish_fetch_response_object_with_body_stream(scope, obj, &head, None);
    }
    let stream = pending_network_body_stream(scope, obj, body_source_id);
    finish_fetch_response_object_with_body_stream(scope, obj, &head, Some(stream))
}

fn build_fetch_response_object_head<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_url: &url::Url,
    head: &moli_fetch::ResponseHead,
    filter: FetchResponseFilter,
    filtered_surface_url: Option<&str>,
) -> v8::Local<'s, v8::Object> {
    let status = filtered_response_status(head, filter);
    let response_type = compute_fetch_response_type(document_url, head, filter);
    let obj = FetchResponseHeadDeclaration::new(
        status as f64,
        (200..300).contains(&status),
        filtered_surface_url
            .unwrap_or_else(|| filtered_response_url(head, filter))
            .to_owned(),
        filtered_response_exposes_redirected(filter) && head.redirected,
        response_type,
    )
    .bind(scope)
    .expect("Fetch Response head declaration should bind");
    FetchResponseInternalUrlDeclaration::new(head.final_url.to_string())
        .initialize(scope, obj)
        .expect("Fetch Response internal URL declaration should initialize");
    if filter != FetchResponseFilter::None {
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
            http_status_text(head.status),
        );
        let internal_headers = filter_headers_for_guard(&head.headers, HeadersGuard::Response);
        let internal_headers_obj =
            build_headers_object_with_state(scope, &internal_headers, HeadersGuard::Response, true);
        install_headers_object_methods(scope, internal_headers_obj);
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
) -> v8::Local<'s, v8::Object> {
    let response_type = response_slot_string(scope, obj, RESPONSE_TYPE_SLOT)
        .unwrap_or_else(|| legacy_fetch_response_type(&head.final_url, &head.final_url).to_owned());
    let filter = match response_type.as_str() {
        "opaque" => FetchResponseFilter::Opaque,
        "opaqueredirect" => FetchResponseFilter::OpaqueRedirect,
        _ => FetchResponseFilter::None,
    };
    let header_entries = if filtered_response_exposes_headers(filter) {
        head.headers.as_slice()
    } else {
        &[][..]
    };
    let headers = filter_headers_for_guard(header_entries, HeadersGuard::Response);
    let headers_obj =
        build_headers_object_with_state(scope, &headers, HeadersGuard::Response, true);
    install_headers_object_methods(scope, headers_obj);
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
    response_type: &str,
    internal_url: &str,
    body: Vec<u8>,
) -> Option<v8::Local<'s, v8::Object>> {
    let response_type = match response_type {
        "opaque" => "opaque",
        "opaqueredirect" => "opaqueredirect",
        _ => return None,
    };
    let obj = FetchResponseHeadDeclaration::new(0.0, false, String::new(), false, response_type)
        .bind(scope)
        .ok()?;
    if !internal_url.is_empty() {
        FetchResponseInternalUrlDeclaration::new(internal_url.to_owned())
            .initialize(scope, obj)
            .ok()?;
    }
    mark_response_object(scope, obj);

    let headers = filter_headers_for_guard(&[], HeadersGuard::Response);
    let headers_obj =
        build_headers_object_with_state(scope, &headers, HeadersGuard::Response, true);
    install_headers_object_methods(scope, headers_obj);
    set_response_slot_value(scope, obj, RESPONSE_HEADERS_SLOT, headers_obj.into());
    set_filtered_response_internal_body_from_bytes(scope, obj, body);
    FetchResponseBodyDeclaration::new(String::new(), v8::null(scope).into())
        .initialize(scope, obj)
        .ok()?;
    Some(obj)
}

#[derive(Debug, Clone)]
pub(crate) struct MaterializedResponseObject {
    pub(crate) final_url: Option<url::Url>,
    pub(crate) response_type: String,
    pub(crate) redirected: bool,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct MaterializedResponseHead {
    pub(crate) final_url: Option<url::Url>,
    pub(crate) response_type: String,
    pub(crate) redirected: bool,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
}

impl MaterializedResponseHead {
    pub(crate) fn with_body(self, body: Vec<u8>) -> MaterializedResponseObject {
        MaterializedResponseObject {
            final_url: self.final_url,
            response_type: self.response_type,
            redirected: self.redirected,
            status: self.status,
            status_text: self.status_text,
            headers: self.headers,
            body,
        }
    }
}

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
    if let Some(stream) = body_stream_object(scope, response) {
        crate::context_bootstrap::begin_readable_stream_body_consumption(scope, stream);
    }
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
    let response_type = response_slot_string(scope, response, RESPONSE_TYPE_SLOT)
        .unwrap_or_else(|| "default".to_owned());
    if response_type == "error" {
        return Err(format!("{context} rejected an error Response."));
    }
    if body_is_used(scope, response) {
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
    let filtered_response_type = matches!(response_type.as_str(), "opaque" | "opaqueredirect");
    let status_allowed =
        (200.0..=599.0).contains(&status) || filtered_response_type && status == 0.0;
    if !status_allowed {
        return Err(format!(
            "{context} requires a Response with status 200-599."
        ));
    }
    let response_url = response_slot_string(scope, response, RESPONSE_URL_SLOT).unwrap_or_default();
    let internal_response_url =
        response_slot_string(scope, response, RESPONSE_INTERNAL_URL_SLOT).unwrap_or_default();
    let materialized_url = if filtered_response_type && !internal_response_url.is_empty() {
        internal_response_url
    } else {
        response_url
    };
    let final_url = if materialized_url.is_empty() {
        None
    } else {
        Some(
            url::Url::parse(&materialized_url)
                .map_err(|_| format!("{context} Response has an invalid URL."))?,
        )
    };
    let status_text =
        response_slot_string(scope, response, RESPONSE_STATUS_TEXT_SLOT).unwrap_or_default();
    let headers = response_slot_object(scope, response, RESPONSE_HEADERS_SLOT)
        .map(|headers| headers_entries(scope, headers))
        .unwrap_or_default();

    Ok((
        MaterializedResponseHead {
            final_url,
            response_type,
            redirected: response_slot_bool(scope, response, RESPONSE_REDIRECTED_SLOT),
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
    if head.response_type == "opaqueredirect"
        && let Some(internal_status) =
            response_slot_number(scope, response, RESPONSE_INTERNAL_STATUS_SLOT)
    {
        head.status = internal_status as u16;
        head.status_text =
            response_slot_string(scope, response, RESPONSE_INTERNAL_STATUS_TEXT_SLOT)
                .unwrap_or_default();
        head.headers = response_slot_object(scope, response, RESPONSE_INTERNAL_HEADERS_SLOT)
            .map(|headers| headers_entries(scope, headers))
            .unwrap_or_default();
    }
    Ok((head, response))
}

pub(crate) fn materialize_response_object_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    context: &str,
) -> MaterializedResponseBody<'s> {
    set_response_slot_bool(scope, response, RESPONSE_BODY_USED_SLOT, true);
    let consumption = consume_filtered_response_internal_body_value_from_object(
        scope,
        response,
        NetworkBodyConsumptionKind::Bytes,
    )
    .unwrap_or_else(|| {
        consume_network_body_value_from_object(scope, response, NetworkBodyConsumptionKind::Bytes)
    });
    match consumption {
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
    }
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
    body_stream_object(scope, response)
        .is_some_and(|stream| crate::context_bootstrap::readable_stream_locked(scope, stream))
}
