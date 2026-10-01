use super::*;
use style::{
    properties::CSSWideKeyword,
    values::computed::font::{FamilyName, FontFamilyNameSyntax},
};

pub(crate) fn normalize_insert_rule_index(index: u32, rules_len: u32) -> Option<u32> {
    (index <= rules_len).then_some(index)
}

pub(crate) fn parse_insert_rule_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rules_len: u32,
) -> Option<CssStyleSheetInsertRuleArgs> {
    let parsed = webidl::parse_args::<CssStyleSheetInsertRuleArgs>(scope, args)?;
    let index = parsed.index;
    let Some(index) = normalize_insert_rule_index(index, rules_len) else {
        webidl::throw_index_size_error(scope);
        return None;
    };
    Some(CssStyleSheetInsertRuleArgs { index, ..parsed })
}

pub(crate) fn parse_grouping_rule_insert_rule_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rules_len: u32,
) -> Option<CssGroupingRuleInsertRuleArgs> {
    let parsed = webidl::parse_args::<CssGroupingRuleInsertRuleArgs>(scope, args)?;
    let index = parsed.index;
    let Some(index) = normalize_insert_rule_index(index, rules_len) else {
        webidl::throw_index_size_error(scope);
        return None;
    };
    Some(CssGroupingRuleInsertRuleArgs { index, ..parsed })
}

pub(crate) fn insert_css_rule_list_rule_with_selector_context_and_rule_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    index: u32,
    css_text: &str,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) {
    let rule = build_css_rule_object_with_rule_context(
        scope,
        css_text,
        parent_style_sheet,
        parent_rule,
        selector_context,
        style_rule_context,
    );
    if style_rule_context != StyleRuleSelectorContext::TopLevel {
        sync_nested_at_rule_style_text_slot(scope, rule, selector_context, style_rule_context);
    }
    insert_css_rule_list_rule_object(scope, rules, index, rule);
}

pub(crate) fn parse_css_rule_list_top_level_snapshots_with_selector_context(
    css_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
) -> Vec<CssRuleSnapshot> {
    let mut selector_context = selector_context.clone();
    parse_top_level_rule_snapshots_with_stylo(css_text, false)
        .into_iter()
        .filter_map(|rule_snapshot| {
            css_style_sheet_rule_text_is_supported_with_selector_context_and_rule_context(
                &rule_snapshot.css_text,
                &selector_context,
                StyleRuleSelectorContext::TopLevel,
            )
            .then(|| {
                selector_context.record_rule_text(&rule_snapshot.css_text);
                rule_snapshot
            })
        })
        .collect()
}

pub(crate) fn parse_top_level_rule_snapshots_with_stylo(
    css_text: &str,
    constructed: bool,
) -> Vec<CssRuleSnapshot> {
    if constructed {
        parse_constructed_stylesheet_rule_snapshots_with_stylo(css_text)
    } else {
        parse_stylesheet_rule_snapshots_with_stylo(css_text)
    }
}

pub(crate) fn css_at_rule_text_parts_from_css_text_with_context(
    css_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<CssAtRuleTextParts> {
    let rule_snapshot =
        css_text_single_rule_snapshot_with_context(css_text, selector_context, style_rule_context)?;
    css_at_rule_text_parts_from_snapshot(&rule_snapshot, selector_context, style_rule_context)
}

pub(crate) fn css_at_rule_text_parts_from_snapshot(
    view: &CssRuleSnapshot,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<CssAtRuleTextParts> {
    let kind = css_at_rule_kind_for_stylo_rule_type(view.rule_type)?;
    let prelude = css_at_rule_prelude_from_snapshot(kind, view);
    let block = css_at_rule_block_from_snapshot(kind, view, selector_context, style_rule_context);
    Some(CssAtRuleTextParts {
        kind,
        prelude,
        block,
    })
}

pub(crate) fn css_at_rule_prelude_from_snapshot(
    kind: CssAtRuleKind,
    view: &CssRuleSnapshot,
) -> String {
    match kind {
        CssAtRuleKind::Page => view
            .selector_text
            .clone()
            .or_else(|| view.prelude_text.clone())
            .unwrap_or_default(),
        CssAtRuleKind::StartingStyle | CssAtRuleKind::FontFace => String::new(),
        _ => view.prelude_text.clone().unwrap_or_default(),
    }
}

pub(crate) fn css_at_rule_block_from_snapshot(
    kind: CssAtRuleKind,
    view: &CssRuleSnapshot,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<String> {
    match kind {
        CssAtRuleKind::Import | CssAtRuleKind::Namespace => None,
        CssAtRuleKind::Layer if view.rule_type == CssRuleType::LayerStatement => None,
        CssAtRuleKind::Media
        | CssAtRuleKind::Scope
        | CssAtRuleKind::Supports
        | CssAtRuleKind::Container
        | CssAtRuleKind::Layer
        | CssAtRuleKind::StartingStyle
        | CssAtRuleKind::Keyframes => {
            let child_context =
                nested_grouping_rule_child_selector_context(kind, style_rule_context);
            Some(css_rule_snapshot_nested_style_block_text(
                &view.child_rules,
                selector_context,
                child_context,
            ))
        }
        CssAtRuleKind::Page => {
            let declarations = view.declaration_text.as_deref().unwrap_or_default();
            let child_rules = css_rule_snapshot_child_block_text(&view.child_rules);
            Some(join_css_rule_blocks(declarations, &child_rules))
        }
        CssAtRuleKind::FontFace
        | CssAtRuleKind::FontFeatureValues
        | CssAtRuleKind::CounterStyle
        | CssAtRuleKind::Property => {
            let declarations = view.declaration_text.as_deref().unwrap_or_default();
            let child_rules = css_rule_snapshot_child_block_text(&view.child_rules);
            Some(join_css_rule_blocks(declarations, &child_rules))
        }
        CssAtRuleKind::Unknown | CssAtRuleKind::Function => None,
    }
}

pub(crate) fn css_style_sheet_rule_text_is_supported_with_selector_context_and_rule_context(
    rule_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> bool {
    let trimmed = rule_text.trim_start();
    if let Some(insertable) = css_text_single_stylo_rule_is_insertable(rule_text) {
        return insertable;
    }
    if css_text_starts_with_at_keyword(trimmed, "function") {
        return css_function_rule_text_is_insertable(rule_text);
    }
    trimmed.starts_with('@')
        || parse_valid_style_rule_text_with_selector_context_and_rule_context(
            rule_text,
            selector_context,
            style_rule_context,
        )
        .is_some()
}

pub(crate) fn replace_detached_css_rule_list_from_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    snapshots: Vec<CssRuleSnapshot>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) {
    for (_, rule) in css_rule_list_materialized_entries(scope, rules) {
        detach_css_rule_from_parent(scope, rule);
    }
    reset_css_rule_list_materialized_items(scope, rules);
    bind_css_rule_list_to_detached_snapshots(
        scope,
        rules,
        parent_style_sheet,
        parent_rule,
        &snapshots,
    );
}

pub(crate) fn css_font_face_rule_css_text_from_style_text(style_text: &str) -> Option<String> {
    let style_text = parse_font_face_descriptor_block_with_stylo(style_text)?;
    Some(if style_text.is_empty() {
        "@font-face { }".to_owned()
    } else {
        format!("@font-face {{ {style_text} }}")
    })
}

pub(crate) fn css_page_rule_css_text_from_parts(
    selector_text: &str,
    style_text: &str,
    nested_rule_text: &str,
) -> Option<String> {
    let selector = normalize_page_selector_text_with_stylo(selector_text).unwrap_or_default();
    let style_text = parse_page_descriptor_block_with_stylo(style_text)?;
    let block = join_css_rule_blocks(&style_text, nested_rule_text);
    css_page_rule_view_from_css_text(&serialize_page_rule_text(&selector, &block))
        .map(|view| view.css_text)
}

pub(crate) fn css_page_margin_rule_css_text_from_parts(
    name: &str,
    style_text: &str,
) -> Option<String> {
    let style_text = parse_page_margin_descriptor_block_with_stylo(name, style_text)?;
    Some(if style_text.is_empty() {
        format!("@{name} {{ }}")
    } else {
        format!("@{name} {{ {style_text} }}")
    })
}

pub(crate) fn commit_detached_css_rule_snapshot_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    css_text: &str,
    sync_parent_rule: bool,
) -> bool {
    if !set_detached_css_rule_snapshot_text(scope, rule, css_text) {
        return false;
    }
    if sync_parent_rule {
        sync_parent_rule_from_child_change(scope, rule);
    }
    true
}

pub(crate) fn stylo_parse_relative_rule_type(
    style_rule_context: StyleRuleSelectorContext,
) -> Option<CssRuleType> {
    match style_rule_context {
        StyleRuleSelectorContext::Nested => Some(CssRuleType::Style),
        StyleRuleSelectorContext::Scope => Some(CssRuleType::Scope),
        StyleRuleSelectorContext::TopLevel => None,
    }
}

pub(crate) fn css_rule_text_is_insertable_with_selector_context_and_rule_context(
    css_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> bool {
    if let Some(insertable) = css_text_single_stylo_rule_is_insertable(css_text) {
        return insertable;
    }
    parse_valid_style_rule_text_with_selector_context_and_rule_context(
        css_text,
        selector_context,
        style_rule_context,
    )
    .is_some()
        || css_function_rule_text_is_insertable(css_text)
}

pub(crate) fn css_text_single_stylo_rule_is_insertable(css_text: &str) -> Option<bool> {
    css_text_single_stylo_rule_type(css_text).and_then(css_stylo_rule_type_is_insertable)
}

pub(crate) fn css_text_starts_with_at_keyword(css_text: &str, keyword: &str) -> bool {
    let Some(after_at) = css_text.strip_prefix('@') else {
        return false;
    };
    let Some((prefix, rest)) = after_at.split_at_checked(keyword.len()) else {
        return false;
    };
    prefix.eq_ignore_ascii_case(keyword)
        && !rest
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(crate) fn css_style_rule_selector_context_for_parent_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> StyleRuleSelectorContext {
    let mut current = parent_rule;
    while let Some(rule) = current {
        match css_rule_current_stylo_rule_type_from_object(scope, rule) {
            Some(CssRuleType::Style) => return StyleRuleSelectorContext::Nested,
            Some(CssRuleType::Scope) => return StyleRuleSelectorContext::Scope,
            _ => {}
        }
        current = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    }
    StyleRuleSelectorContext::TopLevel
}

pub(crate) fn canonical_css_at_rule_text(css_text: &str, kind: CssAtRuleKind) -> String {
    match kind {
        CssAtRuleKind::Namespace => {
            canonical_namespace_rule_text(css_text).unwrap_or_else(|| css_text.trim().to_owned())
        }
        CssAtRuleKind::Import => {
            canonical_import_rule_text(css_text).unwrap_or_else(|| css_text.trim().to_owned())
        }
        CssAtRuleKind::Layer => {
            canonical_layer_rule_text(css_text).unwrap_or_else(|| css_text.trim().to_owned())
        }
        CssAtRuleKind::Media => canonical_grouping_rule_text_with_stylo(css_text, kind)
            .unwrap_or_else(|| css_text.trim().to_owned()),
        CssAtRuleKind::Scope => {
            canonical_scope_rule_text(css_text).unwrap_or_else(|| css_text.trim().to_owned())
        }
        CssAtRuleKind::Page => {
            canonical_page_rule_text(css_text).unwrap_or_else(|| css_text.trim().to_owned())
        }
        CssAtRuleKind::Keyframes => {
            canonical_keyframes_rule_text(css_text).unwrap_or_else(|| css_text.trim().to_owned())
        }
        CssAtRuleKind::Supports => canonical_grouping_rule_text_with_stylo(css_text, kind)
            .unwrap_or_else(|| css_text.trim().to_owned()),
        CssAtRuleKind::Container => {
            canonical_container_rule_text(css_text).unwrap_or_else(|| css_text.trim().to_owned())
        }
        CssAtRuleKind::StartingStyle => canonical_grouping_rule_text_with_stylo(css_text, kind)
            .unwrap_or_else(|| css_text.trim().to_owned()),
        CssAtRuleKind::FontFace => canonical_single_stylesheet_rule_text_with_stylo(
            css_text,
            css_at_rule_stylo_rule_type(kind),
        )
        .unwrap_or_else(|| css_text.trim().to_owned()),
        CssAtRuleKind::Property => canonical_single_stylesheet_rule_text_with_stylo(
            css_text,
            css_at_rule_stylo_rule_type(kind),
        )
        .unwrap_or_else(|| css_text.trim().to_owned()),
        CssAtRuleKind::CounterStyle => canonical_single_stylesheet_rule_text_with_stylo(
            css_text,
            css_at_rule_stylo_rule_type(kind),
        )
        .unwrap_or_else(|| css_text.trim().to_owned()),
        CssAtRuleKind::FontFeatureValues => canonical_single_stylesheet_rule_text_with_stylo(
            css_text,
            css_at_rule_stylo_rule_type(kind),
        )
        .unwrap_or_else(|| css_text.trim().to_owned()),
        _ => css_text.trim().to_owned(),
    }
}

pub(crate) fn canonical_import_rule_text(css_text: &str) -> Option<String> {
    parse_import_rule_view_with_stylo(css_text).map(|view| view.css_text)
}

pub(crate) fn canonical_namespace_rule_text(css_text: &str) -> Option<String> {
    parse_namespace_rule_view_with_stylo(css_text).map(|view| view.css_text)
}

pub(crate) fn canonical_layer_rule_text(css_text: &str) -> Option<String> {
    parse_layer_rule_view_with_stylo(css_text).map(|view| view.css_text)
}

pub(crate) fn canonical_scope_rule_text(css_text: &str) -> Option<String> {
    parse_condition_rule_view_with_stylo(css_text)
        .filter(|view| view.rule_type == CssRuleType::Scope)
        .map(|view| view.css_text)
}

pub(crate) fn canonical_container_rule_text(css_text: &str) -> Option<String> {
    parse_condition_rule_view_with_stylo(css_text)
        .filter(|view| view.rule_type == CssRuleType::Container)
        .map(|view| view.css_text)
}

pub(crate) fn canonical_grouping_rule_text_with_stylo(
    css_text: &str,
    kind: CssAtRuleKind,
) -> Option<String> {
    canonical_single_stylesheet_rule_text_with_stylo(css_text, css_at_rule_stylo_rule_type(kind))
}

pub(crate) fn canonical_single_stylesheet_rule_text_with_stylo(
    css_text: &str,
    rule_type: CssRuleType,
) -> Option<String> {
    let mut rules = parse_stylesheet_rule_snapshots_with_stylo(css_text).into_iter();
    let rule = rules.next()?;
    if rules.next().is_some() || rule.rule_type != rule_type {
        return None;
    }
    Some(rule.css_text)
}

pub(crate) fn canonical_nested_grouping_rule_text_with_context(
    css_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> String {
    if style_rule_context == StyleRuleSelectorContext::TopLevel
        && let Some(rule_snapshot) = css_text_single_rule_snapshot(css_text)
    {
        return rule_snapshot.css_text;
    }
    let Some(parts) = css_at_rule_text_parts_from_css_text_with_context(
        css_text,
        selector_context,
        style_rule_context,
    ) else {
        return css_text.trim().to_owned();
    };
    match parts.kind {
        CssAtRuleKind::Media => {
            let prelude = normalize_media_query_list(&parts.prelude);
            serialized_nested_grouping_rule_text(
                parts.kind,
                &prelude,
                parts.block.as_deref().unwrap_or_default(),
                selector_context,
                style_rule_context,
            )
        }
        CssAtRuleKind::Scope => serialized_nested_grouping_rule_text(
            parts.kind,
            &parts.prelude,
            parts.block.as_deref().unwrap_or_default(),
            selector_context,
            StyleRuleSelectorContext::Scope,
        ),
        CssAtRuleKind::Supports => {
            let prelude = serialize_component_values_single_line(&parts.prelude)
                .unwrap_or_else(|| parts.prelude.clone());
            serialized_nested_grouping_rule_text(
                parts.kind,
                &prelude,
                parts.block.as_deref().unwrap_or_default(),
                selector_context,
                style_rule_context,
            )
        }
        CssAtRuleKind::Container => serialized_nested_grouping_rule_text(
            parts.kind,
            &parts.prelude,
            parts.block.as_deref().unwrap_or_default(),
            selector_context,
            style_rule_context,
        ),
        CssAtRuleKind::Layer if parts.block.is_some() => serialized_nested_grouping_rule_text(
            parts.kind,
            &parts.prelude,
            parts.block.as_deref().unwrap_or_default(),
            selector_context,
            style_rule_context,
        ),
        _ => canonical_css_at_rule_text(css_text, parts.kind),
    }
}

pub(crate) fn canonical_page_rule_text(css_text: &str) -> Option<String> {
    css_page_rule_view_from_css_text(css_text).map(|view| view.css_text)
}

pub(crate) fn canonical_keyframes_rule_text(css_text: &str) -> Option<String> {
    css_keyframes_rule_view_from_css_text(css_text).map(|view| view.css_text)
}

pub(crate) fn css_at_rule_text_parts_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssAtRuleTextParts> {
    let css_text = css_rule_detached_snapshot_text(scope, object);
    let parent_style_sheet = get_private_value(scope, object, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let selector_context = parent_style_sheet
        .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
        .unwrap_or_default();
    let parent_rule = get_private_value(scope, object, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, parent_rule);
    let rule_snapshot = css_text_single_rule_snapshot_for_parent_rule(
        scope,
        parent_style_sheet,
        parent_rule,
        &css_text,
    )?;
    css_at_rule_text_parts_from_snapshot(&rule_snapshot, &selector_context, style_rule_context)
}

pub(crate) fn css_nested_rule_block_with_selector_context(
    selector_context: &CssomSelectorNamespaceContext,
    block_text: &str,
    rule_type: CssRuleType,
    containing_rule_type_bits: u32,
    style_rule_context: StyleRuleSelectorContext,
    wants_first_declaration_block: bool,
) -> Option<CssDetachedRuleListMutation> {
    let parent_stylesheet_rule_texts = selector_context.stylo_parent_rule_texts();
    parse_nested_rule_block_snapshots_with_stylo(
        &parent_stylesheet_rule_texts,
        block_text,
        rule_type,
        containing_rule_type_bits,
        stylo_parse_relative_rule_type(style_rule_context),
        wants_first_declaration_block,
    )
    .ok()
}

pub(crate) fn css_font_feature_values_rule_css_text_with_map_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    slot: &str,
    name: &str,
    values: Vec<u32>,
) -> Option<String> {
    if !font_feature_values_map_values_are_supported(slot, &values) {
        return None;
    }
    let mut view = css_font_feature_values_rule_view_from_object(scope, rule)?;
    let entries = font_feature_values_rule_view_entries_mut(&mut view, slot)?;
    if let Some(entry) = entries.iter_mut().find(|entry| entry.name == name) {
        entry.values = values;
    } else {
        entries.push(CssFontFeatureValueEntryView {
            name: name.to_owned(),
            values,
        });
    }
    Some(serialize_css_font_feature_values_rule_view(&view))
}

pub(crate) fn css_font_feature_values_rule_css_text_without_map_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    slot: &str,
    name: &str,
) -> Option<String> {
    let mut view = css_font_feature_values_rule_view_from_object(scope, rule)?;
    let entries = font_feature_values_rule_view_entries_mut(&mut view, slot)?;
    entries.retain(|entry| entry.name != name);
    Some(serialize_css_font_feature_values_rule_view(&view))
}

pub(crate) fn css_font_feature_values_rule_css_text_with_cleared_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    let mut view = css_font_feature_values_rule_view_from_object(scope, rule)?;
    font_feature_values_rule_view_entries_mut(&mut view, slot)?.clear();
    Some(serialize_css_font_feature_values_rule_view(&view))
}

pub(crate) fn font_feature_values_map_values_are_supported(slot: &str, values: &[u32]) -> bool {
    match slot {
        CSS_FONT_FEATURE_VALUES_RULE_ANNOTATION_SLOT
        | CSS_FONT_FEATURE_VALUES_RULE_ORNAMENTS_SLOT
        | CSS_FONT_FEATURE_VALUES_RULE_STYLISTIC_SLOT
        | CSS_FONT_FEATURE_VALUES_RULE_SWASH_SLOT => values.len() == 1,
        CSS_FONT_FEATURE_VALUES_RULE_CHARACTER_VARIANT_SLOT => (1..=2).contains(&values.len()),
        CSS_FONT_FEATURE_VALUES_RULE_STYLESET_SLOT => !values.is_empty(),
        _ => false,
    }
}

fn font_feature_values_rule_view_entries_mut<'a>(
    view: &'a mut CssFontFeatureValuesRuleView,
    slot: &str,
) -> Option<&'a mut Vec<CssFontFeatureValueEntryView>> {
    match slot {
        CSS_FONT_FEATURE_VALUES_RULE_ANNOTATION_SLOT => Some(&mut view.annotation),
        CSS_FONT_FEATURE_VALUES_RULE_ORNAMENTS_SLOT => Some(&mut view.ornaments),
        CSS_FONT_FEATURE_VALUES_RULE_STYLISTIC_SLOT => Some(&mut view.stylistic),
        CSS_FONT_FEATURE_VALUES_RULE_STYLESET_SLOT => Some(&mut view.styleset),
        CSS_FONT_FEATURE_VALUES_RULE_CHARACTER_VARIANT_SLOT => Some(&mut view.character_variant),
        CSS_FONT_FEATURE_VALUES_RULE_SWASH_SLOT => Some(&mut view.swash),
        _ => None,
    }
}

fn serialize_css_font_feature_values_rule_view(view: &CssFontFeatureValuesRuleView) -> String {
    let mut css_text = format!("@font-feature-values {} {{\n", view.font_family);
    for (keyword, entries) in [
        ("swash", view.swash.as_slice()),
        ("stylistic", view.stylistic.as_slice()),
        ("ornaments", view.ornaments.as_slice()),
        ("annotation", view.annotation.as_slice()),
        ("character-variant", view.character_variant.as_slice()),
        ("styleset", view.styleset.as_slice()),
    ] {
        if entries.is_empty() {
            continue;
        }
        css_text.push('@');
        css_text.push_str(keyword);
        css_text.push_str(" {\n");
        for entry in entries {
            css_text.push_str(&serialize_css_identifier(&entry.name));
            css_text.push_str(": ");
            for (index, value) in entry.values.iter().enumerate() {
                if index != 0 {
                    css_text.push(' ');
                }
                css_text.push_str(&value.to_string());
            }
            css_text.push_str(";\n");
        }
        css_text.push_str("}\n");
    }
    css_text.push('}');
    css_text
}

pub(crate) fn parse_valid_style_rule_text_with_selector_context(
    rule_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
) -> Option<CssStyleRuleTextParts> {
    parse_valid_style_rule_text_with_selector_context_and_rule_context(
        rule_text,
        selector_context,
        StyleRuleSelectorContext::TopLevel,
    )
}

pub(crate) fn parse_valid_style_rule_text_with_selector_context_and_rule_context(
    rule_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    rule_context: StyleRuleSelectorContext,
) -> Option<CssStyleRuleTextParts> {
    let rule_snapshot =
        css_text_single_rule_snapshot_with_context(rule_text, selector_context, rule_context)?;
    let parsed = style_rule_text_from_snapshot(&rule_snapshot, selector_context, rule_context)?;
    Some(canonical_style_rule_text(parsed, selector_context))
}

pub(crate) fn parse_valid_keyframe_rule_text(rule_text: &str) -> Option<CssStyleRuleTextParts> {
    let rule_snapshot = css_text_single_keyframe_child_snapshot(rule_text)?;
    let parsed = keyframe_rule_text_from_snapshot(&rule_snapshot)?;
    Some(canonical_declaration_style_rule_text_with_pdb_kind(
        parsed,
        CssRulePdbDeclarationKind::KeyframeRule,
    ))
}

pub(crate) fn css_text_single_keyframe_child_snapshot(rule_text: &str) -> Option<CssRuleSnapshot> {
    let keyframes = css_text_single_rule_snapshot(&format!(
        "@keyframes moli_keyframes_parse_context {{ {rule_text} }}"
    ))?;
    if keyframes.rule_type != CssRuleType::Keyframes {
        return None;
    }
    let mut child_rules = keyframes.child_rules.into_iter();
    let rule = child_rules.next()?;
    child_rules.next().is_none().then_some(rule)
}

pub(crate) fn sync_css_style_rule_css_text_from_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let css_text = css_style_rule_css_text_from_parts(scope, object);
    set_detached_css_rule_snapshot_text(scope, object, &css_text);
}

pub(crate) fn css_style_rule_css_text_from_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> String {
    let selector = private_string(scope, object, CSS_STYLE_RULE_SELECTOR_TEXT_SLOT);
    let stored_style = private_string(scope, object, CSS_STYLE_RULE_STYLE_TEXT_SLOT);
    let style = css_style_rule_serializable_declaration_text(scope, object, &stored_style);
    let selector_context = get_private_value(scope, object, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
        .unwrap_or_default();
    serialize_style_rule_css_text_with_context(&selector, &style, &selector_context)
}

pub(crate) fn serialize_style_rule_css_text_with_context(
    selector: &str,
    style_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
) -> String {
    let style_text = style_text.trim();
    if let Some(block_text) =
        nested_style_rule_block_text_if_has_rules(style_text, selector_context)
    {
        return serialize_nested_style_rule_css_text_from_block(selector, &block_text);
    }
    if style_text.is_empty() {
        format!("{selector} {{ }}")
    } else {
        format!("{selector} {{ {style_text} }}")
    }
}

pub(crate) fn canonical_style_rule_text(
    parsed: CssStyleRuleTextParts,
    selector_context: &CssomSelectorNamespaceContext,
) -> CssStyleRuleTextParts {
    let selector_text = parsed.selector_text;
    if let Some(style_text) =
        nested_style_rule_block_text_if_has_rules(&parsed.style_text, selector_context)
    {
        let css_text = serialize_nested_style_rule_css_text_from_block(&selector_text, &style_text);
        return CssStyleRuleTextParts {
            css_text,
            selector_text,
            style_text,
        };
    }
    canonical_declaration_style_rule_text_with_pdb_kind(
        CssStyleRuleTextParts {
            css_text: parsed.css_text,
            selector_text,
            style_text: parsed.style_text,
        },
        CssRulePdbDeclarationKind::StyleRule,
    )
}

pub(crate) fn nested_style_rule_block_with_selector_context(
    style_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<CssDetachedRuleListMutation> {
    css_nested_rule_block_with_selector_context(
        selector_context,
        style_text,
        CssRuleType::Style,
        CssRuleType::Style.bit(),
        style_rule_context,
        true,
    )
}

pub(crate) fn canonical_declaration_style_rule_text_with_pdb_kind(
    parsed: CssStyleRuleTextParts,
    kind: CssRulePdbDeclarationKind,
) -> CssStyleRuleTextParts {
    let style_text = if let Some(block) =
        css_rule_pdb_safe_declaration_block(&parsed.style_text, kind)
        && (!block.is_empty() || parsed.style_text.trim().is_empty())
    {
        block.css_text()
    } else {
        let entries = parse_css_declaration_list(&parsed.style_text);
        if entries.is_empty() {
            parsed.style_text
        } else {
            serialize_css_style_entries(&entries)
        }
    };
    let css_text = if style_text.is_empty() {
        format!("{} {{ }}", parsed.selector_text)
    } else {
        format!("{} {{ {style_text} }}", parsed.selector_text)
    };
    CssStyleRuleTextParts {
        css_text,
        selector_text: parsed.selector_text,
        style_text,
    }
}

pub(crate) fn sync_css_keyframe_rule_css_text_from_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let key_text = private_string(scope, object, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT);
    let style = css_rule_stylo_declaration_block_css_text(scope, object)
        .unwrap_or_else(|| private_string(scope, object, CSS_KEYFRAME_RULE_STYLE_TEXT_SLOT));
    let css_text = format!("{key_text} {{ {style} }}");
    set_detached_css_rule_snapshot_text(scope, object, &css_text);
}

pub(crate) fn css_rule_css_text_from_stylo_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if get_private_value(scope, rule, CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT).is_some() {
        return css_rule_stylo_declaration_block_css_text(scope, rule);
    }

    if get_private_value(scope, rule, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT).is_some() {
        let key_text = private_string(scope, rule, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT);
        let style = css_rule_stylo_declaration_block_css_text(scope, rule)?;
        return Some(format!("{key_text} {{ {style} }}"));
    }

    get_private_value(scope, rule, CSS_STYLE_RULE_SELECTOR_TEXT_SLOT)?;
    let stored_style = private_string(scope, rule, CSS_STYLE_RULE_STYLE_TEXT_SLOT);
    let has_materialized_nested_rules =
        get_private_value(scope, rule, CSS_STYLE_RULE_NESTED_RULES_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .is_some_and(|rules| css_rule_list_length(scope, rules) > 0);
    if has_materialized_nested_rules || local_nested_style_block_text_contains_rules(&stored_style)
    {
        return None;
    }
    let selector = private_string(scope, rule, CSS_STYLE_RULE_SELECTOR_TEXT_SLOT);
    let style = css_rule_stylo_declaration_block_css_text(scope, rule)?;
    let style = style.trim();
    Some(if style.is_empty() {
        format!("{selector} {{ }}")
    } else {
        format!("{selector} {{ {style} }}")
    })
}

pub(crate) fn css_style_rule_current_selector<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if let Some(selector_text) = css_rule_attached_native_style_selector_text(scope, rule) {
        return Some(selector_text);
    }
    let selector = private_string(scope, rule, CSS_STYLE_RULE_SELECTOR_TEXT_SLOT);
    (!selector.is_empty()).then_some(selector)
}

pub(crate) fn css_rule_css_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSRule", "cssText") {
        return;
    }
    let css_text = css_rule_serialized_css_text(scope, args.this());
    rv.set(v8_dynamic_string_value(scope, &css_text));
}

pub(crate) fn css_rule_css_text_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSRule", "cssText") {
        return;
    }
    let Some(css_text) = cssom_dom_string_property_value(scope, args.get(0), "CSSRule", "cssText")
    else {
        return;
    };
    let this = args.this();
    if css_rule_current_stylo_rule_type_from_object(scope, this) == Some(CssRuleType::Keyframe) {
        if let Some(parsed) = parse_valid_keyframe_rule_text(&css_text) {
            let use_live_stylesheet_declaration_mutation =
                css_keyframe_rule_css_text_reset_can_use_live_stylesheet_declaration_mutation(
                    scope, this, &parsed,
                );
            sync_css_keyframe_rule_state_from_parsed_css_text(scope, this, &parsed);
            if use_live_stylesheet_declaration_mutation
                && apply_live_stylesheet_rule_declaration_text_mutation(
                    scope,
                    this,
                    CssRulePdbDeclarationKind::KeyframeRule,
                    &parsed.style_text,
                )
            {
                rv.set_undefined();
                return;
            }
            if apply_live_stylesheet_keyframe_rule_replacement_mutation(
                scope,
                this,
                &parsed.css_text,
            ) {
                rv.set_undefined();
                return;
            }
            if !commit_detached_css_rule_snapshot_text(scope, this, &parsed.css_text, true) {
                restore_attached_css_rule_wrapper_from_live_stylesheet(scope, this);
            }
        }
        rv.set_undefined();
        return;
    }
    if apply_live_stylesheet_relative_style_rule_replacement_mutation(scope, this, &css_text) {
        rv.set_undefined();
        return;
    }
    let selector_context = css_style_rule_selector_namespace_context(scope, this);
    if let Some(parsed) =
        parse_valid_style_rule_text_with_selector_context(&css_text, &selector_context)
    {
        let use_live_stylesheet_declaration_mutation =
            css_style_rule_css_text_reset_can_use_live_stylesheet_declaration_mutation(
                scope, this, &parsed,
            );
        sync_css_style_rule_state_from_parsed_css_text(scope, this, &parsed);
        if use_live_stylesheet_declaration_mutation
            && apply_live_stylesheet_rule_declaration_text_mutation(
                scope,
                this,
                CssRulePdbDeclarationKind::StyleRule,
                &parsed.style_text,
            )
        {
            rv.set_undefined();
            return;
        }
        if apply_live_stylesheet_style_rule_replacement_mutation(scope, this, &parsed.css_text) {
            rv.set_undefined();
            return;
        }
        if css_rule_has_attached_native_binding(scope, this) {
            restore_attached_css_rule_wrapper_from_live_stylesheet(scope, this);
        } else {
            set_detached_css_rule_snapshot_text(scope, this, &parsed.css_text);
            if let Some(rules) = get_private_value(scope, this, CSS_STYLE_RULE_NESTED_RULES_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                sync_css_style_rule_rules_array_from_current_text(scope, this, rules);
            }
            sync_parent_rule_from_child_change(scope, this);
        }
    } else {
        set_css_rule_stylo_declaration_block_valid(scope, this, false);
        let css_text = canonical_css_at_rule_reset_text(scope, this, &css_text);
        let reset_stylo_rule_type_matches_current =
            css_rule_css_text_reset_matches_current_stylo_rule_type(scope, this, &css_text);
        if css_rule_css_text_reset_requires_stylo_rule_type_match(scope, this)
            && !reset_stylo_rule_type_matches_current
        {
            rv.set_undefined();
            return;
        }
        if reset_stylo_rule_type_matches_current
            && apply_live_stylesheet_css_rule_replacement_mutation(scope, this, &css_text)
        {
            rv.set_undefined();
            return;
        }
        if css_rule_has_attached_native_binding(scope, this) {
            restore_attached_css_rule_wrapper_from_live_stylesheet(scope, this);
        } else {
            set_detached_css_rule_snapshot_text(scope, this, &css_text);
            sync_local_css_at_rule_wrapper_slots_from_css_text(scope, this, &css_text);
            if let Some(rules) = get_private_value(scope, this, CSS_KEYFRAMES_RULE_RULES_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                sync_css_keyframes_rule_rules_array_from_current_text(scope, this, rules);
            }
            if let Some(rules) = get_private_value(scope, this, CSS_AT_RULE_NESTED_RULES_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                sync_css_grouping_rule_rules_array_from_current_text(scope, this, rules);
            }
            if css_rule_is_at_rule_wrapper(scope, this) {
                sync_parent_rule_from_child_change(scope, this);
            }
        }
    }
    rv.set_undefined();
}

pub(crate) fn css_rule_css_text_reset_matches_current_stylo_rule_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    css_text: &str,
) -> bool {
    let Some(expected_rule_type) = css_rule_current_stylo_rule_type_from_object(scope, rule) else {
        return false;
    };
    let Some(reset_rule_type) = css_text_single_stylo_rule_type(css_text) else {
        return false;
    };
    reset_rule_type == expected_rule_type
}

pub(crate) fn css_rule_css_text_reset_requires_stylo_rule_type_match<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> bool {
    css_rule_current_stylo_rule_type_from_object(scope, rule).is_some_and(|rule_type| {
        matches!(
            rule_type,
            CssRuleType::FontFace
                | CssRuleType::FontFeatureValues
                | CssRuleType::Keyframes
                | CssRuleType::Property
                | CssRuleType::CounterStyle
        )
    })
}

pub(crate) fn css_text_single_rule_snapshot(css_text: &str) -> Option<CssRuleSnapshot> {
    let mut rules = parse_stylesheet_rule_snapshots_with_stylo(css_text).into_iter();
    let rule = rules.next()?;
    rules.next().is_none().then_some(rule)
}

pub(crate) fn css_text_single_rule_snapshot_with_context(
    css_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<CssRuleSnapshot> {
    if style_rule_context == StyleRuleSelectorContext::TopLevel {
        return css_text_single_stylesheet_rule_snapshot_with_selector_context(
            css_text,
            selector_context,
        );
    }
    let parent_rule_type = match style_rule_context {
        StyleRuleSelectorContext::TopLevel => unreachable!(),
        StyleRuleSelectorContext::Nested => CssRuleType::Style,
        StyleRuleSelectorContext::Scope => CssRuleType::Scope,
    };
    css_text_single_nested_rule_snapshot_with_context(
        css_text,
        selector_context,
        parent_rule_type,
        parent_rule_type.bit(),
        style_rule_context,
    )
}

pub(crate) fn css_text_single_stylesheet_rule_snapshot_with_selector_context(
    css_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
) -> Option<CssRuleSnapshot> {
    let parent_stylesheet_rule_texts = selector_context.stylo_parent_rule_texts();
    parse_stylesheet_rule_snapshot_for_insert_with_stylo(
        &parent_stylesheet_rule_texts,
        css_text,
        parent_stylesheet_rule_texts.len(),
        false,
    )
    .ok()
}

pub(crate) fn css_text_single_nested_rule_snapshot_with_context(
    css_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    parent_rule_type: CssRuleType,
    containing_rule_type_bits: u32,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<CssRuleSnapshot> {
    let mutation = css_nested_rule_block_with_selector_context(
        selector_context,
        css_text,
        parent_rule_type,
        containing_rule_type_bits,
        style_rule_context,
        false,
    )?;
    let mut rules = mutation.rules.into_iter();
    let rule = rules.next()?;
    rules.next().is_none().then_some(rule)
}

pub(crate) fn css_text_single_rule_snapshot_for_parent_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    css_text: &str,
) -> Option<CssRuleSnapshot> {
    let Some(parent_rule) = parent_rule else {
        return css_text_single_rule_snapshot(css_text);
    };
    let parent_rule_type = stylo_containing_rule_type_for_stylo_rule_type(
        stylo_rule_type_for_css_rule_object(scope, parent_rule)?,
    )?;
    let style_rule_context =
        css_style_rule_selector_context_for_parent_rule(scope, Some(parent_rule));
    let mutation = css_nested_rule_block_with_stylo_context(
        scope,
        parent_style_sheet,
        parent_rule,
        css_text,
        parent_rule_type,
        style_rule_context,
        false,
    )?;
    let mut rules = mutation.rules.into_iter();
    let rule = rules.next()?;
    rules.next().is_none().then_some(rule)
}

pub(crate) fn css_text_single_stylo_rule_type(css_text: &str) -> Option<CssRuleType> {
    css_text_single_rule_snapshot(css_text).map(|rule| rule.rule_type)
}

pub(crate) fn canonical_css_at_rule_reset_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    css_text: &str,
) -> String {
    if !css_rule_is_at_rule_wrapper(scope, rule) {
        return css_text.trim().to_owned();
    }
    if let Some(rule_snapshot) = css_text_single_rule_snapshot(css_text) {
        return rule_snapshot.css_text;
    }
    css_text.trim().to_owned()
}

pub(crate) fn css_font_feature_values_rule_css_text_with_font_family<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    font_family: &str,
) -> Option<String> {
    let mut view = css_font_feature_values_rule_view_from_object(scope, rule)?;
    view.font_family = font_family.to_owned();
    Some(serialize_css_font_feature_values_rule_view(&view))
}

pub(crate) struct CssomFontFeatureValuesFamilies {
    serialized: String,
    native: Vec<FamilyName>,
}

impl CssomFontFeatureValuesFamilies {
    pub(crate) fn into_parts(self) -> (String, Vec<FamilyName>) {
        (self.serialized, self.native)
    }
}

fn is_css_name_start_code_point(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_' || !character.is_ascii()
}

fn is_css_name_code_point(character: char) -> bool {
    is_css_name_start_code_point(character) || character.is_ascii_digit() || character == '-'
}

// Mirrors Blink's IsCSSTokenizerIdentifier(): an identifier without escape
// sequences. CSSFontFeatureValuesRule intentionally classifies each raw,
// comma-separated family string instead of parsing a <family-name>.
fn is_unescaped_css_tokenizer_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(mut first) = characters.next() else {
        return false;
    };
    if first == '-' {
        let Some(character) = characters.next() else {
            return false;
        };
        first = character;
    }
    is_css_name_start_code_point(first) && characters.all(is_css_name_code_point)
}

fn is_chromium_generic_font_family(value: &str) -> bool {
    // FontFamily::InferredTypeFor performs case-sensitive comparisons.
    matches!(
        value,
        "cursive" | "fantasy" | "monospace" | "sans-serif" | "serif" | "system-ui" | "math"
    )
}

fn cssom_font_feature_values_family_syntax(value: &str) -> FontFamilyNameSyntax {
    // Blink gates `revert-rule` through its runtime feature state here. Use
    // Stylo's enabled CSS-wide keyword set so Moli's enabled feature surface
    // makes the same decision, even when it differs from default Chromium.
    let requires_quotes = CSSWideKeyword::from_ident(value).is_ok()
        || value.eq_ignore_ascii_case("default")
        || is_chromium_generic_font_family(value)
        || !is_unescaped_css_tokenizer_identifier(value);
    if requires_quotes {
        FontFamilyNameSyntax::Quoted
    } else {
        FontFamilyNameSyntax::Identifiers
    }
}

pub(crate) fn normalize_cssom_font_feature_values_families(
    value: &str,
) -> CssomFontFeatureValuesFamilies {
    let mut serialized = String::with_capacity(value.len());
    let mut native = Vec::new();
    for family in value
        .split(',')
        .map(str::trim)
        .filter(|family| !family.is_empty())
    {
        if !native.is_empty() {
            serialized.push_str(", ");
        }
        let syntax = cssom_font_feature_values_family_syntax(family);
        match syntax {
            FontFamilyNameSyntax::Quoted => serialize_string(family, &mut serialized)
                .expect("serializing CSS font family should not fail"),
            FontFamilyNameSyntax::Identifiers => serialized.push_str(family),
        }
        native.push(FamilyName {
            name: style::Atom::from(family),
            syntax,
        });
    }
    CssomFontFeatureValuesFamilies { serialized, native }
}

pub(crate) fn parse_nested_declarations_insert_rule_text(
    rule_text: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<String> {
    if style_rule_context == StyleRuleSelectorContext::TopLevel {
        return None;
    }
    let mutation = nested_style_rule_block_with_selector_context(
        rule_text,
        selector_context,
        style_rule_context,
    )?;
    if !mutation.rules.is_empty() {
        return None;
    }
    mutation
        .first_declaration_text
        .filter(|declaration_text| !declaration_text.trim().is_empty())
}

pub(crate) fn sync_css_grouping_rule_css_text_from_rules<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
) {
    let Some(parts) = css_grouping_rule_text_parts(scope, rule) else {
        return;
    };
    let nested = css_rule_list_current_css_text(scope, rules);
    if parts.kind == CssAtRuleKind::Page {
        let selector = normalize_page_selector_text_with_stylo(&parts.prelude)
            .unwrap_or_else(|| parts.prelude.clone());
        let style_text = css_at_rule_current_style_text(scope, rule);
        let block = join_css_rule_blocks(&style_text, &nested);
        let css_text = serialize_page_rule_text(&selector, &block);
        set_detached_css_rule_snapshot_text(scope, rule, &css_text);
        sync_parent_rule_from_child_change(scope, rule);
        return;
    }
    let prelude = parts.prelude;
    let selector_context = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
        .unwrap_or_default();
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(rule));
    if style_rule_context != StyleRuleSelectorContext::TopLevel {
        set_private_string(scope, rule, CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT, &nested);
    }
    let css_text = if style_rule_context == StyleRuleSelectorContext::TopLevel {
        serialized_grouping_rule_text(parts.kind, &prelude, &nested)
    } else {
        serialized_nested_grouping_rule_text(
            parts.kind,
            &prelude,
            &nested,
            &selector_context,
            style_rule_context,
        )
    };
    set_detached_css_rule_snapshot_text(scope, rule, &css_text);
    sync_parent_rule_from_child_change(scope, rule);
}

pub(crate) fn css_grouping_rule_text_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssGroupingRuleTextParts> {
    if let Some((rule_type, prelude)) = css_rule_attached_native_grouping_prelude(scope, rule) {
        let kind = css_at_rule_kind_for_stylo_rule_type(rule_type)?;
        return Some(CssGroupingRuleTextParts { kind, prelude });
    }
    css_grouping_rule_text_parts_from_current_text(scope, rule)
}

pub(crate) fn css_grouping_rule_text_parts_from_current_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssGroupingRuleTextParts> {
    let parts = css_at_rule_text_parts_from_object(scope, rule)?;
    Some(CssGroupingRuleTextParts {
        kind: parts.kind,
        prelude: parts.prelude,
    })
}

pub(crate) fn sync_css_keyframes_rule_css_text_from_rules<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
) {
    let Some(name) = css_keyframes_rule_current_name(scope, rule) else {
        return;
    };
    let nested = css_rule_list_current_css_text(scope, rules);
    let css_text = format!("@keyframes {name} {{ {nested} }}");
    set_detached_css_rule_snapshot_text(scope, rule, &css_text);
}

pub(crate) fn css_keyframes_rule_current_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if let Some(name) = css_rule_attached_native_keyframes_name(scope, rule) {
        return Some(name);
    }
    css_keyframes_rule_local_name(scope, rule)
}

pub(crate) fn css_keyframes_rule_local_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let css_text = css_rule_detached_snapshot_text(scope, rule);
    css_keyframes_rule_view_from_css_text(&css_text)
        .map(|view| view.name)
        .or_else(|| css_at_rule_text_parts_from_object(scope, rule).map(|parts| parts.prelude))
}

pub(crate) fn css_rule_list_css_texts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    (0..css_rule_list_length(scope, rules))
        .filter_map(|index| {
            if let Some(rule) = css_rule_list_materialized_rule(scope, rules, index) {
                return Some(css_rule_serialized_css_text(scope, rule));
            }
            css_rule_list_detached_snapshot_text_at(scope, rules, index).or_else(|| {
                css_rule_list_item(scope, rules, index)
                    .map(|rule| css_rule_serialized_css_text(scope, rule))
            })
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
}

pub(crate) fn css_rule_list_current_css_texts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    (0..css_rule_list_length(scope, rules))
        .filter_map(|index| {
            if let Some(rule) = css_rule_list_materialized_rule(scope, rules, index) {
                return Some(css_rule_current_css_text(scope, rule));
            }
            css_rule_list_detached_snapshot_text_at(scope, rules, index).or_else(|| {
                css_rule_list_item(scope, rules, index)
                    .map(|rule| css_rule_current_css_text(scope, rule))
            })
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
}

pub(crate) fn css_page_rule_selector_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSPageRule", "selectorText") {
        return;
    }
    let selector = css_page_rule_read_from_object(scope, args.this())
        .map(|read| read.selector_text)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &selector));
}

pub(crate) fn css_page_rule_selector_text_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSPageRule", "selectorText") {
        return;
    }
    let Some(selector) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSPageRule", "selectorText")
    else {
        return;
    };
    let Some(selector) = normalize_page_selector_text_with_stylo(&selector) else {
        return;
    };
    if apply_live_stylesheet_page_rule_selector_mutation(scope, args.this(), &selector) {
        return;
    }
    let (_, style_text, nested_rule_text) = css_page_rule_public_mutation_parts(scope, args.this());
    let block = join_css_rule_blocks(&style_text, &nested_rule_text);
    let css_text = serialize_page_rule_text(&selector, &block);
    let _ = commit_detached_css_rule_snapshot_text(scope, args.this(), &css_text, false);
}

pub(crate) fn serialize_keyframes_name(name: &str) -> String {
    let identifier = serialize_css_identifier(name);
    if keyframes_name_candidate_matches(name, &identifier) {
        return identifier;
    }
    let string = serialize_css_string(name);
    if keyframes_name_candidate_matches(name, &string) {
        return string;
    }
    identifier
}

pub(crate) fn serialize_css_identifier(value: &str) -> String {
    let mut serialized = String::new();
    serialize_identifier(value, &mut serialized)
        .expect("serializing CSS identifier should not fail");
    serialized
}

pub(crate) fn serialize_css_string(value: &str) -> String {
    let mut serialized = String::new();
    serialize_string(value, &mut serialized).expect("serializing CSS string should not fail");
    serialized
}

pub(crate) fn css_margin_rule_css_text_from_style_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_text: &str,
) -> Option<String> {
    let name = private_string(scope, rule, CSS_MARGIN_RULE_NAME_SLOT);
    if let Some(css_text) = css_page_margin_rule_css_text_from_parts(&name, style_text) {
        return Some(css_text);
    }
    (!css_rule_has_attached_native_binding(scope, rule)).then(|| {
        if style_text.is_empty() {
            format!("@{name} {{ }}")
        } else {
            format!("@{name} {{ {style_text} }}")
        }
    })
}

pub(crate) fn css_style_rule_selector_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSStyleRule", "selectorText") {
        return;
    }
    let selector_text = css_style_rule_selector_text_from_live_stylesheet_rule(scope, args.this())
        .unwrap_or_else(|| private_string(scope, args.this(), CSS_STYLE_RULE_SELECTOR_TEXT_SLOT));
    rv.set(v8_dynamic_string_value(scope, &selector_text));
}

pub(crate) fn css_style_rule_selector_text_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSStyleRule", "selectorText") {
        return;
    }
    let Some(selector) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSStyleRule", "selectorText")
    else {
        return;
    };
    let parent_rule = get_private_value(scope, args.this(), CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, parent_rule);
    let selector_context = css_style_rule_selector_namespace_context(scope, args.this());
    let selector = match canonicalize_cssom_style_rule_selector_text(
        &selector,
        &selector_context.style_rule_namespace_context(),
        style_rule_context,
    ) {
        Ok(selector) => selector,
        Err(_) => {
            rv.set_undefined();
            return;
        }
    };
    set_private_string(
        scope,
        args.this(),
        CSS_STYLE_RULE_SELECTOR_TEXT_SLOT,
        &selector,
    );
    if apply_live_stylesheet_style_rule_selector_mutation(scope, args.this(), &selector) {
        rv.set_undefined();
        return;
    }
    let css_text = css_style_rule_css_text_from_parts(scope, args.this());
    if apply_live_stylesheet_style_rule_replacement_mutation(scope, args.this(), &css_text) {
        rv.set_undefined();
        return;
    }
    set_detached_css_rule_snapshot_text(scope, args.this(), &css_text);
    sync_parent_rule_from_child_change(scope, args.this());
    rv.set_undefined();
}

pub(crate) fn set_style_object_css_text_without_notify<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    css_text: &str,
) {
    let _ = set_lightweight_css_style_css_text_without_notify(scope, style, css_text);
}
