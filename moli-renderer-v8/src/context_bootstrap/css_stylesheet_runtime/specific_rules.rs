use super::*;
use crate::webidl_iterator::{
    MaplikeWebIdlIteratorMethod, call_maplike_webidl_for_each, new_maplike_webidl_iterator,
};

pub(crate) fn sync_css_property_rule_slots_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    view: &CssPropertyRuleView,
) {
    set_private_string(scope, rule, CSS_PROPERTY_RULE_NAME_SLOT, &view.name);
    set_private_string(scope, rule, CSS_PROPERTY_RULE_SYNTAX_SLOT, &view.syntax);
    set_private_value(
        scope,
        rule,
        CSS_PROPERTY_RULE_INHERITS_SLOT,
        v8::Boolean::new(scope, view.inherits).into(),
    );
    if let Some(initial_value) = view.initial_value.as_deref() {
        set_private_string(
            scope,
            rule,
            CSS_PROPERTY_RULE_INITIAL_VALUE_SLOT,
            initial_value,
        );
    } else {
        set_private_value(
            scope,
            rule,
            CSS_PROPERTY_RULE_INITIAL_VALUE_SLOT,
            v8::null(scope).into(),
        );
    }
}

pub(crate) fn css_property_rule_view_from_css_text(css_text: &str) -> Option<CssPropertyRuleView> {
    parse_property_rule_view_with_stylo(css_text)
}

pub(crate) fn css_property_rule_view_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssPropertyRuleView> {
    if let Some(view) = css_rule_live_stylesheet_property_rule_read(scope, object) {
        return Some(view);
    }
    css_rule_detached_snapshot_typed_view(scope, object, css_property_rule_view_from_css_text)
}

pub(crate) fn css_counter_style_rule_view_from_css_text(
    css_text: &str,
) -> Option<CssCounterStyleRuleView> {
    parse_counter_style_rule_view_with_stylo(css_text)
}

pub(crate) fn css_counter_style_rule_view_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssCounterStyleRuleView> {
    if let Some(view) = css_rule_live_stylesheet_counter_style_rule_read(scope, object) {
        return Some(view);
    }
    css_rule_detached_snapshot_typed_view(scope, object, css_counter_style_rule_view_from_css_text)
}

pub(crate) fn css_font_face_rule_view_from_css_text(css_text: &str) -> Option<CssFontFaceRuleView> {
    parse_font_face_rule_view_with_stylo(css_text)
}

pub(crate) fn css_font_face_rule_view_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssFontFaceRuleView> {
    if let Some(view) = css_rule_live_stylesheet_font_face_rule_read(scope, object) {
        return Some(view);
    }
    css_rule_detached_snapshot_typed_view(scope, object, css_font_face_rule_view_from_css_text)
}

pub(crate) fn css_font_face_rule_style_text_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if let Some(view) = css_rule_live_stylesheet_font_face_rule_read(scope, object) {
        return Some(view.style_text);
    }
    css_font_face_rule_view_from_object(scope, object).map(|view| view.style_text)
}

pub(crate) fn css_font_face_rule_style_text_from_css_text(css_text: &str) -> Option<String> {
    css_font_face_rule_view_from_css_text(css_text).map(|view| view.style_text)
}

pub(crate) fn css_page_rule_view_from_css_text(css_text: &str) -> Option<CssPageRuleView> {
    parse_page_rule_view_with_stylo(css_text)
}

pub(crate) fn css_page_rule_view_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssPageRuleView> {
    css_rule_detached_snapshot_typed_view(scope, object, css_page_rule_view_from_css_text)
}

pub(crate) fn css_page_rule_read_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::LiveStylesheetPageRuleRead> {
    if let Some(read) = css_rule_attached_native_page_read(scope, object) {
        return Some(read);
    }
    css_page_rule_view_from_object(scope, object).map(|view| {
        crate::live_stylesheet::LiveStylesheetPageRuleRead {
            selector_text: view.selector_text,
            declaration_text: view.style_text,
        }
    })
}

pub(crate) fn css_page_rule_style_text_from_css_text(css_text: &str) -> Option<String> {
    css_page_rule_view_from_css_text(css_text).map(|view| view.style_text)
}

pub(crate) fn css_page_margin_rule_view_from_css_text(css_text: &str) -> Option<CssMarginRuleView> {
    parse_page_margin_rule_view_with_stylo(css_text)
}

pub(crate) fn css_page_margin_rule_view_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssMarginRuleView> {
    if let Some(view) = css_rule_live_stylesheet_margin_rule_read(scope, object) {
        return Some(view);
    }
    css_rule_detached_snapshot_typed_view(scope, object, css_page_margin_rule_view_from_css_text)
}

pub(crate) fn sync_css_margin_rule_slots_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    view: &CssMarginRuleView,
) {
    set_private_string(scope, rule, CSS_MARGIN_RULE_NAME_SLOT, &view.name);
    set_private_string(
        scope,
        rule,
        CSS_MARGIN_RULE_STYLE_TEXT_SLOT,
        &view.style_text,
    );
    if let Some(style) = get_private_value(scope, rule, CSS_MARGIN_RULE_STYLE_OBJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_style_object_css_text_without_notify(scope, style, &view.style_text);
    }
}

pub(crate) fn css_font_feature_values_rule_view_from_css_text(
    css_text: &str,
) -> Option<CssFontFeatureValuesRuleView> {
    parse_font_feature_values_rule_view_with_stylo(css_text)
}

pub(crate) fn css_font_feature_values_rule_view_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssFontFeatureValuesRuleView> {
    if let Some(view) = css_rule_live_stylesheet_font_feature_values_rule_read(scope, object) {
        return Some(view);
    }
    css_rule_detached_snapshot_typed_view(
        scope,
        object,
        css_font_feature_values_rule_view_from_css_text,
    )
}

pub(crate) fn sync_css_font_feature_values_rule_slots_from_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    css_text: &str,
) {
    if let Some(view) = css_font_feature_values_rule_view_from_css_text(css_text) {
        sync_css_font_feature_values_rule_slots_from_stylo_view(scope, rule, &view);
    }
}

pub(crate) fn sync_css_font_feature_values_rule_slots_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    view: &CssFontFeatureValuesRuleView,
) {
    set_private_string(
        scope,
        rule,
        CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT,
        &view.font_family,
    );
    sync_font_feature_values_map_slot(
        scope,
        rule,
        CSS_FONT_FEATURE_VALUES_RULE_ANNOTATION_SLOT,
        &view.annotation,
    );
    sync_font_feature_values_map_slot(
        scope,
        rule,
        CSS_FONT_FEATURE_VALUES_RULE_ORNAMENTS_SLOT,
        &view.ornaments,
    );
    sync_font_feature_values_map_slot(
        scope,
        rule,
        CSS_FONT_FEATURE_VALUES_RULE_STYLISTIC_SLOT,
        &view.stylistic,
    );
    sync_font_feature_values_map_slot(
        scope,
        rule,
        CSS_FONT_FEATURE_VALUES_RULE_STYLESET_SLOT,
        &view.styleset,
    );
    sync_font_feature_values_map_slot(
        scope,
        rule,
        CSS_FONT_FEATURE_VALUES_RULE_CHARACTER_VARIANT_SLOT,
        &view.character_variant,
    );
    sync_font_feature_values_map_slot(
        scope,
        rule,
        CSS_FONT_FEATURE_VALUES_RULE_SWASH_SLOT,
        &view.swash,
    );
}

pub(crate) fn sync_css_font_feature_values_rule_slots_from_current_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) {
    if let Some(view) = css_font_feature_values_rule_view_from_object(scope, rule) {
        sync_css_font_feature_values_rule_slots_from_stylo_view(scope, rule, &view);
    }
}

pub(crate) fn css_namespace_rule_text_for_stylo_context(prefix: Option<&str>, uri: &str) -> String {
    let uri = serialize_css_string(uri);
    match prefix {
        Some(prefix) if !prefix.is_empty() => {
            let prefix = serialize_css_identifier(prefix);
            format!("@namespace {prefix} url({uri});")
        }
        _ => format!("@namespace url({uri});"),
    }
}

pub(crate) fn css_style_rule_selector_namespace_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> CssomSelectorNamespaceContext {
    get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
        .unwrap_or_default()
}

pub(crate) fn parent_rule_is_page_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    parent_rule.and_then(|rule| css_rule_current_stylo_rule_type_from_object(scope, rule))
        == Some(CssRuleType::Page)
}

pub(crate) fn build_css_margin_rule_object_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: CssMarginRuleView,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    CssMarginRuleDeclaration {
        css_text: view.css_text,
        name: view.name,
        style_text: view.style_text,
        parent_rule,
        parent_style_sheet,
    }
    .bind(scope)
    .expect("CSSMarginRule declaration should bind")
}

pub(crate) fn build_css_font_face_rule_object_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: CssFontFaceRuleView,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    let object = CssAtRuleDeclaration {
        css_text: view.css_text,
        rule_type: CSS_RULE_FONT_FACE_RULE_TYPE,
        parent_rule,
        parent_style_sheet,
    }
    .bind(scope)
    .expect("CSSFontFaceRule declaration should bind");
    moli_webapi_declare::initialize_web_api_object(scope, object, "CSSFontFaceRule")
        .expect("CSS rule interface identity should initialize");
    if let Some(prototype) = global_constructor_prototype(scope, "CSSFontFaceRule") {
        let _ = object.set_prototype(scope, prototype.into());
    }
    object
}

pub(crate) fn build_css_keyframe_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: CssStyleRuleTextParts,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    let style_text = parsed.style_text.clone();
    let rule = CssKeyframeRuleDeclaration {
        css_text: parsed.css_text,
        key_text: parsed.selector_text,
        style_text: parsed.style_text,
        parent_rule,
        parent_style_sheet,
    }
    .bind(scope)
    .expect("CSSKeyframeRule declaration should bind");
    seed_css_rule_stylo_declaration_block_from_style_text(
        scope,
        rule,
        &style_text,
        CssRulePdbDeclarationKind::KeyframeRule,
    );
    rule
}

pub(crate) fn build_css_font_feature_values_rule_object_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: CssFontFeatureValuesRuleView,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    let object = CssFontFeatureValuesRuleDeclaration {
        css_text: view.css_text,
        font_family: view.font_family,
        parent_rule,
        parent_style_sheet,
    }
    .bind(scope)
    .expect("CSSFontFeatureValuesRule declaration should bind");
    set_font_feature_values_map_slot(
        scope,
        object,
        CSS_FONT_FEATURE_VALUES_RULE_ANNOTATION_SLOT,
        &view.annotation,
    );
    set_font_feature_values_map_slot(
        scope,
        object,
        CSS_FONT_FEATURE_VALUES_RULE_ORNAMENTS_SLOT,
        &view.ornaments,
    );
    set_font_feature_values_map_slot(
        scope,
        object,
        CSS_FONT_FEATURE_VALUES_RULE_STYLISTIC_SLOT,
        &view.stylistic,
    );
    set_font_feature_values_map_slot(
        scope,
        object,
        CSS_FONT_FEATURE_VALUES_RULE_STYLESET_SLOT,
        &view.styleset,
    );
    set_font_feature_values_map_slot(
        scope,
        object,
        CSS_FONT_FEATURE_VALUES_RULE_CHARACTER_VARIANT_SLOT,
        &view.character_variant,
    );
    set_font_feature_values_map_slot(
        scope,
        object,
        CSS_FONT_FEATURE_VALUES_RULE_SWASH_SLOT,
        &view.swash,
    );
    object
}

pub(crate) fn build_css_property_rule_object_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    view: CssPropertyRuleView,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    bind_css_property_rule_object(
        scope,
        view.css_text,
        view.name,
        view.syntax,
        view.inherits,
        view.initial_value,
        parent_style_sheet,
        parent_rule,
    )
}

pub(crate) fn bind_css_property_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    css_text: String,
    name: String,
    syntax: String,
    inherits: bool,
    initial_value: Option<String>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    CssPropertyRuleDeclaration {
        css_text,
        rule_type: CSS_RULE_UNKNOWN_RULE_TYPE,
        name,
        syntax,
        inherits,
        initial_value,
        parent_rule,
        parent_style_sheet,
    }
    .bind(scope)
    .expect("CSSPropertyRule declaration should bind")
}

pub(crate) fn serialized_import_rule_text_with_media(
    href: &str,
    condition_prefix: &str,
    media_text: &str,
) -> String {
    let condition_prefix = condition_prefix.trim();
    let media_text = normalize_media_query_list(media_text);
    let condition = if media_text.is_empty() {
        condition_prefix.to_owned()
    } else if condition_prefix.is_empty() {
        media_text
    } else {
        format!("{condition_prefix} {media_text}")
    };
    let fallback = serialize_import_rule_fallback_text(href, &condition);
    canonical_single_stylesheet_rule_text_with_stylo(&fallback, CssRuleType::Import)
        .unwrap_or(fallback)
}

pub(crate) fn serialize_import_rule_fallback_text(href: &str, media: &str) -> String {
    let mut href_css = String::new();
    serialize_string(href, &mut href_css).expect("serializing to String cannot fail");
    let media = media.trim();
    if media.is_empty() {
        format!("@import url({href_css});")
    } else {
        format!("@import url({href_css}) {media};")
    }
}

pub(crate) fn serialize_page_rule_text(selector: &str, block: &str) -> String {
    let block = serialize_component_values_single_line(block)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let fallback = raw_page_rule_text(selector, &block);
    let Some(view) = parse_page_rule_view_with_stylo(&fallback) else {
        return fallback;
    };
    let nested = view
        .child_rules
        .iter()
        .map(|rule| rule.css_text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    raw_page_rule_text(
        &view.selector_text,
        &join_css_rule_blocks(&view.style_text, &nested),
    )
}

fn raw_page_rule_text(selector: &str, block: &str) -> String {
    let selector = selector.trim();
    let block = block.trim();
    match (selector.is_empty(), block.is_empty()) {
        (true, true) => "@page { }".to_owned(),
        (false, true) => format!("@page {selector} {{ }}"),
        (true, false) => format!("@page {{ {block} }}"),
        (false, false) => format!("@page {selector} {{ {block} }}"),
    }
}

pub(crate) fn css_import_rule_view_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssImportRuleView> {
    if let Some(view) = css_rule_live_stylesheet_import_rule_read(scope, object) {
        return Some(view);
    }
    css_rule_detached_snapshot_typed_view(scope, object, parse_import_rule_view_with_stylo)
}

pub(crate) fn css_namespace_rule_view_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssNamespaceRuleView> {
    if let Some(view) = css_rule_live_stylesheet_namespace_rule_read(scope, object) {
        return Some(view);
    }
    css_rule_detached_snapshot_typed_view(scope, object, parse_namespace_rule_view_with_stylo)
}

pub(crate) fn build_css_style_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: CssStyleRuleTextParts,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    let style_text = parsed.style_text.clone();
    let rule = bind_css_style_rule_object(scope, parsed, parent_style_sheet, parent_rule);
    seed_css_rule_stylo_declaration_block_from_style_text(
        scope,
        rule,
        &style_text,
        CssRulePdbDeclarationKind::StyleRule,
    );
    rule
}

pub(crate) fn build_css_style_rule_object_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: CssStyleRuleTextParts,
    declaration_text: &str,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    let rule = bind_css_style_rule_object(scope, parsed, parent_style_sheet, parent_rule);
    seed_css_rule_stylo_declaration_block_from_declaration_text(
        scope,
        rule,
        declaration_text,
        CssRulePdbDeclarationKind::StyleRule,
    );
    rule
}

fn bind_css_style_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: CssStyleRuleTextParts,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    CssStyleRuleDeclaration {
        css_text: parsed.css_text,
        selector_text: parsed.selector_text,
        style_text: parsed.style_text,
        parent_rule,
        parent_style_sheet,
    }
    .bind(scope)
    .expect("CSSStyleRule declaration should bind")
}

pub(crate) fn build_css_nested_declarations_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style_text: &str,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let rule = CssNestedDeclarationsRuleDeclaration {
        css_text: style_text.to_owned(),
        style_text: style_text.to_owned(),
        parent_rule,
        parent_style_sheet,
    }
    .bind(scope)
    .expect("CSSNestedDeclarations declaration should bind");
    seed_css_rule_stylo_declaration_block_from_style_text(
        scope,
        rule,
        style_text,
        CssRulePdbDeclarationKind::NestedDeclarations,
    );
    rule
}

pub(crate) fn css_style_rule_current_has_nested_rules<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    local_style_text: &str,
) -> bool {
    if let Some(has_child_rules) = css_rule_attached_native_style_has_child_rules(scope, rule) {
        return has_child_rules;
    }
    local_nested_style_block_text_contains_rules(local_style_text)
}

pub(crate) fn css_style_rule_first_declaration_text_with_stylo(style_text: &str) -> Option<String> {
    css_nested_rule_block_with_selector_context(
        &CssomSelectorNamespaceContext::default(),
        style_text,
        CssRuleType::Style,
        CssRuleType::Style.bit(),
        StyleRuleSelectorContext::Nested,
        true,
    )
    .and_then(|mutation| mutation.first_declaration_text)
}

pub(crate) fn css_style_rule_first_declaration_text_with_stylo_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_text: &str,
) -> Option<String> {
    let parent_style_sheet = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(rule));
    css_nested_rule_block_with_stylo_context(
        scope,
        parent_style_sheet,
        rule,
        style_text,
        CssRuleType::Style,
        style_rule_context,
        true,
    )
    .and_then(|mutation| mutation.first_declaration_text)
}

pub(crate) fn css_style_rule_style_text_has_nested_rules_with_stylo_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_text: &str,
) -> Option<bool> {
    let rule_snapshots = css_style_rule_block_with_stylo_context(scope, rule, style_text)?.rules;
    Some(css_rule_snapshots_contain_nested_rules(&rule_snapshots))
}

pub(crate) fn css_style_rule_block_with_stylo_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_text: &str,
) -> Option<CssDetachedRuleListMutation> {
    let parent_style_sheet = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(rule));
    css_nested_rule_block_with_stylo_context(
        scope,
        parent_style_sheet,
        rule,
        style_text,
        CssRuleType::Style,
        style_rule_context,
        true,
    )
}

pub(crate) fn set_font_feature_values_map_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    entries: &[CssFontFeatureValueEntryView],
) {
    let backing = v8::Map::new(scope);
    let Some(map) = new_css_font_feature_values_map(scope, backing) else {
        return;
    };
    set_font_feature_values_map_metadata(scope, map, object, slot);
    for entry in entries {
        let Some(key) = v8_string(scope, &entry.name) else {
            continue;
        };
        let value = font_feature_values_u32_array(scope, &entry.values);
        let _ = backing.set(scope, key.into(), value.into());
    }
    set_private_value(scope, object, slot, map.into());
}

pub(crate) fn sync_font_feature_values_map_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    entries: &[CssFontFeatureValueEntryView],
) {
    let Some(map) = get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        set_font_feature_values_map_slot(scope, object, slot, entries);
        return;
    };
    let Some(backing) = css_font_feature_values_map_backing(scope, map) else {
        set_font_feature_values_map_slot(scope, object, slot, entries);
        return;
    };
    backing.clear();
    set_font_feature_values_map_metadata(scope, map, object, slot);
    for entry in entries {
        let Some(key) = v8_string(scope, &entry.name) else {
            continue;
        };
        let value = font_feature_values_u32_array(scope, &entry.values);
        let _ = backing.set(scope, key.into(), value.into());
    }
}

pub(crate) fn set_font_feature_values_map_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Object>,
    rule: v8::Local<'s, v8::Object>,
    slot: &'static str,
) {
    set_private_value(
        scope,
        map,
        CSS_FONT_FEATURE_VALUES_MAP_OWNER_RULE_SLOT,
        rule.into(),
    );
    set_private_string(scope, map, CSS_FONT_FEATURE_VALUES_MAP_GROUP_SLOT, slot);
}

pub(crate) fn font_feature_values_u32_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[u32],
) -> v8::Local<'s, v8::Array> {
    serialize_v8_array(scope, values).unwrap_or_else(|| v8::Array::new(scope, 0))
}

pub(crate) fn normalized_font_feature_values_map_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Array>> {
    if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
        let mut normalized = Vec::with_capacity(array.length() as usize);
        for index in 0..array.length() {
            let value = array.get_index(scope, index)?.uint32_value(scope)?;
            normalized.push(value);
        }
        return serialize_v8_array(scope, normalized.as_slice());
    }
    let value = value.uint32_value(scope)?;
    serialize_v8_array(scope, [value])
}

pub(crate) fn font_feature_values_map_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: v8::Local<'s, v8::Array>,
) -> Option<Vec<u32>> {
    let mut normalized = Vec::with_capacity(values.length() as usize);
    for index in 0..values.length() {
        normalized.push(values.get_index(scope, index)?.uint32_value(scope)?);
    }
    Some(normalized)
}

pub(crate) fn font_feature_values_map_group(
    slot: &str,
) -> Option<crate::live_stylesheet::FontFeatureValuesMapGroup> {
    use crate::live_stylesheet::FontFeatureValuesMapGroup;

    match slot {
        CSS_FONT_FEATURE_VALUES_RULE_ANNOTATION_SLOT => Some(FontFeatureValuesMapGroup::Annotation),
        CSS_FONT_FEATURE_VALUES_RULE_ORNAMENTS_SLOT => Some(FontFeatureValuesMapGroup::Ornaments),
        CSS_FONT_FEATURE_VALUES_RULE_STYLISTIC_SLOT => Some(FontFeatureValuesMapGroup::Stylistic),
        CSS_FONT_FEATURE_VALUES_RULE_STYLESET_SLOT => Some(FontFeatureValuesMapGroup::Styleset),
        CSS_FONT_FEATURE_VALUES_RULE_CHARACTER_VARIANT_SLOT => {
            Some(FontFeatureValuesMapGroup::CharacterVariant)
        }
        CSS_FONT_FEATURE_VALUES_RULE_SWASH_SLOT => Some(FontFeatureValuesMapGroup::Swash),
        _ => None,
    }
}

pub(crate) fn style_rule_text_from_snapshot(
    snapshot: &CssRuleSnapshot,
    selector_context: &CssomSelectorNamespaceContext,
    rule_context: StyleRuleSelectorContext,
) -> Option<CssStyleRuleTextParts> {
    if snapshot.rule_type != CssRuleType::Style {
        return None;
    }
    let selector_text = canonicalize_cssom_style_rule_selector_text(
        snapshot.selector_text.as_deref()?,
        &selector_context.style_rule_namespace_context(),
        rule_context,
    )
    .ok()?;
    let style_text = style_rule_snapshot_style_text(snapshot)?;
    let css_text = if snapshot.child_rules.is_empty() {
        serialize_style_rule_css_text_with_context(&selector_text, &style_text, selector_context)
    } else {
        serialize_nested_style_rule_css_text_from_block(&selector_text, &style_text)
    };
    Some(CssStyleRuleTextParts {
        css_text,
        selector_text,
        style_text,
    })
}

pub(crate) fn style_rule_snapshot_style_text(snapshot: &CssRuleSnapshot) -> Option<String> {
    let declaration_text = snapshot.declaration_text.as_deref()?.trim();
    Some(style_rule_snapshot_style_text_from_parts(
        declaration_text,
        &snapshot.child_rules,
    ))
}

pub(crate) fn style_rule_snapshot_style_text_from_parts(
    declaration_text: &str,
    child_rules: &[CssRuleSnapshot],
) -> String {
    let declaration_text = declaration_text.trim();
    if child_rules.is_empty() {
        return declaration_text.to_owned();
    }
    let child_rule_text = child_rules
        .iter()
        .map(|rule| rule.css_text.trim())
        .filter(|css_text| !css_text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    match (declaration_text, child_rule_text.trim()) {
        ("", "") => String::new(),
        (declarations, "") => declarations.to_owned(),
        ("", children) => children.to_owned(),
        (declarations, children) => format!("{declarations}\n{children}"),
    }
}

pub(crate) fn keyframe_rule_text_from_snapshot(
    snapshot: &CssRuleSnapshot,
) -> Option<CssStyleRuleTextParts> {
    if snapshot.rule_type != CssRuleType::Keyframe {
        return None;
    }
    let selector_text =
        normalize_keyframe_selector_text_with_stylo(snapshot.selector_text.as_deref()?)?;
    let style_text = snapshot.declaration_text.clone()?;
    Some(CssStyleRuleTextParts {
        css_text: snapshot.css_text.clone(),
        selector_text,
        style_text,
    })
}

pub(crate) fn css_style_rule_serializable_declaration_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    stored_style: &str,
) -> String {
    if css_style_rule_current_has_nested_rules(scope, rule, stored_style) {
        return stored_style.to_owned();
    }
    css_rule_stylo_declaration_block_css_text(scope, rule)
        .unwrap_or_else(|| stored_style.to_owned())
}

pub(crate) fn nested_style_rule_block_text_if_has_rules(
    style_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
) -> Option<String> {
    let mutation = nested_style_rule_block_with_selector_context(
        style_text,
        selector_context,
        StyleRuleSelectorContext::Nested,
    )?;
    css_rule_snapshots_contain_nested_rules(&mutation.rules)
        .then(|| nested_rule_block_text_from_stylo_mutation(mutation))
}

pub(crate) fn sync_css_style_rule_from_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    if apply_live_stylesheet_rule_declaration_block_mutation(
        scope,
        rule,
        CssRulePdbDeclarationKind::StyleRule,
    ) {
        return;
    }
    let style_text = lightweight_css_style_object_css_text(scope, style);
    if apply_live_stylesheet_rule_declaration_text_mutation(
        scope,
        rule,
        CssRulePdbDeclarationKind::StyleRule,
        &style_text,
    ) {
        return;
    }
    let rules = css_style_rule_rules_array(scope, rule);
    let nested_rules = css_rule_list_current_css_text(scope, rules);
    let style_text = join_css_rule_blocks(&style_text, &nested_rules);
    set_private_string(scope, rule, CSS_STYLE_RULE_STYLE_TEXT_SLOT, &style_text);
    sync_css_style_rule_css_text_from_parts(scope, rule);
    sync_parent_rule_from_child_change(scope, rule);
}

pub(crate) fn sync_css_nested_declarations_from_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    if apply_live_stylesheet_rule_declaration_block_mutation(
        scope,
        rule,
        CssRulePdbDeclarationKind::NestedDeclarations,
    ) {
        return;
    }
    let style_text = lightweight_css_style_object_css_text(scope, style);
    if apply_live_stylesheet_rule_declaration_text_mutation(
        scope,
        rule,
        CssRulePdbDeclarationKind::NestedDeclarations,
        &style_text,
    ) {
        return;
    }
    let old_style_text = private_string(scope, rule, CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT);
    let css_text = css_rule_stylo_declaration_block_css_text(scope, rule)
        .unwrap_or_else(|| style_text.clone());
    set_detached_css_rule_snapshot_text(scope, rule, &css_text);
    set_private_string(
        scope,
        rule,
        CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT,
        &style_text,
    );
    let Some(parent) = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(parent_style_text_slot) = nested_declarations_parent_style_text_slot(scope, parent)
    else {
        return;
    };
    if sync_css_nested_declarations_parent_from_existing_rule_list(
        scope,
        rule,
        parent,
        parent_style_text_slot,
        &old_style_text,
    ) {
        return;
    }
    let Some(next_style_text) = replaced_nested_declarations_parent_style_text_with_stylo(
        scope,
        parent,
        parent_style_text_slot,
        old_style_text.as_str(),
        style_text.as_str(),
    ) else {
        return;
    };
    set_private_string(scope, parent, parent_style_text_slot, &next_style_text);
    if parent_style_text_slot == CSS_STYLE_RULE_STYLE_TEXT_SLOT {
        sync_css_style_rule_css_text_from_parts(scope, parent);
        sync_parent_rule_from_child_change(scope, parent);
    } else if let Some(rules) = get_private_value(scope, parent, CSS_AT_RULE_NESTED_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        sync_css_grouping_rule_css_text_from_rules(scope, parent, rules);
    }
}

pub(crate) fn replaced_nested_declarations_parent_style_text_with_stylo<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    parent_style_text_slot: &'static str,
    old_style_text: &str,
    style_text: &str,
) -> Option<String> {
    let parent_style_text = private_string(scope, parent, parent_style_text_slot);
    let mutation = parent_nested_rule_block_with_stylo_context(
        scope,
        parent,
        parent_style_text_slot,
        &parent_style_text,
    )?;
    Some(replaced_nested_declarations_in_stylo_block(
        mutation,
        old_style_text,
        style_text,
    ))
}

pub(crate) fn replaced_nested_declarations_in_stylo_block(
    mutation: CssDetachedRuleListMutation,
    old_style_text: &str,
    style_text: &str,
) -> String {
    let mut parts = Vec::new();
    let mut replaced = false;
    if let Some(first_declarations) = mutation.first_declaration_text {
        push_replaced_nested_declarations_part(
            &mut parts,
            &mut replaced,
            first_declarations.trim(),
            old_style_text,
            style_text,
        );
    }
    for rule in mutation.rules {
        push_replaced_nested_declarations_part(
            &mut parts,
            &mut replaced,
            rule.css_text.trim(),
            old_style_text,
            style_text,
        );
    }
    if !replaced && !style_text.is_empty() {
        parts.push(style_text.to_owned());
    }
    parts.join(" ")
}

pub(crate) fn push_replaced_nested_declarations_part(
    parts: &mut Vec<String>,
    replaced: &mut bool,
    item_text: &str,
    old_style_text: &str,
    style_text: &str,
) {
    if item_text.is_empty() {
        return;
    }
    if !*replaced && item_text == old_style_text {
        *replaced = true;
        if !style_text.is_empty() {
            parts.push(style_text.to_owned());
        }
    } else {
        parts.push(item_text.to_owned());
    }
}

pub(crate) fn sync_css_nested_declarations_parent_from_existing_rule_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parent: v8::Local<'s, v8::Object>,
    parent_style_text_slot: &'static str,
    old_style_text: &str,
) -> bool {
    if parent_style_text_slot == CSS_STYLE_RULE_STYLE_TEXT_SLOT {
        let rules = css_style_rule_rules_array(scope, parent);
        let _ = replace_css_rule_list_nested_declarations_rule(scope, rules, rule, old_style_text);
        sync_css_style_rule_style_text_from_nested_rules(scope, parent, rules);
        return true;
    }
    if parent_style_text_slot == CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT {
        let Some(rules) = get_private_value(scope, parent, CSS_AT_RULE_NESTED_RULES_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            return false;
        };
        let _ = replace_css_rule_list_nested_declarations_rule(scope, rules, rule, old_style_text);
        sync_css_grouping_rule_css_text_from_rules(scope, parent, rules);
        return true;
    }
    false
}

pub(crate) fn replace_css_rule_list_nested_declarations_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    rule: v8::Local<'s, v8::Object>,
    old_style_text: &str,
) -> bool {
    for (index, candidate) in css_rule_list_materialized_entries(scope, rules) {
        if candidate.strict_equals(rule.into()) {
            return true;
        }
        if get_private_value(scope, candidate, CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT).is_some()
            && private_string(scope, candidate, CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT)
                == old_style_text
        {
            set_css_rule_list_materialized_rule(scope, rules, index, rule);
            return true;
        }
    }
    false
}

pub(crate) fn nested_declarations_parent_style_text_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
) -> Option<&'static str> {
    if get_private_value(scope, parent, CSS_STYLE_RULE_STYLE_TEXT_SLOT).is_some() {
        return Some(CSS_STYLE_RULE_STYLE_TEXT_SLOT);
    }
    if get_private_value(scope, parent, CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT).is_some() {
        return Some(CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT);
    }
    None
}

pub(crate) fn sync_css_style_rule_style_text_from_nested_rules<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
) {
    let leading_declarations = css_rule_stylo_declaration_block_css_text(scope, rule)
        .or_else(|| {
            let style_text = private_string(scope, rule, CSS_STYLE_RULE_STYLE_TEXT_SLOT);
            css_style_rule_first_declaration_text_with_stylo_context(scope, rule, &style_text)
        })
        .unwrap_or_default();
    let nested_rule_texts = css_rule_list_current_css_texts(scope, rules);
    let nested_rules = nested_rule_texts.join(" ");
    let style_text = join_css_rule_blocks(&leading_declarations, &nested_rules);
    set_private_string(scope, rule, CSS_STYLE_RULE_STYLE_TEXT_SLOT, &style_text);
    let Some(selector) = css_style_rule_current_selector(scope, rule) else {
        return;
    };
    let nested_block = nested_rule_texts.join("\n");
    let block = match (leading_declarations.trim(), nested_block.trim()) {
        ("", "") => String::new(),
        (leading, "") => leading.to_owned(),
        ("", nested) => nested.to_owned(),
        (leading, nested) => format!("{leading}\n{nested}"),
    };
    let css_text = serialize_nested_style_rule_css_text_from_block(&selector, &block);
    set_detached_css_rule_snapshot_text(scope, rule, &css_text);
    sync_parent_rule_from_child_change(scope, rule);
}

pub(crate) fn sync_css_keyframe_rule_from_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    if apply_live_stylesheet_rule_declaration_block_mutation(
        scope,
        rule,
        CssRulePdbDeclarationKind::KeyframeRule,
    ) {
        return;
    }
    let style_text = lightweight_css_style_object_css_text(scope, style);
    if apply_live_stylesheet_rule_declaration_text_mutation(
        scope,
        rule,
        CssRulePdbDeclarationKind::KeyframeRule,
        &style_text,
    ) {
        return;
    }
    set_private_string(scope, rule, CSS_KEYFRAME_RULE_STYLE_TEXT_SLOT, &style_text);
    sync_css_keyframe_rule_css_text_from_parts(scope, rule);
    sync_parent_rule_from_child_change(scope, rule);
}

pub(crate) fn css_style_rule_style_changed_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(rule) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    sync_css_style_rule_from_style_object(scope, rule, args.this());
}

pub(crate) fn css_nested_declarations_style_changed_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(rule) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    sync_css_nested_declarations_from_style_object(scope, rule, args.this());
}

pub(crate) fn css_keyframe_rule_style_changed_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(rule) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    sync_css_keyframe_rule_from_style_object(scope, rule, args.this());
}

pub(crate) fn sync_css_keyframe_rule_state_from_parsed_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parsed: &CssStyleRuleTextParts,
) {
    set_private_string(
        scope,
        rule,
        CSS_KEYFRAME_RULE_KEY_TEXT_SLOT,
        &parsed.selector_text,
    );
    set_private_string(
        scope,
        rule,
        CSS_KEYFRAME_RULE_STYLE_TEXT_SLOT,
        &parsed.style_text,
    );
    seed_css_rule_stylo_declaration_block_from_style_text(
        scope,
        rule,
        &parsed.style_text,
        CssRulePdbDeclarationKind::KeyframeRule,
    );
    if let Some(style) = get_private_value(scope, rule, CSS_KEYFRAME_RULE_STYLE_OBJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        attach_css_rule_stylo_declaration_block_to_style(scope, rule, style);
        set_style_object_css_text_without_notify(scope, style, &parsed.style_text);
        sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    }
}

pub(crate) fn sync_css_style_rule_state_from_parsed_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parsed: &CssStyleRuleTextParts,
) {
    set_private_string(
        scope,
        rule,
        CSS_STYLE_RULE_SELECTOR_TEXT_SLOT,
        &parsed.selector_text,
    );
    set_private_string(
        scope,
        rule,
        CSS_STYLE_RULE_STYLE_TEXT_SLOT,
        &parsed.style_text,
    );
    seed_css_rule_stylo_declaration_block_from_style_text(
        scope,
        rule,
        &parsed.style_text,
        CssRulePdbDeclarationKind::StyleRule,
    );
    if let Some(style) = get_private_value(scope, rule, CSS_STYLE_RULE_STYLE_OBJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        attach_css_rule_stylo_declaration_block_to_style(scope, rule, style);
        set_style_object_css_text_without_notify(scope, style, &parsed.style_text);
        sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    }
}

pub(crate) fn css_font_feature_values_rule_font_family_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let font_family = css_font_feature_values_rule_view_from_object(scope, args.this())
        .map(|view| view.font_family)
        .unwrap_or_else(|| {
            private_string(
                scope,
                args.this(),
                CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT,
            )
        });
    rv.set(v8_dynamic_string_value(scope, &font_family));
}

pub(crate) fn css_font_feature_values_rule_font_family_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(font_family) = cssom_dom_string_property_value(
        scope,
        args.get(0),
        "CSSFontFeatureValuesRule",
        "fontFamily",
    ) else {
        return;
    };
    let (font_family, native_families) =
        normalize_cssom_font_feature_values_families(&font_family).into_parts();
    set_private_string(
        scope,
        args.this(),
        CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT,
        &font_family,
    );
    if apply_live_stylesheet_font_feature_values_rule_font_family_mutation(
        scope,
        args.this(),
        native_families,
    ) {
        rv.set_undefined();
        return;
    }
    if let Some(css_text) =
        css_font_feature_values_rule_css_text_with_font_family(scope, args.this(), &font_family)
        && !commit_detached_css_rule_snapshot_text(scope, args.this(), &css_text, false)
    {
        sync_css_font_feature_values_rule_slots_from_current_rule(scope, args.this());
    }
    rv.set_undefined();
}

pub(crate) fn css_font_feature_values_rule_annotation_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_font_feature_values_rule_map_getter(
        scope,
        args,
        rv,
        CSS_FONT_FEATURE_VALUES_RULE_ANNOTATION_SLOT,
    );
}

pub(crate) fn css_font_feature_values_rule_ornaments_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_font_feature_values_rule_map_getter(
        scope,
        args,
        rv,
        CSS_FONT_FEATURE_VALUES_RULE_ORNAMENTS_SLOT,
    );
}

pub(crate) fn css_font_feature_values_rule_stylistic_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_font_feature_values_rule_map_getter(
        scope,
        args,
        rv,
        CSS_FONT_FEATURE_VALUES_RULE_STYLISTIC_SLOT,
    );
}

pub(crate) fn css_font_feature_values_rule_styleset_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_font_feature_values_rule_map_getter(
        scope,
        args,
        rv,
        CSS_FONT_FEATURE_VALUES_RULE_STYLESET_SLOT,
    );
}

pub(crate) fn css_font_feature_values_rule_character_variant_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_font_feature_values_rule_map_getter(
        scope,
        args,
        rv,
        CSS_FONT_FEATURE_VALUES_RULE_CHARACTER_VARIANT_SLOT,
    );
}

pub(crate) fn css_font_feature_values_rule_swash_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_font_feature_values_rule_map_getter(
        scope,
        args,
        rv,
        CSS_FONT_FEATURE_VALUES_RULE_SWASH_SLOT,
    );
}

pub(crate) fn css_font_feature_values_rule_map_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    slot: &'static str,
) {
    if let Some(map) = get_private_value(scope, args.this(), slot) {
        rv.set(map);
        return;
    }
    if let Some(view) = css_font_feature_values_rule_view_from_object(scope, args.this()) {
        sync_css_font_feature_values_rule_slots_from_stylo_view(scope, args.this(), &view);
        if let Some(map) = get_private_value(scope, args.this(), slot) {
            rv.set(map);
            return;
        }
    }
    let Some(map) = new_css_font_feature_values_map(scope, v8::Map::new(scope)) else {
        rv.set_undefined();
        return;
    };
    set_font_feature_values_map_metadata(scope, map, args.this(), slot);
    set_private_value(scope, args.this(), slot, map.into());
    rv.set(map.into());
}

pub(crate) fn css_property_rule_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let name = css_property_rule_view_from_object(scope, args.this())
        .map(|view| view.name)
        .unwrap_or_else(|| private_string(scope, args.this(), CSS_PROPERTY_RULE_NAME_SLOT));
    rv.set(v8_dynamic_string_value(scope, &name));
}

pub(crate) fn css_property_rule_syntax_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let syntax = css_property_rule_view_from_object(scope, args.this())
        .map(|view| view.syntax)
        .unwrap_or_else(|| private_string(scope, args.this(), CSS_PROPERTY_RULE_SYNTAX_SLOT));
    rv.set(v8_dynamic_string_value(scope, &syntax));
}

pub(crate) fn css_property_rule_inherits_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let inherits = css_property_rule_view_from_object(scope, args.this())
        .map(|view| view.inherits)
        .unwrap_or_else(|| {
            get_private_value(scope, args.this(), CSS_PROPERTY_RULE_INHERITS_SLOT)
                .is_some_and(|value| value.boolean_value(scope))
        });
    rv.set(v8::Boolean::new(scope, inherits).into());
}

pub(crate) fn css_property_rule_initial_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(view) = css_property_rule_view_from_object(scope, args.this()) {
        if let Some(initial_value) = view.initial_value {
            rv.set(v8_dynamic_string_value(scope, &initial_value));
        } else {
            rv.set(v8::null(scope).into());
        }
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), CSS_PROPERTY_RULE_INITIAL_VALUE_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(crate) fn css_font_feature_values_map_set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(backing) = require_css_font_feature_values_map_backing(scope, args.this(), "set")
    else {
        return;
    };
    let Some((name, key)) = css_font_feature_values_map_key(scope, &args, "set") else {
        return;
    };
    let Some(value) = normalized_font_feature_values_map_value(scope, args.get(1)) else {
        return;
    };
    let values = font_feature_values_map_values(scope, value);
    if backing.set(scope, key.into(), value.into()).is_none() {
        return;
    }
    let Some(values) = values else {
        rv.set_undefined();
        return;
    };
    let Some((rule, slot, group)) = css_font_feature_values_map_owner(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    if !font_feature_values_map_values_are_supported(&slot, &values) {
        rv.set_undefined();
        return;
    }
    if apply_live_stylesheet_font_feature_values_rule_map_entry_mutation(
        scope, rule, group, &name, &values,
    ) {
        rv.set_undefined();
        return;
    }
    if let Some(css_text) =
        css_font_feature_values_rule_css_text_with_map_entry(scope, rule, &slot, &name, values)
        && !commit_detached_css_rule_snapshot_text(scope, rule, &css_text, false)
    {
        sync_css_font_feature_values_rule_slots_from_current_rule(scope, rule);
    }
    rv.set_undefined();
}

fn css_font_feature_values_map_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Object>,
) -> Option<(
    v8::Local<'s, v8::Object>,
    String,
    crate::live_stylesheet::FontFeatureValuesMapGroup,
)> {
    let rule = get_private_value(scope, map, CSS_FONT_FEATURE_VALUES_MAP_OWNER_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let slot = private_string(scope, map, CSS_FONT_FEATURE_VALUES_MAP_GROUP_SLOT);
    let group = font_feature_values_map_group(&slot)?;
    Some((rule, slot, group))
}

fn require_css_font_feature_values_map_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<v8::Local<'s, v8::Map>> {
    css_font_feature_values_map_backing(scope, receiver).or_else(|| {
        throw_type_error(
            scope,
            &format!(
                "Failed to execute '{member}' on 'CSSFontFeatureValuesMap': Illegal invocation."
            ),
        );
        None
    })
}

fn css_font_feature_values_map_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    member: &'static str,
) -> Option<(String, v8::Local<'s, v8::String>)> {
    let name =
        cssom_dom_string_property_value(scope, args.get(0), "CSSFontFeatureValuesMap", member)?;
    let key = v8_string(scope, &name)?;
    Some((name, key))
}

pub(crate) fn css_font_feature_values_map_size_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_css_font_feature_values_map_backing(scope, args.this(), "get size")
    else {
        return;
    };
    rv.set_uint32(backing.size().min(u32::MAX as usize) as u32);
}

pub(crate) fn css_font_feature_values_map_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_css_font_feature_values_map_backing(scope, args.this(), "get")
    else {
        return;
    };
    let Some((_, key)) = css_font_feature_values_map_key(scope, &args, "get") else {
        return;
    };
    rv.set(
        backing
            .get(scope, key.into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub(crate) fn css_font_feature_values_map_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_css_font_feature_values_map_backing(scope, args.this(), "has")
    else {
        return;
    };
    let Some((_, key)) = css_font_feature_values_map_key(scope, &args, "has") else {
        return;
    };
    rv.set_bool(backing.has(scope, key.into()).unwrap_or(false));
}

pub(crate) fn css_font_feature_values_map_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_css_font_feature_values_map_backing(scope, args.this(), "delete")
    else {
        return;
    };
    let Some((name, key)) = css_font_feature_values_map_key(scope, &args, "delete") else {
        return;
    };
    let deleted = backing.delete(scope, key.into()).unwrap_or(false);
    if !deleted {
        rv.set_bool(false);
        return;
    }
    let Some((rule, slot, group)) = css_font_feature_values_map_owner(scope, args.this()) else {
        rv.set_bool(true);
        return;
    };
    if apply_live_stylesheet_font_feature_values_rule_map_entry_delete(scope, rule, group, &name) {
        rv.set_bool(true);
        return;
    }
    if let Some(css_text) =
        css_font_feature_values_rule_css_text_without_map_entry(scope, rule, &slot, &name)
        && commit_detached_css_rule_snapshot_text(scope, rule, &css_text, false)
    {
        rv.set_bool(true);
        return;
    }
    sync_css_font_feature_values_rule_slots_from_current_rule(scope, rule);
    rv.set_bool(false);
}

pub(crate) fn css_font_feature_values_map_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_css_font_feature_values_map_backing(scope, args.this(), "clear")
    else {
        return;
    };
    backing.clear();
    let Some((rule, slot, group)) = css_font_feature_values_map_owner(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    if apply_live_stylesheet_font_feature_values_rule_map_clear(scope, rule, group) {
        rv.set_undefined();
        return;
    }
    if let Some(css_text) =
        css_font_feature_values_rule_css_text_with_cleared_map(scope, rule, &slot)
        && commit_detached_css_rule_snapshot_text(scope, rule, &css_text, false)
    {
        rv.set_undefined();
        return;
    }
    sync_css_font_feature_values_rule_slots_from_current_rule(scope, rule);
    rv.set_undefined();
}

pub(crate) fn css_font_feature_values_map_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_css_font_feature_values_map_iterator(
        scope,
        args.this(),
        "entries",
        MaplikeWebIdlIteratorMethod::Entries,
        &mut rv,
    );
}

pub(crate) fn css_font_feature_values_map_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_css_font_feature_values_map_iterator(
        scope,
        args.this(),
        "keys",
        MaplikeWebIdlIteratorMethod::Keys,
        &mut rv,
    );
}

pub(crate) fn css_font_feature_values_map_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_css_font_feature_values_map_iterator(
        scope,
        args.this(),
        "values",
        MaplikeWebIdlIteratorMethod::Values,
        &mut rv,
    );
}

fn set_css_font_feature_values_map_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
    method: MaplikeWebIdlIteratorMethod,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_css_font_feature_values_map_backing(scope, receiver, member) else {
        return;
    };
    if let Some(iterator) = new_maplike_webidl_iterator(scope, backing, method) {
        rv.set(iterator.into());
    } else {
        rv.set_undefined();
    }
}

pub(crate) fn css_font_feature_values_map_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_css_font_feature_values_map_backing(scope, args.this(), "forEach")
    else {
        return;
    };
    if let Some(result) = call_maplike_webidl_for_each(
        scope,
        backing,
        args.this(),
        args.get(0),
        args.get(1),
        "CSSFontFeatureValuesMap forEach",
    ) {
        rv.set(result);
    }
}

pub(crate) fn find_css_keyframe_rule_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<u32> {
    if let Some(parent_style_sheet) =
        get_private_object(scope, parent_rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        && let Some(parent_path) =
            css_rule_attached_native_path(scope, parent_rule, parent_style_sheet)
        && let Some(stylesheet) = css_style_sheet_live_stylesheet(scope, parent_style_sheet)
    {
        return stylesheet
            .find_keyframe_rule_index(&parent_path, key)
            .and_then(|index| u32::try_from(index).ok());
    }

    let normalized_key = normalize_keyframe_selector_text_with_stylo(key)?;
    (0..css_rule_list_length(scope, rules))
        .rev()
        .find(|&index| {
            let existing_key = css_rule_list_materialized_rule(scope, rules, index)
                .map(|rule| private_string(scope, rule, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT))
                .or_else(|| {
                    css_rule_list_detached_snapshot_at(scope, rules, index)
                        .and_then(|entry| entry.snapshot.selector_text)
                });
            existing_key.is_some_and(|existing_key| {
                normalize_keyframe_selector_text_with_stylo(&existing_key)
                    .is_some_and(|existing_key| existing_key == normalized_key)
            })
        })
}

pub(crate) fn css_import_rule_href_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSImportRule", "href") {
        return;
    }
    let href = css_import_rule_view_from_object(scope, args.this())
        .map(|view| view.href)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &href));
}

pub(crate) fn css_import_rule_media_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSImportRule", "media") {
        return;
    }
    if let Some(list) = get_private_value(scope, args.this(), CSS_IMPORT_RULE_MEDIA_LIST_SLOT) {
        rv.set(list);
        return;
    }
    let media = css_import_rule_view_from_object(scope, args.this())
        .map(|view| view.media_text)
        .unwrap_or_default();
    let list =
        build_media_list_object(scope, args.this(), &media, CSS_MEDIA_LIST_OWNER_IMPORT_RULE);
    set_private_value(
        scope,
        args.this(),
        CSS_IMPORT_RULE_MEDIA_LIST_SLOT,
        list.into(),
    );
    rv.set(list.into());
}

pub(crate) fn css_import_rule_media_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSImportRule", "media") {
        return;
    }
    let Some(media_text) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSImportRule", "media")
    else {
        return;
    };
    sync_import_rule_media_text(scope, args.this(), &media_text);
}

pub(crate) fn css_import_rule_layer_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSImportRule", "layerName") {
        return;
    }
    let Some(layer_name) =
        css_import_rule_view_from_object(scope, args.this()).and_then(|view| view.layer_name)
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    rv.set(v8_dynamic_string_value(scope, &layer_name));
}

pub(crate) fn css_import_rule_supports_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSImportRule", "supportsText") {
        return;
    }
    let supports_text = css_import_rule_view_from_object(scope, args.this())
        .and_then(|view| view.supports_text)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &supports_text));
}

pub(crate) fn css_font_face_rule_style_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_type_object(
        scope,
        args.this(),
        "CSSFontFaceRule",
        "style",
        CSS_RULE_FONT_FACE_RULE_TYPE,
    ) {
        return;
    }
    let style = css_font_face_rule_style_object(scope, args.this());
    rv.set(style.into());
}

pub(crate) fn css_font_face_rule_style_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_descriptor_rule_style_setter_callback(
        scope,
        args,
        rv,
        "CSSFontFaceRule",
        CSS_RULE_FONT_FACE_RULE_TYPE,
    );
}

pub(crate) fn css_page_rule_style_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_type_object(
        scope,
        args.this(),
        "CSSPageRule",
        "style",
        CSS_RULE_PAGE_RULE_TYPE,
    ) {
        return;
    }
    let style = css_page_rule_style_object(scope, args.this());
    rv.set(style.into());
}

pub(crate) fn css_page_rule_style_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_descriptor_rule_style_setter_callback(
        scope,
        args,
        rv,
        "CSSPageRule",
        CSS_RULE_PAGE_RULE_TYPE,
    );
}

pub(crate) fn css_margin_rule_style_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_type_object(
        scope,
        args.this(),
        "CSSMarginRule",
        "style",
        CSS_RULE_MARGIN_RULE_TYPE,
    ) {
        return;
    }
    let style = css_margin_rule_style_object(scope, args.this());
    rv.set(style.into());
}

pub(crate) fn css_margin_rule_style_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    css_descriptor_rule_style_setter_callback(
        scope,
        args,
        rv,
        "CSSMarginRule",
        CSS_RULE_MARGIN_RULE_TYPE,
    );
}

pub(crate) fn css_font_face_rule_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(style) = get_private_object(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT) {
        return style;
    }
    let style = build_lightweight_css_font_face_descriptors(scope);
    let style_text = css_font_face_rule_style_text_from_object(scope, rule).unwrap_or_default();
    set_style_object_css_text(scope, style, &style_text);
    if let Some(callback) = v8::Function::builder(css_font_face_rule_style_changed_callback)
        .data(rule.into())
        .build(scope)
    {
        set_lightweight_css_style_change_callback(scope, style, callback);
    }
    set_private_value(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT, style.into());
    style
}

pub(crate) fn css_page_rule_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(style) = get_private_object(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT) {
        return style;
    }
    let style = build_lightweight_css_page_descriptors(scope);
    let style_text = css_page_rule_read_from_object(scope, rule)
        .map(|read| read.declaration_text)
        .unwrap_or_default();
    set_style_object_css_text(scope, style, &style_text);
    if let Some(callback) = v8::Function::builder(css_page_rule_style_changed_callback)
        .data(rule.into())
        .build(scope)
    {
        set_lightweight_css_style_change_callback(scope, style, callback);
    }
    set_private_value(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT, style.into());
    style
}

pub(crate) fn css_margin_rule_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSMarginRule", "name") {
        return;
    }
    let name = css_page_margin_rule_view_from_object(scope, args.this())
        .map(|view| view.name)
        .unwrap_or_else(|| private_string(scope, args.this(), CSS_MARGIN_RULE_NAME_SLOT));
    rv.set(v8_dynamic_string_value(scope, &name));
}

pub(crate) fn css_margin_rule_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(style) = get_private_object(scope, rule, CSS_MARGIN_RULE_STYLE_OBJECT_SLOT) {
        return style;
    }
    let style = build_lightweight_css_style_declaration(scope);
    let style_text = if let Some(view) = css_page_margin_rule_view_from_object(scope, rule) {
        sync_css_margin_rule_slots_from_stylo_view(scope, rule, &view);
        view.style_text
    } else {
        private_string(scope, rule, CSS_MARGIN_RULE_STYLE_TEXT_SLOT)
    };
    set_style_object_css_text(scope, style, &style_text);
    if let Some(callback) = v8::Function::builder(css_margin_rule_style_changed_callback)
        .data(rule.into())
        .build(scope)
    {
        set_lightweight_css_style_change_callback(scope, style, callback);
    }
    set_private_value(scope, rule, CSS_MARGIN_RULE_STYLE_OBJECT_SLOT, style.into());
    style
}

pub(crate) fn css_margin_rule_style_changed_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(rule) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let style_text = lightweight_css_style_object_css_text(scope, args.this());
    let margin_name = css_page_margin_rule_view_from_object(scope, rule)
        .map(|view| view.name)
        .unwrap_or_else(|| private_string(scope, rule, CSS_MARGIN_RULE_NAME_SLOT));
    let Some(style_text) = parse_page_margin_descriptor_block_with_stylo(&margin_name, &style_text)
    else {
        sync_css_margin_rule_style_wrapper_from_current_rule(scope, rule, args.this());
        return;
    };
    set_private_string(scope, rule, CSS_MARGIN_RULE_STYLE_TEXT_SLOT, &style_text);
    if apply_live_stylesheet_page_margin_rule_descriptor_block_mutation(scope, rule, &style_text) {
        return;
    }
    if css_rule_has_attached_native_binding(scope, rule) {
        sync_css_margin_rule_style_wrapper_from_current_rule(scope, rule, args.this());
        return;
    }
    let Some(css_text) = css_margin_rule_css_text_from_style_text(scope, rule, &style_text) else {
        sync_css_margin_rule_style_wrapper_from_current_rule(scope, rule, args.this());
        return;
    };
    let _ = commit_detached_css_rule_snapshot_text(scope, rule, &css_text, true);
}

pub(crate) fn sync_css_margin_rule_style_wrapper_from_current_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    if let Some(view) = css_page_margin_rule_view_from_object(scope, rule) {
        set_private_string(
            scope,
            rule,
            CSS_MARGIN_RULE_STYLE_TEXT_SLOT,
            &view.style_text,
        );
        set_style_object_css_text_without_notify(scope, style, &view.style_text);
    }
}

pub(crate) fn css_font_face_rule_style_changed_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(rule) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let style_text = lightweight_css_style_object_css_text(scope, args.this());
    let Some(style_text) = parse_font_face_descriptor_block_with_stylo(&style_text) else {
        sync_css_font_face_rule_style_wrapper_from_current_rule(scope, rule, args.this());
        return;
    };
    if apply_live_stylesheet_font_face_rule_descriptor_block_mutation(scope, rule, &style_text) {
        return;
    }
    if css_rule_has_attached_native_binding(scope, rule) {
        sync_css_font_face_rule_style_wrapper_from_current_rule(scope, rule, args.this());
        return;
    }
    let Some(css_text) = css_font_face_rule_css_text_from_style_text(&style_text) else {
        return;
    };
    let _ = commit_detached_css_rule_snapshot_text(scope, rule, &css_text, true);
}

pub(crate) fn sync_css_font_face_rule_style_wrapper_from_current_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    if let Some(style_text) = css_font_face_rule_style_text_from_object(scope, rule) {
        set_style_object_css_text_without_notify(scope, style, &style_text);
    }
}

pub(crate) fn css_page_rule_style_changed_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(rule) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let style_text = lightweight_css_style_object_css_text(scope, args.this());
    let Some(style_text) = parse_page_descriptor_block_with_stylo(&style_text) else {
        sync_css_page_rule_style_wrapper_from_current_rule(scope, rule, args.this());
        return;
    };
    if apply_live_stylesheet_page_rule_descriptor_block_mutation(scope, rule, &style_text) {
        return;
    }
    if css_rule_has_attached_native_binding(scope, rule) {
        sync_css_page_rule_style_wrapper_from_current_rule(scope, rule, args.this());
        return;
    }
    let (selector, _, nested) = css_page_rule_public_mutation_parts(scope, rule);
    let css_text = if let Some(css_text) =
        css_page_rule_css_text_from_parts(&selector, &style_text, &nested)
    {
        css_text
    } else if !css_rule_has_attached_native_binding(scope, rule) {
        let block = join_css_rule_blocks(&style_text, &nested);
        let selector = normalize_page_selector_text_with_stylo(&selector).unwrap_or_default();
        serialize_page_rule_text(&selector, &block)
    } else {
        sync_css_page_rule_style_wrapper_from_current_rule(scope, rule, args.this());
        return;
    };
    let _ = commit_detached_css_rule_snapshot_text(scope, rule, &css_text, true);
}

pub(crate) fn sync_css_page_rule_style_wrapper_from_current_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    if let Some(read) = css_page_rule_read_from_object(scope, rule) {
        set_style_object_css_text_without_notify(scope, style, &read.declaration_text);
    }
}

pub(crate) fn css_keyframe_rule_key_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let key_text = css_rule_attached_native_keyframe_selector_text(scope, args.this())
        .unwrap_or_else(|| private_string(scope, args.this(), CSS_KEYFRAME_RULE_KEY_TEXT_SLOT));
    rv.set(v8_dynamic_string_value(scope, &key_text));
}

pub(crate) fn css_keyframe_rule_key_text_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CssKeyframeRuleKeyTextArgs>(scope, &args) else {
        return;
    };
    let Some(key_text) = normalize_keyframe_selector_text_with_stylo(&parsed.key_text) else {
        rv.set_undefined();
        return;
    };
    if apply_live_stylesheet_keyframe_rule_selector_mutation(scope, args.this(), &key_text) {
        rv.set_undefined();
        return;
    }
    set_private_string(
        scope,
        args.this(),
        CSS_KEYFRAME_RULE_KEY_TEXT_SLOT,
        &key_text,
    );
    sync_css_keyframe_rule_css_text_from_parts(scope, args.this());
    sync_parent_rule_from_child_change(scope, args.this());
    rv.set_undefined();
}

pub(crate) fn css_keyframe_rule_style_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let style = css_keyframe_rule_style_object(scope, args.this());
    rv.set(style.into());
}

pub(crate) fn css_keyframe_rule_style_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(css_text) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSStyleDeclaration", "cssText")
    else {
        return;
    };
    let style = css_keyframe_rule_style_object(scope, args.this());
    set_style_object_css_text(scope, style, &css_text);
}

pub(crate) fn css_keyframe_rule_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(style) = get_private_object(scope, rule, CSS_KEYFRAME_RULE_STYLE_OBJECT_SLOT) {
        return style;
    }
    let style = build_lightweight_css_keyframe_style_declaration(scope);
    let style_text = private_string(scope, rule, CSS_KEYFRAME_RULE_STYLE_TEXT_SLOT);
    attach_css_rule_stylo_declaration_block_to_style(scope, rule, style);
    set_style_object_css_text(scope, style, &style_text);
    sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    if let Some(callback) = v8::Function::builder(css_keyframe_rule_style_changed_callback)
        .data(rule.into())
        .build(scope)
    {
        set_lightweight_css_style_change_callback(scope, style, callback);
    }
    set_private_value(
        scope,
        rule,
        CSS_KEYFRAME_RULE_STYLE_OBJECT_SLOT,
        style.into(),
    );
    style
}

pub(crate) fn css_namespace_rule_namespace_uri_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSNamespaceRule", "namespaceURI") {
        return;
    }
    let namespace_uri = css_namespace_rule_view_from_object(scope, args.this())
        .map(|view| view.namespace_uri)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &namespace_uri));
}

pub(crate) fn css_namespace_rule_prefix_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSNamespaceRule", "prefix") {
        return;
    }
    let prefix = css_namespace_rule_view_from_object(scope, args.this())
        .map(|view| view.prefix)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &prefix));
}

pub(crate) fn css_counter_style_rule_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSCounterStyleRule", "name") {
        return;
    }
    let name = css_counter_style_rule_view_from_object(scope, args.this())
        .map(|view| view.name)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &name));
}

pub(crate) fn css_style_rule_style_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSStyleRule", "style") {
        return;
    }
    let style = css_style_rule_style_object(scope, args.this());
    rv.set(style.into());
}

pub(crate) fn css_style_rule_style_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSStyleRule", "style") {
        return;
    }
    let Some(css_text) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSStyleDeclaration", "cssText")
    else {
        return;
    };
    let style = css_style_rule_style_object(scope, args.this());
    set_style_object_css_text(scope, style, &css_text);
}

pub(crate) fn css_style_rule_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(style) = get_private_object(scope, rule, CSS_STYLE_RULE_STYLE_OBJECT_SLOT) {
        return style;
    }
    let style = build_lightweight_css_rule_style_declaration(scope);
    let style_text = private_string(scope, rule, CSS_STYLE_RULE_STYLE_TEXT_SLOT);
    attach_css_rule_stylo_declaration_block_to_style(scope, rule, style);
    set_style_object_css_text(scope, style, &style_text);
    sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    if let Some(callback) = v8::Function::builder(css_style_rule_style_changed_callback)
        .data(rule.into())
        .build(scope)
    {
        set_lightweight_css_style_change_callback(scope, style, callback);
    }
    set_private_value(scope, rule, CSS_STYLE_RULE_STYLE_OBJECT_SLOT, style.into());
    style
}

pub(crate) fn css_style_rule_css_rules_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSStyleRule", "cssRules") {
        return;
    }
    rv.set(css_style_rule_rules_array(scope, args.this()).into());
}

pub(crate) fn css_style_rule_rules_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(rules) = get_private_value(scope, rule, CSS_STYLE_RULE_NESTED_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        install_css_rule_list_surface(scope, rules);
        return rules;
    }
    let rules = new_css_rule_list_object(scope);
    if !sync_css_style_rule_rules_array_from_live_stylesheet(scope, rule, rules) {
        let parent_style_sheet = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT);
        if !initialize_detached_css_rule_list_from_parent_snapshot(
            scope,
            rules,
            parent_style_sheet,
            rule,
        ) {
            sync_css_style_rule_rules_array_from_current_text(scope, rule, rules);
        }
    }
    set_private_value(scope, rule, CSS_STYLE_RULE_NESTED_RULES_SLOT, rules.into());
    rules
}

pub(crate) fn sync_css_style_rule_rules_array_from_current_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
) {
    if css_rule_has_attached_native_binding(scope, rule) {
        return;
    }
    let parent_style_sheet = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(rule));
    if let Some(rule_snapshots) =
        css_style_rule_child_snapshots_from_stylesheet_context(scope, rule, parent_style_sheet)
    {
        replace_detached_css_rule_list_from_snapshots(
            scope,
            rules,
            rule_snapshots,
            parent_style_sheet,
            Some(rule),
        );
    } else {
        let style_text = private_string(scope, rule, CSS_STYLE_RULE_STYLE_TEXT_SLOT);
        let rule_snapshots = css_nested_rule_block_snapshots_with_stylo_context(
            scope,
            parent_style_sheet,
            rule,
            &style_text,
            CssRuleType::Style,
            style_rule_context,
            true,
        )
        .unwrap_or_default();
        replace_detached_css_rule_list_from_snapshots(
            scope,
            rules,
            rule_snapshots,
            parent_style_sheet,
            Some(rule),
        );
    }
}

pub(crate) fn css_style_rule_child_snapshots_from_stylesheet_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
) -> Option<Vec<CssRuleSnapshot>> {
    if let Some(snapshots) = detached_css_rule_child_snapshot_array(scope, rule) {
        return Some(detached_css_rule_snapshot_array_snapshots(scope, snapshots));
    }
    let css_text = css_rule_detached_snapshot_text(scope, rule);
    if let Some(sheet) = parent_style_sheet {
        let rule_texts = parent_style_sheet_current_rule_texts(scope, Some(sheet));
        if let Some(child_rules) = css_rule_child_snapshots_from_stylo_stylesheet_context(
            &rule_texts,
            CssRuleType::Style,
            &css_text,
            css_style_sheet_is_constructed(scope, sheet),
        ) {
            return Some(child_rules);
        }
    }
    css_rule_child_snapshots_from_stylo_stylesheet_context(
        std::slice::from_ref(&css_text),
        CssRuleType::Style,
        &css_text,
        false,
    )
}

pub(crate) fn css_nested_declarations_style_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(css_nested_declarations_style_object(scope, args.this()).into());
}

pub(crate) fn css_nested_declarations_style_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSNestedDeclarations", "style") {
        return;
    }
    let Some(css_text) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSStyleDeclaration", "cssText")
    else {
        return;
    };
    let style = css_nested_declarations_style_object(scope, args.this());
    set_style_object_css_text(scope, style, &css_text);
}

pub(crate) fn css_nested_declarations_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(style) = get_private_object(scope, rule, CSS_NESTED_DECLARATIONS_STYLE_OBJECT_SLOT)
    {
        return style;
    }
    let style = build_lightweight_css_rule_style_declaration(scope);
    let style_text = private_string(scope, rule, CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT);
    attach_css_rule_stylo_declaration_block_to_style(scope, rule, style);
    set_style_object_css_text(scope, style, &style_text);
    sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    if let Some(callback) = v8::Function::builder(css_nested_declarations_style_changed_callback)
        .data(rule.into())
        .build(scope)
    {
        set_lightweight_css_style_change_callback(scope, style, callback);
    }
    set_private_value(
        scope,
        rule,
        CSS_NESTED_DECLARATIONS_STYLE_OBJECT_SLOT,
        style.into(),
    );
    style
}
