use super::*;

fn response_charset(input: &str) -> Option<String> {
    parse_response_content_type(input)?
        .charset()
        .map(str::to_owned)
}

fn assert_response_content_type(
    input: &str,
    expected_mime_type: Option<&str>,
    expected_charset: Option<&str>,
) {
    let parsed = parse_response_content_type(input);
    assert_eq!(
        parsed.as_ref().map(ResponseContentType::mime_type),
        expected_mime_type,
        "mime type for {input:?}"
    );
    assert_eq!(
        parsed.as_ref().and_then(|value| value.charset()),
        expected_charset,
        "charset for {input:?}"
    );
}

#[test]
fn whatwg_parser_validates_the_mime_essence() {
    let parsed = parse_mime_type(" Text/Plain ; Charset=\"UTF-8\" ").unwrap();
    assert_eq!(parsed.type_(), "text");
    assert_eq!(parsed.subtype(), "plain");
    assert_eq!(parsed.essence(), "text/plain");
    assert_eq!(parsed.parameter("CHARSET"), Some("UTF-8"));
    assert_eq!(parsed.to_string(), "text/plain;charset=UTF-8");

    assert!(parse_mime_type("garbage; charset=utf-8").is_none());
    assert!(parse_mime_type("text/").is_none());
}

#[test]
fn replacing_a_mime_parameter_preserves_order_and_quoted_value_serialization() {
    let mut mime = parse_mime_type(
        r#"Text/Plain; title="alpha; \"beta\""; Charset=ASCII; charset=gbk; keep=Value"#,
    )
    .unwrap();
    *mime.parameter_mut("CHARSET").unwrap() = "UTF-8".to_owned();
    assert_eq!(
        mime.to_string(),
        r#"text/plain;title="alpha; \"beta\"";charset=UTF-8;keep=Value"#
    );
    assert!(mime.parameter_mut("missing").is_none());
}

#[test]
fn whatwg_parser_recovers_valid_parameters() {
    assert_eq!(
        mime_parameter("text/plain; title=\"alpha;beta\"", "title").as_deref(),
        Some("alpha;beta")
    );
    assert_eq!(
        mime_charset("text/plain; charset=; charset=gbk").as_deref(),
        Some("gbk")
    );
    assert_eq!(
        mime_charset("text/plain; charset=\"\"; charset=gbk").as_deref(),
        Some("")
    );
    assert_eq!(
        mime_charset("text/plain; charset=\"utf-8").as_deref(),
        Some("utf-8")
    );
}

#[test]
fn response_parser_matches_chromium_quoted_parameter_tolerances() {
    assert_eq!(
        response_charset("text/html; boundary=\"; charset=gbk\""),
        None
    );
    assert_eq!(
        response_charset("text/html; name=\"a\\\"; charset=gbk\"; charset=utf-8").as_deref(),
        Some("utf-8")
    );
    assert_eq!(
        response_charset("text/html; charset=\"\\utf\\-\\8\"").as_deref(),
        Some("utf-8")
    );
    assert_eq!(
        response_charset("text/html; charset=\"utf-8").as_deref(),
        Some("utf-8")
    );
    assert_eq!(
        response_charset("text/html; charset=\"\\\\\\\"\\").as_deref(),
        Some("\\\"\\")
    );
}

#[test]
fn response_parser_does_not_reopen_quotes_after_a_value_started() {
    assert_eq!(
        response_charset("text/html; x=a=\"unterminated; charset=gbk").as_deref(),
        Some("gbk")
    );
    assert_eq!(
        response_charset("text/html; x=\"ok\"junk=\"unterminated; charset=gbk").as_deref(),
        Some("gbk")
    );
}

#[test]
fn response_parser_preserves_chromium_empty_and_duplicate_rules() {
    assert_eq!(
        response_charset("text/html; charset=; charset=gbk").as_deref(),
        Some("gbk")
    );
    assert_eq!(
        response_charset("text/html; charset=\"\"; charset=gbk").as_deref(),
        Some("")
    );
    assert_eq!(
        response_charset("text/html; charset=foo; charset=utf-8").as_deref(),
        Some("foo")
    );
}

#[test]
fn response_parser_preserves_parameter_name_and_quote_semantics() {
    assert_eq!(response_charset("text/html; charset =utf-8"), None);
    assert_eq!(
        response_charset("text/html; charset='utf-8'").as_deref(),
        Some("'utf-8'")
    );
    assert_eq!(
        response_charset("text/html; \"; \"\"; charset=utf-8").as_deref(),
        Some("utf-8")
    );
    assert_eq!(
        response_charset("text/html; charset=u\"tf-8\"").as_deref(),
        Some("u\"tf-8\"")
    );
}

#[test]
fn response_parser_http_lws_is_only_space_and_tab() {
    for newline in ['\r', '\n'] {
        let unquoted = format!("text/html; charset={newline}gbk");
        let quoted = format!("text/html; charset=\"{newline}gbk\"");
        let prefixed_name = format!("text/html;{newline}charset=gbk");
        let expected = format!("{newline}gbk");

        assert_eq!(
            response_charset(&unquoted).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            response_charset(&quoted).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(response_charset(&prefixed_name), None);
    }

    assert_eq!(
        response_charset("text/html; charset= \tgbk\t ").as_deref(),
        Some("gbk")
    );
}

#[test]
fn response_parser_ascii_lowercases_charset() {
    let parsed = parse_response_content_type("text/html; charset=\tX-\u{dc}TF-A\t").unwrap();

    assert_eq!(parsed.charset(), Some("x-\u{dc}tf-a"));
}

#[test]
fn response_parser_has_a_distinct_network_level_validity_boundary() {
    assert!(parse_response_content_type("garbage; charset=utf-8").is_none());
    assert!(parse_response_content_type("*/*").is_none());
    assert_eq!(
        parse_response_content_type("text/")
            .map(|parsed| parsed.mime_type().to_owned())
            .as_deref(),
        Some("text/")
    );
    assert_eq!(
        response_charset("*/*; charset=utf-8").as_deref(),
        Some("utf-8")
    );
}

#[test]
fn response_parser_matches_chromium_http_util_regression_matrix() {
    for (input, mime_type, charset) in [
        ("text/html", Some("text/html"), None),
        ("text/html;", Some("text/html"), None),
        ("text/html; charset=UTF-8", Some("text/html"), Some("utf-8")),
        ("text/html; charset =utf-8", Some("text/html"), None),
        (
            "text/html; charset= utf-8",
            Some("text/html"),
            Some("utf-8"),
        ),
        (
            "text/html; charset=utf-8 ",
            Some("text/html"),
            Some("utf-8"),
        ),
        ("text/html; charset", Some("text/html"), None),
        ("text/html; charset=", Some("text/html"), None),
        ("text/html; charset= ", Some("text/html"), None),
        ("text/html; charset= ;", Some("text/html"), None),
        ("text/html; charset=\"\"", Some("text/html"), Some("")),
        ("text/html; charset=\" \"", Some("text/html"), Some("")),
        (
            "text/html; charset=\" foo \"",
            Some("text/html"),
            Some("foo"),
        ),
        (
            "text/html; charset=foo; charset=utf-8",
            Some("text/html"),
            Some("foo"),
        ),
        (
            "text/html; charset; charset=; charset=utf-8",
            Some("text/html"),
            Some("utf-8"),
        ),
        (
            "text/html; charset=utf-8; charset=; charset",
            Some("text/html"),
            Some("utf-8"),
        ),
        (
            "text/html; \"; \"\"; charset=utf-8",
            Some("text/html"),
            Some("utf-8"),
        ),
        (
            "text/html; charset=u\"tf-8\"",
            Some("text/html"),
            Some("u\"tf-8\""),
        ),
        (
            "text/html; charset=\"utf-8",
            Some("text/html"),
            Some("utf-8"),
        ),
        (
            "text/html; charset=\";charset=utf-8;\"",
            Some("text/html"),
            Some(";charset=utf-8;"),
        ),
        (
            "text/html; charset='utf-8'",
            Some("text/html"),
            Some("'utf-8'"),
        ),
        ("text/", Some("text/"), None),
        ("*/*", None, None),
        ("*/*; charset=utf-8", Some("*/*"), Some("utf-8")),
        ("*/* ", Some("*/*"), None),
        ("teXT/html", Some("text/html"), None),
    ] {
        assert_response_content_type(input, mime_type, charset);
    }

    let parsed = parse_response_content_type(
        "text/html; boundary=\"WebKit-ada-df-dsf-adsfadsfs  \"; charset=UTF-8; boundary=ignored",
    )
    .unwrap();
    assert_eq!(parsed.charset(), Some("utf-8"));
    assert_eq!(parsed.boundary(), Some("WebKit-ada-df-dsf-adsfadsfs"));
}

#[test]
fn normalizing_web_api_mime_types_rejects_non_http_bytes() {
    assert_eq!(normalize_web_api_mime_type("Text/Plain"), "text/plain");
    assert_eq!(normalize_web_api_mime_type("text/\nplain"), "");
    assert_eq!(normalize_web_api_mime_type("text/你好"), "");
}
