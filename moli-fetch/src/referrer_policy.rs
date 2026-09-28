use moli_url::same_origin;
use url::Url;

pub const DEFAULT_REFERRER_POLICY: &str = "strict-origin-when-cross-origin";

/// An empty request policy inherits the document policy before using the default.
pub fn effective_referrer_policy<'a>(
    request_policy: Option<&'a str>,
    document_policy: Option<&'a str>,
) -> &'a str {
    request_policy
        .filter(|policy| !policy.is_empty())
        .or_else(|| document_policy.filter(|policy| !policy.is_empty()))
        .unwrap_or(DEFAULT_REFERRER_POLICY)
}

const VALID_REFERRER_POLICIES: &[&str] = &[
    "no-referrer",
    "no-referrer-when-downgrade",
    "origin",
    "origin-when-cross-origin",
    "same-origin",
    "strict-origin",
    "strict-origin-when-cross-origin",
    "unsafe-url",
];

pub fn normalize_referrer_policy(raw: &str) -> Option<String> {
    raw.split(',')
        .filter_map(|token| {
            let token = token.trim().to_ascii_lowercase();
            VALID_REFERRER_POLICIES
                .contains(&token.as_str())
                .then_some(token)
        })
        .next_back()
}

pub fn response_referrer_policy_from_headers(headers: &[(String, Vec<u8>)]) -> Option<String> {
    let mut combined = String::new();
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("referrer-policy"))
    {
        if !combined.is_empty() {
            combined.push_str(", ");
        }
        combined.push_str(&crate::decode_header_value(value));
    }
    (!combined.is_empty())
        .then(|| normalize_referrer_policy(&combined))
        .flatten()
}

pub fn referrer_header_value(
    referrer_url: &Url,
    request_url: &Url,
    referrer_policy: Option<&str>,
    document_referrer_policy: Option<&str>,
) -> Option<String> {
    if !matches!(request_url.scheme(), "http" | "https") {
        return None;
    }

    referrer_value(
        referrer_url,
        request_url,
        referrer_policy,
        document_referrer_policy,
    )
}

/// Applies Referrer Policy without requiring an HTTP(S) destination.
///
/// Local navigations still use the selected value for `Document.referrer`,
/// even though they do not emit a `Referer` request header.
pub fn referrer_value(
    referrer_url: &Url,
    request_url: &Url,
    referrer_policy: Option<&str>,
    document_referrer_policy: Option<&str>,
) -> Option<String> {
    if !matches!(referrer_url.scheme(), "http" | "https") {
        return None;
    }

    let policy = effective_referrer_policy(referrer_policy, document_referrer_policy);
    let same_origin = same_origin(referrer_url, request_url);
    let downgrade = is_downgrade_request(referrer_url, request_url);

    match policy {
        "no-referrer" => None,
        "no-referrer-when-downgrade" => (!downgrade).then(|| full_referrer_url(referrer_url)),
        "origin" => Some(origin_referrer_url(referrer_url)),
        "origin-when-cross-origin" => {
            if same_origin {
                Some(full_referrer_url(referrer_url))
            } else {
                Some(origin_referrer_url(referrer_url))
            }
        }
        "same-origin" => same_origin.then(|| full_referrer_url(referrer_url)),
        "strict-origin" => (!downgrade).then(|| origin_referrer_url(referrer_url)),
        "strict-origin-when-cross-origin" => {
            if downgrade {
                None
            } else if same_origin {
                Some(full_referrer_url(referrer_url))
            } else {
                Some(origin_referrer_url(referrer_url))
            }
        }
        "unsafe-url" => Some(full_referrer_url(referrer_url)),
        _ => {
            if downgrade {
                None
            } else if same_origin {
                Some(full_referrer_url(referrer_url))
            } else {
                Some(origin_referrer_url(referrer_url))
            }
        }
    }
}

pub fn sanitized_referrer_url(url: &Url) -> String {
    full_referrer_url(url)
}

pub fn origin_referrer_url(url: &Url) -> String {
    let mut sanitized = url.clone();
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    sanitized.set_path("/");
    sanitized.set_query(None);
    sanitized.set_fragment(None);
    sanitized.to_string()
}

fn full_referrer_url(url: &Url) -> String {
    let mut sanitized = url.clone();
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    sanitized.set_fragment(None);
    let serialized = sanitized.to_string();
    if serialized.len() > 4096 {
        origin_referrer_url(url)
    } else {
        serialized
    }
}

fn is_downgrade_request(referrer_url: &Url, request_url: &Url) -> bool {
    referrer_url.scheme() == "https" && request_url.scheme() == "http"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_valid_referrer_policy_token_wins() {
        assert_eq!(
            normalize_referrer_policy("not-yet-standardized, no-referrer"),
            Some("no-referrer".to_owned())
        );
        assert_eq!(
            normalize_referrer_policy("origin, not-yet-standardized, strict-origin"),
            Some("strict-origin".to_owned())
        );
    }

    #[test]
    fn invalid_trailing_tokens_do_not_clear_previous_valid_policy() {
        assert_eq!(
            normalize_referrer_policy("same-origin, not-yet-standardized"),
            Some("same-origin".to_owned())
        );
    }

    #[test]
    fn response_referrer_policy_combines_header_instances_before_normalizing() {
        let headers: Vec<(String, Vec<u8>)> = vec![
            ("Referrer-Policy".to_owned(), b"no-referrer".to_vec()),
            ("referrer-policy".to_owned(), b"future-policy".to_vec()),
        ];

        assert_eq!(
            response_referrer_policy_from_headers(&headers),
            Some("no-referrer".to_owned())
        );
    }

    fn url(input: &str) -> Url {
        Url::parse(input).unwrap()
    }

    #[test]
    fn default_policy_is_strict_origin_when_cross_origin() {
        let source = url("https://example.com/docs/page.html?x=1#section");

        assert_eq!(
            referrer_header_value(&source, &url("https://example.com/app.js"), None, None),
            Some("https://example.com/docs/page.html?x=1".to_owned())
        );
        assert_eq!(
            referrer_header_value(&source, &url("https://cdn.example/app.js"), None, None),
            Some("https://example.com/".to_owned())
        );
        assert_eq!(
            referrer_header_value(&source, &url("http://cdn.example/app.js"), None, None),
            None
        );
    }

    #[test]
    fn policy_variants_control_referrer_surface() {
        let source = url("https://example.com/docs/page.html?x=1#section");
        let cases = [
            ("no-referrer", "https://cdn.example/app.js", None),
            (
                "no-referrer-when-downgrade",
                "http://cdn.example/app.js",
                None,
            ),
            (
                "origin",
                "http://cdn.example/app.js",
                Some("https://example.com/"),
            ),
            (
                "origin-when-cross-origin",
                "https://cdn.example/app.js",
                Some("https://example.com/"),
            ),
            ("same-origin", "https://cdn.example/app.js", None),
            ("strict-origin", "http://cdn.example/app.js", None),
            (
                "unsafe-url",
                "http://cdn.example/app.js",
                Some("https://example.com/docs/page.html?x=1"),
            ),
        ];

        for (policy, request_url, expected) in cases {
            assert_eq!(
                referrer_header_value(&source, &url(request_url), Some(policy), None).as_deref(),
                expected,
                "unexpected referer for policy {policy}"
            );
        }
    }

    #[test]
    fn element_policy_overrides_document_policy() {
        let source = url("https://example.com/docs/page.html?x=1#section");
        let request = url("https://cdn.example/app.js");

        assert_eq!(
            referrer_header_value(&source, &request, None, Some("no-referrer")),
            None
        );
        assert_eq!(
            referrer_header_value(&source, &request, Some("origin"), Some("no-referrer")),
            Some("https://example.com/".to_owned())
        );
    }

    #[test]
    fn sanitizes_userinfo_fragment_and_long_referrers() {
        assert_eq!(
            sanitized_referrer_url(&url("https://user:pass@example.com/docs?a=1#frag")),
            "https://example.com/docs?a=1"
        );

        let long_path = format!("https://example.com/docs/{}?x=1#section", "a".repeat(4100));
        assert_eq!(
            referrer_header_value(
                &url(&long_path),
                &url("https://example.com/app.js"),
                Some("unsafe-url"),
                None,
            ),
            Some("https://example.com/".to_owned())
        );
    }

    #[test]
    fn non_http_contexts_do_not_emit_referrers() {
        assert_eq!(
            referrer_header_value(
                &url("data:text/plain,hello"),
                &url("https://example.com/app.js"),
                None,
                None,
            ),
            None
        );
        assert_eq!(
            referrer_header_value(
                &url("https://example.com/page"),
                &url("data:text/plain,hello"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn non_http_navigation_target_still_has_a_policy_selected_referrer() {
        let source = url("https://example.com/docs/page.html?x=1#section");
        let target = url("about:blank");

        assert_eq!(
            referrer_value(&source, &target, None, None),
            Some("https://example.com/".to_owned())
        );
        assert_eq!(referrer_header_value(&source, &target, None, None), None);
    }
}

#[cfg(test)]
mod empty_policy_tests {
    use super::*;

    #[test]
    fn empty_request_policy_inherits_the_document_policy() {
        let source = Url::parse("https://origin.test/private?token=value").unwrap();
        let target = Url::parse("https://destination.test/resource").unwrap();
        for request_policy in [None, Some("")] {
            assert_eq!(
                referrer_value(&source, &target, request_policy, Some("no-referrer")),
                None
            );
            assert_eq!(
                referrer_value(&source, &target, request_policy, Some("unsafe-url")),
                Some(source.to_string())
            );
        }
        assert_eq!(
            referrer_value(&source, &target, Some("origin"), Some("no-referrer")),
            Some("https://origin.test/".to_owned())
        );
        for document_policy in [None, Some("")] {
            assert_eq!(
                referrer_value(&source, &target, Some(""), document_policy),
                Some("https://origin.test/".to_owned())
            );
        }
    }
}
