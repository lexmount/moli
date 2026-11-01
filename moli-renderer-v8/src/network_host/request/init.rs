use super::input::{RequestInfo, normalize_request_method, normalize_request_referrer};
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
    pub(crate) body: Option<bytes::Bytes>,
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
    pub(crate) signal: Option<Option<v8::Global<v8::Object>>>,
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
            signal: None,
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
pub(crate) struct RequestInitMembers<'s> {
    // Keep the opaque BodyInit value until native extraction, matching Blink.
    #[webidl(converter = "raw")]
    pub(super) body: Option<v8::Local<'s, v8::Value>>,
    #[webidl(with = request_init_signal_member)]
    pub(super) signal: Option<Option<v8::Global<v8::Object>>>,
    pub(super) method: Option<String>,
    pub(super) cache: Option<String>,
    #[webidl(converter = "enum")]
    pub(super) mode: Option<RequestModeWebIdl>,
    #[webidl(converter = "enum")]
    pub(super) redirect: Option<RequestRedirectModeWebIdl>,
    pub(super) referrer: Option<String>,
    pub(super) referrer_policy: Option<String>,
    pub(super) integrity: Option<String>,
    pub(super) duplex: Option<String>,
    #[webidl(with = request_init_headers_member)]
    pub(super) headers: Option<Vec<(String, String)>>,
    #[webidl(name = "credentials", converter = "enum")]
    pub(super) credentials_mode: Option<RequestCredentialsModeWebIdl>,
    #[webidl(converter = "enum")]
    pub(super) priority: Option<RequestPriorityWebIdl>,
    pub(super) keepalive: Option<bool>,
}

impl RequestInitMembers<'_> {
    // Match Blink's AreAnyMembersPresent after dictionary conversion. In
    // particular, null body/signal members count, undefined and unknown ones
    // do not, and checking presence must not run JavaScript getters again.
    pub(super) fn has_any_member(&self) -> bool {
        self.body.is_some()
            || self.signal.is_some()
            || self.method.is_some()
            || self.cache.is_some()
            || self.mode.is_some()
            || self.redirect.is_some()
            || self.referrer.is_some()
            || self.referrer_policy.is_some()
            || self.integrity.is_some()
            || self.duplex.is_some()
            || self.headers.is_some()
            || self.credentials_mode.is_some()
            || self.priority.is_some()
            || self.keepalive.is_some()
    }
}

fn request_init_signal_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    _: &str,
) -> Result<Option<Option<v8::Global<v8::Object>>>, webidl::WebIdlError> {
    let context = webidl::Context::member("RequestInit", "signal");
    let Some(value) = webidl::property_result(scope, object, "signal", context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(Some(None));
    }
    let signal = v8::Local::<v8::Object>::try_from(value)
        .ok()
        .and_then(|signal| crate::abort_signal_route::ResolvedAbortSignal::resolve(scope, signal))
        .ok_or_else(|| webidl::WebIdlError::cannot_convert(context, "AbortSignal"))?;
    Ok(Some(Some(v8::Global::new(scope, signal.value()))))
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
    .filter(|value| !value.is_undefined())
    .map(|headers| headers_entries_from_init(scope, headers).map(Some))
    .unwrap_or(Ok(None))
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
    let priority = init.priority.map(|value| value.0);
    let method_present = init.method.is_some();
    let method = init
        .method
        .map(|s| normalize_request_method(&s))
        .transpose()?
        .unwrap_or_else(|| "GET".to_owned());

    let body_present = init.body.is_some_and(|body| !body.is_null());
    if init.keepalive == Some(true)
        && init
            .body
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .is_some_and(|object| {
                crate::web_api_interfaces::ReadableStream::is_instance(scope, object)
            })
    {
        return Err(webidl::WebIdlError::custom_message(
            "Request with keepalive cannot have a ReadableStream body",
        ));
    }
    let prepared_body = init
        .body
        .map(|body| body_init(scope, body, webidl::Context::member("RequestInit", "body")))
        .transpose()?
        .flatten();
    let body = prepared_body.as_ref().map(|body| body.bytes.clone());
    let body_content_type = prepared_body
        .as_ref()
        .and_then(|body| body.content_type.clone());
    let headers_present = init.headers.is_some();
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
        signal: init.signal,
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
