use super::input::{RequestInfo, normalize_fetch_request_method, normalize_request_referrer};
use super::*;
use crate::webidl;
use moli_fetch::{FetchPriorityHint, RequestCredentialsMode, RequestMode, RequestRedirectMode};
use moli_webapi::fetch_request::RequestCredentialsModeWebIdl;
pub(crate) use moli_webapi::fetch_request::RequestInitMembers;
use std::str::FromStr;

/// Fetch rejects with the original conversion exception, including primitive
/// values. Keep the catch boundary around argument conversion, before a body
/// is consumed or any request is dispatched.
pub(crate) fn convert_fetch_arguments<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    convert: impl FnOnce(&mut v8::PinScope<'s, '_>) -> Result<T, FetchArgumentError>,
) -> Result<T, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut conversion_scope = try_catch.init();
    let result = convert(&mut conversion_scope);
    if !conversion_scope.has_caught()
        && let Err(error) = &result
    {
        // WebIDL's PendingException already carries an exception through V8;
        // only ordinary validation failures need a new TypeError here.
        error.throw(&mut conversion_scope);
    }
    if conversion_scope.has_caught() {
        let exception = conversion_scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&conversion_scope).into());
        conversion_scope.reset();
        return Err(exception);
    }
    result.map_err(|_| v8::undefined(&conversion_scope).into())
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedFetchInit {
    pub(crate) method: String,
    pub(crate) method_present: bool,
    pub(crate) body: Option<bytes::Bytes>,
    pub(crate) body_stream: Option<v8::Global<v8::Object>>,
    pub(crate) body_present: bool,
    pub(crate) body_content_type: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) headers_present: bool,
    pub(crate) validation: RequestInitValidation,
    pub(crate) credentials_mode: Option<RequestCredentialsMode>,
    pub(crate) redirect_mode: Option<RequestRedirectMode>,
    pub(crate) priority: Option<FetchPriorityHint>,
    pub(crate) cache: Option<String>,
    pub(crate) referrer_policy: Option<String>,
    pub(crate) integrity: Option<String>,
    pub(crate) keepalive: Option<bool>,
    pub(crate) signal: Option<Option<v8::Global<v8::Object>>>,
}

impl Default for ParsedFetchInit {
    fn default() -> Self {
        Self {
            method: "GET".to_owned(),
            method_present: false,
            body: None,
            body_stream: None,
            body_present: false,
            body_content_type: None,
            headers: Vec::new(),
            headers_present: false,
            validation: RequestInitValidation::default(),
            credentials_mode: None,
            redirect_mode: None,
            priority: None,
            cache: None,
            referrer_policy: None,
            integrity: None,
            keepalive: None,
            signal: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequestInitValidation {
    pub(crate) mode: Option<RequestMode>,
    window_non_null: bool,
    referrer: Option<String>,
}

impl RequestInitValidation {
    pub(super) fn from_members(init: &RequestInitMembers<'_>) -> Self {
        Self {
            mode: init.mode.map(|value| value.0),
            window_non_null: init.window_non_null,
            referrer: init.referrer.clone(),
        }
    }

    // Run construction checks after dictionary conversion, using the effective
    // inherited/overridden values and the request's relevant realm. Fetch must
    // complete these checks before consuming an input body or observing abort.
    pub(crate) fn validate(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        mode: &str,
        cache: &str,
    ) -> Result<Option<String>, webidl::WebIdlError> {
        if self.window_non_null {
            return Err(webidl::WebIdlError::custom_message(
                "RequestInit's window member must be null",
            ));
        }
        let referrer = self
            .referrer
            .as_deref()
            .map(|value| normalize_request_referrer(scope, value))
            .transpose()
            .map_err(|_| {
                webidl::WebIdlError::custom_message("Request referrer is not a valid URL")
            })?;
        if self.mode == Some(RequestMode::Navigate) {
            return Err(webidl::WebIdlError::custom_message(
                "Cannot construct a Request with mode navigate",
            ));
        }
        if cache == "only-if-cached" && mode != "same-origin" {
            return Err(webidl::WebIdlError::custom_message(
                "Request cache only-if-cached requires mode same-origin",
            ));
        }
        Ok(referrer)
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "fetch")]
pub(crate) struct FetchArgs<'s> {
    #[webidl(required, converter = "raw")]
    pub(crate) input: RequestInfo<'s>,
    #[webidl(dictionary)]
    pub(crate) init: RequestInitMembers<'s>,
}

pub(crate) fn parse_fetch_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: RequestInitMembers<'s>,
) -> Result<ParsedFetchInit, webidl::WebIdlError> {
    let validation = RequestInitValidation::from_members(&init);
    let priority = init.priority.map(|value| value.0);
    let method_present = init.method.is_some();
    let method = init
        .method
        .map(|s| normalize_fetch_request_method(&s))
        .transpose()?
        .unwrap_or_else(|| "GET".to_owned());

    let body_value = init.body;
    let body_stream = body_value
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .filter(|object| crate::context_bootstrap::is_readable_stream_object(scope, *object))
        .map(|stream| v8::Global::new(scope, stream));
    let prepared_body = body_value
        .map(|value| body_init(scope, value, webidl::Context::member("RequestInit", "body")))
        .transpose()?
        .flatten();
    // Null and undefined RequestInit bodies inherit an input Request's body.
    let body_present = prepared_body.is_some();
    if body_stream.is_some() && init.duplex.is_none() {
        return Err(webidl::WebIdlError::custom_message(
            "Request with a ReadableStream body requires duplex",
        ));
    }
    if body_stream.is_some() && init.keepalive == Some(true) {
        return Err(webidl::WebIdlError::custom_message(
            "Request with keepalive cannot have a ReadableStream body",
        ));
    }
    let body = prepared_body
        .as_ref()
        .map(|body| match &body.source {
            PreparedBodySource::Bytes(bytes) => Ok(bytes.clone()),
            PreparedBodySource::Blob(_) => body.source.clone().into_buffered_bytes(scope),
            PreparedBodySource::Stream(_) => Ok(bytes::Bytes::new()),
        })
        .transpose()?;
    let body_content_type = prepared_body
        .as_ref()
        .and_then(|body| body.content_type.clone());
    let headers_present = init.headers.is_some();
    let mut extra_headers = init.headers.unwrap_or_default();
    append_default_body_content_type(&mut extra_headers, body_content_type.as_deref());

    let credentials_mode = init.credentials_mode.map(|value| value.0);
    let redirect_mode = init.redirect.map(|value| value.0);

    Ok(ParsedFetchInit {
        method,
        method_present,
        body,
        body_stream,
        body_present,
        body_content_type,
        headers: extra_headers,
        headers_present,
        validation,
        credentials_mode,
        redirect_mode,
        priority,
        cache: init.cache.map(|value| value.0.to_owned()),
        referrer_policy: init.referrer_policy.map(|value| value.0.to_owned()),
        integrity: init.integrity,
        keepalive: init.keepalive,
        signal: init.signal,
    })
}

pub(crate) fn validate_fetch_body(
    has_body: bool,
    has_stream_body: bool,
    method: &str,
    mode: RequestMode,
) -> Result<(), String> {
    if has_body && matches!(method, "GET" | "HEAD") {
        return Err("Request with GET/HEAD method cannot have body".to_owned());
    }
    if has_stream_body && !matches!(mode, RequestMode::Cors | RequestMode::SameOrigin) {
        return Err(
            "Request with a ReadableStream body requires cors or same-origin mode".to_owned(),
        );
    }
    Ok(())
}

pub(crate) fn request_object_credentials_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<moli_fetch::RequestCredentialsMode>, webidl::WebIdlError> {
    if is_branded_request_object(scope, object) {
        return Ok(request_slot_string(scope, object, REQUEST_CREDENTIALS_SLOT)
            .and_then(|value| RequestCredentialsMode::from_str(&value).ok()));
    }
    webidl::optional_member::<webidl::EnumValue<RequestCredentialsModeWebIdl>>(
        scope,
        object,
        "credentials",
        webidl::Context::member("Request", "credentials"),
    )
    .map(|value| value.map(|value| value.0.0))
}

pub(in crate::network_host) fn request_credentials_mode_label(
    mode: RequestCredentialsMode,
) -> &'static str {
    mode.into()
}

pub(crate) fn request_redirect_mode_label(mode: RequestRedirectMode) -> &'static str {
    mode.into()
}

pub(crate) fn parse_request_redirect_mode_label(value: &str) -> Option<RequestRedirectMode> {
    RequestRedirectMode::from_str(value).ok()
}
