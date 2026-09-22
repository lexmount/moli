use url::Url;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CrossOriginEmbedderPolicy {
    #[default]
    None,
    RequireCorp,
    Credentialless,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DocumentIsolationPolicy {
    #[default]
    None,
    IsolateAndRequireCorp,
    IsolateAndCredentialless,
}

pub(crate) fn response_headers_enable_cross_origin_isolation(
    final_url: &Url,
    headers: &[(String, Vec<u8>)],
) -> bool {
    if !moli_url::is_potentially_trustworthy_url(final_url) {
        return false;
    }
    if document_isolation_policy_from_headers(headers).enables_cross_origin_isolation() {
        return true;
    }
    let coop = response_header_policy_value(headers, "cross-origin-opener-policy");
    matches!(coop.as_deref(), Some("same-origin"))
        && cross_origin_embedder_policy_from_headers(headers).enables_cross_origin_isolation()
}

pub(crate) fn cross_origin_embedder_policy_from_headers(
    headers: &[(String, Vec<u8>)],
) -> CrossOriginEmbedderPolicy {
    match response_header_policy_value(headers, "cross-origin-embedder-policy").as_deref() {
        Some("require-corp") => CrossOriginEmbedderPolicy::RequireCorp,
        Some("credentialless") => CrossOriginEmbedderPolicy::Credentialless,
        _ => CrossOriginEmbedderPolicy::None,
    }
}

pub(crate) fn document_isolation_policy_from_headers(
    headers: &[(String, Vec<u8>)],
) -> DocumentIsolationPolicy {
    match response_header_policy_value(headers, "document-isolation-policy").as_deref() {
        Some("isolate-and-require-corp") => DocumentIsolationPolicy::IsolateAndRequireCorp,
        Some("isolate-and-credentialless") => DocumentIsolationPolicy::IsolateAndCredentialless,
        _ => DocumentIsolationPolicy::None,
    }
}

impl CrossOriginEmbedderPolicy {
    pub(crate) fn enables_cross_origin_isolation(self) -> bool {
        matches!(self, Self::RequireCorp | Self::Credentialless)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RequireCorp => "require-corp",
            Self::Credentialless => "credentialless",
        }
    }
}

impl DocumentIsolationPolicy {
    pub(crate) fn enables_cross_origin_isolation(self) -> bool {
        matches!(
            self,
            Self::IsolateAndRequireCorp | Self::IsolateAndCredentialless
        )
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::IsolateAndRequireCorp => "isolate-and-require-corp",
            Self::IsolateAndCredentialless => "isolate-and-credentialless",
        }
    }
}

fn response_header_policy_value(headers: &[(String, Vec<u8>)], name: &str) -> Option<String> {
    headers
        .iter()
        .rev()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| {
            moli_fetch::decode_header_value(value)
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coop_coep_headers_enable_cross_origin_isolation_for_trustworthy_urls() {
        let url = Url::parse("https://example.test/").expect("valid url");
        let headers: Vec<(String, Vec<u8>)> = vec![
            (
                "Cross-Origin-Embedder-Policy".to_owned(),
                b"require-corp".to_vec(),
            ),
            (
                "Cross-Origin-Opener-Policy".to_owned(),
                b"same-origin".to_vec(),
            ),
        ];
        assert!(response_headers_enable_cross_origin_isolation(
            &url, &headers
        ));
    }

    #[test]
    fn cross_origin_isolation_requires_both_headers() {
        let url = Url::parse("https://example.test/").expect("valid url");
        let headers: Vec<(String, Vec<u8>)> = vec![(
            "Cross-Origin-Opener-Policy".to_owned(),
            b"same-origin".to_vec(),
        )];
        assert!(!response_headers_enable_cross_origin_isolation(
            &url, &headers
        ));
    }

    #[test]
    fn document_isolation_policy_enables_cross_origin_isolation_for_trustworthy_urls() {
        let url = Url::parse("https://example.test/").expect("valid url");
        let headers: Vec<(String, Vec<u8>)> = vec![(
            "Document-Isolation-Policy".to_owned(),
            b"isolate-and-require-corp".to_vec(),
        )];
        assert!(response_headers_enable_cross_origin_isolation(
            &url, &headers
        ));
    }

    #[test]
    fn document_isolation_policy_cross_origin_isolation_requires_trustworthy_url() {
        let url = Url::parse("http://example.test/").expect("valid url");
        let headers: Vec<(String, Vec<u8>)> = vec![(
            "Document-Isolation-Policy".to_owned(),
            b"isolate-and-credentialless".to_vec(),
        )];
        assert!(!response_headers_enable_cross_origin_isolation(
            &url, &headers
        ));
    }

    #[test]
    fn parses_cross_origin_embedder_policy_header_values() {
        assert_eq!(
            cross_origin_embedder_policy_from_headers(&[(
                "Cross-Origin-Embedder-Policy".to_owned(),
                b"require-corp; report-to=\"endpoint\"".to_vec()
            )]),
            CrossOriginEmbedderPolicy::RequireCorp
        );
        assert_eq!(
            cross_origin_embedder_policy_from_headers(&[(
                "Cross-Origin-Embedder-Policy".to_owned(),
                b"credentialless".to_vec()
            )]),
            CrossOriginEmbedderPolicy::Credentialless
        );
        assert_eq!(
            cross_origin_embedder_policy_from_headers(&[(
                "Cross-Origin-Embedder-Policy".to_owned(),
                b"invalid".to_vec()
            )]),
            CrossOriginEmbedderPolicy::None
        );
    }

    #[test]
    fn parses_document_isolation_policy_header_values() {
        assert_eq!(
            document_isolation_policy_from_headers(&[(
                "Document-Isolation-Policy".to_owned(),
                b"isolate-and-require-corp; report-to=\"endpoint\"".to_vec()
            )]),
            DocumentIsolationPolicy::IsolateAndRequireCorp
        );
        assert_eq!(
            document_isolation_policy_from_headers(&[(
                "Document-Isolation-Policy".to_owned(),
                b"isolate-and-credentialless".to_vec()
            )]),
            DocumentIsolationPolicy::IsolateAndCredentialless
        );
        assert_eq!(
            document_isolation_policy_from_headers(&[(
                "Document-Isolation-Policy".to_owned(),
                b"invalid".to_vec()
            )]),
            DocumentIsolationPolicy::None
        );
    }
}
