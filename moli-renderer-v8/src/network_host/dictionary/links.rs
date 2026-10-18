use url::Url;

/// Dictionary and preload hints share the HTTP Link parser.
pub(crate) fn dictionary_links(base: &Url, headers: &[(String, Vec<u8>)]) -> Vec<Url> {
    crate::response_links::DocumentResponseLinks::parse(base, headers).dictionaries
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
