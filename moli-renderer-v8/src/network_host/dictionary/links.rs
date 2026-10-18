use std::collections::HashSet;

use moli_header_field::{HeaderFieldTokenMode, HeaderFieldTokenizer};
use url::Url;

/// Link targets are resolved against the response URL, before an HTML base
/// element can change the Document's base. Parse parameters as HTTP fields so
/// commas in targets and quoted strings cannot create additional links.
pub(crate) fn dictionary_links(base: &Url, headers: &[(String, Vec<u8>)]) -> Vec<Url> {
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("link"))
    {
        let value = moli_header_field::decode_header_value(value);
        let mut parser = HeaderFieldTokenizer::new(&value);
        while !parser.is_consumed() {
            let Some(target) = parse_link(&value, &mut parser) else {
                break;
            };
            if let Some(target) = target
                && let Ok(mut url) = base.join(target)
                && matches!(url.scheme(), "http" | "https")
            {
                url.set_fragment(None);
                if seen.insert(url.clone()) {
                    links.push(url);
                }
            }
            if !parser.consume(',') {
                break;
            }
        }
    }
    links
}

fn parse_link<'a>(
    input: &'a str,
    parser: &mut HeaderFieldTokenizer<'a>,
) -> Option<Option<&'a str>> {
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
    let mut relation = None;
    let mut anchor = false;
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
        if name.eq_ignore_ascii_case("rel") && relation.is_none() {
            relation = Some(value);
        } else if name.eq_ignore_ascii_case("anchor") {
            // An anchored link describes another resource, not this response.
            anchor = true;
        }
    }
    if !parser.is_consumed() && !input[parser.byte_index()..].starts_with(',') {
        return None;
    }
    Some(
        (!anchor
            && relation.is_some_and(|value| {
                value
                    .split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("compression-dictionary"))
            }))
        .then_some(target),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_link_fields_preserve_quoted_delimiters_and_target_commas() {
        let base = Url::parse("https://example.test/pages/document").unwrap();
        let headers = vec![
            ("Link".into(), br#"<../dict,one.bin>; title="ignored, \"quote\"; rel=fake"; rel="alternate compression-dictionary", </two.bin>; rel=compression-dictionary"#.to_vec()),
            ("LINK".into(), br#"</two.bin#duplicate>; rel="COMPRESSION-DICTIONARY", <https://cdn.test/three>; rel=compression-dictionary"#.to_vec()),
        ];
        let links = dictionary_links(&base, &headers);
        assert_eq!(
            links.iter().map(Url::as_str).collect::<Vec<_>>(),
            [
                "https://example.test/dict,one.bin",
                "https://example.test/two.bin",
                "https://cdn.test/three"
            ]
        );
    }

    #[test]
    fn dictionary_link_relations_do_not_inherit_later_parameters_or_anchors() {
        let base = Url::parse("https://example.test/page").unwrap();
        let headers = vec![("link".into(), br#"</ignored>; rel=preload; rel=compression-dictionary, </anchored>; rel=compression-dictionary; anchor="/other", </valid>; rel=compression-dictionary; rel=preload"#.to_vec())];
        assert_eq!(
            dictionary_links(&base, &headers),
            [base.join("/valid").unwrap()]
        );
    }

    #[test]
    fn malformed_dictionary_links_cannot_turn_parameter_text_into_requests() {
        let base = Url::parse("https://example.test/page").unwrap();
        for value in [
            br#"/missing-angle; rel=compression-dictionary"#.as_slice(),
            br#"</bad target>; rel=compression-dictionary"#.as_slice(),
            br#"< /dict>; rel=compression-dictionary"#.as_slice(),
            br#"</dict>; rel="compression-dictionary" trailing"#.as_slice(),
            br#"</dict>; rel="unfinished, </other>; rel=compression-dictionary"#.as_slice(),
            br#"<data:text/plain,dictionary>; rel=compression-dictionary"#.as_slice(),
            br#"</other>; title="rel=compression-dictionary""#.as_slice(),
        ] {
            assert!(
                dictionary_links(&base, &[("link".into(), value.to_vec())]).is_empty(),
                "{}",
                String::from_utf8_lossy(value)
            );
        }
    }
}
