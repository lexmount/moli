use super::input::{RequestInfo, normalize_fetch_request_method, normalize_request_referrer};
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

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestCache", parse_with = parse_request_cache_webidl)]
pub(super) struct RequestCacheWebIdl(pub(super) &'static str);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "ReferrerPolicy", parse_with = parse_referrer_policy_webidl)]
pub(super) struct ReferrerPolicyWebIdl(pub(super) &'static str);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestDuplex")]
pub(super) enum RequestDuplexWebIdl {
    #[webidl(token = "half")]
    Half,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RequestInit")]
pub(crate) struct RequestInitMembers<'s> {
    // Keep the opaque BodyInit value until native extraction, matching Blink.
    #[webidl(nullable, converter = "raw")]
    pub(super) body: Option<v8::Local<'s, v8::Value>>,
    #[webidl(with = request_init_signal_member)]
    pub(super) signal: Option<Option<v8::Global<v8::Object>>>,
    #[webidl(converter = "byte_string")]
    pub(super) method: Option<String>,
    #[webidl(converter = "enum")]
    pub(super) cache: Option<RequestCacheWebIdl>,
    #[webidl(converter = "enum")]
    pub(super) mode: Option<RequestModeWebIdl>,
    #[webidl(converter = "enum")]
    pub(super) redirect: Option<RequestRedirectModeWebIdl>,
    #[webidl(converter = "usv_string")]
    pub(super) referrer: Option<String>,
    #[webidl(converter = "enum")]
    pub(super) referrer_policy: Option<ReferrerPolicyWebIdl>,
    pub(super) integrity: Option<String>,
    #[webidl(converter = "enum")]
    pub(super) duplex: Option<RequestDuplexWebIdl>,
    #[webidl(with = request_init_headers_member)]
    pub(super) headers: Option<Vec<(String, String)>>,
    #[webidl(name = "credentials", converter = "enum")]
    pub(super) credentials_mode: Option<RequestCredentialsModeWebIdl>,
    #[webidl(converter = "enum")]
    pub(super) priority: Option<RequestPriorityWebIdl>,
    pub(super) keepalive: Option<bool>,
    #[webidl(name = "window", with = request_init_window_member)]
    window_non_null: bool,
}

impl RequestInitMembers<'_> {
    pub(super) fn validation(&self) -> RequestInitValidation {
        RequestInitValidation {
            mode: self.mode.map(|value| value.0),
            window_non_null: self.window_non_null,
            referrer: self.referrer.clone(),
        }
    }
}

fn request_init_window_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    _key: &str,
) -> Result<bool, webidl::WebIdlError> {
    webidl::property_result(
        scope,
        object,
        "window",
        webidl::Context::member("RequestInit", "window"),
    )
    .map(|value| value.is_some_and(|value| !value.is_null_or_undefined()))
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
    let validation = init.validation();
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
        .filter(|object| crate::web_api_interfaces::ReadableStream::is_instance(scope, *object))
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
    let body_content_type = prepared_body
        .as_ref()
        .and_then(|body| body.content_type.clone());
    let (body, body_stream) = match prepared_body.map(|body| body.source) {
        Some(PreparedBodySource::Bytes(bytes)) => (Some(bytes), None),
        Some(PreparedBodySource::Blob(blob)) => {
            let blob = v8::Local::new(scope, blob);
            (
                blob::blob_bytes_from_object(scope, blob).map(bytes::Bytes::from),
                None,
            )
        }
        Some(PreparedBodySource::Stream(stream)) => (Some(bytes::Bytes::new()), Some(stream)),
        None => (None, None),
    };

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
) -> Result<(), webidl::WebIdlError> {
    if has_body && matches!(method, "GET" | "HEAD") {
        return Err(webidl::WebIdlError::custom_message(
            "Request with GET/HEAD method cannot have body",
        ));
    }
    if has_stream_body && !matches!(mode, RequestMode::Cors | RequestMode::SameOrigin) {
        return Err(webidl::WebIdlError::custom_message(
            "Request with a ReadableStream body requires cors or same-origin mode",
        ));
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


fn parse_request_cache_webidl(value: &str) -> Option<RequestCacheWebIdl> {
    Some(RequestCacheWebIdl(match value {
        "default" => "default",
        "no-store" => "no-store",
        "reload" => "reload",
        "no-cache" => "no-cache",
        "force-cache" => "force-cache",
        "only-if-cached" => "only-if-cached",
        _ => return None,
    }))
}

fn parse_referrer_policy_webidl(value: &str) -> Option<ReferrerPolicyWebIdl> {
    Some(ReferrerPolicyWebIdl(match value {
        "" => "",
        "no-referrer" => "no-referrer",
        "no-referrer-when-downgrade" => "no-referrer-when-downgrade",
        "same-origin" => "same-origin",
        "origin" => "origin",
        "strict-origin" => "strict-origin",
        "origin-when-cross-origin" => "origin-when-cross-origin",
        "strict-origin-when-cross-origin" => "strict-origin-when-cross-origin",
        "unsafe-url" => "unsafe-url",
        _ => return None,
    }))
}
