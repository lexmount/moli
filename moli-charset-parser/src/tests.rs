use encoding_rs::Encoding;

use super::*;

fn spaces(count: usize) -> Vec<u8> {
    vec![b' '; count]
}

#[test]
fn finds_meta_charset() {
    assert_eq!(
        sniff_html_meta_charset(br#"<meta charset="gbk">"#),
        Some(encoding_rs::GBK)
    );
}

#[test]
fn encoding_labels_allow_only_ascii_whitespace() {
    for whitespace in *b"\t\n\x0C\r " {
        let input = [
            br#"<meta charset=""#.as_slice(),
            &[whitespace],
            b"gbk",
            &[whitespace],
            br#"">"#.as_slice(),
        ]
        .concat();
        assert_eq!(sniff_html_meta_charset(&input), Some(encoding_rs::GBK));
    }

    assert_eq!(
        sniff_html_meta_charset(b"<meta charset=\"\xA0gbk\xA0\">"),
        None
    );
}

#[test]
fn ignores_tag_name_prefixes_and_script_text() {
    assert_eq!(
        sniff_html_meta_charset(br#"<metadata charset="gbk"><meta charset="utf-8">"#)
            .map(Encoding::name),
        Some("UTF-8")
    );
    assert_eq!(
        sniff_html_meta_charset(
            br#"<script>document.write('<meta charset="gbk">')</script><meta charset="utf-8">"#
        )
        .map(Encoding::name),
        Some("UTF-8")
    );
}

#[test]
fn content_attribute_requires_content_type_pragma() {
    assert_eq!(
        sniff_html_meta_charset(br#"<meta content="text/html; charset=gbk">"#),
        None
    );
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; charset=gbk">"#
        ),
        Some(encoding_rs::GBK)
    );
}

#[test]
fn invalid_charset_attribute_blocks_content_fallback_for_the_same_meta() {
    let invalid_meta =
        br#"<meta charset="not-a-real-encoding" http-equiv="content-type" content="charset=gbk">"#;
    assert_eq!(sniff_html_meta_charset(invalid_meta), None);

    let mut followed_by_valid_meta = invalid_meta.to_vec();
    followed_by_valid_meta.extend_from_slice(br#"<meta charset="windows-1251">"#);
    assert_eq!(
        sniff_html_meta_charset(&followed_by_valid_meta).map(Encoding::name),
        Some("windows-1251")
    );
}

#[test]
fn ignores_invalid_label_and_continues_to_later_valid_meta() {
    assert_eq!(
        sniff_html_meta_charset(br#"<meta charset="x-not-real"><meta charset="windows-1251">"#)
            .map(Encoding::name),
        Some("windows-1251")
    );
}

#[test]
fn finds_meta_after_1024_bytes_while_still_in_head() {
    let mut input = spaces(HTML_META_CHARSET_PRESCAN_LIMIT);
    input.extend_from_slice(br#"<meta charset="gbk">"#);
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(
        parser.feed(&input),
        HtmlMetaCharsetScanResult::Found(encoding_rs::GBK)
    );
}

#[test]
fn finds_meta_completed_before_1024_bytes_even_with_later_input() {
    let mut input = br#"<meta charset="gbk">"#.to_vec();
    input.extend(spaces(HTML_META_CHARSET_PRESCAN_LIMIT));

    assert_eq!(sniff_html_meta_charset(&input), Some(encoding_rs::GBK));
}

#[test]
fn split_meta_before_1024_bytes_is_scanned() {
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(
        parser.feed(br#"<meta char"#),
        HtmlMetaCharsetScanResult::Pending
    );
    assert_eq!(
        parser.feed(br#"set="gbk">"#),
        HtmlMetaCharsetScanResult::Found(encoding_rs::GBK)
    );
}

#[test]
fn finds_meta_tag_that_crosses_1024_byte_boundary_while_in_head() {
    let partial_meta = b"<meta char";
    let mut input = spaces(HTML_META_CHARSET_PRESCAN_LIMIT - partial_meta.len());
    input.extend_from_slice(partial_meta);
    input.extend_from_slice(br#"set="gbk">"#);
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(
        parser.feed(&input),
        HtmlMetaCharsetScanResult::Found(encoding_rs::GBK)
    );
}

#[test]
fn ignores_meta_after_1024_bytes_after_head_is_over() {
    let mut input = b"<body>".to_vec();
    input.extend(spaces(HTML_META_CHARSET_PRESCAN_LIMIT - input.len()));
    input.extend_from_slice(br#"<meta charset="gbk">"#);
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(parser.feed(&input), HtmlMetaCharsetScanResult::NotFound);
}

#[test]
fn accepts_meta_starting_before_1024_bytes_after_head_is_over() {
    let mut input = b"</head>".to_vec();
    input.extend(spaces(HTML_META_CHARSET_PRESCAN_LIMIT - 1 - input.len()));
    input.extend_from_slice(br#"<meta charset="gbk">"#);
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(
        parser.feed(&input),
        HtmlMetaCharsetScanResult::Found(encoding_rs::GBK)
    );
}

#[test]
fn accepts_split_meta_starting_before_1024_bytes_after_head_is_over() {
    let partial_meta = b"<meta char";
    let mut prefix = b"</head>".to_vec();
    prefix.extend(spaces(
        HTML_META_CHARSET_PRESCAN_LIMIT - partial_meta.len() - prefix.len(),
    ));
    prefix.extend_from_slice(partial_meta);
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(parser.feed(&prefix), HtmlMetaCharsetScanResult::Pending);
    assert_eq!(
        parser.feed(br#"set="gbk">"#),
        HtmlMetaCharsetScanResult::Found(encoding_rs::GBK)
    );
}

#[test]
fn rejects_split_meta_starting_at_1024_bytes_after_head_is_over() {
    let mut prefix = b"</head>".to_vec();
    prefix.extend(spaces(HTML_META_CHARSET_PRESCAN_LIMIT - prefix.len()));
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(parser.feed(&prefix), HtmlMetaCharsetScanResult::NotFound);
    assert_eq!(
        parser.feed(br#"<meta charset="gbk">"#),
        HtmlMetaCharsetScanResult::NotFound
    );
}

#[test]
fn accepts_meta_after_body_before_1024_bytes_like_chromium_prescan() {
    assert_eq!(
        sniff_html_meta_charset(br#"<body><meta charset="gbk">"#),
        Some(encoding_rs::GBK)
    );
}

#[test]
fn split_meta_after_1024_bytes_is_scanned_while_in_head() {
    let mut parser = HtmlMetaCharsetParser::new();
    assert_eq!(
        parser.feed(&spaces(HTML_META_CHARSET_PRESCAN_LIMIT)),
        HtmlMetaCharsetScanResult::Pending
    );
    assert_eq!(
        parser.feed(br#"<meta charset="shift_jis">"#),
        HtmlMetaCharsetScanResult::Found(encoding_rs::SHIFT_JIS)
    );
}

#[test]
fn stops_after_head_ends_beyond_1024_bytes() {
    let mut input = spaces(HTML_META_CHARSET_PRESCAN_LIMIT);
    input.extend_from_slice(br#"</head><meta charset="gbk">"#);
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(parser.feed(&input), HtmlMetaCharsetScanResult::NotFound);
}

#[test]
fn finish_reports_not_found() {
    let mut parser = HtmlMetaCharsetParser::new();
    assert_eq!(
        parser.feed(b"<head><title>x"),
        HtmlMetaCharsetScanResult::Pending
    );
    assert_eq!(parser.finish(), HtmlMetaCharsetScanResult::NotFound);
}

#[test]
fn meta_declared_utf16_is_rewritten_to_utf8() {
    for label in ["utf-16", "utf-16le", "utf-16be", "UTF-16BE", "  utf-16  "] {
        let input = format!(r#"<meta charset="{label}">"#);
        assert_eq!(
            sniff_html_meta_charset(input.as_bytes()).map(Encoding::name),
            Some("UTF-8"),
            "charset={label}"
        );
    }
}

#[test]
fn meta_declared_x_user_defined_is_rewritten_to_windows1252() {
    assert_eq!(
        sniff_html_meta_charset(br#"<meta charset="x-user-defined">"#).map(Encoding::name),
        Some("windows-1252")
    );
}

#[test]
fn content_type_pragma_charset_is_rewritten_too() {
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; charset=utf-16">"#
        )
        .map(Encoding::name),
        Some("UTF-8")
    );
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; charset=x-user-defined">"#
        )
        .map(Encoding::name),
        Some("windows-1252")
    );
}

#[test]
fn other_meta_charset_labels_are_left_alone() {
    for (label, expected) in [
        ("utf-8", "UTF-8"),
        ("gbk", "GBK"),
        ("shift_jis", "Shift_JIS"),
        ("iso-8859-1", "windows-1252"),
    ] {
        let input = format!(r#"<meta charset="{label}">"#);
        assert_eq!(
            sniff_html_meta_charset(input.as_bytes()).map(Encoding::name),
            Some(expected),
            "charset={label}"
        );
    }

    // A label outside the Encoding Standard is still no match at all, rather
    // than being rewritten to one of the two replacements above.
    assert_eq!(sniff_html_meta_charset(br#"<meta charset="utf-32">"#), None);
}

#[test]
fn content_type_charset_is_extracted_by_keyword_not_by_mime_parameter() {
    // The algorithm searches for the keyword; it does not require a media
    // type, and it does not parse MIME parameters.
    assert_eq!(
        sniff_html_meta_charset(br#"<meta http-equiv="content-type" content="charset=utf-8">"#)
            .map(Encoding::name),
        Some("UTF-8")
    );
    // The keyword is matched literally, including inside a longer word.
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; xcharset=utf-8">"#
        )
        .map(Encoding::name),
        Some("UTF-8")
    );
    // Whitespace is allowed on either side of the equals sign.
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; charset = gbk">"#
        )
        .map(Encoding::name),
        Some("GBK")
    );
}

#[test]
fn unquoted_content_type_charset_ends_at_whitespace_or_semicolon() {
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; charset=utf-8 profile=x">"#
        )
        .map(Encoding::name),
        Some("UTF-8")
    );
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; charset=utf-8;q=1">"#
        )
        .map(Encoding::name),
        Some("UTF-8")
    );
}

#[test]
fn content_type_charset_requires_matching_quotes() {
    for quoted in [
        &br#"<meta http-equiv="content-type" content="text/html; charset='utf-8'">"#[..],
        &br#"<meta http-equiv="content-type" content='text/html; charset="utf-8"'>"#[..],
    ] {
        assert_eq!(
            sniff_html_meta_charset(quoted).map(Encoding::name),
            Some("UTF-8")
        );
    }
    // Opened but never closed contributes nothing, rather than being read to
    // the end of the value.
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content='text/html; charset="utf-8'>"#
        ),
        None
    );
    // Opened with one quote and closed with the other is equally unmatched.
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; charset=&quot;utf-8'">"#
        ),
        None
    );
}

#[test]
fn content_type_charset_search_resumes_after_a_bare_keyword() {
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="charset; charset=gbk">"#
        )
        .map(Encoding::name),
        Some("GBK")
    );
    assert_eq!(
        sniff_html_meta_charset(br#"<meta http-equiv="content-type" content="charset">"#),
        None
    );
}

#[test]
fn empty_content_type_charset_ends_the_extraction() {
    // The algorithm returns the empty label it found; it does not keep
    // searching the same value for a second assignment.
    assert_eq!(
        sniff_html_meta_charset(
            br#"<meta http-equiv="content-type" content="text/html; charset=; charset=gbk">"#
        ),
        None
    );
}
