use super::input::{normalize_fetch_request_method, normalize_request_referrer};
use super::*;
use crate::webidl;
use moli_fetch::{FetchPriorityHint, RequestCredentialsMode, RequestMode, RequestRedirectMode};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub(crate) struct ParsedFetchInit {
    pub(crate) method: String,
    pub(crate) method_present: bool,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) body_present: bool,
    pub(crate) body_content_type: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) headers_present: bool,
    pub(crate) suppress_default_content_type: bool,
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
            body_present: false,
            body_content_type: None,
            headers: Vec::new(),
            headers_present: false,
            suppress_default_content_type: false,
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
    webidl::property_non_nullish(scope, object, "headers")
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

    let body_present = init_object
        .has(scope, v8str(scope, "body").into())
        .unwrap_or(false);
    let prepared_body = webidl::property_result(
        scope,
        init_object,
        "body",
        webidl::Context::member("RequestInit", "body"),
    )
    .map_err(|error| error.to_string())?
    .map(|value| body_init(scope, value, webidl::Context::member("RequestInit", "body")))
    .transpose()
    .map_err(|error| error.to_string())?
    .flatten();
    let body = prepared_body.as_ref().map(|body| body.bytes.clone());
    let body_content_type = prepared_body
        .as_ref()
        .and_then(|body| body.content_type.clone());
    if body.as_ref().is_some_and(|body| !body.is_empty())
        && matches!(method.as_str(), "GET" | "HEAD")
    {
        return Err("Request with GET/HEAD method cannot have body".to_owned());
    }

    let headers_present = init_object
        .has(scope, v8str(scope, "headers").into())
        .unwrap_or(false);
    let mut extra_headers = init.headers.unwrap_or_default();
    let mut suppress_default_content_type = false;
    if let Some(prepared_body) = prepared_body.as_ref() {
        if let Some(content_type) = prepared_body.content_type.as_deref() {
            append_default_body_content_type(&mut extra_headers, Some(content_type));
        } else if !has_header(&extra_headers, "content-type") {
            suppress_default_content_type = true;
        }
    }

    let credentials_mode = init.credentials_mode.map(|value| value.0);
    let redirect_mode = init.redirect.map(|value| value.0);
    Ok(ParsedFetchInit {
        method,
        method_present,
        body,
        body_present,
        body_content_type,
        headers: extra_headers,
        headers_present,
        suppress_default_content_type,
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
