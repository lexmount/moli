use super::super::init::RequestInitMembers;
use super::super::input::{
    normalize_request_method, normalize_request_referrer, request_body_already_used_error,
    request_headers_guard_for_mode, request_method_allows_body,
    try_resolve_request_constructor_url_for_scope,
};
use super::super::*;
use crate::webidl;
use moli_fetch::{RequestMode, RequestRedirectMode};

pub(super) struct RequestConstructionState {
    pub(super) url_resolved: String,
    pub(super) method: String,
    pub(super) mode: String,
    pub(super) cache: String,
    pub(super) credentials: String,
    pub(super) redirect_mode: RequestRedirectMode,
    pub(super) referrer: String,
    pub(super) referrer_policy: String,
    pub(super) integrity: String,
    pub(super) keepalive: bool,
    pub(super) is_history_navigation: bool,
    pub(super) is_reload_navigation: bool,
    pub(super) priority: moli_fetch::FetchPriorityHint,
    pub(super) duplex: String,
    pub(super) body: Option<bytes::Bytes>,
    pub(super) inherited_body_unusable: bool,
    pub(super) body_content_type: Option<String>,
    pub(super) headers: Vec<(String, String)>,
    pub(super) signal: Option<v8::Global<v8::Object>>,
}

pub(super) fn request_initial_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: super::super::input::RequestInfo<'s>,
    child_handle: Option<crate::document_runtime::DomHandle>,
    base_url: Option<url::Url>,
) -> Result<RequestConstructionState, webidl::WebIdlError> {
    let (url_str, inherited, _) = input.materialize(scope)?;
    let url_resolved =
        try_resolve_request_constructor_url_for_scope(scope, &url_str, child_handle, base_url)
            .map_err(|_| {
                webidl::WebIdlError::custom_message("Failed to construct 'Request': invalid URL")
            })?;
    let method = inherited
        .as_ref()
        .map(|snapshot| snapshot.method.clone())
        .unwrap_or_else(|| "GET".to_owned());
    let mode = inherited
        .as_ref()
        .map(|snapshot| snapshot.mode.clone())
        .unwrap_or_else(|| "cors".to_owned());
    let cache = inherited
        .as_ref()
        .map(|snapshot| snapshot.cache.clone())
        .unwrap_or_else(|| "default".to_owned());
    let credentials = inherited
        .as_ref()
        .map(|snapshot| snapshot.credentials.clone())
        .unwrap_or_else(|| "same-origin".to_owned());
    let redirect_mode = inherited
        .as_ref()
        .and_then(|snapshot| parse_request_redirect_mode_label(&snapshot.redirect))
        .unwrap_or(RequestRedirectMode::Follow);
    let referrer = inherited
        .as_ref()
        .map(|snapshot| snapshot.referrer.clone())
        .unwrap_or_else(|| "about:client".to_owned());
    let referrer_policy = inherited
        .as_ref()
        .map(|snapshot| snapshot.referrer_policy.clone())
        .unwrap_or_default();
    let integrity = inherited
        .as_ref()
        .map(|snapshot| snapshot.integrity.clone())
        .unwrap_or_default();
    let keepalive = inherited
        .as_ref()
        .map(|snapshot| snapshot.keepalive)
        .unwrap_or(false);
    let is_history_navigation = inherited
        .as_ref()
        .is_some_and(|snapshot| snapshot.is_history_navigation);
    let is_reload_navigation = inherited
        .as_ref()
        .is_some_and(|snapshot| snapshot.is_reload_navigation);
    let priority = inherited
        .as_ref()
        .map(|snapshot| snapshot.priority)
        .unwrap_or_default();
    let duplex = inherited
        .as_ref()
        .map(|snapshot| snapshot.duplex.clone())
        .unwrap_or_else(|| "half".to_owned());
    let body = inherited
        .as_ref()
        .and_then(|snapshot| snapshot.body.clone());
    let inherited_body_unusable = inherited
        .as_ref()
        .is_some_and(|snapshot| snapshot.body_unusable);
    let headers = inherited
        .as_ref()
        .map(|snapshot| snapshot.headers.clone())
        .unwrap_or_default();
    let signal = inherited.and_then(|snapshot| snapshot.signal);

    Ok(RequestConstructionState {
        url_resolved,
        method,
        mode,
        cache,
        credentials,
        redirect_mode,
        referrer,
        referrer_policy,
        integrity,
        keepalive,
        is_history_navigation,
        is_reload_navigation,
        priority,
        duplex,
        body,
        inherited_body_unusable,
        body_content_type: None,
        headers,
        signal,
    })
}

pub(super) fn apply_request_init_overrides<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: RequestInitMembers<'s>,
    state: &mut RequestConstructionState,
) -> Result<(), webidl::WebIdlError> {
    if parsed.has_any_member() {
        if state.mode == "navigate" {
            state.mode = "same-origin".to_owned();
        }
        state.is_history_navigation = false;
        state.is_reload_navigation = false;
        state.referrer = "about:client".to_owned();
        state.referrer_policy.clear();
    }
    let method_overridden = parsed.method.is_some();
    if let Some(method) = parsed.method {
        state.method = normalize_request_method(&method)?;
    }
    let init_body_is_readable_stream = parsed
        .body
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .is_some_and(|object| {
            crate::web_api_interfaces::ReadableStream::is_instance(scope, object)
        });
    let body_from_init = parsed
        .body
        .map(|body| body_init(scope, body, webidl::Context::member("RequestInit", "body")))
        .transpose()?
        .flatten();
    if let Some(init_body) = body_from_init.as_ref() {
        state.body_content_type = init_body.content_type.clone();
        state.body = Some(init_body.bytes.clone());
        state.inherited_body_unusable = false;
    }
    if state.body.is_some()
        && (body_from_init.is_some() || method_overridden)
        && !request_method_allows_body(&state.method)
    {
        return Err(webidl::WebIdlError::custom_message(
            "Request with GET/HEAD method cannot have body",
        ));
    }
    if let Some(headers) = parsed.headers {
        state.headers = headers;
    }
    if let Some(signal) = parsed.signal {
        state.signal = signal;
    }
    if let Some(mode) = parsed.mode.map(|value| value.0) {
        if mode == RequestMode::Navigate {
            return Err(webidl::WebIdlError::custom_message(
                "Cannot construct a Request with a RequestInit whose mode member is \"navigate\".",
            ));
        }
        state.mode = mode.as_ref().to_owned();
    }
    if state.mode == "no-cors" && !moli_fetch::is_cors_safelisted_method(&state.method) {
        return Err(webidl::WebIdlError::custom_message(
            "Request method is unsupported in no-cors mode.",
        ));
    }
    if let Some(credentials_mode) = parsed.credentials_mode.map(|value| value.0) {
        state.credentials = request_credentials_mode_label(credentials_mode).to_owned();
    }
    if let Some(cache) = parsed.cache {
        state.cache = cache;
    }
    if let Some(redirect) = parsed.redirect {
        state.redirect_mode = redirect.0;
    }
    if let Some(referrer) = parsed.referrer {
        state.referrer = normalize_request_referrer(scope, &referrer);
    }
    if let Some(referrer_policy) = parsed.referrer_policy {
        state.referrer_policy = referrer_policy;
    }
    if let Some(integrity) = parsed.integrity {
        state.integrity = integrity;
    }
    if let Some(priority) = parsed.priority {
        state.priority = priority.0;
    }
    if let Some(duplex) = parsed.duplex {
        state.duplex = duplex;
    }
    if parsed.keepalive == Some(true) && init_body_is_readable_stream {
        return Err(webidl::WebIdlError::custom_message(
            "Request with keepalive cannot have a ReadableStream body",
        ));
    }
    if let Some(keepalive) = parsed.keepalive {
        state.keepalive = keepalive;
    }
    let guard = request_headers_guard_for_mode(&state.mode);
    state.headers = filter_headers_for_guard(&state.headers, guard);
    Ok(())
}

pub(super) fn validate_request_body_is_usable(
    state: &RequestConstructionState,
) -> Result<(), webidl::WebIdlError> {
    if state.inherited_body_unusable {
        return Err(request_body_already_used_error());
    }
    Ok(())
}
