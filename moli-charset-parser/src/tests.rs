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
fn ignores_invalid_label_and_continues_to_later_valid_meta() {
    assert_eq!(
        sniff_html_meta_charset(br#"<meta charset="x-not-real"><meta charset="windows-1251">"#)
            .map(Encoding::name),
        Some("windows-1251")
    );
}

#[test]
fn ignores_meta_after_1024_bytes_even_while_still_in_head() {
    let mut input = spaces(HTML_META_CHARSET_PRESCAN_LIMIT);
    input.extend_from_slice(br#"<meta charset="gbk">"#);
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(parser.feed(&input), HtmlMetaCharsetScanResult::NotFound);
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
fn ignores_meta_tag_that_crosses_1024_byte_boundary() {
    let partial_meta = b"<meta char";
    let mut input = spaces(HTML_META_CHARSET_PRESCAN_LIMIT - partial_meta.len());
    input.extend_from_slice(partial_meta);
    input.extend_from_slice(br#"set="gbk">"#);
    let mut parser = HtmlMetaCharsetParser::new();

    assert_eq!(parser.feed(&input), HtmlMetaCharsetScanResult::NotFound);
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
fn accepts_meta_after_body_before_1024_bytes_like_chromium_prescan() {
    assert_eq!(
        sniff_html_meta_charset(br#"<body><meta charset="gbk">"#),
        Some(encoding_rs::GBK)
    );
}

#[test]
fn split_meta_after_1024_bytes_is_not_scanned() {
    let mut parser = HtmlMetaCharsetParser::new();
    assert_eq!(
        parser.feed(&spaces(HTML_META_CHARSET_PRESCAN_LIMIT)),
        HtmlMetaCharsetScanResult::NotFound
    );
    assert_eq!(
        parser.feed(br#"<meta charset="shift_jis">"#),
        HtmlMetaCharsetScanResult::NotFound
    );
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
