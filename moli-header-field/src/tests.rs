use super::*;

#[test]
fn token_and_separator_consumers_skip_optional_whitespace() {
    let mut tokenizer = HeaderFieldTokenizer::new(" \tattachment \t; \tfilename \t= \tfile.txt \t");

    assert_eq!(
        tokenizer.consume_token(HeaderFieldTokenMode::Normal),
        Some("attachment")
    );
    assert!(tokenizer.consume(';'));
    assert_eq!(
        tokenizer.consume_token(HeaderFieldTokenMode::Normal),
        Some("filename")
    );
    assert!(tokenizer.consume('='));
    assert_eq!(
        tokenizer.consume_token(HeaderFieldTokenMode::Normal),
        Some("file.txt")
    );
    assert!(tokenizer.is_consumed());
}

#[test]
fn normal_and_relaxed_modes_match_blink_tspecial_handling() {
    let mut normal = HeaderFieldTokenizer::new("type/subtype");
    assert_eq!(
        normal.consume_token(HeaderFieldTokenMode::Normal),
        Some("type")
    );
    assert!(!normal.is_consumed());

    let mut relaxed = HeaderFieldTokenizer::new("type/subtype");
    assert_eq!(
        relaxed.consume_token(HeaderFieldTokenMode::Relaxed),
        Some("type/subtype")
    );
    assert!(relaxed.is_consumed());

    for value in ["two words", "token;next", "token\"quoted"] {
        let mut tokenizer = HeaderFieldTokenizer::new(value);
        assert_ne!(
            tokenizer.consume_token(HeaderFieldTokenMode::Relaxed),
            Some(value),
            "separator must remain outside relaxed tokens: {value:?}"
        );
    }
}

#[test]
fn quoted_strings_preserve_semicolons_unicode_and_quoted_pairs() {
    let mut tokenizer = HeaderFieldTokenizer::new(r#" "x=y;y=\"\pz; ;;你好" ; next=value"#);

    assert_eq!(
        tokenizer
            .consume_token_or_quoted_string(HeaderFieldTokenMode::Normal)
            .as_deref(),
        Some("x=y;y=\"pz; ;;你好")
    );
    assert!(tokenizer.consume(';'));
    assert_eq!(
        tokenizer.consume_token(HeaderFieldTokenMode::Normal),
        Some("next")
    );
}

#[test]
fn quoted_empty_string_is_distinct_from_an_empty_token() {
    let mut quoted = HeaderFieldTokenizer::new(r#""""#);
    assert_eq!(quoted.consume_quoted_string().as_deref(), Some(""));
    assert!(quoted.is_consumed());

    let mut empty = HeaderFieldTokenizer::new("");
    assert_eq!(empty.consume_token(HeaderFieldTokenMode::Normal), None);
    assert_eq!(
        empty.consume_token_or_quoted_string(HeaderFieldTokenMode::Normal),
        None
    );
}

#[test]
fn malformed_quoted_strings_fail() {
    for value in [r#""unterminated"#, r#""terminal escape\"#] {
        let mut tokenizer = HeaderFieldTokenizer::new(value);
        assert_eq!(tokenizer.consume_quoted_string(), None, "{value:?}");
    }
}

#[test]
fn non_ascii_requires_quoting_but_del_matches_blink_token_behavior() {
    let mut unquoted_unicode = HeaderFieldTokenizer::new("你好");
    assert_eq!(
        unquoted_unicode.consume_token(HeaderFieldTokenMode::Normal),
        None
    );

    let mut quoted_unicode = HeaderFieldTokenizer::new(r#""你好""#);
    assert_eq!(
        quoted_unicode.consume_quoted_string().as_deref(),
        Some("你好")
    );

    // RFC MIME token grammar excludes DEL as a control. Blink's effective
    // implementation accepts it, so the shared compatibility primitive
    // deliberately does too.
    let mut del = HeaderFieldTokenizer::new("\u{7f}");
    assert_eq!(
        del.consume_token(HeaderFieldTokenMode::Normal),
        Some("\u{7f}")
    );
}

#[test]
fn consume_before_any_match_stops_without_consuming_the_separator() {
    let mut tokenizer = HeaderFieldTokenizer::new("alpha,beta;gamma");

    tokenizer.consume_before_any_char_match(&[',', ';']);
    assert_eq!(tokenizer.byte_index(), "alpha".len());
    assert!(tokenizer.consume(','));
    assert_eq!(
        tokenizer.consume_token(HeaderFieldTokenMode::Normal),
        Some("beta")
    );
}

#[test]
fn separators_inside_a_quoted_string_do_not_split() {
    assert_eq!(
        split_outside_quoted_strings("text/html; boundary=\"; charset=gbk\"", ';'),
        vec!["text/html", " boundary=\"; charset=gbk\""]
    );
    assert_eq!(
        split_outside_quoted_strings("private=\"Set-Cookie, X-Auth\", max-age=60", ','),
        vec!["private=\"Set-Cookie, X-Auth\"", " max-age=60"]
    );
}

#[test]
fn an_escaped_quote_does_not_close_a_quoted_string() {
    assert_eq!(
        split_outside_quoted_strings("attachment; name=\"a\\\"; x=1\"; y=2", ';'),
        vec!["attachment", " name=\"a\\\"; x=1\"", " y=2"]
    );
}

#[test]
fn splitting_without_quotes_matches_a_plain_split() {
    for value in [
        "text/html; charset=utf-8",
        "a;b;c",
        "",
        ";",
        "trailing;",
        ";leading",
    ] {
        assert_eq!(
            split_outside_quoted_strings(value, ';'),
            value.split(';').collect::<Vec<_>>(),
            "value={value}"
        );
    }
}

#[test]
fn an_unterminated_quoted_string_runs_to_the_end() {
    assert_eq!(
        split_outside_quoted_strings("text/html; charset=\"gbk; q=1", ';'),
        vec!["text/html", " charset=\"gbk; q=1"]
    );
}

#[test]
fn quoted_values_lose_their_delimiters_and_backslashes() {
    assert_eq!(unquote_parameter_value("\"utf-8\""), "utf-8");
    assert_eq!(unquote_parameter_value("\"utf\\-8\""), "utf-8");
    assert_eq!(unquote_parameter_value("\"a\\\"b\""), "a\"b");
    // Not a quoted string, so it is returned untouched.
    assert_eq!(unquote_parameter_value("utf-8"), "utf-8");
    assert_eq!(unquote_parameter_value("'utf-8'"), "'utf-8'");
    // Unterminated keeps what was read.
    assert_eq!(unquote_parameter_value("\"gbk"), "gbk");
}

#[test]
fn multibyte_values_keep_character_boundaries() {
    let value = "text/plain; name=\"café; x\"; charset=utf-8";
    assert_eq!(
        split_outside_quoted_strings(value, ';'),
        vec!["text/plain", " name=\"café; x\"", " charset=utf-8"]
    );
    assert_eq!(unquote_parameter_value("\"caf\\é\""), "café");
}

#[test]
fn a_quote_outside_a_parameter_value_is_ordinary_data() {
    // A `"` that does not open a parameter value must not swallow the rest of
    // the field, or the parameters behind it become invisible.
    assert_eq!(
        split_outside_quoted_strings("text/html;\";charset=gbk", ';'),
        vec!["text/html", "\"", "charset=gbk"]
    );
    assert_eq!(
        split_outside_quoted_strings("a\"b;c", ';'),
        vec!["a\"b", "c"]
    );
}

#[test]
fn whitespace_between_equals_and_a_quoted_value_still_opens_it() {
    assert_eq!(
        split_outside_quoted_strings("text/html; boundary =  \"; x\"; charset=utf-8", ';'),
        vec!["text/html", " boundary =  \"; x\"", " charset=utf-8"]
    );
}

#[test]
fn a_trailing_backslash_is_kept_rather_than_dropped() {
    // Dropping it would turn a malformed value into a well-formed one.
    assert_eq!(unquote_parameter_value("\"31536000\\"), "31536000\\");
    assert_eq!(unquote_parameter_value("\"a\\"), "a\\");
}

#[test]
fn an_escaped_quote_survives_as_data() {
    assert_eq!(unquote_parameter_value("\"utf-8\\\"\""), "utf-8\"");
}
