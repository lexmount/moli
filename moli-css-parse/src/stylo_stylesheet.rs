//! Thin adapters for Stylo-owned native stylesheets and CSSOM snapshots.

use std::{borrow::Cow, sync::Once};

use cssparser::{Parser, ParserInput, ToCss, Token, TokenSerializationType};
pub use style::moli_rule_tree::{
    CssConditionRuleView, CssCounterStyleRuleView, CssFontFaceRuleView,
    CssFontFeatureValueEntryView, CssFontFeatureValuesRuleView, CssImportRuleView,
    CssKeyframesRuleView, CssLayerRuleView, CssMarginRuleView, CssNamespaceRuleView,
    CssPageDescriptorEntryView, CssPageRuleView, CssPropertyRuleView, CssRuleInsertError,
};
use style::{
    context::QuirksMode,
    custom_properties::AttrTaint,
    font_face::FontFaceRule,
    parser::ParserContext,
    properties::{PropertyDeclarationBlock, parse_property_declaration_list},
    stylesheets::{CssRuleType, Namespaces, Origin, UrlExtraData},
};
use style_traits::{CssStringWriter, ParsingMode};

/// The native stylesheet shared by CSSOM and the cascade.
pub type CssNativeStylesheet = style::moli_rule_tree::CssStylesheetRuleTree;

/// A recursive text-parse and detach DTO, never an attached live-state authority.
pub type CssRuleSnapshot = style::moli_rule_tree::CssStylesheetRuleView;

/// A detached rule-list mutation returned by Stylo's text parser.
pub type CssDetachedRuleListMutation = style::moli_rule_tree::CssStylesheetMutationResult;

/// Canonical text and ordering metadata for one parsed stylesheet rule.
pub type CssParsedRuleText = style::moli_rule_tree::CssStylesheetRuleText;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssFontFaceDescriptorEntryView {
    pub name: String,
    pub value: String,
}

pub fn parse_stylesheet_rule_texts_with_stylo(css_text: &str) -> Vec<CssParsedRuleText> {
    style::moli_rule_tree::parse_stylesheet_rule_texts(css_text)
}

pub fn parse_stylesheet_rule_snapshots_with_stylo(css_text: &str) -> Vec<CssRuleSnapshot> {
    style::moli_rule_tree::parse_stylesheet_rule_views(css_text)
}

pub fn parse_constructed_stylesheet_rule_snapshots_with_stylo(
    css_text: &str,
) -> Vec<CssRuleSnapshot> {
    style::moli_rule_tree::parse_constructed_stylesheet_rule_views(css_text)
}

pub fn parse_counter_style_rule_view_with_stylo(css_text: &str) -> Option<CssCounterStyleRuleView> {
    style::moli_rule_tree::parse_counter_style_rule_view(css_text)
}

pub fn parse_font_face_rule_view_with_stylo(css_text: &str) -> Option<CssFontFaceRuleView> {
    style::moli_rule_tree::parse_font_face_rule_view(css_text)
}

pub fn font_face_descriptor_property_names_with_stylo() -> &'static [&'static str] {
    style::moli_rule_tree::font_face_descriptor_names()
}

pub fn page_descriptor_property_names_with_stylo() -> &'static [&'static str] {
    style::moli_rule_tree::page_descriptor_names()
}

/// Stylo single-descriptor adapter for `CSSFontFaceRule.style` writes.
///
/// Renderer owns CSSOM receiver checks and priority storage, while Stylo owns
/// descriptor name/value validation and serialization.
pub fn parse_font_face_descriptor_entry_with_stylo(
    name: &str,
    value: &str,
) -> Option<CssFontFaceDescriptorEntryView> {
    let name = crate::canonical_style_property_name(name);
    if value.is_empty()
        || crate::escape_top_level_semicolons(value) != value
        || crate::split_important_priority(value).1
    {
        return None;
    }
    let normalized_value = (name == "unicode-range")
        .then(|| remove_css_comments_without_retokenizing(value))
        .flatten();
    let value = normalized_value.as_deref().unwrap_or(value);
    let entry = style::moli_rule_tree::parse_font_face_cssom_descriptor_entry(&name, value)?;
    Some(CssFontFaceDescriptorEntryView {
        name: entry.name,
        value: entry.value,
    })
}

/// Work around cssparser's raw-slice `<urange>` parser, which cannot parse
/// comments between otherwise adjacent range tokens. Refuse the rewrite when
/// removing a comment would merge two tokens, so invalid input stays invalid.
fn remove_css_comments_without_retokenizing(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut output = String::with_capacity(value.len());
    let mut original_tokens = Vec::new();
    let mut removed_comment = false;

    loop {
        let token_start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        match token {
            Token::Comment(_) => {
                removed_comment = true;
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => return None,
            token => {
                original_tokens.push(css_token_fingerprint(&token)?);
                output.push_str(input.slice_from(token_start));
            }
        }
    }

    if !removed_comment || original_tokens != css_token_fingerprints(&output)? {
        return None;
    }
    Some(output)
}

fn css_token_fingerprints(value: &str) -> Option<Vec<(TokenSerializationType, String)>> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut tokens = Vec::new();
    loop {
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        match token {
            Token::Comment(_) => {}
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => return None,
            token => tokens.push(css_token_fingerprint(&token)?),
        }
    }
    Some(tokens)
}

fn css_token_fingerprint(token: &Token<'_>) -> Option<(TokenSerializationType, String)> {
    let mut serialized = String::new();
    token.to_css(&mut serialized).ok()?;
    Some((token.serialization_type(), serialized))
}

/// Parse and serialize a CSSOM `@font-face` descriptor block through Stylo.
pub fn parse_font_face_descriptor_block_with_stylo(style_text: &str) -> Option<String> {
    style::moli_rule_tree::parse_font_face_cssom_descriptor_block(style_text)
}

pub fn parse_font_face_cssom_rule_with_stylo_context(
    context: &ParserContext,
    descriptor_text: &str,
) -> Result<FontFaceRule, CssRuleInsertError> {
    style::moli_rule_tree::parse_font_face_cssom_rule_with_context(context, descriptor_text)
}

pub fn parse_import_rule_view_with_stylo(css_text: &str) -> Option<CssImportRuleView> {
    style::moli_rule_tree::parse_import_rule_view(css_text)
}

pub fn parse_namespace_rule_view_with_stylo(css_text: &str) -> Option<CssNamespaceRuleView> {
    style::moli_rule_tree::parse_namespace_rule_view(css_text)
}

pub fn parse_condition_rule_view_with_stylo(css_text: &str) -> Option<CssConditionRuleView> {
    style::moli_rule_tree::parse_condition_rule_view(css_text)
}

pub fn parse_layer_rule_view_with_stylo(css_text: &str) -> Option<CssLayerRuleView> {
    style::moli_rule_tree::parse_layer_rule_view(css_text)
}

pub fn parse_page_rule_view_with_stylo(css_text: &str) -> Option<CssPageRuleView> {
    style::moli_rule_tree::parse_page_rule_view(css_text)
}

pub fn parse_page_margin_rule_view_with_stylo(css_text: &str) -> Option<CssMarginRuleView> {
    style::moli_rule_tree::parse_page_margin_rule_view(css_text)
}

pub fn parse_page_descriptor_entries_with_stylo(
    name: &str,
    value: &str,
) -> Option<Vec<CssPageDescriptorEntryView>> {
    style::moli_rule_tree::parse_page_descriptor_entries(name, value)
}

/// Parse and serialize a `@page` descriptor declaration block through Stylo.
///
/// Invalid declarations follow CSSOM cssText semantics and are ignored by the
/// Stylo declaration-list parser.
pub fn parse_page_descriptor_block_with_stylo(style_text: &str) -> Option<String> {
    parse_descriptor_declaration_block_with_stylo(style_text, CssRuleType::Page)
}

/// Parse and serialize a page margin rule declaration block through Stylo.
pub fn parse_page_margin_descriptor_block_with_stylo(
    margin_name: &str,
    style_text: &str,
) -> Option<String> {
    style::moli_rule_tree::parse_page_margin_descriptor_block(margin_name, style_text)
}

fn parse_descriptor_declaration_block_with_stylo(
    style_text: &str,
    rule_type: CssRuleType,
) -> Option<String> {
    with_descriptor_declaration_context(rule_type, |context| {
        let mut input = ParserInput::new(style_text);
        let mut input = Parser::new(&mut input);
        let block = parse_property_declaration_list(context, &mut input, &[]);
        Some(declaration_block_css_text(&block))
    })
}

fn declaration_block_css_text(block: &PropertyDeclarationBlock) -> String {
    let mut css_text = CssStringWriter::new();
    block
        .to_css(&mut css_text)
        .expect("serializing a declaration block to string should not fail");
    css_text.trim_end().to_owned()
}

fn with_descriptor_declaration_context<R>(
    rule_type: CssRuleType,
    f: impl FnOnce(&ParserContext) -> Option<R>,
) -> Option<R> {
    ensure_stylo_cssom_prefs_for_descriptor_parser();
    let url_data = UrlExtraData::from(url::Url::parse("about:blank").ok()?);
    let context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(rule_type),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Cow::Owned(Namespaces::default()),
        None,
        None,
        AttrTaint::default(),
    );
    f(&context)
}

fn ensure_stylo_cssom_prefs_for_descriptor_parser() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = style::moli_rule_tree::parse_stylesheet_rule_views("");
    });
}

pub fn parse_keyframes_rule_view_with_stylo(css_text: &str) -> Option<CssKeyframesRuleView> {
    style::moli_rule_tree::parse_keyframes_rule_view(css_text)
}

pub fn parse_font_feature_values_rule_view_with_stylo(
    css_text: &str,
) -> Option<CssFontFeatureValuesRuleView> {
    style::moli_rule_tree::parse_font_feature_values_rule_view(css_text)
}

pub fn parse_property_rule_view_with_stylo(css_text: &str) -> Option<CssPropertyRuleView> {
    style::moli_rule_tree::parse_property_rule_view(css_text)
        .map(normalize_css_property_rule_view_name)
}

fn normalize_css_property_rule_view_name(mut view: CssPropertyRuleView) -> CssPropertyRuleView {
    let decoded_name = {
        let mut input = ParserInput::new(&view.name);
        let mut parser = Parser::new(&mut input);
        parser
            .expect_ident_cloned()
            .ok()
            .filter(|name| parser.expect_exhausted().is_ok() && name.starts_with("--"))
            .map(|name| name.to_string())
    };
    if let Some(decoded_name) = decoded_name {
        view.name = decoded_name;
    }
    view
}

pub fn native_stylesheet_css_text_with_stylo(stylesheet: &CssNativeStylesheet) -> String {
    style::moli_rule_tree::stylesheet_rule_tree_css_text(stylesheet)
}

pub fn refresh_native_stylesheet_namespaces_after_cssom_mutation(stylesheet: &CssNativeStylesheet) {
    style::moli_rule_tree::refresh_stylesheet_namespaces_after_cssom_mutation(stylesheet);
}

pub fn css_rule_snapshot_from_native_with_stylo(
    rule: &style::stylesheets::CssRule,
    guard: &style::shared_lock::SharedRwLockReadGuard,
) -> CssRuleSnapshot {
    style::moli_rule_tree::stylesheet_rule_view_from_native(rule, guard)
}

pub fn keyframe_rule_snapshot_from_native_with_stylo(
    rule: &style::servo_arc::Arc<
        style::shared_lock::Locked<style::stylesheets::keyframes_rule::Keyframe>,
    >,
    guard: &style::shared_lock::SharedRwLockReadGuard,
) -> CssRuleSnapshot {
    style::moli_rule_tree::keyframe_rule_view_from_native(rule, guard)
}

pub fn native_stylesheet_counter_style_rule_read_with_stylo(
    stylesheet: &CssNativeStylesheet,
    rule_path: &[usize],
) -> Option<CssCounterStyleRuleView> {
    style::moli_rule_tree::stylesheet_rule_tree_counter_style_rule_view(stylesheet, rule_path)
}

pub fn native_stylesheet_font_face_rule_read_with_stylo(
    stylesheet: &CssNativeStylesheet,
    rule_path: &[usize],
) -> Option<CssFontFaceRuleView> {
    style::moli_rule_tree::stylesheet_rule_tree_font_face_rule_view(stylesheet, rule_path)
}

pub fn native_stylesheet_import_rule_read_with_stylo(
    stylesheet: &CssNativeStylesheet,
    rule_path: &[usize],
) -> Option<CssImportRuleView> {
    style::moli_rule_tree::stylesheet_rule_tree_import_rule_view(stylesheet, rule_path)
}

pub fn native_stylesheet_namespace_rule_read_with_stylo(
    stylesheet: &CssNativeStylesheet,
    rule_path: &[usize],
) -> Option<CssNamespaceRuleView> {
    style::moli_rule_tree::stylesheet_rule_tree_namespace_rule_view(stylesheet, rule_path)
}

pub fn native_stylesheet_margin_rule_read_with_stylo(
    stylesheet: &CssNativeStylesheet,
    rule_path: &[usize],
) -> Option<CssMarginRuleView> {
    style::moli_rule_tree::stylesheet_rule_tree_margin_rule_view(stylesheet, rule_path)
}

pub fn native_stylesheet_property_rule_read_with_stylo(
    stylesheet: &CssNativeStylesheet,
    rule_path: &[usize],
) -> Option<CssPropertyRuleView> {
    style::moli_rule_tree::stylesheet_rule_tree_property_rule_view(stylesheet, rule_path)
        .map(normalize_css_property_rule_view_name)
}

pub fn native_stylesheet_font_feature_values_rule_read_with_stylo(
    stylesheet: &CssNativeStylesheet,
    rule_path: &[usize],
) -> Option<CssFontFeatureValuesRuleView> {
    style::moli_rule_tree::stylesheet_rule_tree_font_feature_values_rule_view(stylesheet, rule_path)
}

pub fn parse_stylesheet_rule_snapshot_for_insert_with_stylo(
    existing_rule_texts: &[String],
    rule_text: &str,
    index: usize,
    constructed: bool,
) -> Result<CssRuleSnapshot, CssRuleInsertError> {
    style::moli_rule_tree::parse_stylesheet_rule_view_for_insert(
        existing_rule_texts,
        rule_text,
        index,
        constructed,
    )
}

pub fn insert_detached_nested_rule_with_stylo(
    namespace_rule_texts: &[String],
    existing_rule_texts: &[String],
    rule_text: &str,
    index: usize,
    containing_rule_type_bits: u32,
    parse_relative_rule_type: Option<style::stylesheets::CssRuleType>,
) -> Result<CssDetachedRuleListMutation, CssRuleInsertError> {
    style::moli_rule_tree::insert_nested_rule(
        namespace_rule_texts,
        existing_rule_texts,
        rule_text,
        index,
        containing_rule_type_bits,
        parse_relative_rule_type,
    )
}

pub fn delete_detached_nested_rule_with_stylo(
    namespace_rule_texts: &[String],
    existing_rule_texts: &[String],
    index: usize,
    containing_rule_type_bits: u32,
    parse_relative_rule_type: Option<style::stylesheets::CssRuleType>,
) -> Result<CssDetachedRuleListMutation, CssRuleInsertError> {
    style::moli_rule_tree::delete_nested_rule(
        namespace_rule_texts,
        existing_rule_texts,
        index,
        containing_rule_type_bits,
        parse_relative_rule_type,
    )
}

pub fn parse_nested_rule_block_snapshots_with_stylo(
    namespace_rule_texts: &[String],
    block_text: &str,
    rule_type: style::stylesheets::CssRuleType,
    containing_rule_type_bits: u32,
    parse_relative_rule_type: Option<style::stylesheets::CssRuleType>,
    wants_first_declaration_block: bool,
) -> Result<CssDetachedRuleListMutation, CssRuleInsertError> {
    style::moli_rule_tree::parse_nested_rule_block_views(
        namespace_rule_texts,
        block_text,
        rule_type,
        containing_rule_type_bits,
        parse_relative_rule_type,
        wants_first_declaration_block,
    )
}

pub fn insert_detached_keyframe_rule_with_stylo(
    existing_rule_texts: &[String],
    rule_text: &str,
    index: usize,
) -> Result<CssDetachedRuleListMutation, CssRuleInsertError> {
    style::moli_rule_tree::insert_keyframe_rule(&[], existing_rule_texts, rule_text, index)
}

pub fn delete_detached_keyframe_rule_with_stylo(
    existing_rule_texts: &[String],
    index: usize,
) -> Result<CssDetachedRuleListMutation, CssRuleInsertError> {
    style::moli_rule_tree::delete_keyframe_rule(&[], existing_rule_texts, index)
}

pub fn normalize_keyframe_selector_text_with_stylo(selector_text: &str) -> Option<String> {
    style::moli_rule_tree::normalize_keyframe_selector_text(selector_text)
}

pub fn normalize_page_selector_text_with_stylo(selector_text: &str) -> Option<String> {
    style::moli_rule_tree::normalize_page_selector_text(selector_text)
}

pub fn keyframe_selector_texts_match_with_stylo(
    existing_selector_text: &str,
    selector_text: &str,
) -> bool {
    style::moli_rule_tree::keyframe_selector_texts_match(existing_selector_text, selector_text)
}

#[cfg(test)]
mod tests {
    use super::{
        CssRuleInsertError, delete_detached_keyframe_rule_with_stylo,
        delete_detached_nested_rule_with_stylo, font_face_descriptor_property_names_with_stylo,
        insert_detached_keyframe_rule_with_stylo, insert_detached_nested_rule_with_stylo,
        keyframe_selector_texts_match_with_stylo,
        native_stylesheet_counter_style_rule_read_with_stylo,
        native_stylesheet_css_text_with_stylo, native_stylesheet_font_face_rule_read_with_stylo,
        native_stylesheet_font_feature_values_rule_read_with_stylo,
        native_stylesheet_import_rule_read_with_stylo,
        native_stylesheet_margin_rule_read_with_stylo,
        native_stylesheet_namespace_rule_read_with_stylo,
        native_stylesheet_property_rule_read_with_stylo,
        normalize_keyframe_selector_text_with_stylo, normalize_page_selector_text_with_stylo,
        page_descriptor_property_names_with_stylo, parse_condition_rule_view_with_stylo,
        parse_constructed_stylesheet_rule_snapshots_with_stylo,
        parse_counter_style_rule_view_with_stylo, parse_font_face_descriptor_block_with_stylo,
        parse_font_face_descriptor_entry_with_stylo, parse_font_face_rule_view_with_stylo,
        parse_font_feature_values_rule_view_with_stylo, parse_import_rule_view_with_stylo,
        parse_keyframes_rule_view_with_stylo, parse_layer_rule_view_with_stylo,
        parse_namespace_rule_view_with_stylo, parse_nested_rule_block_snapshots_with_stylo,
        parse_page_descriptor_block_with_stylo, parse_page_descriptor_entries_with_stylo,
        parse_page_margin_descriptor_block_with_stylo, parse_page_margin_rule_view_with_stylo,
        parse_page_rule_view_with_stylo, parse_property_rule_view_with_stylo,
        parse_stylesheet_rule_snapshot_for_insert_with_stylo,
        parse_stylesheet_rule_snapshots_with_stylo, parse_stylesheet_rule_texts_with_stylo,
        remove_css_comments_without_retokenizing,
    };
    use style::stylesheets::CssRuleType;

    fn parse_native_stylesheet(css_text: &str) -> super::CssNativeStylesheet {
        style::moli_rule_tree::parse_stylesheet_rule_tree(css_text)
    }

    fn native_rule_snapshots(
        stylesheet: &super::CssNativeStylesheet,
    ) -> Vec<super::CssRuleSnapshot> {
        style::moli_rule_tree::stylesheet_rule_tree_rule_views(stylesheet)
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_nested_snapshots() {
        let rules = parse_stylesheet_rule_snapshots_with_stylo(
            "@media screen { .one { margin: 0; } @supports (display: grid) { .two { display: grid; } } }",
        );

        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].css_text,
            "@media screen {\n  .one { margin: 0px; }\n  @supports (display: grid) {\n  .two { display: grid; }\n}\n}"
        );
        assert_eq!(rules[0].child_rules.len(), 2);
        assert_eq!(rules[0].child_rules[0].css_text, ".one { margin: 0px; }");
        assert_eq!(
            rules[0].child_rules[0].selector_text.as_deref(),
            Some(".one")
        );
        assert_eq!(
            rules[0].child_rules[0].declaration_text.as_deref(),
            Some("margin: 0px;")
        );
        assert_eq!(rules[0].child_rules[1].child_rules.len(), 1);
        assert_eq!(
            rules[0].child_rules[1].child_rules[0].css_text,
            ".two { display: grid; }"
        );
        assert_eq!(
            rules[0].child_rules[1].child_rules[0]
                .selector_text
                .as_deref(),
            Some(".two")
        );
        assert_eq!(
            rules[0].child_rules[1].child_rules[0]
                .declaration_text
                .as_deref(),
            Some("display: grid;")
        );
    }

    #[test]
    fn nested_rule_block_snapshot_adapter_uses_stylo_nested_parser() {
        let namespaces = vec![String::from(
            r#"@namespace svg url("http://www.w3.org/2000/svg");"#,
        )];
        let parsed = parse_nested_rule_block_snapshots_with_stylo(
            &namespaces,
            "color: red; & svg|path { color: blue; } --after: 1;",
            CssRuleType::Style,
            CssRuleType::Style.bit(),
            Some(CssRuleType::Style),
            true,
        )
        .expect("nested block should parse through Stylo");

        assert_eq!(
            parsed.first_declaration_text.as_deref(),
            Some("color: red;")
        );
        assert_eq!(parsed.rules.len(), 2);
        assert_eq!(parsed.rules[0].rule_type, CssRuleType::Style);
        assert_eq!(parsed.rules[0].selector_text.as_deref(), Some("& svg|path"));
        assert_eq!(
            parsed.rules[0].declaration_text.as_deref(),
            Some("color: blue;")
        );
        assert_eq!(parsed.rules[1].rule_type, CssRuleType::NestedDeclarations);
        assert_eq!(
            parsed.rules[1].declaration_text.as_deref(),
            Some("--after: 1;")
        );
    }

    #[test]
    fn native_stylesheet_adapter_uses_stylo_insert_rule_validation() {
        let existing = vec![
            String::from("@import url(\"a.css\");"),
            String::from("@namespace svg url(\"http://www.w3.org/2000/svg\");"),
            String::from(".one { color: red; }"),
        ];

        let inserted = parse_stylesheet_rule_snapshot_for_insert_with_stylo(
            &existing,
            ".two { padding: 0 1px; }",
            3,
            false,
        )
        .expect("style rule should insert");
        assert_eq!(inserted.css_text, ".two { padding: 0px 1px; }");
        assert_eq!(
            parse_stylesheet_rule_snapshot_for_insert_with_stylo(
                &existing,
                "@namespace html url(\"http://www.w3.org/1999/xhtml\");",
                3,
                false,
            ),
            Err(CssRuleInsertError::InvalidState)
        );
        assert_eq!(
            parse_stylesheet_rule_snapshot_for_insert_with_stylo(
                &[],
                "@import url(\"ignored.css\");",
                0,
                true,
            ),
            Err(CssRuleInsertError::Syntax)
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_native_rule_snapshots() {
        let stylesheet = parse_native_stylesheet(
            "@namespace svg url(\"http://www.w3.org/2000/svg\"); .one { color: red; }",
        );

        assert_eq!(
            native_stylesheet_css_text_with_stylo(&stylesheet),
            "@namespace svg url(\"http://www.w3.org/2000/svg\"); .one { color: red; }"
        );
        assert_eq!(
            native_rule_snapshots(&stylesheet)
                .iter()
                .map(|rule| rule.css_text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "@namespace svg url(\"http://www.w3.org/2000/svg\");",
                ".one { color: red; }",
            ]
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_keyframes_snapshot() {
        let view = parse_keyframes_rule_view_with_stylo(
            r#"@keyframes "slide show" { from { opacity: 0; } to { opacity: 1; } }"#,
        )
        .expect("valid @keyframes should produce a CSSOM view");
        assert_eq!(view.name, "slide show");
        assert_eq!(
            view.css_text,
            "@keyframes slide\\ show {\n0% { opacity: 0; }\n100% { opacity: 1; }\n}"
        );
        assert!(
            parse_keyframes_rule_view_with_stylo("@keyframes none { from { opacity: 0; } }")
                .is_none(),
            "Stylo rejects invalid keyframes names"
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_property_rules() {
        let property_rule =
            r#"@property --accent { syntax: "<color>"; inherits: false; initial-value: red; }"#;
        let rules = parse_stylesheet_rule_snapshots_with_stylo(property_rule);

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_type, CssRuleType::Property);

        let view = parse_property_rule_view_with_stylo(property_rule)
            .expect("valid @property should produce a CSSOM view");
        assert_eq!(view.name, "--accent");
        assert_eq!(view.syntax, "<color>");
        assert!(!view.inherits);
        assert_eq!(view.initial_value.as_deref(), Some("red"));
        assert_eq!(view.css_text, rules[0].css_text);

        assert!(
            parse_property_rule_view_with_stylo(
                r#"@property --accent { syntax: "<color>"; inherits: false; initial-value: 10px; }"#
            )
            .is_none(),
            "Stylo rejects initial values that do not match the syntax descriptor"
        );
    }

    #[test]
    fn property_rule_views_decode_escaped_names_without_changing_css_text() {
        let property_rule = r#"@property --tab\9 tab { syntax: "*"; inherits: true; }"#;
        let parsed = parse_property_rule_view_with_stylo(property_rule)
            .expect("escaped @property name should produce a CSSOM view");

        assert_eq!(parsed.name, "--tab\ttab");
        assert_eq!(parsed.css_text, property_rule);

        let stylesheet = parse_native_stylesheet(property_rule);
        assert_eq!(
            native_stylesheet_property_rule_read_with_stylo(&stylesheet, &[0]),
            Some(parsed),
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_counter_style_rules() {
        let counter_style_rule =
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; }"#;
        let rules = parse_stylesheet_rule_snapshots_with_stylo(counter_style_rule);

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_type, CssRuleType::CounterStyle);
        assert_eq!(
            rules[0].css_text,
            r#"@counter-style thumbs { system: cyclic; suffix: " "; symbols: "*"; }"#
        );

        let view = parse_counter_style_rule_view_with_stylo(counter_style_rule)
            .expect("valid @counter-style should produce a CSSOM view");
        assert_eq!(view.name, "thumbs");
        assert_eq!(view.css_text, rules[0].css_text);
        assert!(
            parse_counter_style_rule_view_with_stylo(
                r#"@counter-style thumbs { system: cyclic; suffix: " "; }"#
            )
            .is_none(),
            "Stylo rejects counter styles whose system requires symbols"
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_font_face_rules() {
        let font_face_rule = r#"@font-face { src: url(http://foo/bar/font.ttf); font-family: Foo; font-weight: bold; }"#;
        let rules = parse_stylesheet_rule_snapshots_with_stylo(font_face_rule);

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_type, CssRuleType::FontFace);

        let view = parse_font_face_rule_view_with_stylo(font_face_rule)
            .expect("valid @font-face should produce a CSSOM view");
        assert_eq!(view.css_text, rules[0].css_text);
        assert_eq!(
            view.style_text,
            r#"font-family: Foo; src: url("http://foo/bar/font.ttf"); font-weight: bold;"#
        );
        assert!(parse_font_face_rule_view_with_stylo(".foo { color: red; }").is_none());
    }

    #[test]
    fn descriptor_property_names_are_exported_from_stylo_metadata() {
        let font_face = font_face_descriptor_property_names_with_stylo();
        assert!(font_face.contains(&"font-display"));
        assert!(font_face.contains(&"ascent-override"));
        assert!(font_face.contains(&"size-adjust"));

        let page = page_descriptor_property_names_with_stylo();
        assert!(page.contains(&"margin-top"));
        assert!(page.contains(&"page-orientation"));
        assert!(page.contains(&"marks"));
        assert!(page.contains(&"bleed"));
    }

    #[test]
    fn font_face_descriptor_entry_adapter_uses_stylo_descriptor_api() {
        let entry = parse_font_face_descriptor_entry_with_stylo(
            "src",
            "local(STIXGeneral), url(/stixfonts/STIXGeneral.otf)",
        )
        .expect("font-face src should parse through Stylo");
        assert_eq!(entry.name, "src");
        assert_eq!(
            entry.value,
            r#"local(STIXGeneral), url("/stixfonts/STIXGeneral.otf")"#
        );
        let family =
            parse_font_face_descriptor_entry_with_stylo("font-family", "Bar").expect("font-family");
        assert_eq!(family.name, "font-family");
        assert_eq!(family.value, "Bar");

        assert!(
            parse_font_face_descriptor_entry_with_stylo(
                "src",
                r#"url("a.woff2"); font-family: injected"#
            )
            .is_none(),
            "font-face descriptor values must not inject extra declarations"
        );
        assert!(
            parse_font_face_descriptor_entry_with_stylo("font-weight", "definitely-invalid")
                .is_none(),
            "font-face descriptor values should be Stylo-validated"
        );
        assert!(
            parse_font_face_descriptor_entry_with_stylo("font-weight", "400 !important").is_none(),
            "font-face descriptor priority must stay outside the single-entry adapter"
        );
        assert!(
            parse_font_face_descriptor_entry_with_stylo("font", "16px serif").is_none(),
            "ordinary declaration names must not enter the font-face descriptor adapter"
        );
    }

    #[test]
    fn font_face_descriptor_entry_adapter_uses_cssom_value_fragment_eof() {
        assert_eq!(
            parse_font_face_descriptor_block_with_stylo("src: local(Foo)").as_deref(),
            Some("src: local(Foo);"),
            "font-face descriptor blocks should recover a final declaration at EOF"
        );

        let entry = parse_font_face_descriptor_entry_with_stylo("src", "local(Bar")
            .expect("single descriptor writes should parse CSSOM value fragments at EOF");
        assert_eq!(entry.name, "src");
        assert_eq!(entry.value, "local(Bar)");

        assert!(
            parse_font_face_descriptor_entry_with_stylo(
                "src",
                r#"url("a.woff2"); font-family: injected"#
            )
            .is_none(),
            "CSSOM value fragment parsing must still reject declaration injection"
        );
    }

    #[test]
    fn font_face_unicode_range_descriptor_accepts_comments_between_tokens() {
        for (value, compacted, expected) in [
            ("u/**/+/**/a/**/?", "u+a?", "U+A0-AF"),
            ("u/**/+0a/**/?", "u+0a?", "U+A0-AF"),
            ("u/**/+0/**/?", "u+0?", "U+0-F"),
            ("u/**/+0/**/-0a", "u+0-0a", "U+0-A"),
            ("u/**/+0/**/-1", "u+0-1", "U+0-1"),
            ("u/**/+/**/?", "u+?", "U+0-F"),
        ] {
            assert_eq!(
                remove_css_comments_without_retokenizing(value).as_deref(),
                Some(compacted)
            );
            let entry = parse_font_face_descriptor_entry_with_stylo("unicode-range", value)
                .unwrap_or_else(|| panic!("comments may occur between tokens in {value}"));

            assert_eq!(entry.value, expected);
        }

        let entry = parse_font_face_descriptor_entry_with_stylo(
            "unicode-range",
            "u/**/+a/**/?, u/**/+0/**/-1",
        )
        .expect("commented unicode ranges may occur in a list");
        assert_eq!(entry.value, "U+A0-AF, U+0-1");
    }

    #[test]
    fn font_face_unicode_range_comment_recovery_does_not_merge_tokens() {
        for invalid in ["u/**/0", "u/**/+a/**/b", "u/**/+0000000"] {
            assert!(
                parse_font_face_descriptor_entry_with_stylo("unicode-range", invalid).is_none(),
                "{invalid} must remain invalid"
            );
        }
    }

    #[test]
    fn font_face_descriptor_block_uses_stylo_descriptor_api() {
        assert_eq!(
            parse_font_face_descriptor_block_with_stylo(
                "src: local(STIXGeneral), url(/stixfonts/STIXGeneral.otf); font-family: STIX;"
            )
            .as_deref(),
            Some(
                r#"font-family: STIX; src: local(STIXGeneral), url("/stixfonts/STIXGeneral.otf");"#
            )
        );
        assert_eq!(
            parse_font_face_descriptor_block_with_stylo("").as_deref(),
            Some("")
        );
        assert_eq!(
            parse_font_face_descriptor_block_with_stylo(
                "font-family: Bar !important; src: local(Bar);"
            )
            .as_deref(),
            Some("font-family: Bar !important; src: local(Bar);")
        );
        assert!(
            parse_font_face_descriptor_block_with_stylo(
                "font-family: Bar !important extra; src: local(Bar);"
            )
            .is_none(),
            "malformed CSSOM descriptor priority must not be accepted"
        );
        assert!(
            parse_font_face_descriptor_block_with_stylo("font: 16px serif;").is_none(),
            "ordinary declaration names must not enter the font-face descriptor block adapter"
        );
        assert!(
            parse_font_face_descriptor_block_with_stylo(
                r#"src: url("a.woff2"); font-family: injected;"#
            )
            .is_some(),
            "valid font-face descriptor lists should parse as a block"
        );
        assert!(
            parse_font_face_descriptor_block_with_stylo(r#"src: url("a.woff2"); color: red;"#)
                .is_none(),
            "unsupported descriptors must not be silently dropped by the block adapter"
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_import_rule_view() {
        let view = parse_import_rule_view_with_stylo(
            r#"@import url("support/c.css") layer(A.B) supports((display: flex) or (foo: bar)) print and (WiDtH);"#,
        )
        .expect("valid import rule should produce a Stylo view");

        assert_eq!(view.href, "support/c.css");
        assert_eq!(view.layer_name.as_deref(), Some("A.B"));
        assert_eq!(
            view.supports_text.as_deref(),
            Some("(display: flex) or (foo: bar)")
        );
        assert_eq!(view.media_text, "print and (width)");
        assert_eq!(
            view.condition_prefix,
            "layer(A.B) supports((display: flex) or (foo: bar))"
        );
        assert!(parse_import_rule_view_with_stylo(".foo { color: red; }").is_none());
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_namespace_rule_view() {
        let view = parse_namespace_rule_view_with_stylo("@namespace svg url(http://servo);")
            .expect("valid namespace rule should produce a Stylo view");

        assert_eq!(view.prefix, "svg");
        assert_eq!(view.namespace_uri, "http://servo");
        assert_eq!(view.css_text, r#"@namespace svg url("http://servo");"#);
        assert!(parse_namespace_rule_view_with_stylo(".foo { color: red; }").is_none());
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_condition_rule_view() {
        let container =
            parse_condition_rule_view_with_stylo("@container card (inline-size > 10px) {}")
                .expect("valid container rule should produce a Stylo view");

        assert_eq!(container.rule_type, CssRuleType::Container);
        assert_eq!(container.condition_text, "card (inline-size > 10px)");
        assert_eq!(container.container_name.as_deref(), Some("card"));
        assert_eq!(
            container.container_query.as_deref(),
            Some("(inline-size > 10px)")
        );

        let scope = parse_condition_rule_view_with_stylo("@scope (.a) to (> .b) {}")
            .expect("valid scope rule should produce a Stylo view");
        assert_eq!(scope.rule_type, CssRuleType::Scope);
        assert_eq!(scope.scope_start.as_deref(), Some(".a"));
        assert_eq!(scope.scope_end.as_deref(), Some("> .b"));
        assert!(parse_condition_rule_view_with_stylo("@layer a {}").is_none());
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_layer_rule_view() {
        let block = parse_layer_rule_view_with_stylo("@layer A.B {}")
            .expect("valid layer block should produce a Stylo view");
        assert_eq!(block.rule_type, CssRuleType::LayerBlock);
        assert_eq!(block.name.as_deref(), Some("A.B"));
        assert!(block.names.is_empty());

        let statement = parse_layer_rule_view_with_stylo("@layer A, B.C;")
            .expect("valid layer statement should produce a Stylo view");
        assert_eq!(statement.rule_type, CssRuleType::LayerStatement);
        assert_eq!(statement.names, vec!["A", "B.C"]);
        assert!(parse_layer_rule_view_with_stylo("@media screen {}").is_none());
    }

    #[test]
    fn native_stylesheet_adapter_exposes_typed_rule_reads_by_path() {
        let css_text = concat!(
            r#"@import url("support/c.css") layer(A.B) print;"#,
            r#"@namespace svg url(http://servo);"#,
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; }"#,
            r#"@font-face { font-family: Foo; src: local(Foo); }"#,
            r#"@property --accent { syntax: "<color>"; inherits: false; initial-value: red; }"#,
            r#"@font-feature-values test_family { @annotation { the_first: 6; } }"#,
            "@keyframes slide { from { opacity: 0; } to { opacity: 1; } }",
            "@media screen {}",
            "@layer A.B {}",
        );
        let stylesheet = parse_native_stylesheet(css_text);
        let rules = native_rule_snapshots(&stylesheet);

        assert_eq!(rules.len(), 9);
        assert_eq!(
            native_stylesheet_import_rule_read_with_stylo(&stylesheet, &[0]),
            parse_import_rule_view_with_stylo(&rules[0].css_text)
        );
        assert_eq!(
            native_stylesheet_namespace_rule_read_with_stylo(&stylesheet, &[1]),
            parse_namespace_rule_view_with_stylo(&rules[1].css_text)
        );
        assert_eq!(
            native_stylesheet_counter_style_rule_read_with_stylo(&stylesheet, &[2]),
            parse_counter_style_rule_view_with_stylo(&rules[2].css_text)
        );
        assert_eq!(
            native_stylesheet_font_face_rule_read_with_stylo(&stylesheet, &[3]),
            parse_font_face_rule_view_with_stylo(&rules[3].css_text)
        );
        assert_eq!(
            native_stylesheet_property_rule_read_with_stylo(&stylesheet, &[4]),
            parse_property_rule_view_with_stylo(&rules[4].css_text)
        );
        assert_eq!(
            native_stylesheet_font_feature_values_rule_read_with_stylo(&stylesheet, &[5]),
            parse_font_feature_values_rule_view_with_stylo(&rules[5].css_text)
        );
        assert_eq!(rules[6].prelude_text.as_deref(), Some("slide"));
        assert_eq!(rules[7].prelude_text.as_deref(), Some("screen"));
        assert_eq!(rules[8].prelude_text.as_deref(), Some("A.B"));
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_font_feature_values_rules() {
        let font_feature_values_rule = "@font-feature-values test_family { @annotation { the_first: 6; } @styleset { yo: 7; di: 10 9 4 5; } }";
        let rules = parse_stylesheet_rule_snapshots_with_stylo(font_feature_values_rule);

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_type, CssRuleType::FontFeatureValues);
        assert_eq!(
            rules[0].css_text,
            "@font-feature-values test_family {\n@annotation {\nthe_first: 6;\n}\n@styleset {\nyo: 7;\ndi: 10 9 4 5;\n}\n}"
        );

        let view = parse_font_feature_values_rule_view_with_stylo(font_feature_values_rule)
            .expect("font-feature-values view");
        assert_eq!(view.css_text, rules[0].css_text);
        assert_eq!(view.font_family, "test_family");
        assert_eq!(view.annotation[0].name, "the_first");
        assert_eq!(view.annotation[0].values, vec![6]);
        assert_eq!(view.styleset[0].name, "yo");
        assert_eq!(view.styleset[0].values, vec![7]);
        assert_eq!(view.styleset[1].name, "di");
        assert_eq!(view.styleset[1].values, vec![10, 9, 4, 5]);
        assert!(
            parse_font_feature_values_rule_view_with_stylo(
                "@font-feature-values serif { @annotation { the_first: 6; } }"
            )
            .is_none(),
            "Stylo rejects generic family names"
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_page_rules() {
        let page_rule =
            r#"@page :first { margin-top: 1px; @top-left { content: "x"; color: red; } }"#;
        let rules = parse_stylesheet_rule_snapshots_with_stylo(page_rule);

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_type, CssRuleType::Page);
        assert_eq!(
            rules[0].css_text,
            "@page :first {\n  margin-top: 1px;\n  @top-left { content: \"x\"; color: red; }\n}"
        );
        assert_eq!(rules[0].child_rules.len(), 1);
        assert_eq!(rules[0].child_rules[0].rule_type, CssRuleType::Margin);
        assert_eq!(
            rules[0].child_rules[0].css_text,
            "@top-left { content: \"x\"; color: red; }"
        );

        let view = parse_page_rule_view_with_stylo(page_rule)
            .expect("valid @page should produce a CSSOM view");
        assert_eq!(view.css_text, rules[0].css_text);
        assert_eq!(view.selector_text, ":first");
        assert_eq!(view.style_text, "margin-top: 1px;");
        assert_eq!(view.child_rules.len(), 1);
        assert_eq!(view.child_rules[0].name, "top-left");
        assert_eq!(
            view.child_rules[0].style_text,
            r#"content: "x"; color: red;"#
        );
        let stylesheet = parse_native_stylesheet(page_rule);
        assert_eq!(
            native_stylesheet_margin_rule_read_with_stylo(&stylesheet, &[0, 0]).as_ref(),
            Some(&view.child_rules[0])
        );

        let margin_view =
            parse_page_margin_rule_view_with_stylo(r#"@top-left { content: "x"; color: red; }"#)
                .expect("valid page margin rule should produce a CSSOM view");
        assert_eq!(margin_view, view.child_rules[0]);
        let descriptor_entries =
            parse_page_descriptor_entries_with_stylo("margin", "1px 2px 3px 4px")
                .expect("page margin shorthand should parse through Stylo");
        assert_eq!(
            descriptor_entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("margin-top", "1px"),
                ("margin-right", "2px"),
                ("margin-bottom", "3px"),
                ("margin-left", "4px"),
            ]
        );
        assert!(
            parse_page_descriptor_entries_with_stylo("margin-top", "1px; margin-bottom: 2px")
                .is_none(),
            "page descriptor values must not inject extra declarations"
        );
        assert_eq!(
            parse_page_descriptor_entries_with_stylo("size", "jis-b5 landscape")
                .expect("page size descriptor should parse through Stylo")
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("size", "jis-b5 landscape")]
        );
        assert_eq!(
            parse_page_descriptor_entries_with_stylo("page-orientation", "rotate-left")
                .expect("page orientation descriptor should parse through Stylo")
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("page-orientation", "rotate-left")]
        );
        assert!(parse_page_rule_view_with_stylo(".foo { color: red; }").is_none());
        assert!(parse_page_margin_rule_view_with_stylo("@media screen { }").is_none());
    }

    #[test]
    fn page_and_margin_descriptor_blocks_match_stylo_rule_views() {
        let page_style = "margin: 1px 2px; size: portrait; color: red;";
        let page_block = parse_page_descriptor_block_with_stylo(page_style)
            .expect("page descriptor block should parse through Stylo");
        let page_view = parse_page_rule_view_with_stylo(&format!("@page {{ {page_style} }}"))
            .expect("page rule should parse through Stylo");
        assert_eq!(page_block, page_view.style_text);
        assert!(!page_block.contains("color"));
        assert_eq!(
            parse_page_descriptor_block_with_stylo("color: red;").as_deref(),
            Some("")
        );

        let margin_style = r#"content: "x"; color: red; margin-top: 4px; bad-descriptor: 1;"#;
        let margin_block =
            parse_page_margin_descriptor_block_with_stylo("bottom-right", margin_style)
                .expect("page margin descriptor block should parse through Stylo");
        let margin_view =
            parse_page_margin_rule_view_with_stylo(&format!("@bottom-right {{ {margin_style} }}"))
                .expect("page margin rule should parse through Stylo");
        assert_eq!(margin_block, margin_view.style_text);
        assert_eq!(margin_view.name, "bottom-right");
        assert!(margin_block.contains("margin-top: 4px"));
        assert!(!margin_block.contains("bad-descriptor"));
        assert_eq!(
            parse_page_margin_descriptor_block_with_stylo("top-left", "bad-descriptor: 1;")
                .as_deref(),
            Some("")
        );
        assert!(
            parse_page_margin_descriptor_block_with_stylo("not-a-margin", r#"content: "x";"#)
                .is_none()
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_nested_mutation_snapshots() {
        let existing = vec![String::from(".one { color: red; }")];
        let inserted = insert_detached_nested_rule_with_stylo(
            &[],
            &existing,
            "@supports (display: grid) { .two { display: grid; } }",
            1,
            CssRuleType::Media.bit(),
            None,
        )
        .expect("supports rule should insert into media rule");
        assert_eq!(inserted.rules.len(), 2);
        assert_eq!(inserted.rules[1].rule_type, CssRuleType::Supports);
        assert_eq!(
            inserted.rules[1].child_rules[0].css_text,
            ".two { display: grid; }"
        );

        let deleted = delete_detached_nested_rule_with_stylo(
            &[],
            &inserted
                .rules
                .iter()
                .map(|rule| rule.css_text.clone())
                .collect::<Vec<_>>(),
            0,
            CssRuleType::Media.bit(),
            None,
        )
        .expect("nested style rule should delete");
        assert_eq!(deleted.rules.len(), 1);
        assert_eq!(
            deleted.css_text,
            "@supports (display: grid) {\n  .two { display: grid; }\n}"
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_keyframe_mutation_snapshots() {
        let existing = vec![String::from("0% { opacity: 0; }")];
        let inserted = insert_detached_keyframe_rule_with_stylo(
            &existing,
            "to { opacity: 1; transform: translateX(10px); }",
            1,
        )
        .expect("keyframe rule should insert into keyframes rule");
        assert_eq!(inserted.rules.len(), 2);
        assert_eq!(inserted.rules[1].rule_type, CssRuleType::Keyframe);
        assert_eq!(
            inserted.rules[1].css_text,
            "100% { opacity: 1; transform: translateX(10px); }"
        );

        let deleted = delete_detached_keyframe_rule_with_stylo(
            &inserted
                .rules
                .iter()
                .map(|rule| rule.css_text.clone())
                .collect::<Vec<_>>(),
            0,
        )
        .expect("keyframe rule should delete");
        assert_eq!(deleted.rules.len(), 1);
        assert_eq!(
            deleted.css_text,
            "100% { opacity: 1; transform: translateX(10px); }"
        );
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_keyframe_selector_helpers() {
        assert_eq!(
            normalize_keyframe_selector_text_with_stylo("from"),
            Some(String::from("0%"))
        );
        assert_eq!(
            normalize_keyframe_selector_text_with_stylo("50%, to"),
            Some(String::from("50%, 100%"))
        );
        assert_eq!(normalize_keyframe_selector_text_with_stylo("body"), None);

        assert!(keyframe_selector_texts_match_with_stylo("from", "0%"));
        assert!(keyframe_selector_texts_match_with_stylo(
            "50%, to",
            "50%, 100%"
        ));
        assert!(!keyframe_selector_texts_match_with_stylo("50%, to", "50%"));
    }

    #[test]
    fn native_stylesheet_adapter_exposes_stylo_page_selector_helper() {
        assert_eq!(
            normalize_page_selector_text_with_stylo(":RIGHT"),
            Some(String::from(":right"))
        );
        assert_eq!(
            normalize_page_selector_text_with_stylo(":first, named:left"),
            Some(String::from(":first, named:left"))
        );
        assert_eq!(
            normalize_page_selector_text_with_stylo(":notapagepseudo"),
            None
        );
    }

    #[test]
    fn native_stylesheet_adapter_uses_stylo_parse_and_serialize() {
        let rules = parse_stylesheet_rule_texts_with_stylo(
            "@import url(\"a.css\") screen; .one { margin: 0; }",
        );

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].css_text, "@import url(\"a.css\") screen;");
        assert_eq!(rules[1].css_text, ".one { margin: 0px; }");
    }

    #[test]
    fn constructed_native_stylesheet_adapter_drops_import_rules() {
        let rules = parse_constructed_stylesheet_rule_snapshots_with_stylo(
            "@import url(\"ignored.css\"); .target { color: blue; }",
        );

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].css_text, ".target { color: blue; }");
    }
}
