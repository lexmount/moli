use super::{
    CssAtRuleKind, CssRuleOrderKind, CssRulePdbDeclarationKind, CssomSelectorNamespaceContext,
    StyleRuleSelectorContext, canonical_css_at_rule_text, css_at_rule_kind,
    css_function_rule_text_is_insertable, css_grouping_rule_child_snapshots,
    css_keyframes_rule_child_snapshots, css_rule_child_snapshots_from_stylo_stylesheet_context,
    css_rule_pdb_safe_declaration_block_from_declaration_text,
    css_rule_text_is_insertable_with_selector_context_and_rule_context, css_rule_text_order_kind,
    keyframe_rule_text_from_snapshot, nested_style_rule_block_text_if_has_rules,
    normalize_cssom_font_feature_values_families, normalize_insert_rule_index,
    parse_condition_rule_view_with_stylo,
    parse_css_rule_list_top_level_snapshots_with_selector_context,
    parse_stylesheet_rule_snapshots_with_stylo, parse_valid_keyframe_rule_text,
    parse_valid_style_rule_text_with_selector_context, serialize_style_rule_css_text_with_context,
    serialized_nested_grouping_rule_text, style_rule_text_from_snapshot,
};
use style::stylesheets::CssRuleType;
use style_traits::ToCss;

fn parsed_top_level_rule_texts(css_text: &str) -> Vec<String> {
    parse_css_rule_list_top_level_snapshots_with_selector_context(
        css_text,
        &CssomSelectorNamespaceContext::default(),
    )
    .into_iter()
    .map(|snapshot| snapshot.css_text)
    .collect()
}

fn top_level_rule_is_insertable(
    css_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
) -> bool {
    css_rule_text_is_insertable_with_selector_context_and_rule_context(
        css_text,
        selector_context,
        StyleRuleSelectorContext::TopLevel,
    )
}

#[test]
fn missing_insert_rule_index_defaults_to_zero_by_contract() {
    assert_eq!(normalize_insert_rule_index(0, 3), Some(0));
}

#[test]
fn explicit_insert_rule_index_greater_than_length_is_rejected_by_contract() {
    assert_eq!(normalize_insert_rule_index(4, 3), None);
    assert_eq!(normalize_insert_rule_index(u32::MAX, 3), None);
}

#[test]
fn explicit_insert_rule_index_at_or_below_length_is_allowed() {
    assert_eq!(normalize_insert_rule_index(1, 3), Some(1));
    assert_eq!(normalize_insert_rule_index(3, 3), Some(3));
}

#[test]
fn rule_pdb_safe_seed_replays_canonical_webkit_aliases_through_pdb_mutation() {
    let block = css_rule_pdb_safe_declaration_block_from_declaration_text(
        "-webkit-transition: opacity 1s; -webkit-box-shadow: 1px 2px 3px red; -webkit-appearance: none; -webkit-user-select: text;",
        CssRulePdbDeclarationKind::StyleRule,
    )
    .expect("canonical WebKit aliases should seed a PDB declaration block");

    assert_eq!(
        block.property_value("transition").as_deref(),
        Some("opacity 1s")
    );
    assert_eq!(
        block.property_value("box-shadow").as_deref(),
        Some("red 1px 2px 3px")
    );
    assert_eq!(block.property_value("appearance").as_deref(), Some("none"));
    assert_eq!(block.property_value("user-select").as_deref(), Some("text"));
    assert_eq!(
        block.css_text(),
        "transition: opacity 1s; box-shadow: red 1px 2px 3px; appearance: none; user-select: text;"
    );

    let text_fill = css_rule_pdb_safe_declaration_block_from_declaration_text(
        "-webkit-text-fill-color: red;",
        CssRulePdbDeclarationKind::StyleRule,
    )
    .expect("the Moli Stylo fork should own -webkit-text-fill-color");
    assert_eq!(
        text_fill
            .property_value("-webkit-text-fill-color")
            .as_deref(),
        Some("red")
    );
}

#[test]
fn insert_rule_order_kind_uses_stylo_or_at_keyword_boundary() {
    assert!(matches!(
        css_rule_text_order_kind("@import url(\"a.css\");"),
        CssRuleOrderKind::Import
    ));
    assert!(matches!(
        css_rule_text_order_kind("@namespace svg url(\"http://www.w3.org/2000/svg\");"),
        CssRuleOrderKind::Namespace
    ));
    assert!(matches!(
        css_rule_text_order_kind("@imported url(\"a.css\");"),
        CssRuleOrderKind::Other
    ));
    assert!(matches!(
        css_rule_text_order_kind("@namespacex svg url(\"http://www.w3.org/2000/svg\");"),
        CssRuleOrderKind::Other
    ));
}

#[test]
fn css_style_sheet_rule_text_parser_keeps_top_level_rules() {
    assert_eq!(
        parsed_top_level_rule_texts(
            "  @import url(\"a.css\"); .one { color: red; } /* skip */ .two { content: \"}\"; }  "
        ),
        vec![
            String::from("@import url(\"a.css\");"),
            String::from(".one { color: red; }"),
            String::from(".two { content: \"}\"; }"),
        ]
    );
}

#[test]
fn css_style_sheet_rule_text_parser_keeps_nested_blocks_as_one_rule() {
    assert_eq!(
        parsed_top_level_rule_texts(
            "@media screen { body { color: red; } p { color: blue; } } .next { display: block; }"
        ),
        vec![
            String::from("@media screen {\n  body { color: red; }\n  p { color: blue; }\n}"),
            String::from(".next { display: block; }"),
        ]
    );
}

#[test]
fn css_style_sheet_rule_text_parser_keeps_escaped_namespace_attribute_selectors() {
    assert_eq!(
        parsed_top_level_rule_texts(
            r#"@namespace ns\:odd url(ns);[ns\:odd|odd\:name] { color: red; }"#
        ),
        vec![
            String::from(r#"@namespace ns\:odd url("ns");"#),
            String::from(r#"[ns\:odd|odd\:name] { color: red; }"#),
        ]
    );
}

#[test]
fn simple_style_rule_snapshot_fields_materialize_without_reparse() {
    let rule_snapshots = parse_stylesheet_rule_snapshots_with_stylo(
        ".one { color: red; background-image: url(icon.png); }",
    );
    let rule_snapshot = &rule_snapshots[0];
    assert_eq!(rule_snapshot.rule_type, CssRuleType::Style);
    assert_eq!(rule_snapshot.selector_text.as_deref(), Some(".one"));
    assert_eq!(
        rule_snapshot.declaration_text.as_deref(),
        Some(r#"color: red; background-image: url("icon.png");"#)
    );

    let parsed = style_rule_text_from_snapshot(
        rule_snapshot,
        &CssomSelectorNamespaceContext::default(),
        StyleRuleSelectorContext::TopLevel,
    )
    .expect("plain style rule view should provide selector/declaration fields");
    assert_eq!(parsed.selector_text, ".one");
    assert_eq!(
        parsed.style_text,
        r#"color: red; background-image: url("icon.png");"#
    );
    assert_eq!(
        parsed.css_text,
        r#".one { color: red; background-image: url("icon.png"); }"#
    );
}

#[test]
fn nested_style_rule_snapshot_fields_materialize_without_reparse() {
    let rule_snapshots = parse_stylesheet_rule_snapshots_with_stylo(
        ".host { color: red; & .child { color: blue; } --after: 1; }",
    );
    let rule_snapshot = &rule_snapshots[0];
    assert_eq!(rule_snapshot.rule_type, CssRuleType::Style);
    assert_eq!(rule_snapshot.selector_text.as_deref(), Some(".host"));
    assert!(!rule_snapshot.child_rules.is_empty());
    assert_eq!(
        rule_snapshot.declaration_text.as_deref(),
        Some("color: red;")
    );
    assert_eq!(
        rule_snapshot.child_rules[0].css_text,
        "& .child { color: blue; }"
    );
    assert_eq!(rule_snapshot.child_rules[1].css_text, "--after: 1;");

    let parsed = style_rule_text_from_snapshot(
        rule_snapshot,
        &CssomSelectorNamespaceContext::default(),
        StyleRuleSelectorContext::TopLevel,
    )
    .expect("nested style rule view should provide selector/declaration/child fields");
    assert_eq!(parsed.selector_text, ".host");
    assert_eq!(
        parsed.style_text,
        "color: red;\n& .child { color: blue; }\n--after: 1;"
    );
    assert_eq!(
        parsed.css_text,
        ".host {\n  color: red;\n  & .child { color: blue; }\n  --after: 1;\n}"
    );
}

#[test]
fn descriptor_rule_snapshots_expose_declaration_fields() {
    let rule_snapshots = parse_stylesheet_rule_snapshots_with_stylo(
        r#"@font-face { font-family: Foo; src: local(Foo); } @page :first { margin-top: 1px; @top-left { content: "x"; } }"#,
    );

    assert_eq!(rule_snapshots[0].rule_type, CssRuleType::FontFace);
    assert_eq!(
        rule_snapshots[0].declaration_text.as_deref(),
        Some("font-family: Foo; src: local(Foo);")
    );
    assert_eq!(rule_snapshots[1].rule_type, CssRuleType::Page);
    assert_eq!(rule_snapshots[1].selector_text.as_deref(), Some(":first"));
    assert_eq!(
        rule_snapshots[1].declaration_text.as_deref(),
        Some("margin-top: 1px;")
    );
    assert_eq!(
        rule_snapshots[1].child_rules[0].rule_type,
        CssRuleType::Margin
    );
    assert_eq!(
        rule_snapshots[1].child_rules[0].declaration_text.as_deref(),
        Some(r#"content: "x";"#)
    );
}

#[test]
fn css_keyframes_rule_child_snapshots_use_native_rule_children() {
    let child_rules = css_keyframes_rule_child_snapshots(
        "@keyframes slide { from { opacity: 0; } to { opacity: 1; } }",
    );

    assert_eq!(
        child_rules
            .iter()
            .map(|rule| rule.css_text.as_str())
            .collect::<Vec<_>>(),
        ["0% { opacity: 0; }", "100% { opacity: 1; }"]
    );
    assert_eq!(child_rules[0].selector_text.as_deref(), Some("0%"));
    assert_eq!(
        child_rules[0].declaration_text.as_deref(),
        Some("opacity: 0;")
    );
    assert_eq!(child_rules[1].selector_text.as_deref(), Some("100%"));
    assert_eq!(
        child_rules[1].declaration_text.as_deref(),
        Some("opacity: 1;")
    );
    assert!(child_rules.iter().all(|rule| rule.child_rules.is_empty()));

    let parsed = keyframe_rule_text_from_snapshot(&child_rules[0]).expect("keyframe view fields");
    assert_eq!(parsed.selector_text, "0%");
    assert_eq!(parsed.style_text, "opacity: 0;");
    assert_eq!(parsed.css_text, "0% { opacity: 0; }");
}

#[test]
fn user_style_rule_text_validation_uses_stylo_snapshot() {
    let parsed = parse_valid_style_rule_text_with_selector_context(
        ".one { color: red; }",
        &CssomSelectorNamespaceContext::default(),
    )
    .expect("valid style rule should parse through Stylo rule view");
    assert_eq!(parsed.selector_text, ".one");
    assert_eq!(parsed.style_text, "color: red;");
    assert_eq!(parsed.css_text, ".one { color: red; }");
    assert!(
        parse_valid_style_rule_text_with_selector_context(
            ".one { color: red; } .two { color: blue; }",
            &CssomSelectorNamespaceContext::default(),
        )
        .is_none(),
        "single-rule user input must not accept multiple rules"
    );
}

#[test]
fn user_keyframe_rule_text_validation_uses_stylo_keyframe_view() {
    let parsed = parse_valid_keyframe_rule_text("from { opacity: 0; }")
        .expect("valid keyframe child rule should parse in keyframes context");
    assert_eq!(parsed.selector_text, "0%");
    assert_eq!(parsed.style_text, "opacity: 0;");
    assert_eq!(parsed.css_text, "0% { opacity: 0; }");
    assert!(
        parse_valid_keyframe_rule_text("from { opacity: 0; } to { opacity: 1; }").is_none(),
        "single keyframe rule input must not accept multiple keyframe blocks"
    );
}

#[test]
fn css_grouping_rule_child_snapshots_use_native_rule_children() {
    let child_rules = css_grouping_rule_child_snapshots(
            "@media screen { .one { color: red; } @supports (display: grid) { .two { display: grid; } } }",
            CssAtRuleKind::Media,
        )
        .expect("media rule should expose child views");

    assert_eq!(
        child_rules
            .iter()
            .map(|rule| rule.css_text.as_str())
            .collect::<Vec<_>>(),
        [
            ".one { color: red; }",
            "@supports (display: grid) {\n  .two { display: grid; }\n}",
        ]
    );
    assert_eq!(
        child_rules[1].child_rules[0].css_text,
        ".two { display: grid; }"
    );
}

#[test]
fn css_grouping_rule_child_snapshots_use_stylo_page_margin_children() {
    let child_rules = css_grouping_rule_child_snapshots(
        r#"@page :first { margin-top: 1px; @top-left { content: "x"; color: red; } }"#,
        CssAtRuleKind::Page,
    )
    .expect("page rule should expose margin child views");

    assert_eq!(
        child_rules
            .iter()
            .map(|rule| rule.css_text.as_str())
            .collect::<Vec<_>>(),
        [r#"@top-left { content: "x"; color: red; }"#]
    );
}

#[test]
fn css_style_rule_child_snapshots_use_stylo_stylesheet_context() {
    let stylesheet_rules = parse_stylesheet_rule_snapshots_with_stylo(
            ".a { --a: 1; & { --c: 1; } --d: 1; @media (width > 100px) { --x: 1; .b { } --z: 1; } --w: 1; }",
        )
        .into_iter()
        .map(|rule| rule.css_text)
        .collect::<Vec<_>>();
    let child_rules = css_rule_child_snapshots_from_stylo_stylesheet_context(
        &stylesheet_rules,
        CssRuleType::Style,
        &stylesheet_rules[0],
        true,
    )
    .expect("style rule should expose Stylo child views");

    assert_eq!(
        child_rules
            .iter()
            .map(|rule| rule.css_text.as_str())
            .collect::<Vec<_>>(),
        [
            "& { --c: 1; }",
            "--d: 1;",
            "@media (width > 100px) {\n  --x: 1;\n  & .b { }\n  --z: 1;\n}",
            "--w: 1;",
        ]
    );
    assert_eq!(child_rules[1].rule_type, CssRuleType::NestedDeclarations);
    assert_eq!(
        child_rules[2].child_rules[0].rule_type,
        CssRuleType::NestedDeclarations
    );
    assert_eq!(child_rules[2].child_rules[1].css_text, "& .b { }");
}

#[test]
fn css_style_rule_child_snapshots_use_parent_namespace_context() {
    let stylesheet_rules = parse_stylesheet_rule_snapshots_with_stylo(
            r#"@namespace svg url("http://www.w3.org/2000/svg"); .host { & > svg|path { color: blue; } }"#,
        )
        .into_iter()
        .map(|rule| rule.css_text)
        .collect::<Vec<_>>();
    let child_rules = css_rule_child_snapshots_from_stylo_stylesheet_context(
        &stylesheet_rules,
        CssRuleType::Style,
        &stylesheet_rules[1],
        true,
    )
    .expect("style rule should parse nested namespace selector with parent context");

    assert_eq!(
        child_rules
            .iter()
            .map(|rule| rule.css_text.as_str())
            .collect::<Vec<_>>(),
        ["& > svg|path { color: blue; }"]
    );
}

#[test]
fn selector_namespace_context_feeds_stylo_nested_parser() {
    let mut selector_context = CssomSelectorNamespaceContext::default();
    selector_context.record_rule_text(r#"@namespace svg url("http://www.w3.org/2000/svg");"#);

    assert_eq!(
        selector_context.stylo_parent_rule_texts(),
        [String::from(
            r#"@namespace svg url("http://www.w3.org/2000/svg");"#
        )]
    );
    assert_eq!(
        nested_style_rule_block_text_if_has_rules(
            "& > svg|path { color: blue; }",
            &selector_context,
        )
        .as_deref(),
        Some("& > svg|path { color: blue; }")
    );
}

#[test]
fn nested_style_rule_string_serialization_uses_stylo_snapshots() {
    assert_eq!(
        serialize_style_rule_css_text_with_context(
            ".host",
            "color: red; & .child { color: blue } --after: 1",
            &CssomSelectorNamespaceContext::default(),
        ),
        ".host {\n  color: red;\n  & .child { color: blue; }\n  --after: 1;\n}"
    );
}

#[test]
fn nested_grouping_string_serialization_uses_stylo_snapshots() {
    assert_eq!(
        serialized_nested_grouping_rule_text(
            CssAtRuleKind::Media,
            "screen",
            "color: red; & .child { color: blue }",
            &CssomSelectorNamespaceContext::default(),
            StyleRuleSelectorContext::Nested,
        ),
        "@media screen {\n  color: red;\n  & .child { color: blue; }\n}"
    );
}

#[test]
fn css_at_rule_kind_classifies_common_cssom_rule_subclasses() {
    assert_eq!(
        css_at_rule_kind("@import url(\"a.css\");"),
        CssAtRuleKind::Import
    );
    assert_eq!(css_at_rule_kind("@layer A;"), CssAtRuleKind::Layer);
    assert_eq!(
        css_at_rule_kind("@media screen { body { color: red; } }"),
        CssAtRuleKind::Media
    );
    assert_eq!(
        css_at_rule_kind("@scope (.a) { body { color: red; } }"),
        CssAtRuleKind::Scope
    );
    assert_eq!(
        css_at_rule_kind("@supports (display: grid) { body { display: grid; } }"),
        CssAtRuleKind::Supports
    );
    assert_eq!(
        css_at_rule_kind("@container name (min-width: 100px) { body { color: red; } }"),
        CssAtRuleKind::Container
    );
    assert_eq!(
        css_at_rule_kind("@font-face { font-family: Test; src: url(test.woff2); }"),
        CssAtRuleKind::FontFace
    );
    assert_eq!(
        css_at_rule_kind("@keyframes fade { from { opacity: 0; } to { opacity: 1; } }"),
        CssAtRuleKind::Keyframes
    );
    assert_eq!(
        css_at_rule_kind("@page { margin: 0; }"),
        CssAtRuleKind::Page
    );
    assert_eq!(
        css_at_rule_kind("@namespace svg url(\"http://www.w3.org/2000/svg\");"),
        CssAtRuleKind::Namespace
    );
    assert_eq!(
        css_at_rule_kind("@counter-style thumbs { system: cyclic; symbols: \"*\"; }"),
        CssAtRuleKind::CounterStyle
    );
    assert_eq!(
        css_at_rule_kind("@function --double() { result: 2px; }"),
        CssAtRuleKind::Function
    );
    assert_eq!(
        css_at_rule_kind("@document url(\"https://example.test/\") { body { color: red; } }"),
        CssAtRuleKind::Unknown
    );
}

#[test]
fn css_insert_rule_validation_rejects_block_at_rules_without_blocks() {
    assert!(!top_level_rule_is_insertable(
        "@media bad syntax;",
        &Default::default()
    ));
    assert!(top_level_rule_is_insertable(
        "@media print {}",
        &Default::default()
    ));
    assert!(top_level_rule_is_insertable(
        "@import url(\"a.css\");",
        &Default::default()
    ));
    assert!(!top_level_rule_is_insertable(
        "@import url(\"a.css\") {}",
        &Default::default()
    ));
    assert!(!top_level_rule_is_insertable(
        "@counter-style bad { system: cyclic; }",
        &Default::default()
    ));
    assert!(!top_level_rule_is_insertable(
        "@property --bad { syntax: \"<color>\"; inherits: false; }",
        &Default::default()
    ));
    assert!(top_level_rule_is_insertable(
        "@function --double() { result: 2px; }",
        &Default::default()
    ));
    assert!(!top_level_rule_is_insertable(
        "@function color() { result: red; }",
        &Default::default()
    ));
    assert!(!css_function_rule_text_is_insertable(
        "@function --double();"
    ));
    assert!(!css_function_rule_text_is_insertable(
        "@function --double() { result: 2px; } @function --again() {}"
    ));
    assert!(!css_function_rule_text_is_insertable(
        "@function --double() { result: 2px; } div {}"
    ));
    assert!(!top_level_rule_is_insertable(
        "@function --double() { result: 2px; } @function --again() {}",
        &Default::default()
    ));
}

#[test]
fn css_insert_rule_validation_accepts_valid_layer_rules_only() {
    for css_text in [
        "@layer A;",
        "@layer A, B, C;",
        "@layer A.A;",
        "@layer A, B.C.D, C;",
        "@layer {}",
        "@layer A {}",
        "@layer A.B {}",
    ] {
        assert!(
            top_level_rule_is_insertable(css_text, &Default::default()),
            "{css_text} should be insertable"
        );
    }

    for css_text in [
        "@layer;",
        "@layer A . A;",
        "@layer A . B {}",
        "@layer A, B, C {}",
    ] {
        assert!(
            !top_level_rule_is_insertable(css_text, &Default::default()),
            "{css_text} should not be insertable"
        );
    }
}

#[test]
fn css_insert_rule_validation_accepts_valid_scope_rules_only() {
    for css_text in [
        "@scope (.a) {}",
        "@scope (.a + .b) {}",
        "@scope (.a:hover, #b, div) {}",
        "@scope (.a) to (.b) {}",
        "@scope {}",
        "@scope to (.a) {}",
        "@scope (.a) to (& > &) {}",
        "@scope (.a) to (> .b) {}",
    ] {
        assert!(
            top_level_rule_is_insertable(css_text, &Default::default()),
            "{css_text} should be insertable"
        );
    }

    for css_text in [
        "@scope ();",
        "@scope ();",
        "@scope () {}",
        "@scope div {}",
        "@scope (.a) unknown (.c) {}",
        "@scope (div::before) {}",
        "@scope (.a) to (div::before) {}",
        "@scope (> &) to (>>) {}",
        "@scope to {}",
    ] {
        assert!(
            !top_level_rule_is_insertable(css_text, &Default::default()),
            "{css_text} should not be insertable"
        );
    }
}

#[test]
fn css_insert_rule_validation_accepts_valid_container_rules_only() {
    for css_text in [
        "@container (width) {}",
        "@container (width: 100px) {}",
        "@container name (height) {}",
        "@container screen {}",
        "@container --foo {}",
        "@container container, container2 {}",
    ] {
        assert!(
            top_level_rule_is_insertable(css_text, &Default::default()),
            "{css_text} should be insertable"
        );
    }

    for css_text in [
        "@container {}",
        "@container name screen {}",
        "@container screen and (width: 100px) {}",
        "@container foo (width: 100px) garbage {}",
        "@container foo foo not (width) {}",
        "@container inherit not (width) {}",
        "@container none not (width) {}",
    ] {
        assert!(
            !top_level_rule_is_insertable(css_text, &Default::default()),
            "{css_text} should not be insertable"
        );
    }
}

#[test]
fn container_rule_css_text_is_canonicalized() {
    assert_eq!(
        canonical_css_at_rule_text(
            "@container (width=100px) { #id { color: green } }",
            CssAtRuleKind::Container
        ),
        "@container (width = 100px) {\n  #id { color: green; }\n}"
    );
    assert_eq!(
        canonical_css_at_rule_text(
            "@container NAMe   (inline-sizE  < 1300px ) { #id { color: lime } }",
            CssAtRuleKind::Container
        ),
        "@container NAMe (inline-size < 1300px) {\n  #id { color: lime; }\n}"
    );
    assert_eq!(
        canonical_css_at_rule_text(
            "@container (width),(height) ,--foo ,--bar { }",
            CssAtRuleKind::Container
        ),
        "@container (width), (height), --foo, --bar {\n}"
    );
}

#[test]
fn namespace_rule_css_text_is_canonicalized() {
    assert_eq!(
        canonical_css_at_rule_text(
            "@namespace svg url(http://servo);",
            CssAtRuleKind::Namespace
        ),
        "@namespace svg url(\"http://servo\");"
    );
    assert_eq!(
        canonical_css_at_rule_text("@font-face { font-family: Test; }", CssAtRuleKind::FontFace),
        "@font-face { font-family: Test; }"
    );
}

#[test]
fn layer_rule_css_text_is_canonicalized() {
    assert_eq!(
        canonical_css_at_rule_text("@layer A;", CssAtRuleKind::Layer),
        "@layer A;"
    );
    assert_eq!(
        canonical_css_at_rule_text("@layer A,B.C.D,C;", CssAtRuleKind::Layer),
        "@layer A, B.C.D, C;"
    );
    assert_eq!(
        canonical_css_at_rule_text("@layer {}", CssAtRuleKind::Layer),
        "@layer {\n}"
    );
    assert_eq!(
        canonical_css_at_rule_text("@layer A.B {}", CssAtRuleKind::Layer),
        "@layer A.B {\n}"
    );
    assert_eq!(
        canonical_css_at_rule_text(r"@layer abc\;oops\!;", CssAtRuleKind::Layer),
        r"@layer abc\;oops\!;"
    );
    assert_eq!(
        canonical_css_at_rule_text(r"@layer a\.b.c {}", CssAtRuleKind::Layer),
        "@layer a\\.b.c {\n}"
    );
}

#[test]
fn scope_rule_css_text_is_canonicalized() {
    assert_eq!(
        canonical_css_at_rule_text("@scope (.a){}", CssAtRuleKind::Scope),
        "@scope (.a) {\n}"
    );
    assert_eq!(
        canonical_css_at_rule_text("@scope (.a)to (.b){}", CssAtRuleKind::Scope),
        "@scope (.a) to (.b) {\n}"
    );
    assert_eq!(
        canonical_css_at_rule_text("@scope{}", CssAtRuleKind::Scope),
        "@scope {\n}"
    );
    assert_eq!(
        canonical_css_at_rule_text(
            "@scope (.a) to (.b) { div { display: block; } }",
            CssAtRuleKind::Scope
        ),
        "@scope (.a) to (.b) {\n  div { display: block; }\n}"
    );
    let view =
        parse_condition_rule_view_with_stylo("@scope (.a) to (> .b) {}").expect("scope rule view");
    assert_eq!(view.scope_start.as_deref(), Some(".a"));
    assert_eq!(view.scope_end.as_deref(), Some("> .b"));
}

#[test]
fn grouping_rule_css_text_canonicalizes_nested_style_rules() {
    assert_eq!(
        canonical_css_at_rule_text(
            "@supports (padding: 0) { dfn { width:0; } @supports (width: 0) { br { height:0; } } }",
            CssAtRuleKind::Supports
        ),
        "@supports (padding: 0) {\n  dfn { width: 0px; }\n  @supports (width: 0) {\n  br { height: 0px; }\n}\n}"
    );
}

#[test]
fn grouping_rule_css_text_uses_stylo_rule_view_serialization() {
    let css_text = "@media screen { .one { margin: 0; } @supports (display: grid) { .two { display: grid; } } }";
    let stylo_css_text = parse_stylesheet_rule_snapshots_with_stylo(css_text)
        .into_iter()
        .next()
        .expect("media rule should parse")
        .css_text;

    assert_eq!(
        canonical_css_at_rule_text(css_text, CssAtRuleKind::Media),
        stylo_css_text
    );
}

#[test]
fn starting_style_rule_accepts_an_empty_prelude_and_rejects_nonempty_preludes() {
    crate::style_engine::ensure_stylo_browser_compat_prefs();

    assert_eq!(
        parsed_top_level_rule_texts("@starting-style {}"),
        ["@starting-style {\n}"]
    );
    for css_text in [
        "@starting-style div {}",
        "@starting-style () {}",
        "@starting-style ( {}",
        "@starting-style }{}",
    ] {
        assert!(
            parsed_top_level_rule_texts(css_text).is_empty(),
            "unexpectedly accepted {css_text:?}"
        );
    }
}

#[test]
fn grouping_rule_css_text_uses_stylo_page_rule_child_serialization() {
    let css_text =
        r#"@media screen { @page :first { margin-top: 1px; @top-left { content: "x"; } } }"#;
    let stylo_css_text = parse_stylesheet_rule_snapshots_with_stylo(css_text)
        .into_iter()
        .next()
        .expect("media rule should parse")
        .css_text;

    assert_eq!(
        canonical_css_at_rule_text(css_text, CssAtRuleKind::Media),
        stylo_css_text
    );
}

#[test]
fn keyframes_rule_css_text_canonicalizes_keyframe_declarations() {
    assert_eq!(
        canonical_css_at_rule_text(
            "@keyframes foo { from { top: 0; left: 0; } to { top: 100px; left: 100px; } }",
            CssAtRuleKind::Keyframes
        ),
        "@keyframes foo {\n0% { top: 0px; left: 0px; }\n100% { top: 100px; left: 100px; }\n}"
    );
}

#[test]
fn keyframes_default_name_is_reserved_custom_ident() {
    assert!(!top_level_rule_is_insertable(
        "@keyframes default { from { opacity: 0; } to { opacity: 1; } }",
        &Default::default()
    ));
    assert_eq!(
        canonical_css_at_rule_text(
            "@keyframes default { from { opacity: 0; } to { opacity: 1; } }",
            CssAtRuleKind::Keyframes
        ),
        "@keyframes default { from { opacity: 0; } to { opacity: 1; } }"
    );
    assert_eq!(super::serialize_keyframes_name("default"), "\"default\"");
}

#[test]
fn keyframes_reserved_names_are_not_custom_ident_names() {
    for name in [
        "none",
        "initial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "revert-rule",
    ] {
        let css_text = format!("@keyframes {name} {{ from {{ opacity: 0; }} }}");
        assert!(
            !top_level_rule_is_insertable(&css_text, &Default::default()),
            "{name} should not be insertable as a bare keyframes name"
        );
        assert_eq!(super::serialize_keyframes_name(name), format!("\"{name}\""));
    }
}

#[test]
fn keyframes_name_setter_serialization_is_validated_by_stylo() {
    let serialized = super::serialize_keyframes_name("slide show");
    assert_eq!(serialized, r"slide\ show");
    assert_eq!(
        super::css_keyframes_rule_name_from_css_text(&format!("@keyframes {serialized} {{}}"))
            .as_deref(),
        Some("slide show")
    );
}

#[test]
fn descriptor_block_rule_css_text_is_single_line() {
    assert_eq!(
        canonical_css_at_rule_text(
            "@font-face {\n src: local(\"foo\");\n font-family: foo;\n}",
            CssAtRuleKind::FontFace
        ),
        "@font-face { font-family: foo; src: local(\"foo\"); }"
    );
    assert_eq!(
        canonical_css_at_rule_text(
            "@counter-style foo {\n system: cyclic;\n symbols: \"*\";\n suffix: \" \";\n}",
            CssAtRuleKind::CounterStyle
        ),
        "@counter-style foo { system: cyclic; suffix: \" \"; symbols: \"*\"; }"
    );
}

#[test]
fn property_rule_css_text_uses_stylo_rule_view_serialization() {
    let css_text =
        r#"@property --accent { inherits: false; initial-value: red; syntax: "<color>"; }"#;
    let stylo_css_text = parse_stylesheet_rule_snapshots_with_stylo(css_text)
        .into_iter()
        .next()
        .expect("property rule should parse")
        .css_text;

    assert_eq!(
        canonical_css_at_rule_text(css_text, CssAtRuleKind::Property),
        stylo_css_text
    );
}

#[test]
fn font_feature_values_rule_css_text_uses_stylo_rule_view_serialization() {
    let css_text = "@font-feature-values test_family { @annotation { the_first: 6; } @styleset { yo: 7; di: 10 9 4 5; } }";
    let stylo_css_text = parse_stylesheet_rule_snapshots_with_stylo(css_text)
        .into_iter()
        .next()
        .expect("font-feature-values rule should parse")
        .css_text;

    assert_eq!(
        canonical_css_at_rule_text(css_text, CssAtRuleKind::FontFeatureValues),
        stylo_css_text
    );
}

#[test]
fn font_feature_values_family_normalization_matches_chromium_raw_string_contract() {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let cases = [
        (
            "serif, foo bar, changed_family,,",
            "\"serif\", \"foo bar\", changed_family",
        ),
        (
            "SERIF, System-UI, math, default, initial, revert-layer",
            "SERIF, System-UI, \"math\", \"default\", \"initial\", \"revert-layer\"",
        ),
        (
            "foo\\ bar, --custom, -valid, 1bad, _ok, 日本語",
            "\"foo\\\\ bar\", \"--custom\", -valid, \"1bad\", _ok, 日本語",
        ),
        ("\"foo,bar\", baz", "\"\\\"foo\", \"bar\\\"\", baz"),
        ("\"serif\"", "\"\\\"serif\\\"\""),
        ("foo/*x*/bar", "\"foo/*x*/bar\""),
        (r"--, -, café, \66 oo", r#""--", "-", café, "\\66 oo""#),
    ];

    for (input, expected) in cases {
        let (serialized, native) = normalize_cssom_font_feature_values_families(input).into_parts();
        assert_eq!(serialized, expected, "input: {input:?}");
        assert_eq!(
            native
                .iter()
                .map(ToCss::to_css_string)
                .collect::<Vec<_>>()
                .join(", "),
            expected,
            "typed native state diverged for input: {input:?}"
        );
    }
}

#[test]
fn font_feature_values_family_normalization_tracks_enabled_css_wide_keywords() {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let (serialized, native) =
        normalize_cssom_font_feature_values_families("revert-rule, ReVeRt-RuLe").into_parts();

    assert_eq!(serialized, r#""revert-rule", "ReVeRt-RuLe""#);
    assert_eq!(
        native
            .iter()
            .map(ToCss::to_css_string)
            .collect::<Vec<_>>()
            .join(", "),
        serialized
    );
}

#[test]
fn page_rule_css_text_uses_stylo_rule_view_serialization() {
    let css_text = r#"@page :first { margin-top: 1px; @top-left { content: "x"; color: red; } }"#;
    let stylo_css_text = parse_stylesheet_rule_snapshots_with_stylo(css_text)
        .into_iter()
        .next()
        .expect("page rule should parse")
        .css_text;

    assert_eq!(
        canonical_css_at_rule_text(css_text, CssAtRuleKind::Page),
        stylo_css_text
    );
}
