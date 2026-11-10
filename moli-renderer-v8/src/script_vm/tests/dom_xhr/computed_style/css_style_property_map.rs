use super::*;

#[test]
fn native_column_count_numbers_reify_without_changing_range_validation() {
    use moli_css_parse::parse_typed_style_value;
    use style::typed_om::{NumericValue, TypedValue};

    crate::style_engine::ensure_stylo_browser_compat_prefs();
    for (source, expected) in [("1", 1.0), ("2", 2.0), ("12", 12.0)] {
        let parsed = parse_typed_style_value("column-count", source, None).unwrap();
        assert!(matches!(
            parsed.values.unwrap().native().unwrap().values.as_slice(),
            [TypedValue::Numeric(NumericValue::Unit(value))]
                if value.unit_str() == "number" && value.value == expected
        ));
    }
    for source in [
        "calc(0)",
        "calc(-3.14)",
        "calc(3.14)",
        "calc(2 + 3)",
        "min(2, 3)",
    ] {
        let parsed = parse_typed_style_value("column-count", source, None).unwrap();
        assert!(matches!(
            parsed.values.unwrap().native().unwrap().values.as_slice(),
            [TypedValue::Numeric(_)]
        ));
    }
    for source in ["auto", "AUTO", "\\61 uto"] {
        let parsed = parse_typed_style_value("column-count", source, None).unwrap();
        assert!(matches!(
            parsed.values.unwrap().native().unwrap().values.as_slice(),
            [TypedValue::Keyword(value)] if value.0 == "auto"
        ));
    }
    for source in ["0", "-1", "1.5", "none", "2px", "50%", "1 2"] {
        assert!(parse_typed_style_value("column-count", source, None).is_none());
    }
}

#[test]
fn css_style_property_maps_reify_numeric_column_count_values() {
    let mut vm = new_parsed_test_vm(
        "https://css-column-count.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm.eval(include_str!("css_typed_column_count.js")).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["cases"].as_array().map(Vec::len), Some(325));
}

#[test]
fn native_font_palette_builtin_keywords_leave_named_palettes_opaque() {
    use moli_css_parse::parse_typed_style_value;
    use style::typed_om::TypedValue;

    crate::style_engine::ensure_stylo_browser_compat_prefs();
    for (source, expected) in [
        ("normal", "normal"),
        ("NORMAL", "normal"),
        ("light", "light"),
        ("LIGHT", "light"),
        ("dark", "dark"),
        ("d\\61rk", "dark"),
    ] {
        let parsed = parse_typed_style_value("font-palette", source, None).unwrap();
        assert_eq!(parsed.css_text, expected);
        assert!(matches!(
            parsed.values.unwrap().native().unwrap().values.as_slice(),
            [TypedValue::Keyword(value)] if value.0 == expected
        ));
    }
    for source in [
        "--foo",
        "--Fóo",
        "--normal",
        "--foo\\2e bar",
        "--foo\\20 bar",
    ] {
        let parsed = parse_typed_style_value("font-palette", source, None).unwrap();
        assert!(parsed.values.is_none(), "{source}");
    }
    assert!(parse_typed_style_value("font-palette", "Augusta", None).is_none());
}

#[test]
fn css_style_property_maps_reify_builtin_font_palettes() {
    let mut vm = new_parsed_test_vm(
        "https://css-font-palette.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm.eval(include_str!("css_typed_font_palette.js")).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["cases"].as_array().map(Vec::len), Some(225));
}

#[test]
fn native_font_style_keywords_preserve_angular_and_calculated_values() {
    use moli_css_parse::parse_typed_style_value;
    use style::typed_om::{ToTyped, TypedValue};
    use style::values::computed::font::FontStyle as ComputedFontStyle;

    crate::style_engine::ensure_stylo_browser_compat_prefs();
    for (source, expected) in [
        ("normal", "normal"),
        ("ITALIC", "italic"),
        ("oblique", "oblique"),
        ("o\\62lique", "oblique"),
        ("oblique 0deg", "normal"),
        ("oblique 14deg", "oblique"),
    ] {
        let parsed = parse_typed_style_value("font-style", source, None).unwrap();
        assert_eq!(parsed.css_text, expected);
        assert!(matches!(
            parsed.values.unwrap().native().unwrap().values.as_slice(),
            [TypedValue::Keyword(value)] if value.0 == expected
        ));
    }
    for source in [
        "oblique 23deg",
        "oblique -10deg",
        "oblique calc(7deg + 7deg)",
        "oblique calc(0deg)",
        "oblique calc(200deg)",
    ] {
        let parsed = parse_typed_style_value("font-style", source, None).unwrap();
        assert!(parsed.values.is_none(), "{source}");
    }
    for (style, keyword) in [
        (ComputedFontStyle::NORMAL, "normal"),
        (ComputedFontStyle::ITALIC, "italic"),
        (ComputedFontStyle::OBLIQUE, "oblique"),
    ] {
        let values = style.to_typed_value_list().unwrap();
        assert!(matches!(values.values.as_slice(),
            [TypedValue::Keyword(value)] if value.0 == keyword));
    }
    for degrees in [-90., -10., 23., 90.] {
        assert!(
            ComputedFontStyle::oblique(degrees)
                .to_typed_value_list()
                .is_none()
        );
    }
}

#[test]
fn css_style_property_maps_reify_font_style_keywords_without_losing_angles() {
    let mut vm = new_parsed_test_vm(
        "https://css-font-style.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm.eval(include_str!("css_typed_font_style.js")).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["cases"].as_array().map(Vec::len), Some(275));
}

#[test]
fn native_scoped_names_preserve_decoded_identifiers_and_opaque_lists() {
    use moli_css_parse::parse_typed_style_value;
    use style::typed_om::TypedValue;

    crate::style_engine::ensure_stylo_browser_compat_prefs();
    for property in ["anchor-scope", "timeline-scope"] {
        for (source, expected) in [
            ("none", "none"),
            ("ALL", "all"),
            ("--a", "--a"),
            ("--A", "--A"),
            ("--é", "--é"),
            ("--foo\\2e bar", "--foo.bar"),
            ("--foo\\20 bar", "--foo bar"),
            ("--foo\\5c bar", "--foo\\bar"),
        ] {
            let parsed = parse_typed_style_value(property, source, None).unwrap();
            assert!(matches!(
                parsed.values.unwrap().native().unwrap().values.as_slice(),
                [TypedValue::Keyword(value)] if value.0 == expected
            ));
        }
        for source in ["--a, --b", "--a, --a", "--A, --a", "--foo\\2e bar, --é"] {
            let parsed = parse_typed_style_value(property, source, None).unwrap();
            assert!(parsed.values.is_none(), "{property}: {source}");
        }
    }
}

#[test]
fn css_style_property_maps_reify_scoped_names_without_splitting_lists() {
    let mut vm = new_parsed_test_vm(
        "https://css-scoped-names.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm.eval(include_str!("css_typed_scoped_names.js")).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["cases"].as_array().map(Vec::len), Some(600));
}

#[test]
fn native_font_alternates_reify_keywords_without_exposing_function_arguments() {
    use moli_css_parse::parse_typed_style_value;
    use style::typed_om::TypedValue;

    crate::style_engine::ensure_stylo_browser_compat_prefs();
    for (source, expected) in [
        ("normal", "normal"),
        ("historical-forms", "historical-forms"),
        ("HISTORICAL-FORMS", "historical-forms"),
        ("h\\69storical-forms", "historical-forms"),
    ] {
        let parsed = parse_typed_style_value("font-variant-alternates", source, None).unwrap();
        assert_eq!(parsed.css_text, expected);
        assert!(matches!(
            parsed.values.unwrap().native().unwrap().values.as_slice(),
            [TypedValue::Keyword(value)] if value.0 == expected
        ));
    }
    for source in [
        "stylistic(foo)",
        "styleset(foo, bar)",
        "character-variant(foo, bar)",
        "swash(foo)",
        "ornaments(foo)",
        "annotation(foo)",
        "historical-forms swash(foo)",
        "swash(foo) historical-forms",
        "swash(foo) annotation(bar)",
    ] {
        let parsed = parse_typed_style_value("font-variant-alternates", source, None).unwrap();
        assert!(parsed.values.is_none(), "{source}");
    }
}

#[test]
fn css_style_property_maps_reify_font_alternates_and_preserve_functional_values() {
    let mut vm = new_parsed_test_vm(
        "https://css-font-alternates.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm
        .eval(include_str!("css_typed_font_alternates.js"))
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["cases"].as_array().map(Vec::len), Some(150));
}

#[test]
fn native_default_keywords_reify_while_compound_values_remain_opaque() {
    use moli_css_parse::parse_typed_style_value;
    use style::typed_om::TypedValue;

    crate::style_engine::ensure_stylo_browser_compat_prefs();
    for (property, keyword, compound) in [
        ("counter-increment", "none", "chapter 3"),
        ("counter-reset", "none", "chapter 3"),
        ("counter-set", "none", "chapter 3"),
        ("quotes", "none", "\"a\" \"b\""),
        ("font-feature-settings", "normal", "\"kern\" 1"),
        ("font-variation-settings", "normal", "\"wght\" 400"),
        ("font-variant-alternates", "normal", "swash(foo)"),
        ("font-language-override", "normal", "\"ENG\""),
        ("transition-property", "none", "width"),
    ] {
        for source in [
            keyword.to_owned(),
            keyword.to_ascii_uppercase(),
            format!("\\6e {}", &keyword[1..]),
        ] {
            let parsed = parse_typed_style_value(property, &source, None)
                .unwrap_or_else(|| panic!("valid {property}: {source}"));
            assert_eq!(parsed.css_text, keyword);
            let values = parsed.values.unwrap();
            assert!(
                matches!(values.native().unwrap().values.as_slice(),
                [TypedValue::Keyword(value)] if value.0 == keyword),
                "{property}: {source}"
            );
        }
        let parsed = parse_typed_style_value(property, compound, None)
            .unwrap_or_else(|| panic!("valid {property}: {compound}"));
        assert!(parsed.values.is_none(), "{property}: {compound}");
    }
}

#[test]
fn css_style_property_maps_reify_native_default_keywords_and_preserve_opaque_values() {
    let mut vm = new_parsed_test_vm(
        "https://css-default-keywords.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm
        .eval(include_str!("css_typed_default_keywords.js"))
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]));
    assert_eq!(result["checks"], 3780);
    assert_eq!(result["cases"].as_array().map(Vec::len), Some(675));
}

#[test]
fn css_style_property_maps_preserve_step_keywords_and_opaque_easing_lists() {
    let mut vm = new_parsed_test_vm(
        "https://css-typed-easings.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm.eval(include_str!("css_typed_easings.js")).unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 1000, "{result}");
}

#[test]
fn css_style_property_maps_reject_numeric_shorthand_writes_atomically() {
    let mut vm = new_parsed_test_vm(
        "https://css-typed-shorthands.test/",
        "<!doctype html><body>",
    );
    let result = vm
        .eval(include_str!("css_typed_shorthand_writes.js"))
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["checks"], 725, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
}

#[test]
fn css_style_declarations_commit_before_custom_element_reactions() {
    let mut vm = new_parsed_test_vm("https://css-style-reactions.test/", "<!doctype html><body>");
    assert_eq!(
        vm.eval(include_str!("css_style_reactions.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_retain_declared_units_until_the_declaration_changes() {
    let mut vm = new_parsed_test_vm(
        "https://css-declared-units.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_declared_units.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_accept_percentage_units_without_weakening_type_checks() {
    let mut vm = new_parsed_test_vm(
        "https://css-percentages.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_percentage_writes.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_defer_percentage_ranges_until_computed_values() {
    let mut vm = new_parsed_test_vm(
        "https://css-percentage-ranges.test/",
        "<!doctype html><body><div id=target></div>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
                const element = document.getElementById('target');
                for (const property of ['width', 'height', 'padding-left']) {
                    const input = CSS.percent(-12.5);
                    element.attributeStyleMap.set(property, input);
                    if (element.style.getPropertyValue(property) !== 'calc(-12.5%)')
                        throw new Error(property + ': range must be deferred in the declaration');
                    if (input.value !== -12.5) throw new Error('input was mutated');
                    const computed = element.computedStyleMap().get(property);
                    if (!(computed instanceof CSSUnitValue) || computed.unit !== 'percent' || computed.value !== 0)
                        throw new Error(property + ': computed percentage was not clamped: ' + computed);
                }
                return true;
            })()"#,
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_share_declarations_and_validate_native_types() {
    let mut vm = new_parsed_test_vm(
        "https://css-style-map.test/",
        "<!doctype html><body><iframe id=child></iframe>",
    );
    assert_eq!(
        vm.eval(include_str!("css_style_property_map.js")).unwrap(),
        "true"
    );
}

#[test]
fn css_style_property_maps_reject_unsupported_unparsed_storage_atomically() {
    let mut vm = new_storage_test_vm("https://css-style-map-unparsed.test/");
    assert_eq!(vm.eval(r#"(() => {
        const sheet = new CSSStyleSheet(); sheet.replaceSync('div { width: 2px; --x: old; }');
        const el = document.createElement('div'); el.style.cssText = 'width: 2px; --x: old;';
        for (const [style, map] of [[el.style, el.attributeStyleMap], [sheet.cssRules[0].style, sheet.cssRules[0].styleMap]]) {
            for (const [property, parts] of [['width', ['red']], ['width', ['10px']], ['width', []], ['--x', []]]) {
                const before = style.cssText;
                try { map.set(property, new CSSUnparsedValue(parts)); return false; }
                catch (e) { if (e.name !== 'NotSupportedError') throw e; }
                if (style.cssText !== before) return false;
            }
        }
        return true;
    })()"#).unwrap(), "true");
}
