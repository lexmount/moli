use super::input::{normalize_request_method, normalize_request_referrer};
use super::*;
use crate::webidl;
use moli_fetch::{FetchPriorityHint, RequestCredentialsMode, RequestMode, RequestRedirectMode};
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
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) body_present: bool,
    pub(crate) body_content_type: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) headers_present: bool,
    pub(crate) request_mode: Option<RequestMode>,
    pub(crate) credentials_mode: Option<RequestCredentialsMode>,
    pub(crate) redirect_mode: Option<RequestRedirectMode>,
    pub(crate) priority: Option<FetchPriorityHint>,
    pub(crate) cache: Option<String>,
    pub(crate) referrer: Option<String>,
    pub(crate) referrer_policy: Option<String>,
    pub(crate) integrity: Option<String>,
    pub(crate) keepalive: Option<bool>,
}

impl Default for ParsedFetchInit {
    fn default() -> Self {
        Self {
            method: "GET".to_owned(),
            method_present: false,
            body: None,
            body_present: false,
            body_content_type: None,
            headers: Vec::new(),
            headers_present: false,
            request_mode: None,
            credentials_mode: None,
            redirect_mode: None,
            priority: None,
            cache: None,
            referrer: None,
            referrer_policy: None,
            integrity: None,
            keepalive: None,
        }
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestCredentials", parse_with = parse_request_credentials_mode_webidl)]
pub(super) struct RequestCredentialsModeWebIdl(pub(super) RequestCredentialsMode);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestMode", parse_with = parse_request_mode_webidl)]
pub(super) struct RequestModeWebIdl(pub(super) RequestMode);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestRedirect", parse_with = parse_request_redirect_mode_webidl)]
pub(super) struct RequestRedirectModeWebIdl(pub(super) RequestRedirectMode);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestPriority", parse_with = parse_request_priority_webidl)]
pub(super) struct RequestPriorityWebIdl(pub(super) FetchPriorityHint);

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RequestInit")]
pub(super) struct RequestInitMembers {
    #[webidl(legacy_nullish)]
    pub(super) method: Option<String>,
    #[webidl(legacy_nullish)]
    pub(super) cache: Option<String>,
    #[webidl(converter = "enum")]
    pub(super) mode: Option<RequestModeWebIdl>,
    #[webidl(legacy_nullish, converter = "enum")]
    pub(super) redirect: Option<RequestRedirectModeWebIdl>,
    #[webidl(legacy_nullish)]
    pub(super) referrer: Option<String>,
    #[webidl(legacy_nullish)]
    pub(super) referrer_policy: Option<String>,
    #[webidl(legacy_nullish)]
    pub(super) integrity: Option<String>,
    #[webidl(legacy_nullish)]
    pub(super) duplex: Option<String>,
    #[webidl(with = request_init_headers_member)]
    pub(super) headers: Option<Vec<(String, String)>>,
    #[webidl(name = "credentials", converter = "enum")]
    pub(super) credentials_mode: Option<RequestCredentialsModeWebIdl>,
    #[webidl(converter = "enum")]
    pub(super) priority: Option<RequestPriorityWebIdl>,
    pub(super) keepalive: Option<bool>,
}

fn request_init_headers_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    _key: &str,
) -> Result<Option<Vec<(String, String)>>, webidl::WebIdlError> {
    webidl::property_result(
        scope,
        object,
        "headers",
        webidl::Context::member("RequestInit", "headers"),
    )?
    .filter(|value| !value.is_null_or_undefined())
    .map(|headers| headers_entries_from_init(scope, headers).map(Some))
    .unwrap_or(Ok(None))
}

pub(crate) fn parse_fetch_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ParsedFetchInit, webidl::WebIdlError> {
    if args.length() <= index {
        return Ok(ParsedFetchInit::default());
    }

    let Some(init_object) = webidl::optional_object_arg(args, index) else {
        return Ok(ParsedFetchInit::default());
    };

    let init = webidl::parse_dictionary_object::<RequestInitMembers>(scope, init_object)?;
    let priority = init.priority.map(|value| value.0);
    let method_present = init_object
        .has(scope, v8str(scope, "method").into())
        .ok_or_else(|| {
            webidl::WebIdlError::pending_exception(webidl::Context::member("RequestInit", "method"))
        })?;
    let method = init
        .method
        .map(|s| normalize_request_method(&s).map_err(webidl::WebIdlError::custom_message))
        .transpose()?
        .unwrap_or_else(|| "GET".to_owned());

    let body_present = init_object
        .has(scope, v8str(scope, "body").into())
        .ok_or_else(|| {
            webidl::WebIdlError::pending_exception(webidl::Context::member("RequestInit", "body"))
        })?;
    let prepared_body = webidl::property_result(
        scope,
        init_object,
        "body",
        webidl::Context::member("RequestInit", "body"),
    )?
    .map(|value| body_init(scope, value, webidl::Context::member("RequestInit", "body")))
    .transpose()?
    .flatten();
    let body = prepared_body.as_ref().map(|body| body.bytes.clone());
    let body_content_type = prepared_body
        .as_ref()
        .and_then(|body| body.content_type.clone());
    if body.as_ref().is_some_and(|body| !body.is_empty())
        && matches!(method.as_str(), "GET" | "HEAD")
    {
        return Err(webidl::WebIdlError::custom_message(
            "Request with GET/HEAD method cannot have body",
        ));
    }

    let headers_present = init_object
        .has(scope, v8str(scope, "headers").into())
        .ok_or_else(|| {
            webidl::WebIdlError::pending_exception(webidl::Context::member(
                "RequestInit",
                "headers",
            ))
        })?;
    let mut extra_headers = init.headers.unwrap_or_default();
    append_default_body_content_type(&mut extra_headers, body_content_type.as_deref());

    let request_mode = init.mode.map(|value| value.0);
    let credentials_mode = init.credentials_mode.map(|value| value.0);
    let redirect_mode = init.redirect.map(|value| value.0);
    let referrer = init
        .referrer
        .map(|referrer| normalize_request_referrer(scope, &referrer));

    Ok(ParsedFetchInit {
        method,
        method_present,
        body,
        body_present,
        body_content_type,
        headers: extra_headers,
        headers_present,
        request_mode,
        credentials_mode,
        redirect_mode,
        priority,
        cache: init.cache,
        referrer,
        referrer_policy: init.referrer_policy,
        integrity: init.integrity,
        keepalive: init.keepalive,
    })
}

pub(crate) fn request_object_credentials_mode<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<Option<moli_fetch::RequestCredentialsMode>, webidl::WebIdlError> {
    if is_branded_request_object(scope, object) {
        return Ok(request_slot_string(scope, object, REQUEST_CREDENTIALS_SLOT)
            .and_then(|value| RequestCredentialsMode::from_str(&value).ok()));
    }
    webidl::parse_dictionary_object::<RequestInitMembers>(scope, object)
        .map(|parsed| parsed.credentials_mode.map(|value| value.0))
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

fn parse_request_credentials_mode_webidl(value: &str) -> Option<RequestCredentialsModeWebIdl> {
    RequestCredentialsMode::from_str(value)
        .ok()
        .map(RequestCredentialsModeWebIdl)
}

fn parse_request_mode_webidl(value: &str) -> Option<RequestModeWebIdl> {
    RequestMode::from_str(value).ok().map(RequestModeWebIdl)
}

fn parse_request_redirect_mode_webidl(value: &str) -> Option<RequestRedirectModeWebIdl> {
    RequestRedirectMode::from_str(value)
        .ok()
        .map(RequestRedirectModeWebIdl)
}

fn parse_request_priority_webidl(value: &str) -> Option<RequestPriorityWebIdl> {
    FetchPriorityHint::from_str(value)
        .ok()
        .map(RequestPriorityWebIdl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_priority_webidl_enum_is_case_sensitive() {
        assert!(<RequestPriorityWebIdl as webidl::WebIdlEnum>::parse_token("high").is_some());
        assert!(<RequestPriorityWebIdl as webidl::WebIdlEnum>::parse_token("HIGH").is_none());
    }
}
