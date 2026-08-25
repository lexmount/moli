use httpdate::parse_http_date;
use moli_header_field::{split_outside_quoted_strings, unquote_parameter_value};
use url::Url;

/// Cache storage and freshness policy derived from response headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpCacheResponsePolicy {
    pub store: bool,
    pub expires_at_unix_ms: Option<u64>,
}

/// Parses the subset of HTTP response cache policy currently enforced by
/// Moli's conservative disk cache.
pub fn response_cache_policy(headers: &[(String, String)]) -> HttpCacheResponsePolicy {
    let mut store = true;
    let mut requires_validation = false;
    let mut max_age_seconds = None;
    let mut expires_header_unix_ms = None;
    let mut date_unix_ms = None;
    let mut age_seconds = 0u64;

    for (name, value) in headers {
        if name.eq_ignore_ascii_case("cache-control") {
            // A quoted directive argument may contain `,`, as in
            // `private="Set-Cookie, X-Auth"`, so splitting on every comma
            // would read the argument's own text as further directives.
            for directive in split_outside_quoted_strings(value, ',')
                .into_iter()
                .map(str::trim)
            {
                let (directive_name, directive_value) = directive
                    .split_once('=')
                    .map(|(name, value)| (name.trim(), Some(unquote_parameter_value(value.trim()))))
                    .unwrap_or((directive, None));
                let directive_value = directive_value.as_deref();
                if directive_name.eq_ignore_ascii_case("no-store")
                    || directive_name.eq_ignore_ascii_case("private")
                {
                    store = false;
                } else if directive_name.eq_ignore_ascii_case("no-cache") {
                    requires_validation = true;
                } else if directive_name.eq_ignore_ascii_case("max-age")
                    && let Some(raw) = directive_value
                    && let Ok(seconds) = raw.parse::<u64>()
                {
                    max_age_seconds = Some(seconds);
                }
            }
        } else if name.eq_ignore_ascii_case("pragma") {
            // HTTP/1.0 caches treat Pragma: no-cache like a storage opt-out.
            if value
                .split(',')
                .map(str::trim)
                .any(|directive| directive.eq_ignore_ascii_case("no-cache"))
            {
                store = false;
            }
        } else if name.eq_ignore_ascii_case("expires")
            && let Ok(expires_at) = parse_http_date(value)
            && let Ok(duration) = expires_at.duration_since(std::time::UNIX_EPOCH)
        {
            expires_header_unix_ms = Some(duration.as_millis() as u64);
        } else if name.eq_ignore_ascii_case("date")
            && let Ok(date) = parse_http_date(value)
            && let Ok(duration) = date.duration_since(std::time::UNIX_EPOCH)
        {
            date_unix_ms = Some(duration.as_millis() as u64);
        } else if name.eq_ignore_ascii_case("age")
            && let Ok(seconds) = value.trim().parse::<u64>()
        {
            age_seconds = seconds;
        }
    }

    // Cache-Control: no-cache is not a storage opt-out. It permits storing the
    // body, but every reuse must validate with the origin server first.
    let expires_at_unix_ms = (!requires_validation)
        .then(|| {
            cache_expires_at_unix_ms(
                unix_now_ms(),
                max_age_seconds,
                expires_header_unix_ms,
                date_unix_ms,
                age_seconds,
            )
        })
        .flatten();

    HttpCacheResponsePolicy {
        store,
        expires_at_unix_ms,
    }
}

/// Returns the response cache policy only when response parts are safe to store
/// before request-specific `Vary` matching is considered.
pub fn cacheable_response_parts_policy(
    request_url: &Url,
    final_url: &Url,
    status: u16,
    headers: &[(String, String)],
    redirected: bool,
) -> Option<HttpCacheResponsePolicy> {
    if !cacheable_response_status(status)
        || redirected
        || final_url != request_url
        || headers.iter().any(|(name, _)| name == "set-cookie")
    {
        return None;
    }
    let policy = response_cache_policy(headers);
    policy.store.then_some(policy)
}

fn cacheable_response_status(status: u16) -> bool {
    (200..300).contains(&status) || matches!(status, 301 | 302 | 303 | 307 | 308)
}

pub fn cached_response_is_stale(expires_at_unix_ms: Option<u64>, force_validate: bool) -> bool {
    cached_response_is_stale_at(unix_now_ms(), expires_at_unix_ms, force_validate)
}

fn cached_response_is_stale_at(
    now_ms: u64,
    expires_at_unix_ms: Option<u64>,
    force_validate: bool,
) -> bool {
    if force_validate {
        return true;
    }
    expires_at_unix_ms.is_none_or(|expires_at| now_ms >= expires_at)
}

pub fn cached_response_is_fresh_immutable(
    headers: &[(String, String)],
    expires_at_unix_ms: Option<u64>,
) -> bool {
    cached_response_is_fresh_immutable_at(unix_now_ms(), headers, expires_at_unix_ms)
}

fn cached_response_is_fresh_immutable_at(
    now_ms: u64,
    headers: &[(String, String)],
    expires_at_unix_ms: Option<u64>,
) -> bool {
    expires_at_unix_ms.is_some_and(|expires_at| now_ms < expires_at)
        && headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
            .flat_map(|(_, value)| split_outside_quoted_strings(value, ','))
            .map(str::trim)
            .any(|directive| {
                directive
                    .split_once('=')
                    .map(|(name, _)| name.trim())
                    .unwrap_or(directive)
                    .eq_ignore_ascii_case("immutable")
            })
}

pub fn request_header_requires_validation(name: &str, value: &str) -> bool {
    if name.eq_ignore_ascii_case("cache-control") {
        request_cache_control_requires_validation(value)
    } else if name.eq_ignore_ascii_case("pragma") {
        request_pragma_requires_validation(value)
    } else {
        false
    }
}

pub fn request_cache_control_requires_validation(value: &str) -> bool {
    split_outside_quoted_strings(value, ',')
        .into_iter()
        .map(str::trim)
        .any(|directive| {
            let (name, value) = directive
                .split_once('=')
                .map(|(name, value)| (name.trim(), Some(unquote_parameter_value(value.trim()))))
                .unwrap_or((directive, None));
            name.eq_ignore_ascii_case("no-cache")
                || (name.eq_ignore_ascii_case("max-age") && value.as_deref() == Some("0"))
        })
}

pub fn request_pragma_requires_validation(value: &str) -> bool {
    split_outside_quoted_strings(value, ',')
        .into_iter()
        .map(str::trim)
        .any(|directive| directive.eq_ignore_ascii_case("no-cache"))
}

pub fn cache_expires_at_unix_ms(
    now_ms: u64,
    max_age_seconds: Option<u64>,
    expires_header_unix_ms: Option<u64>,
    date_unix_ms: Option<u64>,
    age_seconds: u64,
) -> Option<u64> {
    if let Some(max_age_seconds) = max_age_seconds {
        let apparent_age_ms = date_unix_ms
            .map(|date_ms| now_ms.saturating_sub(date_ms))
            .unwrap_or_default();
        let response_age_ms = apparent_age_ms.max(age_seconds.saturating_mul(1000));
        let freshness_ms = max_age_seconds
            .saturating_mul(1000)
            .saturating_sub(response_age_ms);
        return Some(now_ms.saturating_add(freshness_ms));
    }
    expires_header_unix_ms
}

pub fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_requires_expiration_to_be_strictly_after_now() {
        let immutable_headers = vec![(
            "cache-control".to_owned(),
            "max-age=60, immutable".to_owned(),
        )];

        assert!(!cached_response_is_stale_at(99, Some(100), false));
        assert!(cached_response_is_fresh_immutable_at(
            99,
            &immutable_headers,
            Some(100)
        ));

        assert!(cached_response_is_stale_at(100, Some(100), false));
        assert!(!cached_response_is_fresh_immutable_at(
            100,
            &immutable_headers,
            Some(100)
        ));
    }

    #[test]
    fn quoted_directive_arguments_do_not_leak_further_directives() {
        // RFC 9111 lets `private` and `no-cache` carry a quoted field list,
        // and that list may contain commas. Splitting on every comma read the
        // list's own text as further directives.
        assert!(!request_cache_control_requires_validation(
            "max-age=600, private=\"Set-Cookie, X-Auth\""
        ));
        assert!(request_cache_control_requires_validation(
            "max-age=600, no-cache=\"Set-Cookie, X-Auth\""
        ));
        // A directive name appearing inside a quoted argument is not a
        // directive of its own.
        assert!(!request_cache_control_requires_validation(
            "max-age=600, private=\"a, no-cache, b\""
        ));
    }

    #[test]
    fn response_policy_ignores_directives_inside_a_quoted_argument() {
        let headers = vec![(
            "Cache-Control".to_owned(),
            "max-age=600, community=\"x, no-store, y\"".to_owned(),
        )];

        assert!(response_cache_policy(&headers).store);
    }

    #[test]
    fn response_policy_still_honours_real_directives() {
        for value in [
            "no-store",
            "private",
            "max-age=0, no-store",
            "no-store, max-age=600",
        ] {
            let headers = vec![("Cache-Control".to_owned(), value.to_owned())];
            assert!(!response_cache_policy(&headers).store, "value={value}");
        }
        let headers = vec![(
            "Cache-Control".to_owned(),
            "private=\"Set-Cookie\"".to_owned(),
        )];
        assert!(!response_cache_policy(&headers).store);
    }

    #[test]
    fn quoted_pragma_arguments_do_not_leak_directives() {
        assert!(!request_pragma_requires_validation(
            "token=\"a, no-cache, b\""
        ));
        assert!(request_pragma_requires_validation("no-cache"));
    }

    #[test]
    fn a_trailing_backslash_does_not_manufacture_a_freshness_lifetime() {
        // `31536000\` is malformed and must not parse as one year.
        let headers = vec![(
            "Cache-Control".to_owned(),
            "max-age=\"31536000\\".to_owned(),
        )];

        assert_eq!(response_cache_policy(&headers).expires_at_unix_ms, None);
    }
}
