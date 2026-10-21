mod sources;
mod subsumption;

use crate::{ContentSecurityPolicy, IframeCspAttribute};
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequiredCspResponse {
    Satisfied,
    Enforce(Url),
    Blocked,
}

impl IframeCspAttribute {
    pub fn subsumes(&self, policies: &[ContentSecurityPolicy], origin: &Url) -> bool {
        subsumption::subsumes(self.policy(), policies, origin)
    }

    pub fn response_disposition(
        &self,
        request_origin: &str,
        response_url: &Url,
        headers: &[(String, Vec<u8>)],
        response_policies: &[ContentSecurityPolicy],
    ) -> RequiredCspResponse {
        if matches!(
            response_url.scheme(),
            "about" | "data" | "blob" | "file" | "filesystem"
        ) {
            // Local Documents inherit the embedder's interpretation of 'self',
            // including data URLs whose own origin is opaque.
            return RequiredCspResponse::Enforce(
                Url::parse(request_origin).unwrap_or_else(|_| response_url.clone()),
            );
        }
        // Explicit acceptance installs the requirement even if the response
        // also happens to subsume it. Only implicit acceptance uses the
        // response policy alone (and keeps nonce values private).
        if allows_blanket_enforcement(request_origin, headers) {
            RequiredCspResponse::Enforce(response_url.clone())
        } else if self.subsumes(response_policies, response_url) {
            RequiredCspResponse::Satisfied
        } else {
            RequiredCspResponse::Blocked
        }
    }
}

fn allows_blanket_enforcement(request_origin: &str, headers: &[(String, Vec<u8>)]) -> bool {
    let mut values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Allow-CSP-From"));
    let Some((_, value)) = values.next() else {
        return false;
    };
    if values.next().is_some() {
        return false;
    }
    let Ok(value) = std::str::from_utf8(value) else {
        return false;
    };
    let value = value.trim_ascii();
    value == "*" || (request_origin != "null" && value == request_origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_policies_compare_effective_source_intersections_without_disclosing_nonces() {
        let origin = Url::parse("https://embedee.test/frame").unwrap();
        for (required, returned, expected) in [
            (
                "img-src 'none'",
                vec!["img-src http://example.test", "img-src wss://example.test"],
                false,
            ),
            (
                "img-src https://example.test",
                vec!["img-src http://example.test", "img-src wss://example.test"],
                true,
            ),
            (
                "img-src http://example.test",
                vec!["img-src http://example.test:80"],
                true,
            ),
            (
                "img-src http://example.test:36",
                vec!["img-src https://example.test:36"],
                true,
            ),
            (
                "img-src http://example.test",
                vec!["img-src https://example.test:80"],
                false,
            ),
            (
                "img-src http://example.test:80",
                vec!["img-src https://example.test:80"],
                true,
            ),
            (
                "script-src 'NoNcE-parent'",
                vec!["script-src 'nonce-child'"],
                true,
            ),
            (
                "script-src 'none'",
                vec![
                    "script-src 'nonce-a'",
                    "script-src 'sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU='",
                ],
                false,
            ),
            (
                "script-src 'nonce-parent'",
                vec![
                    "script-src 'nonce-child' 'unsafe-inline'",
                    "script-src 'unsafe-inline'",
                ],
                true,
            ),
            (
                "script-src 'strict-dynamic' 'nonce-parent'",
                vec!["script-src 'nonce-child'", "script-src https://cdn.test"],
                true,
            ),
            (
                "script-src 'nonce-parent'",
                vec![
                    "script-src 'strict-dynamic' 'nonce-child'",
                    "script-src https://cdn.test",
                ],
                false,
            ),
            (
                "script-src 'nonce-parent'",
                vec!["script-src 'nonce-child'"],
                true,
            ),
            (
                "script-src 'nonce-parent'",
                vec!["script-src 'unsafe-inline'"],
                false,
            ),
            (
                "script-src 'none'",
                vec!["script-src 'nonce-a'", "script-src 'nonce-b'"],
                true,
            ),
            (
                "img-src https://*.example.test/asset/",
                vec![
                    "img-src https://*.test:*",
                    "default-src https://cdn.example.test/asset/icon",
                ],
                true,
            ),
            (
                "img-src https://*.example.test/asset/",
                vec![
                    "img-src https://*.test:*",
                    "default-src https://cdn.example.test/other/",
                ],
                false,
            ),
            (
                "img-src https://example.test/a%2Fb",
                vec!["img-src https://example.test/a/b"],
                false,
            ),
            (
                "frame-src http://example.test:80",
                vec!["child-src https://example.test:443"],
                true,
            ),
            (
                "frame-src https://example.test:443",
                vec!["child-src http://example.test:80"],
                false,
            ),
            (
                "script-src-attr 'none'",
                vec!["default-src 'none'; script-src 'unsafe-inline'"],
                false,
            ),
            ("plugin-types application/pdf", vec![], true),
            ("default-src 'none'", vec![], false),
        ] {
            let policy = IframeCspAttribute::parse(required).unwrap();
            let returned = returned
                .into_iter()
                .map(ContentSecurityPolicy::from)
                .collect::<Vec<_>>();
            assert_eq!(
                policy.subsumes(&returned, &origin),
                expected,
                "{required:?}, {returned:?}"
            );
        }
    }

    #[test]
    fn required_policy_needs_explicit_response_acceptance_or_enforced_subsumption() {
        let required = IframeCspAttribute::parse("script-src 'nonce-parent'").unwrap();
        let origin = Url::parse("https://embedee.test/frame").unwrap();
        for (headers, expected) in [
            (vec![], RequiredCspResponse::Blocked),
            (
                vec![("Allow-CSP-From", b"*".as_slice())],
                RequiredCspResponse::Enforce(origin.clone()),
            ),
            (
                vec![("Allow-CSP-From", b"https://embedder.test".as_slice())],
                RequiredCspResponse::Enforce(origin.clone()),
            ),
            (
                vec![("Allow-CSP-From", b"https://other.test".as_slice())],
                RequiredCspResponse::Blocked,
            ),
            (
                vec![("Allow-CSP-From", b"*, https://embedder.test".as_slice())],
                RequiredCspResponse::Blocked,
            ),
            (
                vec![
                    ("Allow-CSP-From", b"*".as_slice()),
                    ("allow-csp-from", b"*".as_slice()),
                ],
                RequiredCspResponse::Blocked,
            ),
            (
                vec![("Allow-CSP-From", b"*\xff".as_slice())],
                RequiredCspResponse::Blocked,
            ),
        ] {
            let headers = headers
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_vec()))
                .collect::<Vec<_>>();
            assert_eq!(
                required.response_disposition("https://embedder.test", &origin, &headers, &[]),
                expected
            );
        }
        assert_eq!(
            required.response_disposition(
                "null",
                &origin,
                &[("Allow-CSP-From".to_owned(), b"null".to_vec())],
                &[]
            ),
            RequiredCspResponse::Blocked
        );
        assert_eq!(
            required.response_disposition(
                "https://embedder.test",
                &origin,
                &[],
                &["script-src 'nonce-response'".into()]
            ),
            RequiredCspResponse::Satisfied
        );
        assert_eq!(
            required.response_disposition(
                "https://embedder.test",
                &origin,
                &[("Allow-CSP-From".to_owned(), b"*".to_vec())],
                &["script-src 'nonce-response'".into()]
            ),
            RequiredCspResponse::Enforce(origin.clone())
        );
        assert_eq!(
            required.response_disposition(
                "https://embedder.test",
                &Url::parse("data:text/html,hello").unwrap(),
                &[],
                &[]
            ),
            RequiredCspResponse::Enforce(Url::parse("https://embedder.test").unwrap())
        );
    }
}
