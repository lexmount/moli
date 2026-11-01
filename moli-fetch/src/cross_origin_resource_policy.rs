use moli_site::registrable_domain_for_host;
use moli_url::WebOrigin;
use moli_web_mime::response_header_values;
use url::{Host, Url};

/// The explicit CORP response header. COEP/DIP defaulting is a separate decision
/// made with the consumer's policy and the response's credentials state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CrossOriginResourcePolicy {
    SameOrigin,
    SameSite,
    CrossOrigin,
}

impl CrossOriginResourcePolicy {
    pub fn from_headers(headers: &[(String, Vec<u8>)]) -> Option<Self> {
        // Repeated field lines form a comma-separated value, which is invalid.
        // Tokens are case-sensitive and only HTTP whitespace is stripped.
        let values = response_header_values(headers, "cross-origin-resource-policy");
        let [value] = values.as_slice() else {
            return None;
        };
        match value.trim_matches([' ', '\t']) {
            "same-origin" => Some(Self::SameOrigin),
            "same-site" => Some(Self::SameSite),
            "cross-origin" => Some(Self::CrossOrigin),
            _ => None,
        }
    }

    pub fn check(self, origin: &WebOrigin, response_url: &Url) -> Result<(), String> {
        if !matches!(response_url.scheme(), "http" | "https")
            || origin.same_origin_url(response_url)
        {
            return Ok(());
        }
        let label = match self {
            Self::CrossOrigin => return Ok(()),
            Self::SameSite if allows_same_site(origin, response_url) => return Ok(()),
            Self::SameSite => "same-site",
            Self::SameOrigin => "same-origin",
        };
        Err(format!(
            "Cross-Origin-Resource-Policy check failed: `{label}` does not allow {} to load {response_url}",
            origin.ascii_serialization()
        ))
    }
}

fn allows_same_site(origin: &WebOrigin, target: &Url) -> bool {
    let Ok(initiator) = Url::parse(origin.ascii_serialization()) else {
        return false;
    };
    let (Some(initiator_host), Some(target_host)) = (initiator.host(), target.host()) else {
        return false;
    };
    let same_registrable_domain = match (&initiator_host, &target_host) {
        (Host::Domain(left), Host::Domain(right)) => registrable_domain_for_host(left)
            .zip(registrable_domain_for_host(right))
            .is_some_and(|(left, right)| left == right),
        _ => false,
    };
    // CORP uses schemeless sites plus an asymmetric HTTPS requirement. Exact
    // hosts cover IP/public-suffix/single-label cases without equating nulls.
    (initiator_host == target_host || same_registrable_domain)
        && (initiator.scheme() == "https" || target.scheme() != "https")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corp_tokens_are_case_sensitive_and_repeated_lines_are_invalid() {
        for (value, expected) in [
            ("same-origin", Some(CrossOriginResourcePolicy::SameOrigin)),
            ("\t same-site \t", Some(CrossOriginResourcePolicy::SameSite)),
            ("cross-origin", Some(CrossOriginResourcePolicy::CrossOrigin)),
            ("Same-Origin", None),
            ("same-origin, same-site", None),
            ("same-origin\n", None),
        ] {
            assert_eq!(
                CrossOriginResourcePolicy::from_headers(&[(
                    "cRoSs-OrIgIn-ReSoUrCe-PoLiCy".into(),
                    value.as_bytes().to_vec()
                )]),
                expected
            );
        }
        assert_eq!(
            CrossOriginResourcePolicy::from_headers(&[
                (
                    "Cross-Origin-Resource-Policy".into(),
                    b"same-origin".to_vec()
                ),
                (
                    "Cross-Origin-Resource-Policy".into(),
                    b"same-origin".to_vec()
                ),
            ]),
            None
        );
    }

    #[test]
    fn same_site_uses_corp_https_state_and_private_registries() {
        for (origin, target, allowed) in [
            (
                "https://app.example.test",
                "http://cdn.example.test/a",
                true,
            ),
            (
                "http://app.example.test",
                "https://cdn.example.test/a",
                false,
            ),
            (
                "http://app.example.test:80",
                "http://cdn.example.test:81/a",
                true,
            ),
            ("https://a.github.io", "https://cdn.a.github.io/a", true),
            ("https://a.github.io", "https://b.github.io/a", false),
            ("http://127.0.0.1:80", "http://127.0.0.1:81/a", true),
            ("http://[::1]:80", "http://[::1]:81/a", true),
            ("http://127.0.0.1:80", "http://127.0.0.1:80/a", true),
            ("http://example.test", "http://different.test/a", false),
            ("http://localhost:80", "http://localhost:81/a", true),
            ("http://localhost", "http://intranet/a", false),
            ("http://com", "http://net/a", false),
            ("http://.com", "http://com/a", false),
            ("http://127.0.0.1", "http://127.0.0.2/a", false),
        ] {
            let origin = WebOrigin::from_url(&Url::parse(origin).unwrap());
            assert_eq!(
                CrossOriginResourcePolicy::SameSite
                    .check(&origin, &Url::parse(target).unwrap())
                    .is_ok(),
                allowed,
                "{} -> {target}",
                origin.ascii_serialization()
            );
        }
        assert!(
            CrossOriginResourcePolicy::SameSite
                .check(
                    &WebOrigin::Opaque,
                    &Url::parse("https://example.test").unwrap()
                )
                .is_err()
        );
    }
}
