use std::collections::HashSet;

use moli_header_field::{HeaderFieldTokenMode, HeaderFieldTokenizer};
use url::Url;

use crate::modulepreload::{ModulepreloadAsState, ModulepreloadFetchCandidate};
use crate::planning::ScriptFetchMetadata;

/// Response hints retain the response URL as their base, independently of
/// subsequent HTML base elements. They do not create DOM nodes.
#[derive(Debug, Default)]
pub(crate) struct DocumentResponseLinks {
    pub(crate) dictionaries: Vec<Url>,
    pub(crate) scripts: Vec<ResponseScriptPreload>,
    pub(crate) styles: Vec<ResponseStylePreload>,
    pub(crate) modules: Vec<ResponseModulepreload>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseScriptPreload {
    pub(crate) url: Url,
    pub(crate) metadata: ScriptFetchMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseStylePreload {
    pub(crate) url: Url,
    pub(crate) options: crate::stylesheet_blocking::StylesheetFetchOptions,
}

#[derive(Debug, Clone)]
pub(crate) enum ResponsePreloadOptions {
    Script(ScriptFetchMetadata),
    Style(crate::stylesheet_blocking::StylesheetFetchOptions),
}

impl ResponsePreloadOptions {
    pub(crate) fn destination(&self) -> moli_web_mime::FetchDestination {
        match self {
            Self::Script(_) => moli_web_mime::FetchDestination::Script,
            Self::Style(_) => moli_web_mime::FetchDestination::Style,
        }
    }

    pub(crate) fn integrity(&self) -> Option<&str> {
        match self {
            Self::Script(metadata) => metadata.integrity.as_deref(),
            Self::Style(options) => options.integrity(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseModulepreload {
    pub(crate) url: Url,
    pub(crate) destination: ModulepreloadAsState,
    pub(crate) metadata: ScriptFetchMetadata,
}

impl ResponseModulepreload {
    pub(crate) fn fetch_candidate(
        &self,
        document_url: &Url,
        document_referrer_policy: Option<String>,
    ) -> Option<ModulepreloadFetchCandidate> {
        crate::modulepreload::modulepreload_fetch_candidate_for_options(
            self.destination,
            self.metadata.clone(),
            self.url.clone(),
            document_url,
            document_referrer_policy,
        )
    }
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
                            url: url.clone(),
                            metadata: link.script_fetch_metadata(),
                        };
                        if !result.scripts.contains(&preload) {
                            result.scripts.push(preload);
                        }
                    }
                    if link.has_relation("preload")
                        && link
                            .parameter("as")
                            .is_some_and(|value| value.eq_ignore_ascii_case("style"))
                        && link.parameter("media").is_none_or(|value| {
                            crate::modulepreload::modulepreload_media_attribute_matches(Some(value))
                        })
                        && crate::link_as::LinkAsDestination::Style
                            .preload_type_matches(link.parameter("type"))
                    {
                        let preload = ResponseStylePreload {
                            url: url.clone(),
                            options: crate::stylesheet_blocking::StylesheetFetchOptions::from_link_attributes(
                                link.parameter("crossorigin"),
                                link.parameter("referrerpolicy"),
                                link.parameter("integrity"),
                                link.parameter("nonce"),
                                None,
                                link.parameter("fetchpriority"),
                            ),
                        };
                        if !result.styles.contains(&preload) {
                            result.styles.push(preload);
                        }
                    }
                    let destination = crate::modulepreload::modulepreload_as_state_from_attribute(
                        link.parameter("as"),
                    );
                    if link.has_relation("modulepreload")
                        && destination != ModulepreloadAsState::Invalid
                        && crate::modulepreload::modulepreload_media_attribute_matches(
                            link.parameter("media"),
                        )
                    {
                        let preload = ResponseModulepreload {
                            url,
                            destination,
                            metadata: link.script_fetch_metadata(),
                        };
                        if !result.modules.contains(&preload) {
                            result.modules.push(preload);
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
    fn script_fetch_metadata(&self) -> ScriptFetchMetadata {
        ScriptFetchMetadata::from_script_attributes(
            self.parameter("crossorigin"),
            self.parameter("referrerpolicy"),
            None,
            self.parameter("integrity"),
            self.parameter("nonce"),
            self.parameter("fetchpriority"),
        )
    }

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
        // Empty semicolon-delimited segments do not supply a parameter.
        match input.as_bytes().get(parser.byte_index()) {
            None | Some(b',') => break,
            Some(b';') => continue,
            _ => {}
        }
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
    fn response_stylesheet_preloads_use_canonical_stylesheet_options() {
        let base = Url::parse("https://example.test/pages/document").unwrap();
        let field = br#"<../theme.css#fragment>;rel=preload;as=style;type="text/css; charset=utf-8";nonce=az;nonce=wrong;crossorigin=use-credentials;referrerpolicy=no-referrer;integrity="sha256-value";fetchpriority=high, <../theme.css#fragment>;rel=preload;as=style;type="text/css; charset=utf-8";nonce=az;crossorigin=use-credentials;referrerpolicy=no-referrer;integrity="sha256-value";fetchpriority=high"#;
        let parsed = DocumentResponseLinks::parse(&base, &[("Link".into(), field.to_vec())]);
        assert!(parsed.scripts.is_empty());
        assert!(parsed.modules.is_empty());
        assert_eq!(parsed.styles.len(), 1);
        let preload = &parsed.styles[0];
        assert_eq!(preload.url, base.join("../theme.css#fragment").unwrap());
        assert_eq!(preload.options.nonce(), Some("az"));
        assert_eq!(preload.options.integrity(), Some("sha256-value"));
        assert_eq!(preload.options.referrer_policy(), Some("no-referrer"));
        assert_eq!(preload.options.fetch_priority(), Some("high"));
        assert_eq!(
            preload.options.request_mode_and_credentials(),
            (
                moli_fetch::RequestMode::Cors,
                moli_fetch::RequestCredentialsMode::Include
            )
        );
    }

    #[test]
    fn response_stylesheet_preload_eligibility_precedes_fetch() {
        let base = Url::parse("https://example.test/page").unwrap();
        let field = br#"</anchor>;rel=preload;as=style;anchor="/other", </media>;rel=preload;as=style;media="not all", </type>;rel=preload;as=style;type="text/plain", </ignored>;rel=alternate;rel=preload;as=style, <data:text/css,body{}>;rel=preload;as=style, </style.css>;rel=preload;;as=style;"#;
        let parsed = DocumentResponseLinks::parse(&base, &[("Link".into(), field.to_vec())]);
        assert_eq!(parsed.styles.len(), 1);
        assert_eq!(parsed.styles[0].url, base.join("/style.css").unwrap());
        assert_eq!(
            parsed.styles[0].options.request_mode_and_credentials(),
            (
                moli_fetch::RequestMode::NoCors,
                moli_fetch::RequestCredentialsMode::Include
            )
        );
    }

    #[test]
    fn response_links_ignore_empty_semicolon_parameter_segments() {
        let base = Url::parse("https://example.test/pages/document").unwrap();
        let relations = "preload modulepreload compression-dictionary";
        for parameters in [
            format!(r#";;rel="{relations}";as=script"#),
            format!("; \t ;rel=\"{relations}\";as=script"),
            format!(r#";rel="{relations}";;as=script"#),
            format!(";rel=\"{relations}\"; \t ;as=script"),
            format!(r#";rel="{relations}";as=script;"#),
            format!(r#";rel="{relations}";as=script;;;"#),
        ] {
            let field = format!("<../entry.js#module>{parameters}");
            let parsed =
                DocumentResponseLinks::parse(&base, &[("Link".into(), field.as_bytes().to_vec())]);
            assert_eq!(
                parsed.dictionaries,
                vec![base.join("../entry.js").unwrap()],
                "{field}"
            );
            assert_eq!(parsed.scripts.len(), 1, "{field}");
            assert_eq!(parsed.modules.len(), 1, "{field}");
            assert_eq!(
                parsed.scripts[0].url,
                base.join("../entry.js#module").unwrap(),
                "{field}"
            );
            assert_eq!(parsed.modules[0].url, parsed.scripts[0].url, "{field}");
        }
    }

    #[test]
    fn response_link_empty_segments_preserve_boundaries_quotes_and_first_parameters() {
        let base = Url::parse("https://example.test/page").unwrap();
        let field = br#"</ignored>;rel=alternate;;rel="preload modulepreload compression-dictionary";as=script;, </entry.js#module>; ;rel="PRELOAD modulepreload compression-dictionary";;as=script;title="fake;;, </fake>;rel=preload";nonce="a\"z;,";;nonce=wrong;, </anchor>;rel="preload modulepreload compression-dictionary";as=script;;anchor="/other";"#;
        let parsed = DocumentResponseLinks::parse(&base, &[("Link".into(), field.to_vec())]);
        assert_eq!(parsed.dictionaries, vec![base.join("/entry.js").unwrap()]);
        assert_eq!(parsed.scripts.len(), 1);
        assert_eq!(parsed.modules.len(), 1);
        assert_eq!(
            parsed.scripts[0].url,
            base.join("/entry.js#module").unwrap()
        );
        assert_eq!(parsed.modules[0].url, parsed.scripts[0].url);
        assert_eq!(parsed.scripts[0].metadata.nonce.as_deref(), Some("a\"z;,"));
        assert_eq!(parsed.modules[0].metadata.nonce.as_deref(), Some("a\"z;,"));
    }

    #[test]
    fn response_link_empty_segment_recovery_keeps_invalid_assignments_rejected() {
        let base = Url::parse("https://example.test/page").unwrap();
        for invalid in [
            ";=ignored",
            ";as=",
            ";nonce=",
            ";title=\"unterminated",
            ";nonce=\"a\rz\"",
        ] {
            let field = format!(
                r#"</entry.js>;rel="preload modulepreload compression-dictionary"{invalid}"#
            );
            let parsed =
                DocumentResponseLinks::parse(&base, &[("Link".into(), field.into_bytes())]);
            assert!(parsed.dictionaries.is_empty(), "{invalid}");
            assert!(parsed.scripts.is_empty(), "{invalid}");
            assert!(parsed.modules.is_empty(), "{invalid}");
        }
    }

    #[test]
    fn response_modulepreload_keeps_module_options_and_the_response_url_base() {
        let base = Url::parse("https://example.test/pages/document").unwrap();
        let headers = vec![("Link".into(), br#"<../entry.mjs#module>; REL="alternate MODULEPRELOAD"; nonce="a\"z"; nonce=wrong; crossorigin=use-credentials; referrerpolicy=no-referrer; integrity="sha256-value", </ignored>; rel=alternate; rel=modulepreload, </image>;rel=modulepreload;as=image, </media>;rel=modulepreload;media="not all", </anchor>;rel=modulepreload;anchor="/other", <../entry.mjs#module>;REL="alternate MODULEPRELOAD";nonce="a\"z";crossorigin=use-credentials;referrerpolicy=no-referrer;integrity="sha256-value", </style>;rel=modulepreload;as=style, </json>;rel=modulepreload;as=json"#.to_vec())];
        let parsed = DocumentResponseLinks::parse(&base, &headers);
        assert!(parsed.scripts.is_empty());
        assert_eq!(parsed.modules.len(), 3);
        let hint = &parsed.modules[0];
        assert_eq!(hint.url, base.join("../entry.mjs#module").unwrap());
        assert_eq!(hint.destination, ModulepreloadAsState::ScriptLike);
        let candidate = hint
            .fetch_candidate(&base, Some("same-origin".into()))
            .unwrap();
        assert_eq!(candidate.key.url(), &hint.url);
        let fetch = candidate.request.fetch_request();
        let metadata = candidate.request.fetch_metadata();
        assert_eq!(metadata.nonce(), Some("a\"z"));
        assert_eq!(metadata.integrity(), Some("sha256-value"));
        assert_eq!(
            metadata.credentials_mode,
            moli_fetch::RequestCredentialsMode::Include
        );
        assert_eq!(
            metadata.request_metadata.referrer_policy.as_deref(),
            Some("no-referrer")
        );
        assert_eq!(fetch.initiator_url(), &base);
        assert_eq!(parsed.modules[1].destination, ModulepreloadAsState::Style);
        assert_eq!(
            parsed.modules[1]
                .fetch_candidate(&base, None)
                .unwrap()
                .key
                .kind(),
            crate::module_runtime::ModuleKind::Css
        );
        assert_eq!(parsed.modules[2].destination, ModulepreloadAsState::Json);
        assert_eq!(
            parsed.modules[2]
                .fetch_candidate(&base, None)
                .unwrap()
                .key
                .kind(),
            crate::module_runtime::ModuleKind::Json
        );
    }

    #[test]
    fn modulepreload_style_policy_uses_style_nonce_and_the_captured_document() {
        use crate::content_security_policy::ContentSecurityPolicyRedirectStatus;
        use crate::document_runtime::DocumentRequestPolicySnapshot;
        let document = Url::parse("https://example.test/page").unwrap();
        let target = Url::parse("https://cdn.test/entry.css").unwrap();
        let policy = DocumentRequestPolicySnapshot::from_policies(
            vec!["script-src 'none'; style-src 'nonce-az'".into()],
            Vec::new(),
            Default::default(),
        );
        let allowed = policy.clone().for_style_preload(Some("az".into()));
        assert!(
            allowed
                .check_url(
                    &document,
                    &target,
                    ContentSecurityPolicyRedirectStatus::NoRedirect
                )
                .has_no_violations()
        );
        assert!(
            allowed
                .check_redirect(&document, &target)
                .has_no_violations()
        );
        assert!(
            !policy
                .for_style_preload(Some("wrong".into()))
                .check_url(
                    &document,
                    &target,
                    ContentSecurityPolicyRedirectStatus::NoRedirect
                )
                .has_no_violations()
        );
    }

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
