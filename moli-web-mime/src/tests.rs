use super::*;

#[test]
fn reads_essence_with_parameters() {
    assert_eq!(
        mime_essence(" Video/MP4 ; codecs=\"avc1.42E01E\" ").as_deref(),
        Some("video/mp4")
    );
    assert_eq!(
        mime_charset("Text/Plain ; Charset=\"utf-16le\"").as_deref(),
        Some("utf-16le")
    );
    assert_eq!(
        mime_charset("text/plain; charset=utf-8").as_deref(),
        Some("utf-8")
    );
    assert_eq!(mime_charset("charset=utf-8"), None);
    assert_eq!(
        mime_parameter("video/mp4; codecs=\"avc1.42E01E\"", "codecs").as_deref(),
        Some("avc1.42E01E")
    );
}

#[test]
fn parses_mime_with_whatwg_parameter_rules() {
    let parsed = parse_mime("Text/Plain ; Charset=\"utf-8\"").unwrap();
    assert_eq!(parsed.essence_str(), "text/plain");
    assert_eq!(
        parsed.get_param(mime::CHARSET).map(|value| value.as_str()),
        Some("utf-8")
    );

    assert_eq!(
        mime_parameter("text/plain; title=\"alpha;beta\"", "title").as_deref(),
        Some("alpha;beta")
    );
    assert_eq!(
        mime_parameter("text/plain; charset=utf-8; charset=windows-1252", "charset").as_deref(),
        Some("utf-8")
    );
    assert_eq!(
        mime_parameter("text/plain; Charset=utf-8", "CHARSET").as_deref(),
        Some("utf-8")
    );
    assert_eq!(
        mime_charset("text/plain; bad parameter=value; charset=utf-8").as_deref(),
        Some("utf-8")
    );
    assert_eq!(mime_essence("*/*").as_deref(), Some("*/*"));
}

#[test]
fn extracts_request_header_content_type_essence_for_fetch_rules() {
    assert_eq!(
        request_header_content_type_essence(" Text/Plain ; charset=utf-8 ").as_deref(),
        Some("text/plain")
    );
    assert_eq!(
        request_header_content_type_essence(&format!("text/plain;{}", "s".repeat(116))).as_deref(),
        Some("text/plain")
    );
    assert_eq!(
        request_header_content_type_essence("text/plain, text/plain"),
        None
    );
    for input in [
        "application/json, text/plain",
        "text/plain, application/json",
    ] {
        assert_eq!(request_header_content_type_essence(input), None, "{input}");
    }
    for input in [
        "text/plain;charset=utf8, extra",
        "text/plain;charset=utf8, application/json",
    ] {
        assert_eq!(
            request_header_content_type_essence(input).as_deref(),
            Some("text/plain"),
            "{input}"
        );
    }
    assert_eq!(request_header_content_type_essence("text"), None);
    assert_eq!(request_header_content_type_essence("text/"), None);
    assert_eq!(request_header_content_type_essence("te xt/plain"), None);
    assert_eq!(request_header_content_type_essence("text /plain"), None);
    assert_eq!(request_header_content_type_essence("text/ plain"), None);
    assert_eq!(request_header_content_type_essence("text / plain"), None);
}

#[test]
fn matches_document_types() {
    assert!(is_html_document_mime("text/html;charset=utf-8"));
    assert!(is_dom_parser_xml_mime("image/svg+xml;charset=utf-8"));
    assert!(!is_dom_parser_xml_mime("text/html"));
}

#[test]
fn classifies_document_resource_mime_from_headers_and_url() {
    let headers: Vec<(String, Vec<u8>)> = vec![
        ("Content-Type".to_owned(), b"text/plain".to_vec()),
        (
            "content-type".to_owned(),
            b" Application/XHTML+XML ; charset=utf-8 ".to_vec(),
        ),
    ];
    assert_eq!(
        response_document_content_type(&headers).as_deref(),
        Some("application/xhtml+xml")
    );
    assert_eq!(
        response_document_content_type(&[("Content-Type".to_owned(), b" ".to_vec())]),
        None
    );

    assert_eq!(
        resource_mime_essence_for_url("data:text/html;charset=gbk,%C4%E3", "/ignored.png",)
            .as_deref(),
        Some("text/html")
    );
    assert_eq!(
        resource_mime_essence_for_url("https://example.test/assets/hero.PNG", "/assets/hero.PNG")
            .as_deref(),
        Some("image/png")
    );
    assert_eq!(
        resource_mime_essence_for_path("/fonts/site.woff2"),
        Some("font/woff2")
    );
    assert_eq!(
        resource_mime_essence_for_path("/media/clip.webm"),
        Some("video/webm")
    );
    assert_eq!(resource_mime_essence_for_path("/download.bin"), None);
}

#[test]
fn classifies_binary_main_document_headers() {
    assert!(response_headers_indicate_raw_document(&[(
        "Content-Type".to_owned(),
        b"application/pdf; charset=binary".to_vec(),
    )]));
    assert!(response_headers_indicate_raw_document(&[(
        "Content-Type".to_owned(),
        b"image/png".to_vec(),
    )]));
    assert!(response_headers_indicate_raw_document(&[(
        "Content-Disposition".to_owned(),
        b"attachment; filename=report.html".to_vec(),
    )]));
    assert!(response_headers_indicate_raw_document(&[(
        "Content-Type".to_owned(),
        b"application/wasm".to_vec(),
    )]));
    assert!(!response_headers_indicate_raw_document(&[(
        "Content-Type".to_owned(),
        b"text/html; charset=utf-8".to_vec(),
    )]));
    assert!(!response_headers_indicate_raw_document(&[(
        "Content-Type".to_owned(),
        b"text/plain".to_vec(),
    )]));
    assert!(!response_headers_indicate_raw_document(&[(
        "Content-Disposition".to_owned(),
        b"inline; filename=report.pdf".to_vec(),
    )]));
}

#[test]
fn classifies_supported_lightweight_document_mime_types() {
    for mime in [
        "text/html; charset=utf-8",
        "text/plain",
        "application/xml",
        "application/atom+xml",
        "application/vnd.example+json",
        "text/javascript",
        "image/png",
        "audio/ogg",
        "video/webm",
    ] {
        assert!(
            is_supported_document_mime_type(mime),
            "{mime} should be representable as a lightweight Document"
        );
    }
    for mime in [
        "application/octet-stream",
        "application/pdf",
        "application/zip",
        "application/wasm",
        "font/woff2",
    ] {
        assert!(
            !is_supported_document_mime_type(mime),
            "{mime} requires raw/download handling instead of a child Document"
        );
    }
}

#[test]
fn matches_script_and_form_content_types() {
    assert!(is_javascript_mime("Text/JavaScript; charset=utf-8"));
    assert!(is_javascript_mime("application/ecmascript"));
    assert!(!is_javascript_mime("text/html"));
    assert!(is_javascript_mime_essence("text/javascript1.5"));
    assert!(!is_javascript_mime_essence("TEXT/JAVASCRIPT"));

    assert!(is_css_mime("Text/CSS; charset=utf-8"));
    assert!(!is_css_mime("text/html"));
    assert!(is_text_mime("Text/Plain; charset=utf-8"));
    assert!(is_text_mime_essence("text/csv"));
    assert!(!is_text_mime("application/json"));
    assert!(is_css_style_element_type_attribute(None));
    assert!(is_css_style_element_type_attribute(Some("")));
    assert!(is_css_style_element_type_attribute(Some("TeXt/CsS")));
    assert!(!is_css_style_element_type_attribute(Some(" text/css ")));
    assert!(!is_css_style_element_type_attribute(Some(
        "text/css; charset=utf-8"
    )));
    assert!(!is_css_style_element_type_attribute(Some("text/plain")));

    assert!(is_css_stylesheet_type_hint(None));
    assert!(is_css_stylesheet_type_hint(Some("")));
    assert!(is_css_stylesheet_type_hint(Some(
        " Text/CSS ; charset=utf-8 "
    )));
    assert!(!is_css_stylesheet_type_hint(Some("text/plain")));

    assert!(is_image_mime("Image/PNG; charset=binary"));
    assert!(is_image_mime_essence("image/svg+xml"));
    assert!(!is_image_mime("application/svg+xml"));
    assert!(is_png_image_mime("Image/PNG; charset=binary"));
    assert!(is_png_image_mime_essence("image/png"));
    assert!(!is_png_image_mime("image/png-bad"));
    assert!(is_svg_image_mime("Image/SVG+XML; charset=utf-8"));
    assert!(is_svg_image_mime_essence("image/svg+xml"));
    assert!(!is_svg_image_mime("image/svg+xml-bad"));
    assert!(is_audio_mime("audio/mpeg"));
    assert!(is_audio_mime_essence("audio/wave"));
    assert!(is_video_mime("video/mp4; codecs=\"avc1.42E01E\""));
    assert!(is_video_mime_essence("video/webm"));
    assert!(is_font_mime("font/woff2"));
    assert!(is_font_mime_essence("font/ttf"));
    assert!(!is_font_mime("application/font-woff"));
    assert_eq!(
        media_mime_support("Video/MP4; codecs=\"avc1.42E01E\""),
        MediaMimeSupport::Probably
    );
    assert_eq!(media_mime_support("audio/flac").as_can_play_type(), "maybe");
    assert_eq!(
        media_mime_support("application/octet-stream"),
        MediaMimeSupport::Unsupported
    );

    assert!(is_form_urlencoded_mime(
        " Application/X-WWW-Form-Urlencoded ; charset=utf-8 "
    ));
    assert!(!is_form_urlencoded_mime("text/plain"));
    assert!(is_multipart_form_data_mime(
        " Multipart/Form-Data ; boundary=abc "
    ));
    assert_eq!(
        multipart_form_data_boundary("multipart/form-data; boundary=\"abc\"").as_deref(),
        Some("abc")
    );
    assert_eq!(multipart_form_data_boundary("text/plain"), None);

    assert!(is_json_module_mime("Application/JSON; charset=utf-8"));
    assert!(is_json_module_mime("application/manifest+json"));
    assert!(!is_json_module_mime("text/json"));

    assert!(is_media_source_type_supported(
        "video/mp4; codecs=\"avc1.42E01E\""
    ));
    assert!(is_media_source_type_supported(
        "Video/MP4; codecs=\"mp4a.40.2, avc1.64001f\""
    ));
    assert!(!is_media_source_type_supported(
        "video/mp4; codecs=\"hev1.1.6.L93.B0\""
    ));
    assert!(!is_media_source_type_supported(
        "video/webm; codecs=\"avc1.42E01E\""
    ));
    assert!(!is_media_source_type_supported(
        "notvideo/mp4; codecs=\"avc1.42E01E\""
    ));
}

#[test]
fn normalizes_valid_web_api_mime_types() {
    assert_eq!(
        normalize_web_api_mime_type("Text/Plain; Charset=UTF-8"),
        "text/plain; charset=utf-8"
    );
    assert_eq!(normalize_web_api_mime_type(" image/gif "), " image/gif ");
    assert_eq!(normalize_web_api_mime_type("text/plain\n"), "");
    assert_eq!(
        normalize_web_api_mime_type("custom ascii type"),
        "custom ascii type"
    );
}

#[test]
fn reads_response_header_values_case_insensitively() {
    let headers: Vec<(String, Vec<u8>)> = vec![
        ("Content-Type".to_owned(), b"text/html".to_vec()),
        ("content-type".to_owned(), b"application/json".to_vec()),
        ("Bad Header".to_owned(), b"ignored".to_vec()),
    ];

    assert_eq!(
        response_header_value(&headers, "content-type"),
        Some("text/html".to_owned())
    );
    assert_eq!(
        response_header_values(&headers, "content-type"),
        vec!["text/html".to_owned(), "application/json".to_owned()]
    );
    assert_eq!(
        response_content_type(&headers),
        Some("text/html".to_owned())
    );
    assert_eq!(response_header_value(&headers, "x-missing"), None);
    assert_eq!(response_header_value(&headers, "Bad Header"), None);
}

#[test]
fn extracts_response_mime_essence_from_the_combined_header_list() {
    let cases: &[(&[&str], Option<&str>)] = &[
        (&[], None),
        (&[""], None),
        (&["*/*", "not-a-mime-type"], None),
        (&["application/json"], Some("application/json")),
        (&["Text/CSS; charset=utf-16"], Some("text/css")),
        (
            &["text/plain", "application/json"],
            Some("application/json"),
        ),
        (&["application/json", "text/plain"], Some("text/plain")),
        (&["application/json", "invalid"], Some("application/json")),
        (&["application/json", "*/*", ""], Some("application/json")),
        (&["text/plain, application/json"], Some("application/json")),
        (&["application/json, text/plain"], Some("text/plain")),
        (
            &["application/json, invalid, */*"],
            Some("application/json"),
        ),
        (&["text/plain; charset=gbk, text/css"], Some("text/css")),
        (
            &[r#"text/plain; a=",application/json""#],
            Some("text/plain"),
        ),
        (
            &[r#"text/plain; a="x\",application/json""#],
            Some("text/plain"),
        ),
        (
            &[r#"text/plain; a="x\\", application/json"#],
            Some("application/json"),
        ),
        (
            &[r#"text/plain; a=""#, "application/json"],
            Some("text/plain"),
        ),
        (&[r#"text/plain;""#, "application/json"], Some("text/plain")),
        (&["applic(ation/vnd.api+json"], None),
        (&["application/vnd)api+json"], None),
        (&["text /css"], None),
        (&["application/vnd.中文+json"], None),
    ];
    for (values, expected) in cases {
        let mut headers: Vec<(String, Vec<u8>)> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                (
                    if index % 2 == 0 {
                        "Content-Type"
                    } else {
                        "cOnTeNt-TyPe"
                    }
                    .to_owned(),
                    value.as_bytes().to_vec(),
                )
            })
            .collect();
        headers.push(("X-Content-Type".to_owned(), b"application/json".to_vec()));
        assert_eq!(
            extract_response_mime_essence(&headers).as_deref(),
            *expected,
            "{values:?}"
        );
    }
}

#[test]
fn extracts_response_mime_parameters_with_charset_inheritance() {
    let cases: &[(&[&str], Option<&str>)] = &[
        (&[], None),
        (&["", "invalid", "*/*"], None),
        (&["TEXT/PLAIN;CHARSET=GBK"], Some("text/plain;charset=GBK")),
        (&["text/plain;p=MiXeD"], Some("text/plain;p=MiXeD")),
        (&["text/plain;p=ÿ"], Some("text/plain;p=\"ÿ\"")),
        (&["text/plain;charset=GBK", "text/html"], Some("text/html")),
        (
            &["text/html;charset=GBK;a=b", "text/html;x=y"],
            Some("text/html;x=y;charset=GBK"),
        ),
        (
            &["text/html;charset=GBK;a=b, text/html;x=y"],
            Some("text/html;x=y;charset=GBK"),
        ),
        (
            &[
                "text/html;charset=GBK",
                "text/html;charset=UTF-8",
                "text/html",
            ],
            Some("text/html;charset=GBK"),
        ),
        (
            &["text/html", "text/html;charset=GBK", "text/html"],
            Some("text/html"),
        ),
        (
            &["text/html;charset=GBK", "x/x", "text/html;x=y"],
            Some("text/html;x=y"),
        ),
        (
            &["text/html;charset=GBK", "invalid", "*/*", "", "text/html"],
            Some("text/html;charset=GBK"),
        ),
        (
            &["text/html;charset=\"\"", "text/html;x=y"],
            Some("text/html;x=y;charset=\"\""),
        ),
        (
            &[r#"text/html;charset="A\"B""#, "text/html"],
            Some(r#"text/html;charset="A\"B""#),
        ),
        (
            &[r#"text/html;x="A,B";charset=GBK"#, "text/html"],
            Some("text/html;charset=GBK"),
        ),
        (
            &[r#"text/html;x="A"#, "text/plain;charset=GBK"],
            Some(r#"text/html;x="A, text/plain;charset=GBK""#),
        ),
    ];
    for (values, expected) in cases {
        let headers = values
            .iter()
            .map(|value| ("cOnTeNt-TyPe".to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>();
        let mut headers = moli_header_field::headers_from_byte_strings(&headers).unwrap();
        headers.push(("X-Content-Type".to_owned(), b"text/ignored".to_vec()));
        assert_eq!(
            extract_response_mime_type(&headers)
                .map(|mime| mime.to_string())
                .as_deref(),
            *expected,
            "{values:?}"
        );
    }
}

#[test]
fn derives_effective_response_mime_for_body_consumers() {
    let headers: Vec<(String, Vec<u8>)> = vec![(
        "Content-Type".to_owned(),
        b"Text/HTML; Charset=UTF-8".to_vec(),
    )];

    assert_eq!(
        effective_response_mime_type(&headers, None),
        Some("Text/HTML; Charset=UTF-8".to_owned())
    );
    assert_eq!(
        effective_response_mime_type(&headers, Some("application/xml")),
        Some("application/xml".to_owned())
    );
    assert_eq!(
        effective_response_mime_essence(&headers, Some("Application/XHTML+XML")),
        Some("application/xhtml+xml".to_owned())
    );
    assert_eq!(
        response_blob_mime_type(&headers),
        "text/html; charset=utf-8"
    );

    let invalid: Vec<(String, Vec<u8>)> =
        vec![("Content-Type".to_owned(), b"text/plain\n".to_vec())];
    assert_eq!(response_blob_mime_type(&invalid), "");
}

#[test]
fn determines_nosniff_from_first_option_token() {
    assert!(determine_nosniff(&[(
        "X-Content-Type-Options".to_owned(),
        b"NoSniff".to_vec()
    )]));
    assert!(determine_nosniff(&[(
        "x-content-type-options".to_owned(),
        b"nosniff, other".to_vec()
    )]));
    assert!(!determine_nosniff(&[(
        "x-content-type-options".to_owned(),
        b"other, nosniff".to_vec()
    )]));
    assert!(!determine_nosniff(&[(
        "x-content-type-options".to_owned(),
        b"sniff".to_vec()
    )]));
}

#[test]
fn nosniff_blocks_script_like_and_style_mismatches_only() {
    let html_nosniff: Vec<(String, Vec<u8>)> = vec![
        ("content-type".to_owned(), b"text/html".to_vec()),
        ("x-content-type-options".to_owned(), b"nosniff".to_vec()),
    ];
    let script_nosniff: Vec<(String, Vec<u8>)> = vec![
        (
            "content-type".to_owned(),
            b"application/javascript".to_vec(),
        ),
        ("x-content-type-options".to_owned(), b"nosniff".to_vec()),
    ];
    let css_nosniff: Vec<(String, Vec<u8>)> = vec![
        ("content-type".to_owned(), b"text/css".to_vec()),
        ("x-content-type-options".to_owned(), b"nosniff".to_vec()),
    ];

    assert!(should_response_be_blocked_due_to_nosniff(
        &html_nosniff,
        FetchDestination::Script
    ));
    assert!(should_response_be_blocked_due_to_nosniff(
        &html_nosniff,
        FetchDestination::Style
    ));
    assert!(!should_response_be_blocked_due_to_nosniff(
        &html_nosniff,
        FetchDestination::Other
    ));
    assert!(!should_response_be_blocked_due_to_nosniff(
        &script_nosniff,
        FetchDestination::Worker
    ));
    assert!(!should_response_be_blocked_due_to_nosniff(
        &css_nosniff,
        FetchDestination::Style
    ));
}

#[test]
fn nosniff_blocks_missing_content_type_for_script_like_and_style() {
    let headers: Vec<(String, Vec<u8>)> =
        vec![("x-content-type-options".to_owned(), b"nosniff".to_vec())];

    assert!(should_response_be_blocked_due_to_nosniff(
        &headers,
        FetchDestination::Script
    ));
    assert!(should_response_be_blocked_due_to_nosniff(
        &headers,
        FetchDestination::Style
    ));
    assert!(!should_response_be_blocked_due_to_nosniff(
        &headers,
        FetchDestination::Other
    ));
}

#[test]
fn orb_blocks_opaque_response_blocklisted_mime_types() {
    for content_type in [
        "text/plain",
        "text/html",
        "application/json",
        "text/json",
        "application/ld+json",
        "text/xml",
        "application/xml",
        "application/xhtml+xml",
        "font/ttf",
        "application/pdf",
        "text/csv",
    ] {
        assert!(
            should_opaque_response_be_blocked_by_orb(&[(
                "Content-Type".to_owned(),
                content_type.as_bytes().to_vec()
            )]),
            "{content_type} should be ORB-blocked"
        );
    }
}

#[test]
fn orb_allows_safelisted_opaque_response_mime_types() {
    for content_type in [
        "image/png",
        "image/svg+xml",
        "text/css",
        "text/javascript",
        "application/javascript",
    ] {
        assert!(
            !should_opaque_response_be_blocked_by_orb(&[(
                "Content-Type".to_owned(),
                content_type.as_bytes().to_vec()
            )]),
            "{content_type} should be ORB-allowed"
        );
    }
}

#[test]
fn orb_blocks_missing_or_empty_content_type_with_nosniff() {
    assert!(should_opaque_response_be_blocked_by_orb(&[(
        "X-Content-Type-Options".to_owned(),
        b"nosniff".to_vec()
    )]));
    assert!(should_opaque_response_be_blocked_by_orb(&[
        ("Content-Type".to_owned(), Vec::new()),
        ("X-Content-Type-Options".to_owned(), b"nosniff".to_vec()),
    ]));
    assert!(!should_opaque_response_be_blocked_by_orb(&[]));
}

#[test]
fn orb_body_sniffing_allows_mislabeled_images() {
    assert!(!should_opaque_response_be_blocked_by_orb_with_body(
        &[("Content-Type".to_owned(), b"text/html".to_vec())],
        b"\x89PNG\r\n\x1A\nrest"
    ));
    assert!(should_opaque_response_be_blocked_by_orb_with_body(
        &[("Content-Type".to_owned(), b"text/html".to_vec())],
        b"<!doctype html><title>secret</title>"
    ));
}

#[test]
fn orb_body_sniffing_allows_mislabeled_javascript_but_blocks_json() {
    assert!(!should_opaque_response_be_blocked_by_orb_with_body(
        &[("Content-Type".to_owned(), b"application/json".to_vec())],
        b"\"use strict\";\nfunction fn() { return 42; }"
    ));
    assert!(should_opaque_response_be_blocked_by_orb_with_body(
        &[("Content-Type".to_owned(), b"application/json".to_vec())],
        br#"{"hello":"world"}"#
    ));
}

#[test]
fn orb_body_sniffing_decodes_utf16_javascript_candidates() {
    let body: Vec<u8> = "\"use strict\";"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert!(!should_opaque_response_be_blocked_by_orb_with_body(
        &[(
            "Content-Type".to_owned(),
            b"application/json; charset=utf-16".to_vec()
        )],
        &body
    ));
}

#[test]
fn computes_response_mime_type_from_headers_and_body() {
    let image_without_type =
        computed_response_mime_type(&[], MimeSniffingContext::Image, b"\x89PNG\r\n\x1A\nrest");
    assert_eq!(image_without_type, "image/png");

    let explicit_script = computed_response_mime_type(
        &[("Content-Type".to_owned(), b"Text/JavaScript".to_vec())],
        MimeSniffingContext::Script,
        b"",
    );
    assert_eq!(explicit_script, "text/javascript");
}

#[test]
fn computes_data_url_mime_type_with_resource_context() {
    let (body, computed_mime_type) = data_url_body_and_computed_mime_type(
        "data:text/plain,GIF89a%01%00%01%00",
        MimeSniffingContext::Image,
    )
    .unwrap();
    assert_eq!(body, b"GIF89a\x01\x00\x01\x00");
    assert_eq!(computed_mime_type, "image/gif");

    let (_, computed_mime_type) =
        data_url_body_and_computed_mime_type("data:,hello", MimeSniffingContext::Browsing).unwrap();
    assert_eq!(computed_mime_type, "text/plain");

    assert!(
        data_url_body_and_computed_mime_type(
            "data:text/plain;base64,%%%%",
            MimeSniffingContext::Browsing,
        )
        .is_none()
    );

    assert_eq!(
        data_url_mime_type("data:application/importmap+json,%7B%7D").as_deref(),
        Some("application/importmap+json")
    );
    assert_eq!(
        data_url_mime_type("data:,hello").as_deref(),
        Some("text/plain;charset=US-ASCII")
    );
    let (body, mime_type) = data_url_body_and_mime_type("data:text/html;charset=gbk,%C4%E3")
        .expect("valid data URL should decode body and MIME type together");
    assert_eq!(body, vec![0xC4, 0xE3]);
    assert_eq!(mime_type, "text/html;charset=gbk");
    assert!(data_url_mime_type("data:text/plain;base64,%%%%").is_some());
    assert!(data_url_mime_type("https://example.test/").is_none());
}

#[test]
fn maps_known_url_path_mime_essences() {
    for (path, expected) in [
        ("/asset.BMP", "image/bmp"),
        ("/asset.css", "text/css"),
        ("/asset.gif", "image/gif"),
        ("/asset.jpg", "image/jpeg"),
        ("/asset.jpeg", "image/jpeg"),
        ("/asset.png", "image/png"),
        ("/asset.txt", "text/plain"),
        ("/asset.html", "text/html"),
        ("/asset.htm", "text/html"),
        ("/asset.svg", "image/svg+xml"),
        ("/asset.xhtml", "application/xhtml+xml"),
        ("/asset.xml", "application/xml"),
    ] {
        assert_eq!(known_url_path_mime_essence(path), Some(expected), "{path}");
    }

    assert_eq!(known_url_path_mime_essence("/asset.json"), None);
    assert_eq!(known_url_path_mime_essence("/asset.png/download"), None);
    assert_eq!(known_url_path_mime_essence("/assetpng"), None);
}

#[test]
fn checks_script_response_mime_for_nosniff_strict_and_classic_rules() {
    let html_nosniff: Vec<(String, Vec<u8>)> = vec![
        ("content-type".to_owned(), b"text/html".to_vec()),
        ("x-content-type-options".to_owned(), b"nosniff".to_vec()),
    ];
    assert_eq!(
        check_script_response_mime(&html_nosniff, b"", FetchDestination::Script, false),
        Err(ScriptResponseMimeError::Nosniff)
    );

    let html: Vec<(String, Vec<u8>)> = vec![("content-type".to_owned(), b"text/html".to_vec())];
    assert_eq!(
        check_script_response_mime(&html, b"", FetchDestination::Script, true),
        Err(ScriptResponseMimeError::Unsupported("text/html".to_owned()))
    );
    assert!(check_script_response_mime(&html, b"", FetchDestination::Script, false).is_ok());

    let image: Vec<(String, Vec<u8>)> = vec![("content-type".to_owned(), b"image/png".to_vec())];
    assert_eq!(
        check_script_response_mime(&image, b"", FetchDestination::Script, false),
        Err(ScriptResponseMimeError::Unsupported("image/png".to_owned()))
    );
}

#[test]
fn strict_script_response_mime_never_uses_a_sniffing_default() {
    for content_type in [None, Some(""), Some("not a mime type"), Some("text/")] {
        let headers: Vec<_> = content_type
            .map(|value| ("Content-Type".to_owned(), value.as_bytes().to_vec()))
            .into_iter()
            .collect();
        for body in [b"".as_slice(), b"self.executed = true;"] {
            for destination in [FetchDestination::Script, FetchDestination::Worker] {
                assert_eq!(
                    check_script_response_mime(&headers, body, destination, true),
                    Err(ScriptResponseMimeError::Unsupported(String::new())),
                    "{content_type:?}, {destination:?}"
                );
            }
            // Classic document scripts retain their permissive MIME rules.
            assert!(
                check_script_response_mime(&headers, body, FetchDestination::Script, false).is_ok()
            );
        }
    }
}

#[test]
fn strict_script_response_mime_accepts_supplied_javascript_essences() {
    for content_type in [
        "Text/JavaScript; charset=utf-8",
        "text/javascript; broken parameter",
        "application/x-javascript",
        "text/javascript1.5",
    ] {
        let headers = [("Content-Type".to_owned(), content_type.as_bytes().to_vec())];
        assert!(
            check_script_response_mime(&headers, b"", FetchDestination::Worker, true).is_ok(),
            "{content_type}"
        );
    }
}

#[test]
fn script_like_mime_type_block_matches_fetch_response_rule() {
    let image: Vec<(String, Vec<u8>)> = vec![("content-type".to_owned(), b"image/png".to_vec())];
    let video: Vec<(String, Vec<u8>)> = vec![("content-type".to_owned(), b"video/mp4".to_vec())];
    let csv: Vec<(String, Vec<u8>)> = vec![("content-type".to_owned(), b"text/csv".to_vec())];
    let html: Vec<(String, Vec<u8>)> = vec![("content-type".to_owned(), b"text/html".to_vec())];

    assert!(should_script_like_response_be_blocked_due_to_mime_type(
        &image
    ));
    assert!(should_script_like_response_be_blocked_due_to_mime_type(
        &video
    ));
    assert!(should_script_like_response_be_blocked_due_to_mime_type(
        &csv
    ));
    assert!(!should_script_like_response_be_blocked_due_to_mime_type(
        &html
    ));
    assert!(!should_script_like_response_be_blocked_due_to_mime_type(&[]));
}
#[test]
fn document_text_classification_excludes_html_and_xml() {
    for mime in [
        "text/plain",
        "Text/Css; charset=utf-8",
        "application/json",
        "application/ld+json",
        "application/javascript",
    ] {
        assert!(crate::is_text_document_mime(mime), "{mime}");
    }
    for mime in [
        "text/html",
        "text/xml",
        "application/xhtml+xml",
        "image/svg+xml",
        "image/png",
        "application/octet-stream",
    ] {
        assert!(!crate::is_text_document_mime(mime), "{mime}");
    }
}
