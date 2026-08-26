use std::borrow::Cow;

use cookie::SameSite;
use url::Url;

use crate::cookie::{Cookie, CookieSourceScheme};
use crate::utils::is_http_scheme;

use super::*;

pub(super) fn source_scheme_mismatch(cookie: &Cookie<'_>, url: &Url) -> bool {
    let source_scheme = cookie.source_scheme();
    source_scheme != CookieSourceScheme::Unset && source_scheme != CookieSourceScheme::from_url(url)
}

pub(super) fn effective_same_site(cookie: &Cookie<'_>) -> CookieEffectiveSameSite {
    match cookie.same_site() {
        Some(SameSite::Strict) => CookieEffectiveSameSite::Strict,
        Some(SameSite::Lax) => CookieEffectiveSameSite::Lax,
        _ => CookieEffectiveSameSite::NoRestriction,
    }
}

pub(super) fn access_semantics(cookie: &Cookie<'_>) -> CookieAccessSemantics {
    match cookie.same_site() {
        // The fork currently enforces the modern model for cookies with an
        // explicit SameSite attribute. Cookies without an attribute remain
        // `Unknown` until lax-by-default/legacy compatibility is modeled.
        Some(_) => CookieAccessSemantics::NonLegacy,
        None => CookieAccessSemantics::Unknown,
    }
}

pub(super) fn scope_semantics(_cookie: &Cookie<'_>) -> CookieScopeSemantics {
    // Chromium distinguishes origin-bound/legacy scope semantics. The fork
    // does not yet model that split, so keep the field explicit but
    // conservatively unknown.
    CookieScopeSemantics::Unknown
}

pub(super) fn sort_included_cookies_for_projection(cookies: &mut [Cookie<'static>]) {
    cookies.sort_by(|left, right| {
        right
            .path
            .len()
            .cmp(&left.path.len())
            .then_with(|| left.creation_index().cmp(&right.creation_index()))
    });
}

pub(super) fn sort_included_cookie_accesses_for_projection(cookies: &mut [CookieWithAccessResult]) {
    cookies.sort_by(|left, right| {
        right
            .cookie
            .path
            .len()
            .cmp(&left.cookie.path.len())
            .then_with(|| {
                left.cookie
                    .creation_index()
                    .cmp(&right.cookie.creation_index())
            })
    });
}

pub(super) fn cookie_matches_delete_filter(
    cookie: &Cookie<'_>,
    filter: &CookieDeleteFilter<'_>,
) -> bool {
    if filter.name.is_some_and(|wanted| cookie.name() != wanted) {
        return false;
    }

    let cookie_domain = canonical_cookie_domain(cookie);
    if filter.domain.is_some_and(|wanted| cookie_domain != wanted) {
        return false;
    }

    if filter
        .path
        .is_some_and(|wanted| cookie.path.as_ref() != wanted)
    {
        return false;
    }

    if filter.url_host.is_some_and(|wanted| {
        !delete_filter_domain_matches_host(
            wanted,
            &cookie_domain,
            matches!(cookie.domain, crate::CookieDomain::HostOnly(_)),
        )
    }) {
        return false;
    }

    if filter
        .partition_key
        .is_some_and(|wanted| cookie.partition_key() != Some(wanted))
    {
        return false;
    }

    true
}

pub(super) fn delete_filter_domain_matches_host(
    request_host: &str,
    cookie_domain: &str,
    host_only: bool,
) -> bool {
    let request_host = request_host.to_ascii_lowercase();
    let cookie_domain = cookie_domain.to_ascii_lowercase();

    if host_only {
        return request_host == cookie_domain;
    }

    request_host == cookie_domain || request_host.ends_with(&format!(".{cookie_domain}"))
}

pub(super) fn cookie_name_value_too_large(name: &str, value: &str) -> bool {
    name.len().saturating_add(value.len()) > MAX_COOKIE_NAME_VALUE_BYTES
}

pub(super) fn is_valid_cookie_attribute_value(value: &str) -> bool {
    value.len() <= MAX_COOKIE_ATTRIBUTE_VALUE_BYTES
        && !value
            .chars()
            .any(|ch| ch == '\u{7f}' || (ch.is_control() && ch != '\t'))
}

#[derive(Debug, Clone, Default)]
pub(super) struct SanitizedCookieLine<'a> {
    pub(super) line: Cow<'a, str>,
    pub(super) warning_reasons: Vec<CookieSetWarningReason>,
}

pub(super) fn sanitize_cookie_line_for_browser_parse(cookie_line: &str) -> SanitizedCookieLine<'_> {
    let mut segments = cookie_line.split(';');
    let Some(first) = segments.next() else {
        return SanitizedCookieLine {
            line: Cow::Borrowed(cookie_line),
            warning_reasons: Vec::new(),
        };
    };

    let mut changed = false;
    let mut rebuilt = vec![first.trim().to_owned()];
    let mut warning_reasons = Vec::new();
    for segment in segments {
        let trimmed = segment.trim();
        let keep = match trimmed.split_once('=') {
            Some((name, value))
                if name.trim().eq_ignore_ascii_case("domain")
                    || name.trim().eq_ignore_ascii_case("path") =>
            {
                // Chromium-style handling for oversized/invalid Domain and Path attributes is to
                // ignore the attribute rather than reject the whole cookie. Do that before
                // parsing into the canonical cookie form so host-only/default-path state is
                // computed from the same effective attribute set the browser would keep.
                let keep = is_valid_cookie_attribute_value(value.trim());
                if !keep {
                    let warning = if name.trim().eq_ignore_ascii_case("domain") {
                        CookieSetWarningReason::DomainAttributeIgnored
                    } else {
                        CookieSetWarningReason::PathAttributeIgnored
                    };
                    if !warning_reasons.contains(&warning) {
                        warning_reasons.push(warning);
                    }
                }
                keep
            }
            _ => true,
        };

        if keep {
            rebuilt.push(trimmed.to_owned());
        } else {
            changed = true;
        }
    }

    if changed {
        SanitizedCookieLine {
            line: Cow::Owned(rebuilt.join("; ")),
            warning_reasons,
        }
    } else {
        SanitizedCookieLine {
            line: Cow::Borrowed(cookie_line),
            warning_reasons,
        }
    }
}

pub(super) fn canonical_cookie_domain(cookie: &Cookie<'_>) -> String {
    match &cookie.domain {
        crate::CookieDomain::HostOnly(domain) | crate::CookieDomain::Suffix(domain) => {
            domain.clone()
        }
        crate::CookieDomain::NotPresent | crate::CookieDomain::Empty => String::new(),
    }
}

pub(super) fn prefixes_are_valid(cookie: &Cookie<'_>) -> bool {
    // Prefix rules must run on the canonical cookie view, because `__Host-*`
    // depends on host-only and explicit-path state rather than raw attribute
    // text alone.
    if cookie.name().is_empty() {
        return !protected_prefix_in_value(cookie.value());
    }

    let secure = cookie.secure().unwrap_or(false);
    let http_only = cookie.http_only().unwrap_or(false);
    let host_only = matches!(cookie.domain, crate::CookieDomain::HostOnly(_));
    let explicit_root_path = cookie.path.is_from_path_attr() && cookie.path.as_ref() == "/";

    if has_case_insensitive_prefix(cookie.name(), "__Host-Http-") {
        return secure && http_only && host_only && explicit_root_path;
    }
    if has_case_insensitive_prefix(cookie.name(), "__Http-") {
        return secure && http_only;
    }
    if has_case_insensitive_prefix(cookie.name(), "__Host-") {
        return secure && host_only && explicit_root_path;
    }
    if has_case_insensitive_prefix(cookie.name(), "__Secure-") {
        return secure;
    }

    true
}

pub(super) fn has_case_insensitive_prefix(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

pub(super) fn protected_prefix_in_value(value: &str) -> bool {
    ["__Host-Http-", "__Http-", "__Host-", "__Secure-"]
        .iter()
        .any(|prefix| has_case_insensitive_prefix(value, prefix))
}

pub(super) fn domains_overlap(left: &str, right: &str) -> bool {
    left == right || left.ends_with(&format!(".{right}")) || right.ends_with(&format!(".{left}"))
}

pub(super) fn path_overlap(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    if !left.starts_with(right) {
        return false;
    }

    right.ends_with('/')
        || left
            .as_bytes()
            .get(right.len())
            .is_some_and(|byte| *byte == b'/')
}

/// Infer the default insertion context for the simple `insert*()` APIs.
///
/// Older entry points infer "non-HTTP API" solely from the URL scheme. Keep
/// that behavior for callers that still use those helpers, while new code
/// should prefer explicit `InsertContext` constructors.
pub(super) fn inferred_insert_context(url: &Url) -> InsertContext<'_> {
    if is_http_scheme(url) {
        InsertContext {
            url,
            source: CookieAccessSource::Http,
            browser_context: BrowserSiteContext::empty(),
            enforce_browser_policy: false,
        }
    } else {
        InsertContext {
            url,
            source: CookieAccessSource::Document,
            browser_context: BrowserSiteContext::empty(),
            enforce_browser_policy: false,
        }
    }
}
