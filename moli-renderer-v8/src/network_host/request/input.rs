use super::super::headers::HeadersGuard;
use super::*;
use crate::web_api_interfaces;
use crate::webidl;

pub(in crate::network_host) fn normalize_request_method(
    method: &str,
) -> Result<String, &'static str> {
    if method.is_empty() {
        return Ok("GET".to_owned());
    }
    let normalized = method.to_ascii_uppercase();
    if matches!(normalized.as_str(), "CONNECT" | "TRACE" | "TRACK") {
        return Err("Request method is forbidden");
    }
    if matches!(
        normalized.as_str(),
        "DELETE" | "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT"
    ) {
        Ok(normalized)
    } else {
        Ok(method.to_owned())
    }
}

pub(super) fn request_method_allows_body(method: &str) -> bool {
    !matches!(method, "GET" | "HEAD")
}

pub(super) fn request_headers_guard_for_mode(mode: &str) -> HeadersGuard {
    if mode == "no-cors" {
        HeadersGuard::RequestNoCors
    } else {
        HeadersGuard::Request
    }
}

pub(crate) struct RequestInputSnapshot {
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) mode: String,
    pub(crate) cache: String,
    pub(crate) credentials: String,
    pub(crate) redirect: String,
    pub(crate) referrer: String,
    pub(crate) referrer_policy: String,
    pub(crate) integrity: String,
    pub(crate) keepalive: bool,
    pub(crate) priority: moli_fetch::FetchPriorityHint,
    pub(crate) duplex: String,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) body_unusable: bool,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) signal: Option<v8::Global<v8::Object>>,
}

pub(super) fn request_input_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<String, webidl::WebIdlError> {
    webidl::convert::<webidl::UsvString>(scope, value, webidl::Context::argument("Request", 1))
        .map(Into::into)
}

pub(crate) fn request_input_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<RequestInputSnapshot>, webidl::WebIdlError> {
    request_input_snapshot_inner(scope, value, false)
}

pub(crate) fn mark_request_input_body_used_for_fetch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    if !is_branded_request_object(scope, object) {
        return;
    }
    let has_body = request_slot_value(scope, object, REQUEST_BODY_SLOT)
        .is_some_and(|body| !body.is_null_or_undefined());
    if has_body {
        set_request_slot_bool(scope, object, REQUEST_BODY_USED_SLOT, true);
    }
}

pub(super) fn request_input_snapshot_for_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<RequestInputSnapshot>, webidl::WebIdlError> {
    request_input_snapshot_inner(scope, value, true)
}

fn request_input_snapshot_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    allow_unusable_body: bool,
) -> Result<Option<RequestInputSnapshot>, webidl::WebIdlError> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(None);
    };
    if is_branded_request_object(scope, object) {
        return request_input_snapshot_from_private_slots(scope, object, allow_unusable_body)
            .map(Some);
    }
    let Some(url) = defined_object_string_property(scope, object, "url") else {
        return Ok(None);
    };
    let method = defined_object_string_property(scope, object, "method")
        .map(|value| {
            normalize_request_method(&value)
                .map_err(|_| webidl::WebIdlError::custom_message("Request method is forbidden"))
        })
        .transpose()?
        .unwrap_or_else(|| "GET".to_owned());
    let mode =
        defined_object_string_property(scope, object, "mode").unwrap_or_else(|| "cors".to_owned());
    let cache = defined_object_string_property(scope, object, "cache")
        .unwrap_or_else(|| "default".to_owned());
    let credentials = defined_object_string_property(scope, object, "credentials")
        .unwrap_or_else(|| "same-origin".to_owned());
    let redirect = defined_object_string_property(scope, object, "redirect")
        .unwrap_or_else(|| "follow".to_owned());
    let referrer = defined_object_string_property(scope, object, "referrer")
        .unwrap_or_else(|| "about:client".to_owned());
    let referrer_policy =
        defined_object_string_property(scope, object, "referrerPolicy").unwrap_or_default();
    let integrity = defined_object_string_property(scope, object, "integrity").unwrap_or_default();
    let keepalive = object_bool_property(scope, object, "keepalive").unwrap_or(false);
    let priority = defined_object_string_property(scope, object, REQUEST_PRIORITY_SLOT)
        .and_then(|value| value.parse().ok())
        .unwrap_or_default();
    let duplex = defined_object_string_property(scope, object, "duplex")
        .unwrap_or_else(|| "half".to_owned());
    let body = try_network_body_bytes_from_object(scope, object)
        .map_err(|_| webidl::WebIdlError::custom_message("Failed to materialize request body"))?;
    let headers = webidl::property_result(
        scope,
        object,
        "headers",
        webidl::Context::member("Request", "headers"),
    )?
    .map(|value| headers_entries_from_init(scope, value))
    .transpose()?
    .unwrap_or_default();
    let signal = request_signal_snapshot_from_property(scope, object)?;
    Ok(Some(RequestInputSnapshot {
        url,
        method,
        mode,
        cache,
        credentials,
        redirect,
        referrer,
        referrer_policy,
        integrity,
        keepalive,
        priority,
        duplex,
        body,
        body_unusable: false,
        headers,
        signal,
    }))
}

fn request_input_snapshot_from_private_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    allow_unusable_body: bool,
) -> Result<RequestInputSnapshot, webidl::WebIdlError> {
    let url = request_slot_string(scope, object, REQUEST_URL_SLOT).unwrap_or_default();
    let method = request_slot_string(scope, object, REQUEST_METHOD_SLOT)
        .map(|value| {
            normalize_request_method(&value)
                .map_err(|_| webidl::WebIdlError::custom_message("Request method is forbidden"))
        })
        .transpose()?
        .unwrap_or_else(|| "GET".to_owned());
    let mode =
        request_slot_string(scope, object, REQUEST_MODE_SLOT).unwrap_or_else(|| "cors".to_owned());
    let cache = request_slot_string(scope, object, REQUEST_CACHE_SLOT)
        .unwrap_or_else(|| "default".to_owned());
    let credentials = request_slot_string(scope, object, REQUEST_CREDENTIALS_SLOT)
        .unwrap_or_else(|| "same-origin".to_owned());
    let redirect = request_slot_string(scope, object, REQUEST_REDIRECT_SLOT)
        .unwrap_or_else(|| "follow".to_owned());
    let referrer = request_slot_string(scope, object, REQUEST_REFERRER_SLOT)
        .unwrap_or_else(|| "about:client".to_owned());
    let referrer_policy =
        request_slot_string(scope, object, REQUEST_REFERRER_POLICY_SLOT).unwrap_or_default();
    let integrity = request_slot_string(scope, object, REQUEST_INTEGRITY_SLOT).unwrap_or_default();
    let keepalive = request_slot_bool(scope, object, REQUEST_KEEPALIVE_SLOT);
    let priority = request_slot_string(scope, object, REQUEST_PRIORITY_SLOT)
        .and_then(|value| value.parse().ok())
        .unwrap_or_default();
    let duplex = request_slot_string(scope, object, REQUEST_DUPLEX_SLOT)
        .unwrap_or_else(|| "half".to_owned());
    let body_unusable = request_body_unusable(scope, object);
    if body_unusable && !allow_unusable_body {
        return Err(request_body_already_used_error());
    }
    let body = if body_unusable {
        None
    } else {
        try_network_body_bytes_from_object(scope, object).map_err(|_| {
            webidl::WebIdlError::custom_message("Failed to materialize request body")
        })?
    };
    let headers = webidl::property_result(
        scope,
        object,
        "headers",
        webidl::Context::member("Request", "headers"),
    )?
    .map(|value| headers_entries_from_init(scope, value))
    .transpose()?
    .unwrap_or_default();
    let signal = request_slot_value(scope, object, REQUEST_SIGNAL_SLOT)
        .map(|value| request_signal_snapshot_from_value(scope, value))
        .transpose()?
        .flatten();
    Ok(RequestInputSnapshot {
        url,
        method,
        mode,
        cache,
        credentials,
        redirect,
        referrer,
        referrer_policy,
        integrity,
        keepalive,
        priority,
        duplex,
        body,
        body_unusable,
        headers,
        signal,
    })
}

pub(super) fn request_body_already_used_error() -> webidl::WebIdlError {
    webidl::WebIdlError::custom_message(
        "Cannot construct a Request with a Request object that has already been used.",
    )
}

fn request_body_unusable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(body) = request_slot_value(scope, object, REQUEST_BODY_SLOT) else {
        return false;
    };
    if body.is_null_or_undefined() {
        return false;
    }
    request_slot_bool(scope, object, REQUEST_BODY_USED_SLOT)
        || request_body_stream_locked(scope, body)
}

fn request_body_stream_locked<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    body: v8::Local<'s, v8::Value>,
) -> bool {
    let Ok(stream) = v8::Local::<v8::Object>::try_from(body) else {
        return false;
    };
    if !web_api_interfaces::ReadableStream::is_instance(scope, stream) {
        return false;
    }
    stream
        .get(scope, v8str(scope, "locked").into())
        .map(|value| value.boolean_value(scope))
        .unwrap_or(false)
}

fn request_signal_snapshot_from_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<v8::Global<v8::Object>>, webidl::WebIdlError> {
    webidl::property_result(
        scope,
        object,
        "signal",
        webidl::Context::member("Request", "signal"),
    )?
    .map(|value| request_signal_snapshot_from_value(scope, value))
    .transpose()
    .map(Option::flatten)
}

pub(super) fn request_signal_snapshot_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<v8::Global<v8::Object>>, webidl::WebIdlError> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let Ok(signal) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(webidl::WebIdlError::custom_message(
            "Request signal must be an AbortSignal",
        ));
    };
    Ok(Some(v8::Global::new(scope, signal)))
}

fn object_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<bool> {
    let key = v8_string(scope, key)?;
    object
        .get(scope, key.into())
        .map(|value| value.boolean_value(scope))
}

pub(super) fn resolve_request_constructor_url(
    scope: &mut v8::PinScope<'_, '_>,
    input: &str,
) -> String {
    try_resolve_request_constructor_url(scope, input).unwrap_or_else(|_| input.to_owned())
}

pub(crate) fn try_resolve_request_constructor_url(
    scope: &mut v8::PinScope<'_, '_>,
    input: &str,
) -> Result<String, String> {
    try_resolve_request_constructor_url_for_scope(scope, input, None, None)
}

pub(crate) fn try_resolve_request_constructor_url_for_child(
    scope: &mut v8::PinScope<'_, '_>,
    input: &str,
    child_handle: Option<crate::document_runtime::DomHandle>,
) -> Result<String, String> {
    try_resolve_request_constructor_url_for_scope(scope, input, child_handle, None)
}

pub(crate) fn try_resolve_request_constructor_url_for_base(
    scope: &mut v8::PinScope<'_, '_>,
    input: &str,
    base_url: Option<url::Url>,
) -> Result<String, String> {
    try_resolve_request_constructor_url_for_scope(scope, input, None, base_url)
}

pub(crate) fn try_resolve_request_constructor_url_for_scope(
    scope: &mut v8::PinScope<'_, '_>,
    input: &str,
    child_handle: Option<crate::document_runtime::DomHandle>,
    base_url: Option<url::Url>,
) -> Result<String, String> {
    if url::Url::parse(input).is_ok() {
        return Ok(input.to_owned());
    }
    if let Some(base_url) = base_url {
        return resolve_context_url(&base_url, input, None).map(|url| url.to_string());
    }
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        let document_url = child_handle
            .and_then(|handle| {
                host.child_browsing_context_request_scope(handle)
                    .map(|(_, url)| url)
            })
            .unwrap_or_else(|| {
                let (_, document_url, _) = effective_subresource_request_scope(scope, host, None);
                document_url
            });
        resolve_context_url(&document_url, input, None).map(|url| url.to_string())
    } else if let Some(worker_url) = crate::context_bootstrap::current_worker_script_url(scope) {
        if worker_url.scheme() == "blob" {
            return Err(format!("Failed to parse URL from {input}"));
        }
        resolve_context_url(&worker_url, input, None).map(|url| url.to_string())
    } else {
        Ok(input.to_owned())
    }
}

pub(super) fn normalize_request_referrer(scope: &mut v8::PinScope<'_, '_>, input: &str) -> String {
    if input.is_empty() || input == "about:client" {
        return input.to_owned();
    }

    let resolved = resolve_request_constructor_url(scope, input);
    let Some(context_url) = current_request_context_url(scope) else {
        return resolved;
    };
    if moli_url::parsed_same_origin(&resolved, &context_url) {
        resolved
    } else {
        "about:client".to_owned()
    }
}

fn current_request_context_url(scope: &mut v8::PinScope<'_, '_>) -> Option<String> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        let (_, document_url, _) = effective_subresource_request_scope(scope, host, None);
        Some(document_url.to_string())
    } else {
        crate::context_bootstrap::current_worker_script_url(scope).map(|url| url.to_string())
    }
}
