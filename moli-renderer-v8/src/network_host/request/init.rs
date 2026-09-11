use super::input::{normalize_fetch_request_method, normalize_request_referrer};
use super::*;
use crate::webidl;
use moli_fetch::{FetchPriorityHint, RequestCredentialsMode, RequestMode, RequestRedirectMode};
use std::str::FromStr;

/// Fetch rejects with the original conversion exception, including primitive
/// values. Keep the catch boundary around argument conversion, before a body
/// is consumed or any request is dispatched.
pub(crate) fn convert_fetch_arguments<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    convert: impl FnOnce(&mut v8::PinScope<'s, '_>) -> Result<T, String>,
) -> Result<T, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut conversion_scope = try_catch.init();
    let result = convert(&mut conversion_scope);
    if conversion_scope.has_caught() {
        let exception = conversion_scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&conversion_scope).into());
        conversion_scope.reset();
        return Err(exception);
    }
    result.map_err(|message| {
        crate::util::v8_string(&conversion_scope, &message)
            .map(|message| v8::Exception::type_error(&conversion_scope, message))
            .unwrap_or_else(|| v8::undefined(&conversion_scope).into())
    })
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedFetchInit {
    pub(crate) method: String,
    pub(crate) method_present: bool,
    pub(crate) body: Option<Vec<u8>>,
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
    ) -> Result<Option<String>, &'static str> {
        if self.window_non_null {
            return Err("RequestInit's window member must be null");
        }
        let referrer = self
            .referrer
            .as_deref()
            .map(|value| normalize_request_referrer(scope, value))
            .transpose()
            .map_err(|_| "Request referrer is not a valid URL")?;
        if self.mode == Some(RequestMode::Navigate) {
            return Err("Cannot construct a Request with mode navigate");
        }
        if cache == "only-if-cached" && mode != "same-origin" {
            return Err("Request cache only-if-cached requires mode same-origin");
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
pub(super) struct RequestInitMembers {
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
    #[webidl(legacy_nullish)]
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

impl RequestInitMembers {
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
) -> Result<ParsedFetchInit, String> {
    if args.length() <= index {
        return Ok(ParsedFetchInit::default());
    }

    let Some(init_object) = webidl::optional_object_arg(args, index) else {
        return Ok(ParsedFetchInit::default());
    };

    let init = webidl::parse_dictionary_object::<RequestInitMembers>(scope, init_object)
        .map_err(|error| error.to_string())?;
    let validation = init.validation();
    let priority = init.priority.map(|value| value.0);
    let method_present = init_object
        .has(scope, v8str(scope, "method").into())
        .unwrap_or(false);
    let method = init
        .method
        .map(|s| normalize_fetch_request_method(&s).map_err(str::to_owned))
        .transpose()?
        .unwrap_or_else(|| "GET".to_owned());

    let body_value = webidl::property_result(
        scope,
        init_object,
        "body",
        webidl::Context::member("RequestInit", "body"),
    )
    .map_err(|error| error.to_string())?;
    let body_stream = body_value
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .filter(|object| crate::context_bootstrap::is_readable_stream_object(scope, *object))
        .map(|stream| v8::Global::new(scope, stream));
    let prepared_body = body_value
        .map(|value| body_init(scope, value, webidl::Context::member("RequestInit", "body")))
        .transpose()
        .map_err(|error| error.to_string())?
        .flatten();
    // Null and undefined RequestInit bodies inherit an input Request's body.
    let body_present = prepared_body.is_some();
    if body_stream.is_some() && init.duplex.is_none() {
        return Err("Request with a ReadableStream body requires duplex".to_owned());
    }
    if body_stream.is_some() && init.keepalive == Some(true) {
        return Err("Request with keepalive cannot have a ReadableStream body".to_owned());
    }
    let body = prepared_body.as_ref().map(|body| body.bytes.clone());
    let body_content_type = prepared_body
        .as_ref()
        .and_then(|body| body.content_type.clone());

    let headers_present = init_object
        .has(scope, v8str(scope, "headers").into())
        .unwrap_or(false);
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
) -> Result<Option<moli_fetch::RequestCredentialsMode>, String> {
    if is_branded_request_object(scope, object) {
        return Ok(request_slot_string(scope, object, REQUEST_CREDENTIALS_SLOT)
            .and_then(|value| RequestCredentialsMode::from_str(&value).ok()));
    }
    webidl::parse_dictionary_object::<RequestInitMembers>(scope, object)
        .map(|parsed| parsed.credentials_mode.map(|value| value.0))
        .map_err(|error| error.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_priority_webidl_enum_is_case_sensitive() {
        assert!(<RequestPriorityWebIdl as webidl::WebIdlEnum>::parse_token("high").is_some());
        assert!(<RequestPriorityWebIdl as webidl::WebIdlEnum>::parse_token("HIGH").is_none());
    }
}
