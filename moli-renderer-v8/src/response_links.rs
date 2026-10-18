use std::collections::HashSet;

use moli_header_field::{HeaderFieldTokenMode, HeaderFieldTokenizer};
use url::Url;

use crate::planning::ScriptFetchMetadata;

/// Response hints retain the response URL as their base, independently of
/// subsequent HTML base elements. They do not create DOM nodes.
#[derive(Debug, Default)]
pub(crate) struct DocumentResponseLinks {
    pub(crate) dictionaries: Vec<Url>,
    pub(crate) scripts: Vec<ResponseScriptPreload>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseScriptPreload {
    pub(crate) url: Url,
    pub(crate) metadata: ScriptFetchMetadata,
}

impl DocumentResponseLinks {
    pub(crate) fn parse(base: &Url, headers: &[(String, Vec<u8>)]) -> Self {
        let mut result = Self::default();
        let mut dictionaries = HashSet::new();
        for (_, value) in headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("link"))
        {
            let value = moli_header_field::decode_header_value(value);
            let mut parser = HeaderFieldTokenizer::new(&value);
            while !parser.is_consumed() {
                let Some(link) = parse_link(&value, &mut parser) else {
                    break;
                };
                if link.parameter("anchor").is_none()
                    && let Ok(url) = base.join(link.target)
                    && matches!(url.scheme(), "http" | "https")
                {
                    if link.has_relation("compression-dictionary") {
                        let mut dictionary_url = url.clone();
                        dictionary_url.set_fragment(None);
                        if dictionaries.insert(dictionary_url.clone()) {
                            result.dictionaries.push(dictionary_url);
                        }
                    }
                    if link.has_relation("preload")
                        && link
                            .parameter("as")
                            .is_some_and(|value| value.eq_ignore_ascii_case("script"))
                        && link.parameter("media").is_none_or(|value| {
                            crate::modulepreload::modulepreload_media_attribute_matches(Some(value))
                        })
                        && crate::link_as::LinkAsDestination::Script
                            .preload_type_matches(link.parameter("type"))
                    {
                        let preload = ResponseScriptPreload {
                            url,
                            metadata: ScriptFetchMetadata::from_script_attributes(
                                link.parameter("crossorigin"),
                                link.parameter("referrerpolicy"),
                                None,
                                link.parameter("integrity"),
                                link.parameter("nonce"),
                                link.parameter("fetchpriority"),
                            ),
                        };
                        if !result.scripts.contains(&preload) {
                            result.scripts.push(preload);
                        }
                    }
                }
                if !parser.consume(',') {
                    break;
                }
            }
        }
        result
    }
}

struct ResponseLink<'a> {
    target: &'a str,
    parameters: Vec<(&'a str, std::borrow::Cow<'a, str>)>,
}

impl ResponseLink<'_> {
    fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_ref())
    }

    fn has_relation(&self, relation: &str) -> bool {
        self.parameter("rel").is_some_and(|value| {
            value
                .split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case(relation))
        })
    }
}

fn parse_link<'a>(
    input: &'a str,
    parser: &mut HeaderFieldTokenizer<'a>,
) -> Option<ResponseLink<'a>> {
    let start = parser.byte_index() + 1;
    if !parser.consume('<') {
        return None;
    }
    parser.consume_before_any_char_match(&['>']);
    let target = &input[start..parser.byte_index()];
    if !parser.consume('>')
        || target
            .bytes()
            .any(|byte| byte <= b' ' || matches!(byte, b'<' | 0x7f))
    {
        return None;
    }
    let mut parameters = Vec::new();
    while parser.consume(';') {
        let name = parser.consume_token(HeaderFieldTokenMode::Normal)?;
        let value = if parser.consume('=') {
            parser.consume_token_or_quoted_string(HeaderFieldTokenMode::Normal)?
        } else {
            "".into()
        };
        if value
            .chars()
            .any(|character| character.is_ascii_control() && character != '\t')
        {
            return None;
        }
        // RFC 8288 parameters are first-wins, including rel and nonce. Keeping
        // quoted values intact prevents commas from manufacturing new hints.
        if !parameters
            .iter()
            .any(|(key, _): &(&str, std::borrow::Cow<'_, str>)| key.eq_ignore_ascii_case(name))
        {
            parameters.push((name, value));
        }
    }
    if !parser.is_consumed() && !input[parser.byte_index()..].starts_with(',') {
        return None;
    }
    Some(ResponseLink { target, parameters })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_preloads_preserve_first_parameters_and_quoted_delimiters() {
        let base = Url::parse("https://example.test/pages/document").unwrap();
        let headers = vec![("Link".into(), br#"</ignored>; rel=alternate; rel=preload; as=script, <../script,one.js#fragment>; title="fake, rel=modulepreload; as=style"; REL="alternate PRELOAD compression-dictionary"; AS=SCRIPT; nonce="a\"z"; nonce=wrong; crossorigin=anonymous; referrerpolicy=no-referrer; integrity="sha256-value"; fetchpriority=high"#.to_vec())];
        let parsed = DocumentResponseLinks::parse(&base, &headers);
        assert_eq!(
            parsed.dictionaries,
            vec![base.join("../script,one.js").unwrap()]
        );
        assert_eq!(parsed.scripts.len(), 1);
        let preload = &parsed.scripts[0];
        assert_eq!(
            preload.url.as_str(),
            "https://example.test/script,one.js#fragment"
        );
        assert_eq!(preload.metadata.nonce.as_deref(), Some("a\"z"));
        assert_eq!(preload.metadata.cross_origin.as_deref(), Some("anonymous"));
        assert_eq!(
            preload.metadata.referrer_policy.as_deref(),
            Some("no-referrer")
        );
        assert_eq!(preload.metadata.integrity.as_deref(), Some("sha256-value"));
        assert_eq!(
            preload.metadata.fetch_priority,
            Some(moli_fetch::FetchPriorityHint::High)
        );
    }

    #[test]
    fn response_preload_eligibility_precedes_fetch_and_identical_hints_are_deduplicated() {
        let base = Url::parse("https://example.test/page").unwrap();
        let headers = vec![
            ("link".into(), br#"</anchor>; rel=preload; as=script; anchor="/other", </missing>; rel=preload, </image>; rel=preload; as=image, </type>; rel=preload; as=script; type="image/png", </media>; rel=preload; as=script; media="not all", <data:text/javascript,0>; rel=preload; as=script, </ok>; rel=preload; as=script; nonce=first"#.to_vec()),
            ("LINK".into(), br#"</ok>; rel=preload; as=script; nonce=first, </ok#same>; rel=preload; as=script; nonce=first, </ok>; rel=preload; as=script; nonce=second"#.to_vec()),
        ];
        let parsed = DocumentResponseLinks::parse(&base, &headers);
        assert_eq!(parsed.scripts.len(), 3);
        assert_eq!(parsed.scripts[0].url, base.join("/ok").unwrap());
        assert_eq!(parsed.scripts[1].url, base.join("/ok#same").unwrap());
        assert_eq!(parsed.scripts[2].metadata.nonce.as_deref(), Some("second"));
    }

    #[test]
    fn response_preload_policy_retains_nonce_integrity_and_non_parser_metadata() {
        use crate::content_security_policy::ContentSecurityPolicyRedirectStatus as Redirect;
        use crate::document_runtime::DocumentRequestPolicySnapshot;

        let document = Url::parse("https://example.test/page").unwrap();
        let source = Url::parse("https://cdn.test/script.js").unwrap();
        let policy = DocumentRequestPolicySnapshot::from_policies(
            vec!["script-src 'nonce-az'".into()],
            Vec::new(),
            Default::default(),
        );
        assert!(
            !policy
                .clone()
                .for_script_preload(ScriptFetchMetadata::default())
                .check_url(&document, &source, Redirect::NoRedirect)
                .has_no_violations()
        );
        let nonce =
            ScriptFetchMetadata::from_script_attributes(None, None, None, None, Some("az"), None);
        let allowed = policy.for_script_preload(nonce);
        assert!(
            allowed
                .check_url(&document, &source, Redirect::NoRedirect)
                .has_no_violations()
        );
        assert!(
            allowed
                .check_redirect(&document, &source)
                .has_no_violations()
        );

        let dynamic = DocumentRequestPolicySnapshot::from_policies(
            vec!["script-src 'nonce-az' 'strict-dynamic'".into()],
            Vec::new(),
            Default::default(),
        )
        .for_script_preload(ScriptFetchMetadata::default());
        assert!(
            dynamic
                .check_url(&document, &source, Redirect::NoRedirect)
                .has_no_violations()
        );
    }
}
