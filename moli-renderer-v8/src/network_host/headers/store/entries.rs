use super::super::*;
use crate::util::{get_private_value, set_private_value};
use http::header::HeaderName;

pub(super) const HEADERS_ENTRIES_SLOT: &str = "__lmHeadersEntriesJson";
pub(super) const HEADERS_IMMUTABLE_SLOT: &str = "__lmHeadersImmutable";
pub(super) const HEADERS_GUARD_SLOT: &str = "__lmHeadersGuard";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeadersGuard {
    None,
    Request,
    RequestNoCors,
    Response,
}

impl HeadersGuard {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Request => "request",
            Self::RequestNoCors => "request-no-cors",
            Self::Response => "response",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "request" => Self::Request,
            "request-no-cors" => Self::RequestNoCors,
            "response" => Self::Response,
            _ => Self::None,
        }
    }
}

pub(in crate::network_host) fn mark_headers_immutable(
    scope: &mut v8::PinScope<'_, '_>,
    obj: v8::Local<'_, v8::Object>,
) {
    set_private_value(
        scope,
        obj,
        HEADERS_IMMUTABLE_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}

pub(in crate::network_host::headers) fn headers_are_immutable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, obj, HEADERS_IMMUTABLE_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(in crate::network_host::headers) fn headers_guard<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
) -> HeadersGuard {
    private_string_value(scope, obj, HEADERS_GUARD_SLOT)
        .as_deref()
        .map(HeadersGuard::from_str)
        .unwrap_or(HeadersGuard::None)
}

pub(in crate::network_host::headers) fn header_allowed_by_guard(
    guard: HeadersGuard,
    name: &str,
    value: &str,
) -> bool {
    match guard {
        HeadersGuard::None => true,
        HeadersGuard::Request => {
            !moli_fetch::is_forbidden_request_header_name(name)
                && !moli_fetch::is_forbidden_request_header_override_value(name, value)
        }
        HeadersGuard::RequestNoCors => {
            !moli_fetch::is_forbidden_request_header_name(name)
                && !moli_fetch::is_forbidden_request_header_override_value(name, value)
                && moli_fetch::is_no_cors_safelisted_request_header(name, value)
        }
        HeadersGuard::Response => !moli_fetch::is_forbidden_response_header_name(name),
    }
}

pub(crate) fn filter_headers_for_guard(
    entries: &[(String, String)],
    guard: HeadersGuard,
) -> Vec<(String, String)> {
    entries
        .iter()
        .filter(|(name, value)| header_allowed_by_guard(guard, name, value))
        .cloned()
        .collect()
}

pub(in crate::network_host) fn set_headers_entries(
    scope: &mut v8::PinScope<'_, '_>,
    obj: v8::Local<'_, v8::Object>,
    entries: &[(String, String)],
) {
    let json = headers_entries_json(entries);
    let Some(json) = v8_string(scope, &json) else {
        return;
    };
    set_private_value(scope, obj, HEADERS_ENTRIES_SLOT, json.into());
}

pub(in crate::network_host) fn headers_entries_json(entries: &[(String, String)]) -> String {
    let entries = normalized_headers_entries(entries);
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_owned())
}

pub(in crate::network_host::headers) fn normalized_headers_entries(
    entries: &[(String, String)],
) -> Vec<(String, String)> {
    let mut normalized = Vec::<(String, String)>::new();
    for (name, value) in entries {
        let Some(lower) = normalized_header_name(name) else {
            continue;
        };
        let value = normalize_header_value(value);
        if !is_valid_header_value(&value) {
            continue;
        }
        if lower == "set-cookie" {
            normalized.push((lower, value));
            continue;
        }
        if let Some((_, existing)) = normalized
            .iter_mut()
            .find(|(entry_name, _)| *entry_name == lower)
        {
            existing.push_str(", ");
            existing.push_str(&value);
        } else {
            normalized.push((lower, value));
        }
    }
    normalized.sort_by(|(left, _), (right, _)| left.cmp(right));
    normalized
}

pub(in crate::network_host::headers) fn normalize_header_value(value: &str) -> String {
    value.trim_matches(is_http_whitespace).to_owned()
}

pub(in crate::network_host::headers) fn normalized_header_name_or_throw(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
) -> Option<String> {
    match normalized_header_name(name) {
        Some(name) => Some(name),
        None => {
            throw_type_error(scope, "Invalid header name");
            None
        }
    }
}

pub(in crate::network_host::headers) fn normalized_header_value_or_throw(
    scope: &mut v8::PinScope<'_, '_>,
    value: &str,
) -> Option<String> {
    let value = normalize_header_value(value);
    if is_valid_header_value(&value) {
        Some(value)
    } else {
        throw_type_error(scope, "Invalid header value");
        None
    }
}

pub(in crate::network_host) fn normalized_header_entry_or_throw(
    scope: &mut v8::PinScope<'_, '_>,
    name: String,
    value: String,
) -> Option<(String, String)> {
    let name = normalized_header_name_or_throw(scope, &name)?;
    let value = normalized_header_value_or_throw(scope, &value)?;
    Some((name, value))
}

fn normalized_header_name(name: &str) -> Option<String> {
    HeaderName::from_bytes(name.as_bytes())
        .ok()
        .map(|name| name.as_str().to_owned())
}

fn is_valid_header_value(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch as u32 <= 0xff && !matches!(ch, '\0' | '\n' | '\r'))
}

fn is_http_whitespace(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r' | ' ')
}

pub(crate) fn headers_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
) -> Vec<(String, String)> {
    headers_entries_if_present(scope, obj).unwrap_or_default()
}

pub(in crate::network_host) fn headers_entries_if_present<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
) -> Option<Vec<(String, String)>> {
    private_string_value(scope, obj, HEADERS_ENTRIES_SLOT)
        .and_then(|json| serde_json::from_str::<Vec<(String, String)>>(&json).ok())
}

fn private_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    get_private_value(scope, obj, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

#[cfg(test)]
mod tests {
    use super::{HeadersGuard, header_allowed_by_guard, normalized_headers_entries};

    #[test]
    fn normalized_headers_entries_keeps_set_cookie_values_separate() {
        let normalized = normalized_headers_entries(&[
            ("Set-Cookie".to_owned(), "a=1".to_owned()),
            ("set-cookie".to_owned(), "b=2".to_owned()),
            ("X-Test".to_owned(), "one".to_owned()),
            ("x-test".to_owned(), "two".to_owned()),
        ]);

        assert_eq!(
            normalized,
            vec![
                ("set-cookie".to_owned(), "a=1".to_owned()),
                ("set-cookie".to_owned(), "b=2".to_owned()),
                ("x-test".to_owned(), "one, two".to_owned()),
            ]
        );
    }

    #[test]
    fn normalized_headers_entries_uses_http_header_validation() {
        let normalized = normalized_headers_entries(&[
            ("Bad Name".to_owned(), "ignored".to_owned()),
            ("X-Test".to_owned(), "bad\0value".to_owned()),
            ("Accept".to_owned(), "text/html".to_owned()),
        ]);

        assert_eq!(
            normalized,
            vec![("accept".to_owned(), "text/html".to_owned())]
        );
    }

    #[test]
    fn normalized_headers_entries_trims_http_whitespace() {
        let normalized = normalized_headers_entries(&[
            ("X-Test".to_owned(), "\r\n\tvalue\n".to_owned()),
            ("X-Form-Feed".to_owned(), "\t\u{c}\tvalue\n".to_owned()),
        ]);

        assert_eq!(
            normalized,
            vec![
                ("x-form-feed".to_owned(), "\u{c}\tvalue".to_owned()),
                ("x-test".to_owned(), "value".to_owned()),
            ]
        );
    }

    #[test]
    fn request_guard_filters_forbidden_method_override_values() {
        assert!(!header_allowed_by_guard(
            HeadersGuard::Request,
            "x-http-method-override",
            "GET, track "
        ));
        assert!(!header_allowed_by_guard(
            HeadersGuard::Request,
            "x-method-override",
            "\tTRACE"
        ));
        assert!(header_allowed_by_guard(
            HeadersGuard::Request,
            "x-http-method",
            "GETTRACE"
        ));
        assert!(header_allowed_by_guard(
            HeadersGuard::Request,
            "x-http-method",
            "\",TRACE\","
        ));
    }

    #[test]
    fn no_cors_guard_checks_safelisted_header_values() {
        assert!(header_allowed_by_guard(
            HeadersGuard::RequestNoCors,
            "accept",
            "text/html"
        ));
        assert!(!header_allowed_by_guard(
            HeadersGuard::RequestNoCors,
            "accept",
            "\""
        ));
        assert!(header_allowed_by_guard(
            HeadersGuard::RequestNoCors,
            "accept-language",
            "en-US, zh"
        ));
        assert!(!header_allowed_by_guard(
            HeadersGuard::RequestNoCors,
            "accept-language",
            "@"
        ));
        assert!(header_allowed_by_guard(
            HeadersGuard::RequestNoCors,
            "content-type",
            "text/plain;charset=UTF-8"
        ));
        assert!(!header_allowed_by_guard(
            HeadersGuard::RequestNoCors,
            "content-type",
            "text/html"
        ));
    }

    #[test]
    fn response_guard_filters_forbidden_response_headers() {
        assert!(!header_allowed_by_guard(
            HeadersGuard::Response,
            "set-cookie",
            "a=b"
        ));
        assert!(!header_allowed_by_guard(
            HeadersGuard::Response,
            "set-cookie2",
            "a=b"
        ));
        assert!(header_allowed_by_guard(
            HeadersGuard::Response,
            "content-type",
            "text/plain"
        ));
    }
}
