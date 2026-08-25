use std::borrow::Cow;

use encoding_rs::Encoding;

use super::*;
use moli_charset_parser::HTML_META_CHARSET_PRESCAN_LIMIT;

fn gbk_bytes(input: &str) -> Vec<u8> {
    encoding_rs::GBK.encode(input).0.into_owned()
}

#[test]
fn content_type_charset_is_selected() {
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; charset=gbk".to_owned(),
    )];
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    assert_eq!(decoder.push(&gbk_bytes("太平洋")), vec!["太平洋"]);
    assert_eq!(decoder.selected_encoding_name(), Some("GBK"));
}

#[test]
fn meta_charset_is_selected_without_header_charset() {
    let headers = vec![("Content-Type".to_owned(), "text/html".to_owned())];
    let mut input = b"<!doctype html><meta charset=\"gbk\"><p>".to_vec();
    input.extend_from_slice(&gbk_bytes("家居"));
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    assert_eq!(
        decoder.push(&input),
        vec!["<!doctype html><meta charset=\"gbk\"><p>家居"]
    );
    assert_eq!(decoder.selected_encoding_name(), Some("GBK"));
}

#[test]
fn meta_charset_can_be_split_across_chunks() {
    let headers = vec![("Content-Type".to_owned(), "text/html".to_owned())];
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    assert_eq!(
        decoder.push(b"<!doctype html><meta char"),
        vec!["<!doctype html><meta char"]
    );
    let mut tail = b"set=\"gbk\"><p>".to_vec();
    tail.extend_from_slice(&gbk_bytes("装修"));

    assert_eq!(decoder.push(&tail), vec!["set=\"gbk\"><p>装修"]);
    assert_eq!(decoder.selected_encoding_name(), Some("GBK"));
}

#[test]
fn ascii_prefix_streams_while_charset_sniffing_continues() {
    let headers = vec![("Content-Type".to_owned(), "text/html".to_owned())];
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    assert_eq!(
        decoder.push(b"<!doctype html><script src=\"/gate.js\"></script>"),
        vec!["<!doctype html><script src=\"/gate.js\"></script>"]
    );
    assert_eq!(decoder.selected_encoding_name(), None);
    assert_eq!(decoder.finish(), None);
    assert_eq!(decoder.selected_encoding_name(), Some("windows-1252"));
}

#[test]
fn later_meta_charset_decodes_unemitted_non_ascii_after_ascii_prefix() {
    let headers = vec![("Content-Type".to_owned(), "text/html".to_owned())];
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);
    let mut tail = b"<meta charset=\"gbk\"><p>".to_vec();
    tail.extend_from_slice(&gbk_bytes("家居"));

    assert_eq!(decoder.push(b"<!doctype html>"), vec!["<!doctype html>"]);
    assert_eq!(decoder.push(&tail), vec!["<meta charset=\"gbk\"><p>家居"]);
    assert_eq!(decoder.selected_encoding_name(), Some("GBK"));
}

#[test]
fn meta_charset_after_1024_bytes_still_in_head_is_ignored() {
    let headers = vec![("Content-Type".to_owned(), "text/html".to_owned())];
    let mut input = vec![b' '; HTML_META_CHARSET_PRESCAN_LIMIT];
    input.extend_from_slice(b"<meta charset=\"gbk\"><p>");
    input.extend_from_slice(&gbk_bytes("家居"));
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    let decoded = decoder.push(&input).join("");

    assert_eq!(decoder.selected_encoding_name(), Some("windows-1252"));
    assert!(!decoded.contains("家居"));
}

#[test]
fn meta_charset_crossing_1024_byte_boundary_is_ignored() {
    let headers = vec![("Content-Type".to_owned(), "text/html".to_owned())];
    let partial_meta = b"<meta char";
    let mut input = vec![b' '; HTML_META_CHARSET_PRESCAN_LIMIT - partial_meta.len()];
    input.extend_from_slice(partial_meta);
    input.extend_from_slice(b"set=\"gbk\"><p>");
    input.extend_from_slice(&gbk_bytes("家居"));
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    let decoded = decoder.push(&input).join("");

    assert_eq!(decoder.selected_encoding_name(), Some("windows-1252"));
    assert!(!decoded.contains("家居"));
}

#[test]
fn meta_charset_after_1024_bytes_after_head_is_ignored() {
    let headers = vec![("Content-Type".to_owned(), "text/html".to_owned())];
    let mut input = b"<body>".to_vec();
    input.extend(vec![b' '; HTML_META_CHARSET_PRESCAN_LIMIT - input.len()]);
    input.extend_from_slice(b"<meta charset=\"gbk\"><p>");
    input.extend_from_slice(&gbk_bytes("家居"));
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    let decoded = decoder.push(&input).join("");

    assert_eq!(decoder.selected_encoding_name(), Some("windows-1252"));
    assert!(!decoded.contains("家居"));
}

#[test]
fn meta_charset_prescan_matches_real_meta_start_tags_only() {
    use moli_charset_parser::sniff_html_meta_charset;

    assert_eq!(
        sniff_html_meta_charset(br#"<metadata charset="gbk"><meta charset="utf-8">"#)
            .map(Encoding::name),
        Some("UTF-8")
    );
    assert_eq!(
        sniff_html_meta_charset(br#"<metaverse charset="gbk"><p>ok</p>"#),
        None
    );
}

#[test]
fn meta_charset_prescan_ignores_script_text_and_requires_pragma_for_content() {
    use moli_charset_parser::sniff_html_meta_charset;

    assert_eq!(
        sniff_html_meta_charset(
            br#"<script>document.write('<meta charset="gbk">')</script><meta charset="utf-8">"#
        )
        .map(Encoding::name),
        Some("UTF-8")
    );
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
fn gbk_multibyte_can_be_split_across_chunks() {
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; charset=gbk".to_owned(),
    )];
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    assert!(decoder.push(&[0xCC]).is_empty());
    assert_eq!(decoder.push(&[0xAB]), vec!["太"]);
}

#[test]
fn bom_wins_over_header_charset() {
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; charset=gbk".to_owned(),
    )];
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    assert_eq!(decoder.push(&[0xEF]), Vec::<String>::new());
    assert_eq!(decoder.push(&[0xBB]), Vec::<String>::new());
    assert_eq!(decoder.push(&[0xBF, b'o', b'k']), vec!["ok"]);
    assert_eq!(decoder.selected_encoding_name(), Some("UTF-8"));
}

#[test]
fn unknown_charset_falls_back_to_html_default_on_finish() {
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; charset=x-unknown".to_owned(),
    )];
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    assert_eq!(decoder.push(b"<p>ok"), vec!["<p>ok"]);
    assert_eq!(decoder.finish(), None);
    assert_eq!(decoder.selected_encoding_name(), Some("windows-1252"));
}

#[test]
fn no_label_html_document_falls_back_to_windows_1252() {
    let (text, encoding) = decode_html_document(b"\x80\x80 Hello", &[]);

    assert_eq!(encoding, "windows-1252");
    assert_eq!(text, "\u{20ac}\u{20ac} Hello");
}

#[test]
fn no_label_html_document_can_inherit_parent_fallback() {
    let (text, encoding) = decode_html_document_with_fallback(&gbk_bytes("家居"), &[], Some("GBK"));

    assert_eq!(encoding, "GBK");
    assert_eq!(text, "家居");
}

#[test]
fn utf32_little_endian_bom_is_treated_as_utf16le_bom() {
    let (text, encoding) = decode_html_document(&[0xFF, 0xFE, 0x00, 0x00, b'<', 0x00], &[]);

    assert_eq!(encoding, "UTF-16LE");
    assert!(text.starts_with('\0'));
}

#[test]
fn document_decoding_removes_only_one_bom() {
    let (utf8_text, utf8_encoding) =
        decode_html_document(&[0xEF, 0xBB, 0xBF, 0xEF, 0xBB, 0xBF], &[]);
    let (utf16le_text, utf16le_encoding) = decode_html_document(&[0xFF, 0xFE, 0xFF, 0xFE], &[]);
    let (utf16be_text, utf16be_encoding) = decode_html_document(&[0xFE, 0xFF, 0xFE, 0xFF], &[]);

    assert_eq!(utf8_encoding, "UTF-8");
    assert_eq!(utf8_text, "\u{feff}");
    assert_eq!(utf16le_encoding, "UTF-16LE");
    assert_eq!(utf16le_text, "\u{feff}");
    assert_eq!(utf16be_encoding, "UTF-16BE");
    assert_eq!(utf16be_text, "\u{feff}");
}

#[test]
fn form_submission_uses_first_valid_accept_charset_label() {
    let encoding = form_submission_encoding(Some("unknown iso-8859-1 gbk"), "GBK");

    assert_eq!(encoding.name(), "windows-1252");
}

#[test]
fn form_submission_falls_back_to_document_character_set() {
    let encoding = form_submission_encoding(None, "gbk");

    assert_eq!(encoding.name(), "GBK");
}

#[test]
fn charset_sentinel_name_matches_ascii_case_insensitively() {
    assert!(is_charset_sentinel_name("_charset_"));
    assert!(is_charset_sentinel_name("_CHARSET_"));
    assert!(is_charset_sentinel_name("_Charset_"));
    assert!(!is_charset_sentinel_name("_charſet_"));
}

#[test]
fn form_urlencoded_serializer_uses_selected_legacy_encoding() {
    let encoded = form_urlencoded_serialize_pairs([("q", "家居")], encoding_rs::GBK);

    assert_eq!(encoded, "q=%BC%D2%BE%D3");
}

#[test]
fn form_urlencoded_serializer_uses_numeric_references_for_unmappable_text() {
    let encoded = form_urlencoded_serialize_pairs([("emoji", "💩")], encoding_rs::WINDOWS_1252);

    assert_eq!(encoded, "emoji=%26%23128169%3B");
}

#[test]
fn form_urlencoded_serializer_handles_iso_2022_jp_stateful_unmappables() {
    let encoded = form_urlencoded_serialize_pairs(
        [("utf16", "ABC~¤•★星🌟星★•¤~XYZ")],
        encoding_rs::ISO_2022_JP,
    );

    assert_eq!(
        encoded,
        "utf16=ABC%7E%26%23164%3B%26%238226%3B%1B%24B%21z%401%1B%28B%26%23127775%3B%1B%24B%401%21z%1B%28B%26%238226%3B%26%23164%3B%7EXYZ"
    );
}

#[test]
fn text_decoder_uses_legacy_charset_label_or_utf8() {
    assert_eq!(
        decode_text_for_legacy_web(&gbk_bytes("家居"), Some("gbk")),
        "家居"
    );
    assert_eq!(
        decode_text_for_legacy_web("plain".as_bytes(), None),
        "plain"
    );
}

#[test]
fn html_document_decoder_returns_selected_encoding() {
    let mut bytes = b"<!doctype html><meta charset=\"shift_jis\"><p>".to_vec();
    bytes.extend_from_slice(&encoding_rs::SHIFT_JIS.encode("目次").0);

    let (text, encoding) = decode_html_document(&bytes, &[]);

    assert_eq!(encoding, "Shift_JIS");
    assert!(text.contains("目次"), "text={text}");
}

#[test]
fn classic_script_decoding_inherits_document_character_set() {
    let script = r#"document.body.textContent = "目次";"#;
    let bytes = encoding_rs::SHIFT_JIS.encode(script).0.into_owned();

    assert_eq!(
        decode_classic_script_source(
            &bytes,
            &[(
                "Content-Type".to_owned(),
                "application/javascript".to_owned()
            )],
            None,
            Some("shift_jis"),
        ),
        script
    );
}

#[test]
fn classic_script_header_charset_wins_over_document_character_set() {
    let script = r#"document.body.textContent = "Привет";"#;
    let bytes = encoding_rs::WINDOWS_1251.encode(script).0.into_owned();

    assert_eq!(
        decode_classic_script_source(
            &bytes,
            &[(
                "Content-Type".to_owned(),
                "application/javascript; charset=windows-1251".to_owned(),
            )],
            None,
            Some("shift_jis"),
        ),
        script
    );
}

#[test]
fn classic_script_charset_attribute_is_fallback_before_document_character_set() {
    let script = r#"document.body.textContent = "目次";"#;
    let bytes = encoding_rs::SHIFT_JIS.encode(script).0.into_owned();

    assert_eq!(
        decode_classic_script_source(&bytes, &[], Some("shift_jis"), Some("gbk")),
        script
    );
}

#[test]
fn classic_script_bom_wins_over_labels() {
    let script = r#"document.body.textContent = "目次";"#;
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());

    assert_eq!(
        decode_classic_script_source(
            &bytes,
            &[(
                "Content-Type".to_owned(),
                "application/javascript; charset=gbk".to_owned(),
            )],
            Some("shift_jis"),
            Some("gbk"),
        ),
        script
    );
}

#[test]
fn url_query_encoder_uses_selected_legacy_encoding() {
    let encoded =
        encode_url_query_for_legacy_web("/search?q=家居&safe=a+b%20c#frag", encoding_rs::GBK);

    assert_eq!(encoded, "/search?q=%BC%D2%BE%D3&safe=a+b%20c#frag");
}

#[test]
fn url_query_encoder_percent_encodes_unmappable_entity_fallback() {
    let encoded = encode_url_query_for_legacy_web("/search?q=ChineseＧ", encoding_rs::WINDOWS_1252);

    assert_eq!(encoded, "/search?q=Chinese%26%2365319%3B");
}

#[test]
fn url_query_encoder_preserves_query_separators_between_components() {
    let encoded = encode_url_query_for_legacy_web(
        "/search?first=家居&second=💩;third=ok#frag",
        encoding_rs::GBK,
    );

    assert_eq!(
        encoded,
        "/search?first=%BC%D2%BE%D3&second=%26%23128169%3B;third=ok#frag"
    );
}

#[test]
fn url_query_encoder_preserves_ampersand_from_encoded_bytes() {
    let encoded = encode_url_query_for_legacy_web("/search?q=Γ", encoding_rs::ISO_2022_JP);

    assert_eq!(encoded, "/search?q=%1B$B&%23%1B(B");
}

#[test]
fn url_query_encoder_percent_encodes_iso_2022_jp_unmappables() {
    let encoded =
        encode_url_query_for_legacy_web("/search?q=Γ\x0E\x0F\x1Bx", encoding_rs::ISO_2022_JP);

    assert_eq!(
        encoded,
        "/search?q=%1B$B&%23%1B(B%26%2365533%3B%26%2365533%3B%26%2365533%3Bx"
    );
}

#[test]
fn url_query_encoder_handles_iso_2022_jp_stateful_output() {
    let encoded = encode_url_query_for_legacy_web("/search?q=¥‾s\\ﾐ佩", encoding_rs::ISO_2022_JP);

    assert_eq!(encoded, "/search?q=%1B(J\\~s%1B(B\\%1B$B%_PP%1B(B");
}

#[test]
fn url_query_encoder_leaves_utf8_and_queryless_inputs_borrowed() {
    assert!(matches!(
        encode_url_query_for_legacy_web("/search?q=家居", encoding_rs::UTF_8),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        encode_url_query_for_legacy_web("/search#家居", encoding_rs::GBK),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        encode_url_query_for_legacy_web("/search#frag?q=家居", encoding_rs::GBK),
        Cow::Borrowed(_)
    ));
}

#[test]
fn meta_declared_utf16_document_decodes_as_utf8() {
    let (text, encoding) = decode_html_document(
        br#"<!DOCTYPE html><meta charset="utf-16"><p>Hello, world!</p>"#,
        &[],
    );

    assert_eq!(encoding, "UTF-8");
    assert!(text.contains("Hello, world!"), "decoded as {text:?}");
}

/// `encoding_rs` has no UTF-16 encoder — the Encoding Standard replaces it
/// with UTF-8 for output — so UTF-16LE input has to be built by hand.
fn utf16le_bytes(input: &str) -> Vec<u8> {
    input
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect()
}

#[test]
fn meta_declared_utf16_does_not_override_a_bom() {
    let mut input = vec![0xFF, 0xFE];
    input.extend_from_slice(&utf16le_bytes("<meta charset=\"utf-16\">hi"));

    let (text, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "UTF-16LE");
    assert!(text.ends_with("hi"), "decoded as {text:?}");
}

#[test]
fn transport_utf16_still_wins_over_the_meta_rewrite() {
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; charset=utf-16le".to_owned(),
    )];
    let input = utf16le_bytes("<meta charset=\"utf-16\">hi");

    let (text, encoding) = decode_html_document(&input, &headers);

    assert_eq!(encoding, "UTF-16LE");
    assert!(text.ends_with("hi"), "decoded as {text:?}");
}

#[test]
fn bom_less_utf16le_xml_declaration_is_detected() {
    let input = utf16le_bytes(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html><head></head><body>hi</body></html>",
    );

    let (text, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "UTF-16LE");
    assert!(text.contains("hi"), "decoded as {text:?}");
}

#[test]
fn transport_charset_wins_over_utf16_xml_signature() {
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; charset=windows-1252".to_owned(),
    )];
    let input = utf16le_bytes(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html><head></head><body>hi</body></html>",
    );

    let (_, encoding) = decode_html_document(&input, &headers);

    assert_eq!(encoding, "windows-1252");
}

fn utf16be_bytes(input: &str) -> Vec<u8> {
    input
        .encode_utf16()
        .flat_map(|unit| unit.to_be_bytes())
        .collect()
}

fn assert_bom_less_utf16_xml_decodes_across_signature_splits(
    input: &[u8],
    expected_text: &str,
    expected_encoding: &str,
) {
    for split in 1..6 {
        let mut decoder = HtmlDocumentStreamingDecoder::new(&[]);
        assert_eq!(
            decoder.push(&input[..split]),
            Vec::<String>::new(),
            "split after signature byte {split} must remain buffered"
        );

        let mut decoded = decoder.push(&input[split..]).concat();
        if let Some(tail) = decoder.finish() {
            decoded.push_str(&tail);
        }

        assert_eq!(
            decoder.selected_encoding_name(),
            Some(expected_encoding),
            "wrong encoding after signature byte {split} split"
        );
        assert_eq!(
            decoded, expected_text,
            "misdecoded document after signature byte {split} split"
        );
    }
}

#[test]
fn bom_less_utf16le_xml_signature_survives_every_streaming_split() {
    let source = "<?xml version=\"1.0\"?><html><head></head><body>little endian</body></html>";
    let input = utf16le_bytes(source);

    assert_bom_less_utf16_xml_decodes_across_signature_splits(&input, source, "UTF-16LE");
}

#[test]
fn bom_less_utf16be_xml_signature_survives_every_streaming_split() {
    let source = "<?xml version=\"1.0\"?><html><head></head><body>big endian</body></html>";
    let input = utf16be_bytes(source);

    assert_bom_less_utf16_xml_decodes_across_signature_splits(&input, source, "UTF-16BE");
}

#[test]
fn diverged_utf16_xml_signature_prefix_resumes_ascii_streaming() {
    let mut decoder = HtmlDocumentStreamingDecoder::new(&[]);

    assert!(decoder.push(b"<").is_empty());
    assert_eq!(
        decoder.push(b"!doctype html><p>ordinary html"),
        vec!["<!doctype html><p>ordinary html"]
    );
    assert_eq!(decoder.finish(), None);
    assert_eq!(decoder.selected_encoding_name(), Some("windows-1252"));
}

#[test]
fn transport_charset_does_not_wait_for_a_utf16_xml_signature_prefix() {
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; charset=windows-1252".to_owned(),
    )];
    let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

    assert_eq!(decoder.push(b"<"), vec!["<"]);
    assert_eq!(decoder.selected_encoding_name(), Some("windows-1252"));
}

#[test]
fn bom_less_utf16be_xml_declaration_is_detected() {
    let input = utf16be_bytes(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html><head></head><body>hi</body></html>",
    );

    let (text, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "UTF-16BE");
    assert!(text.contains("hi"), "decoded as {text:?}");
}

#[test]
fn bom_less_utf16le_non_xml_not_detected() {
    let input = utf16le_bytes("<?y>");

    let (_, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "windows-1252");
}

#[test]
fn bom_less_utf16le_uppercase_x_not_detected() {
    let input = utf16le_bytes("<?X>");

    let (_, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "windows-1252");
}

#[test]
fn bom_less_utf16le_truncated_not_detected() {
    let input: Vec<u8> = vec![0x3C, 0x00, 0x3F, 0x00];

    let (text, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "windows-1252");
    assert_eq!(text, "<\0?\0");
}

#[test]
fn bom_less_utf16be_non_xml_not_detected() {
    let input = utf16be_bytes("<?y>");

    let (_, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "windows-1252");
}

#[test]
fn bom_less_utf16be_uppercase_x_not_detected() {
    let input = utf16be_bytes("<?X>");

    let (_, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "windows-1252");
}

#[test]
fn bom_less_utf16be_truncated_not_detected() {
    let input: Vec<u8> = vec![0x00, 0x3C, 0x00, 0x3F];

    let (text, encoding) = decode_html_document(&input, &[]);

    assert_eq!(encoding, "windows-1252");
    assert_eq!(text, "\0<\0?");
}

#[test]
fn header_charset_stays_inside_another_parameters_quoted_string() {
    assert_eq!(
        charset_from_content_type("text/html; boundary=\"; charset=gbk\""),
        None
    );
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; boundary=\"; charset=gbk\"".to_owned(),
    )];
    assert_eq!(
        decode_html_document(b"<p>hi</p>", &headers).1,
        "windows-1252"
    );
}

#[test]
fn header_charset_is_not_displaced_by_an_escaped_quote() {
    assert_eq!(
        charset_from_content_type("text/html; name=\"a\\\"; charset=gbk\"; charset=utf-8")
            .as_deref(),
        Some("utf-8")
    );
}

#[test]
fn header_charset_removes_quoting_backslashes() {
    assert_eq!(
        charset_from_content_type("text/html; charset=\"utf\\-8\"").as_deref(),
        Some("utf-8")
    );
    let headers = vec![(
        "Content-Type".to_owned(),
        "text/html; charset=\"utf\\-8\"".to_owned(),
    )];
    assert_eq!(decode_html_document(b"<p>hi</p>", &headers).1, "UTF-8");
}

#[test]
fn header_charset_keeps_its_existing_tolerances() {
    for header in [
        "text/html; charset=utf-8",
        "text/html;charset=utf-8",
        "TEXT/HTML; CHARSET=UTF-8",
        "text/html; charset = utf-8 ",
        "text/html; charset=\"utf-8\"",
        "text/html;charset=utf-8;",
        "text/html; charset='utf-8'",
        "text/html; charset=\"utf-8",
    ] {
        assert_eq!(
            charset_from_content_type(header)
                .as_deref()
                .and_then(encoding_for_label)
                .map(Encoding::name),
            Some("UTF-8"),
            "header={header}"
        );
    }
    assert_eq!(charset_from_content_type("text/html"), None);
    assert_eq!(charset_from_content_type("text/html; charset="), None);
    assert_eq!(charset_from_content_type("charset=utf-8"), None);
    assert_eq!(
        charset_from_content_type("text/html; charset=gbk; boundary=x").as_deref(),
        Some("gbk")
    );
}

#[test]
fn header_charset_recovers_after_a_stray_quote() {
    // WPT MIME case: the `"` does not open a parameter value, so the following
    // `;` still separates parameters and the real charset is found.
    assert_eq!(
        charset_from_content_type("text/html;\";charset=gbk").as_deref(),
        Some("gbk")
    );
}

#[test]
fn header_charset_keeps_an_escaped_quote_as_data() {
    // The quoted value is `utf-8"`, which is not a valid label. Trimming the
    // data quote would manufacture a valid one.
    let label = charset_from_content_type("text/html; charset=\"utf-8\\\"\"")
        .expect("a parameter value is present");

    assert_eq!(label, "utf-8\"");
    assert!(encoding_for_label(&label).is_none());
}
