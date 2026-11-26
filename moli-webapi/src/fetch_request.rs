use crate::headers::headers_entries_from_init;
use moli_fetch_types::{
    FetchPriorityHint, RequestCredentialsMode, RequestMode, RequestRedirectMode,
};
use moli_webidl as webidl;
use std::str::FromStr;

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestCredentials", parse_with = parse_request_credentials_mode_webidl)]
pub struct RequestCredentialsModeWebIdl(pub RequestCredentialsMode);

impl Default for RequestCredentialsModeWebIdl {
    fn default() -> Self {
        Self(RequestCredentialsMode::SameOrigin)
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestMode", parse_with = parse_request_mode_webidl)]
pub struct RequestModeWebIdl(pub RequestMode);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestRedirect", parse_with = parse_request_redirect_mode_webidl)]
pub struct RequestRedirectModeWebIdl(pub RequestRedirectMode);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestPriority", parse_with = parse_request_priority_webidl)]
pub struct RequestPriorityWebIdl(pub FetchPriorityHint);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestCache", parse_with = parse_request_cache_webidl)]
pub struct RequestCacheWebIdl(pub &'static str);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "ReferrerPolicy", parse_with = parse_referrer_policy_webidl)]
pub struct ReferrerPolicyWebIdl(pub &'static str);

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RequestDuplex")]
pub enum RequestDuplexWebIdl {
    #[webidl(token = "half")]
    Half,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RequestInit")]
pub struct RequestInitMembers<'s> {
    // Keep the opaque BodyInit value until native extraction, matching Blink.
    #[webidl(converter = "raw")]
    pub body: Option<v8::Local<'s, v8::Value>>,
    #[webidl(with = request_init_signal_member)]
    pub signal: Option<Option<v8::Global<v8::Object>>>,
    #[webidl(converter = "byte_string")]
    pub method: Option<String>,
    #[webidl(converter = "enum")]
    pub cache: Option<RequestCacheWebIdl>,
    #[webidl(converter = "enum")]
    pub mode: Option<RequestModeWebIdl>,
    #[webidl(converter = "enum")]
    pub redirect: Option<RequestRedirectModeWebIdl>,
    #[webidl(converter = "usv_string")]
    pub referrer: Option<String>,
    #[webidl(converter = "enum")]
    pub referrer_policy: Option<ReferrerPolicyWebIdl>,
    pub integrity: Option<String>,
    #[webidl(converter = "enum")]
    pub duplex: Option<RequestDuplexWebIdl>,
    #[webidl(with = request_init_headers_member)]
    pub headers: Option<Vec<(String, String)>>,
    #[webidl(name = "credentials", converter = "enum")]
    pub credentials_mode: Option<RequestCredentialsModeWebIdl>,
    #[webidl(converter = "enum")]
    pub priority: Option<RequestPriorityWebIdl>,
    pub keepalive: Option<bool>,
    #[webidl(name = "window", with = request_init_window_member)]
    pub window_non_null: bool,
}

impl RequestInitMembers<'_> {
    pub fn has_any_member(&self) -> bool {
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

    pub fn body_is_present(&self) -> bool {
        // Explicit null is an init member but does not replace an input body.
        self.body.is_some_and(|body| !body.is_null_or_undefined())
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
        .and_then(|signal| crate::abort_signal::ResolvedAbortSignal::resolve(scope, signal))
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
