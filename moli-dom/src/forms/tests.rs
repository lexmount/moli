use super::*;

#[test]
fn form_control_validity_computes_valid_and_default_message() {
    assert!(FormControlValidity::default().valid());
    assert_eq!(FormControlValidity::default().validation_message(""), "");

    let missing = FormControlValidity {
        value_missing: true,
        ..FormControlValidity::default()
    };
    assert!(!missing.valid());
    assert_eq!(
        missing.validation_message(""),
        "Please fill out this field."
    );
    assert_eq!(missing.validation_message("custom"), "custom");

    let range = FormControlValidity {
        range_overflow: true,
        ..FormControlValidity::default()
    };
    assert_eq!(
        range.validation_message(""),
        "Please select a value in the allowed range."
    );

    let bad_input = FormControlValidity {
        bad_input: true,
        ..FormControlValidity::default()
    };
    assert_eq!(bad_input.validation_message(""), "Please enter a number.");
}

#[test]
fn custom_validation_message_normalizes_to_lf() {
    assert_eq!(
        normalize_custom_validation_message("a\r\nb\rc\nd"),
        "a\nb\nc\nd"
    );
}

#[test]
fn form_submission_newlines_normalize_to_crlf() {
    assert_eq!(
        normalize_form_submission_newlines("a\r\nb\rc\nd"),
        "a\r\nb\r\nc\r\nd"
    );
}

#[test]
fn integer_prefix_parsers_follow_html_attribute_rules() {
    assert_eq!(parse_non_negative_integer_prefix("  12px"), 12);
    assert_eq!(parse_non_negative_integer_prefix("12像素"), 12);
    assert_eq!(parse_non_negative_integer_prefix("\u{a0}12px"), 12);
    assert_eq!(parse_non_negative_integer_prefix("2147483647"), i32::MAX);
    assert_eq!(parse_non_negative_integer_prefix("2147483648"), 0);
    assert_eq!(parse_non_negative_integer_prefix("abc"), 0);
    assert_eq!(parse_non_negative_integer_prefix("999999999999"), 0);

    assert_eq!(parse_positive_integer_prefix("  12px"), Some(12));
    assert_eq!(parse_positive_integer_prefix("  +12px"), Some(12));
    assert_eq!(parse_positive_integer_prefix("\u{a0}12px"), None);
    assert_eq!(parse_positive_integer_prefix("0"), None);
    assert_eq!(parse_positive_integer_prefix("-1"), None);
    assert_eq!(parse_positive_integer_prefix("abc"), None);
    assert_eq!(parse_positive_integer_prefix("999999999999"), None);

    assert_eq!(parse_non_negative_length_attribute("  +12px"), Some(12));
    assert_eq!(parse_non_negative_length_attribute("-0tail"), Some(0));
    assert_eq!(parse_non_negative_length_attribute("-1"), None);
    assert_eq!(parse_non_negative_length_attribute("abc"), None);
    assert_eq!(parse_non_negative_length_attribute("2147483648"), None);
}

#[test]
fn textarea_wrapping_transformation_respects_hard_state_and_character_width() {
    let wrap = |value: &str, wrap, cols| {
        apply_textarea_wrapping_transformation(value.to_owned(), wrap, cols)
    };

    assert_eq!(wrap("hello world", Some("soft"), Some("7")), "hello world");
    assert_eq!(wrap("hello world", Some("ſoft"), Some("7")), "hello world");
    assert_eq!(wrap("1234567", Some("hard"), Some("7")), "1234567");
    assert_eq!(
        wrap("hello world", Some("HaRd"), Some("7")),
        "hello w\norld"
    );
    assert_eq!(wrap("ab\ncdef", Some("hard"), Some("3")), "ab\ncde\nf");
    assert_eq!(wrap("é🙂x", Some("hard"), Some("2")), "é🙂\nx");
    assert_eq!(
        wrap("123456789012345678901", Some("hard"), None),
        "12345678901234567890\n1"
    );
}

#[test]
fn input_type_tokens_and_support_matrices_are_shared() {
    assert!(InputType::Week.supports_value_as_number());
    assert!(!InputType::Email.supports_value_as_number());

    assert!(InputType::Password.supports_pattern());
    assert!(!InputType::Number.supports_pattern());

    assert!(InputType::Email.supports_text_length_validation());
    assert!(!InputType::Date.supports_text_length_validation());

    assert!(InputType::Number.supports_readonly());
    assert!(!InputType::Checkbox.supports_readonly());

    assert!(form_control_type_supports_intrinsic_validation(
        "input",
        Some(InputType::Text),
        false
    ));
    assert!(!form_control_type_supports_intrinsic_validation(
        "input",
        Some(InputType::Hidden),
        false
    ));
    assert!(form_control_type_supports_intrinsic_validation(
        "input",
        Some(InputType::Submit),
        false
    ));
    assert!(form_control_type_supports_intrinsic_validation(
        "input",
        Some(InputType::Image),
        false
    ));
    assert!(!form_control_type_supports_intrinsic_validation(
        "button", None, false
    ));
    assert!(form_control_type_supports_intrinsic_validation(
        "button", None, true
    ));
}

#[test]
fn input_type_mismatch_and_length_rules_are_shared() {
    assert!(!input_type_value_mismatch(
        InputType::Email,
        "a@example.com",
        false
    ));
    assert!(input_type_value_mismatch(InputType::Email, "a@", false));
    assert!(!input_type_value_mismatch(
        InputType::Email,
        "a@example.com, b@example.org",
        true
    ));
    assert!(input_type_value_mismatch(
        InputType::Email,
        "a@example.com, broken",
        true
    ));

    assert!(!input_type_value_mismatch(
        InputType::Url,
        "https://example.com/a",
        false
    ));
    assert!(input_type_value_mismatch(
        InputType::Url,
        "not a url",
        false
    ));
    assert!(!input_type_value_mismatch(
        InputType::Text,
        "not a url",
        false
    ));

    assert_eq!(text_control_value_length("a\u{1f980}"), 3);
    assert!(text_control_suffers_too_long("abcd", Some("3")));
    assert!(!text_control_suffers_too_long("abcd", Some("bad")));
    assert!(text_control_suffers_too_short("ab", Some("3")));
    assert!(!text_control_suffers_too_short("", Some("3")));
}

#[test]
fn input_value_sanitization_uses_shared_temporal_parsers() {
    assert_eq!(
        sanitize_input_value_for_type(InputType::Text, "a\nb\rc"),
        "abc"
    );
    assert_eq!(
        sanitize_input_value_for_type(InputType::Search, "a\r\nb"),
        "ab"
    );
    assert_eq!(
        sanitize_input_value_for_type(InputType::Password, "\nsecret\r"),
        "secret"
    );
    assert_eq!(sanitize_input_value_for_type(InputType::Number, "NaN"), "");
    assert_eq!(sanitize_input_value_for_type(InputType::Number, "1."), "");
    assert_eq!(sanitize_input_value_for_type(InputType::Number, "+1"), "");
    assert_eq!(sanitize_input_value_for_type(InputType::Number, " 1"), "");
    assert_eq!(
        sanitize_input_value_for_type(InputType::Date, "2024-02-30"),
        ""
    );
    assert_eq!(sanitize_input_value_for_type(InputType::Time, "24:00"), "");
    assert_eq!(
        sanitize_input_value_for_type(InputType::DatetimeLocal, "2024-02-29 12:30"),
        "2024-02-29T12:30"
    );
    assert_eq!(
        sanitize_input_value_for_type(InputType::DatetimeLocal, "2022-04-19T12:34:56.010"),
        "2022-04-19T12:34:56.01"
    );
    assert_eq!(
        sanitize_input_value_for_type(InputType::Month, "2024-13"),
        ""
    );
    assert_eq!(
        sanitize_input_value_for_type(InputType::Week, "2024-W99"),
        ""
    );
    assert_eq!(
        sanitize_input_value_for_type(InputType::Text, "2024-W99"),
        "2024-W99"
    );
    assert!(input_type_has_value_sanitization(InputType::DatetimeLocal));
}

#[test]
fn input_numeric_value_and_step_rules_are_shared() {
    assert_eq!(
        parse_input_numeric_value(InputType::Number, "1.25"),
        Some(1.25)
    );
    assert_eq!(
        parse_input_numeric_value(InputType::Range, "50"),
        Some(50.0)
    );
    assert_eq!(
        parse_input_numeric_value(InputType::Date, "1970-01-02"),
        Some(MS_PER_DAY)
    );
    assert_eq!(
        input_number_to_value_string(InputType::Month, 13.0),
        Some("1971-02".to_owned())
    );
    assert_eq!(
        input_number_to_value_string(InputType::Range, 50.0),
        Some("50".to_owned())
    );
    assert_eq!(
        input_number_to_value_string(InputType::Time, -MS_PER_HOUR),
        Some("23:00".to_owned())
    );
    assert_eq!(
        input_number_to_value_string(InputType::Time, 2.734_333_707_189_448e26),
        Some("10:54:10.944".to_owned())
    );

    assert_eq!(input_step(InputType::Number, None), Some(1.0));
    assert_eq!(
        input_step(InputType::Time, Some("2")),
        Some(2.0 * MS_PER_SECOND)
    );
    assert_eq!(input_step(InputType::Week, Some("any")), None);
    assert_eq!(
        input_step_base(InputType::Week, None, None),
        WEEK_INPUT_STEP_BASE
    );
    assert_eq!(
        input_step_base(InputType::Date, Some("1970-01-02"), Some("1970-01-03")),
        MS_PER_DAY
    );

    assert!(number_aligns_to_step(4.0, 0.0, 2.0));
    assert!(input_range_underflow(
        InputType::Number,
        2.0,
        Some("3"),
        Some("8")
    ));
    assert!(!input_range_underflow(
        InputType::Number,
        4.0,
        Some("3"),
        Some("8")
    ));
    assert!(input_range_overflow(
        InputType::Number,
        9.0,
        Some("3"),
        Some("8")
    ));
    assert!(!input_range_overflow(
        InputType::Number,
        4.0,
        Some("3"),
        Some("8")
    ));
    assert!(input_range_underflow(
        InputType::Time,
        time_input_milliseconds("06:00").unwrap(),
        Some("23:00"),
        Some("05:00")
    ));
    assert!(input_range_overflow(
        InputType::Time,
        time_input_milliseconds("06:00").unwrap(),
        Some("23:00"),
        Some("05:00")
    ));
    assert_eq!(
        number_step_mismatch("1.5", Some("0.5"), None, None),
        Some(false)
    );
    assert_eq!(
        number_step_mismatch("1.5", Some("1"), None, None),
        Some(true)
    );
    assert_eq!(number_step_mismatch("1.5", Some("any"), None, None), None);
}

#[test]
fn progress_values_share_html_numeric_and_clamping_rules() {
    assert_eq!(
        progress_element_values(None, None),
        ProgressElementValues {
            value: 0.0,
            max: 1.0,
            position: -1.0,
        }
    );
    assert_eq!(
        progress_element_values(Some("1"), Some("3")),
        ProgressElementValues {
            value: 1.0,
            max: 3.0,
            position: 1.0 / 3.0,
        }
    );
    assert_eq!(
        progress_element_values(Some("8"), Some("4")),
        ProgressElementValues {
            value: 4.0,
            max: 4.0,
            position: 1.0,
        }
    );
    assert_eq!(
        progress_element_values(Some("invalid"), Some("0")),
        ProgressElementValues {
            value: 0.0,
            max: 1.0,
            position: 0.0,
        }
    );
    assert_eq!(
        parse_html_floating_point_prefix("  +1.25e2tail"),
        Some(125.0)
    );
    assert_eq!(parse_html_floating_point_prefix("invalid"), None);
}

#[test]
fn meter_values_share_html_numeric_clamping_and_gauge_region_rules() {
    assert_eq!(
        meter_element_values(None, None, None, None, None, None),
        MeterElementValues {
            value: 0.0,
            min: 0.0,
            max: 1.0,
            low: 0.0,
            high: 1.0,
            optimum: 0.5,
            position: 0.0,
            gauge_region: MeterGaugeRegion::Optimum,
        }
    );
    assert_eq!(
        meter_element_values(
            Some("15"),
            Some("10"),
            Some("20"),
            Some("12"),
            Some("18"),
            Some("15"),
        ),
        MeterElementValues {
            value: 15.0,
            min: 10.0,
            max: 20.0,
            low: 12.0,
            high: 18.0,
            optimum: 15.0,
            position: 0.5,
            gauge_region: MeterGaugeRegion::Optimum,
        }
    );
    assert_eq!(
        meter_element_values(
            Some("90"),
            Some("0"),
            Some("100"),
            Some("25"),
            Some("75"),
            Some("10"),
        )
        .gauge_region,
        MeterGaugeRegion::EvenLessGood
    );
    assert_eq!(
        meter_element_values(
            Some("25"),
            Some("0"),
            Some("100"),
            Some("25"),
            Some("75"),
            Some("90"),
        )
        .gauge_region,
        MeterGaugeRegion::Suboptimum
    );
    assert_eq!(
        meter_element_values(
            Some("7"),
            Some("10"),
            Some("5"),
            Some("80"),
            Some("20"),
            Some("999"),
        ),
        MeterElementValues {
            value: 10.0,
            min: 10.0,
            max: 10.0,
            low: 10.0,
            high: 10.0,
            optimum: 10.0,
            position: 0.0,
            gauge_region: MeterGaugeRegion::Optimum,
        }
    );
}

#[test]
fn input_step_up_down_rules_are_shared() {
    let stepped = step_input_value(
        InputStepState {
            input_type: InputType::Number,
            value: "2",
            min: Some("0"),
            max: Some("10"),
            step: Some("2"),
            value_attribute: None,
        },
        InputStepDirection::Up,
        1.0,
    )
    .unwrap();
    assert_eq!(stepped, InputStepOutcome::Set("4".to_owned()));

    let unaligned = step_input_value(
        InputStepState {
            input_type: InputType::Number,
            value: "3",
            min: Some("0"),
            max: Some("10"),
            step: Some("2"),
            value_attribute: None,
        },
        InputStepDirection::Down,
        1.0,
    )
    .unwrap();
    assert_eq!(unaligned, InputStepOutcome::Set("2".to_owned()));

    let empty_with_positive_min = step_input_value(
        InputStepState {
            input_type: InputType::Number,
            value: "",
            min: Some("7"),
            max: None,
            step: None,
            value_attribute: None,
        },
        InputStepDirection::Down,
        1.0,
    )
    .unwrap();
    assert_eq!(
        empty_with_positive_min,
        InputStepOutcome::Set("7".to_owned())
    );

    let below_min_step_down = step_input_value(
        InputStepState {
            input_type: InputType::Number,
            value: "3",
            min: Some("7"),
            max: None,
            step: None,
            value_attribute: None,
        },
        InputStepDirection::Down,
        1.0,
    )
    .unwrap();
    assert_eq!(below_min_step_down, InputStepOutcome::NoChange);

    let clamped = step_input_value(
        InputStepState {
            input_type: InputType::Date,
            value: "1970-01-01",
            min: Some("1970-01-01"),
            max: Some("1970-01-02"),
            step: None,
            value_attribute: None,
        },
        InputStepDirection::Up,
        5.0,
    )
    .unwrap();
    assert_eq!(clamped, InputStepOutcome::Set("1970-01-02".to_owned()));

    let huge_date_step = step_input_value(
        InputStepState {
            input_type: InputType::Date,
            value: "",
            min: Some("2010-02-10"),
            max: None,
            step: Some("9223372036854775556"),
            value_attribute: None,
        },
        InputStepDirection::Down,
        1.0,
    )
    .unwrap();
    assert_eq!(
        huge_date_step,
        InputStepOutcome::Set("2010-02-10".to_owned())
    );

    let impossible_range = step_input_value(
        InputStepState {
            input_type: InputType::Number,
            value: "1",
            min: Some("10"),
            max: Some("1"),
            step: None,
            value_attribute: None,
        },
        InputStepDirection::Up,
        1.0,
    )
    .unwrap();
    assert_eq!(impossible_range, InputStepOutcome::NoChange);

    let no_allowed_step = step_input_value(
        InputStepState {
            input_type: InputType::Number,
            value: "1",
            min: None,
            max: None,
            step: Some("any"),
            value_attribute: None,
        },
        InputStepDirection::Up,
        1.0,
    );
    assert_eq!(no_allowed_step, Err(InputStepError::NoAllowedStep));

    let unsupported = step_input_value(
        InputStepState {
            input_type: InputType::Text,
            value: "1",
            min: None,
            max: None,
            step: None,
            value_attribute: None,
        },
        InputStepDirection::Up,
        1.0,
    );
    assert_eq!(unsupported, Err(InputStepError::Unsupported));
}
