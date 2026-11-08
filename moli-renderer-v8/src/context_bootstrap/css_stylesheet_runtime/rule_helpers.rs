use super::*;
pub(crate) use crate::webidl::dom_string_property_value as cssom_dom_string_property_value;

pub(crate) fn object_list_contains<'s>(
    objects: &[v8::Local<'s, v8::Object>],
    target: v8::Local<'s, v8::Object>,
) -> bool {
    objects
        .iter()
        .any(|candidate| candidate.strict_equals(target.into()))
}

pub(crate) fn dom_handle_from_marker_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<DomHandle> {
    if let Ok(value) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = value.u64_value();
        return lossless.then(|| DomHandle::new(index as usize));
    }
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| DomHandle::new(value as usize))
}

pub(crate) fn constructor_document_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    document_handle: Option<DomHandle>,
) -> Option<url::Url> {
    let host = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &*host };
    let document_handle = document_handle.unwrap_or_else(|| host.document_handle());
    host.dom_host()
        .node(document_handle)
        .and_then(Node::as_document)
        .map(|document| document.base_url().clone())
        .or_else(|| Some(host.document_url().clone()))
}

pub(crate) fn top_level_rule_texts_from_stylo_snapshots(css_text: &str) -> Vec<String> {
    parse_top_level_rule_snapshots_with_stylo(css_text, false)
        .into_iter()
        .map(|rule| rule.css_text)
        .collect()
}

pub(crate) fn stylo_containing_rule_type_bits_for_parent_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    let mut bits = 0;
    let mut current = Some(parent_rule);
    while let Some(rule) = current {
        let rule_type = stylo_rule_type_for_css_rule_object(scope, rule)?;
        bits |= rule_type.bit();
        current = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    }
    Some(bits)
}

pub(crate) fn stylo_containing_rule_type_for_stylo_rule_type(
    rule_type: CssRuleType,
) -> Option<CssRuleType> {
    (rule_type != CssRuleType::Keyframes
        && css_rule_nested_rules_slot_for_stylo_rule_type(rule_type).is_some())
    .then_some(rule_type)
}

pub(crate) fn css_nested_rule_block_snapshots_with_stylo_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: v8::Local<'s, v8::Object>,
    block_text: &str,
    rule_type: CssRuleType,
    style_rule_context: StyleRuleSelectorContext,
    wants_first_declaration_block: bool,
) -> Option<Vec<CssRuleSnapshot>> {
    css_nested_rule_block_with_stylo_context(
        scope,
        parent_style_sheet,
        parent_rule,
        block_text,
        rule_type,
        style_rule_context,
        wants_first_declaration_block,
    )
    .map(|mutation| mutation.rules)
}

pub(crate) fn css_nested_rule_block_with_stylo_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: v8::Local<'s, v8::Object>,
    block_text: &str,
    rule_type: CssRuleType,
    style_rule_context: StyleRuleSelectorContext,
    wants_first_declaration_block: bool,
) -> Option<CssDetachedRuleListMutation> {
    let containing_rule_type_bits =
        stylo_containing_rule_type_bits_for_parent_rule(scope, parent_rule)?;
    let namespace_rule_texts = parent_style_sheet
        .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
        .unwrap_or_default()
        .stylo_parent_rule_texts();
    parse_nested_rule_block_snapshots_with_stylo(
        &namespace_rule_texts,
        block_text,
        rule_type,
        containing_rule_type_bits,
        stylo_parse_relative_rule_type(style_rule_context),
        wants_first_declaration_block,
    )
    .ok()
}

pub(crate) fn css_stylo_rule_type_is_insertable(rule_type: CssRuleType) -> Option<bool> {
    match rule_type {
        CssRuleType::Import
        | CssRuleType::Namespace
        | CssRuleType::LayerBlock
        | CssRuleType::LayerStatement
        | CssRuleType::Media
        | CssRuleType::Supports
        | CssRuleType::Container
        | CssRuleType::Scope
        | CssRuleType::StartingStyle
        | CssRuleType::FontFace
        | CssRuleType::FontFeatureValues
        | CssRuleType::FontPaletteValues
        | CssRuleType::Keyframes
        | CssRuleType::Page
        | CssRuleType::CounterStyle
        | CssRuleType::Property => Some(true),
        CssRuleType::Style => None,
        _ => None,
    }
}

pub(crate) fn css_function_rule_text_is_insertable(css_text: &str) -> bool {
    single_custom_css_function_projection(css_text)
        .is_some_and(|rule| rule.block.is_some() && css_function_rule_name(&rule.prelude).is_some())
}

pub(crate) fn css_function_rule_name(prelude: &str) -> Option<&str> {
    let trimmed = prelude.trim();
    let name = trimmed.strip_suffix("()")?.trim();
    (name.starts_with("--") && name.len() > 2).then_some(name)
}

pub(crate) fn set_private_string(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_private_value(scope, object, slot, value.into());
    }
}

pub(crate) fn v8_string_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[String],
) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, values.len() as i32);
    for (index, value) in values.iter().enumerate() {
        if let Some(value) = v8_string(scope, value) {
            let _ = array.set_index(scope, index as u32, value.into());
        }
    }
    let _ = array.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    array
}

pub(crate) fn set_private_u32(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: u32,
) {
    set_private_value(
        scope,
        object,
        slot,
        v8::Integer::new_from_unsigned(scope, value).into(),
    );
}

pub(crate) fn set_private_u64(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: u64,
) {
    set_private_value(
        scope,
        object,
        slot,
        v8::BigInt::new_from_u64(scope, value).into(),
    );
}

pub(crate) fn private_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> String {
    get_private_value(scope, object, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

pub(crate) fn private_u32<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<u32> {
    get_private_value(scope, object, slot).and_then(|value| value.uint32_value(scope))
}

pub(crate) fn private_u64<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<u64> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (id, lossless) = value.u64_value();
            (lossless && id != 0).then_some(id)
        })
}

pub(crate) fn css_rule_snapshots_contain_nested_rules(rule_snapshots: &[CssRuleSnapshot]) -> bool {
    rule_snapshots
        .iter()
        .any(|rule_snapshot| rule_snapshot.rule_type != CssRuleType::NestedDeclarations)
}

pub(crate) fn local_nested_style_block_text_contains_rules(style_text: &str) -> bool {
    css_nested_rule_block_with_selector_context(
        &CssomSelectorNamespaceContext::default(),
        style_text,
        CssRuleType::Style,
        CssRuleType::Style.bit(),
        StyleRuleSelectorContext::Nested,
        true,
    )
    .map(|mutation| css_rule_snapshots_contain_nested_rules(&mutation.rules))
    .unwrap_or(true)
}

pub(crate) fn lightweight_css_style_object_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> String {
    lightweight_css_style_css_text(scope, style).unwrap_or_else(|| {
        style
            .get(scope, v8str(scope, "cssText").into())
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default()
    })
}

pub(crate) fn parent_nested_rule_block_with_stylo_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    parent_style_text_slot: &'static str,
    parent_style_text: &str,
) -> Option<CssDetachedRuleListMutation> {
    if parent_style_text_slot == CSS_STYLE_RULE_STYLE_TEXT_SLOT {
        return css_style_rule_block_with_stylo_context(scope, parent, parent_style_text);
    }
    if parent_style_text_slot != CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT {
        return None;
    }
    let parent_style_sheet = get_private_value(scope, parent, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let rule_type = css_rule_current_stylo_rule_type_from_object(scope, parent)?;
    let kind = css_at_rule_kind_for_stylo_rule_type(rule_type)?;
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(parent));
    let child_context = nested_grouping_rule_child_selector_context(kind, style_rule_context);
    css_nested_rule_block_with_stylo_context(
        scope,
        parent_style_sheet,
        parent,
        parent_style_text,
        rule_type,
        child_context,
        false,
    )
}

pub(crate) fn sync_parent_rule_from_child_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    child: v8::Local<'s, v8::Object>,
) {
    let Some(parent) = get_private_value(scope, child, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    match css_rule_current_stylo_rule_type_from_object(scope, parent) {
        Some(CssRuleType::Keyframes) => {
            if let Some(rules) = get_private_value(scope, parent, CSS_KEYFRAMES_RULE_RULES_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                sync_css_keyframes_rule_css_text_from_rules(scope, parent, rules);
            }
        }
        Some(CssRuleType::Style) => {
            if let Some(rules) = get_private_value(scope, parent, CSS_STYLE_RULE_NESTED_RULES_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                sync_css_style_rule_style_text_from_nested_rules(scope, parent, rules);
            }
        }
        Some(
            CssRuleType::Media
            | CssRuleType::Supports
            | CssRuleType::Container
            | CssRuleType::Scope
            | CssRuleType::LayerBlock
            | CssRuleType::StartingStyle
            | CssRuleType::Page,
        ) => {
            if let Some(rules) = get_private_value(scope, parent, CSS_AT_RULE_NESTED_RULES_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                sync_css_grouping_rule_css_text_from_rules(scope, parent, rules);
            }
        }
        _ => {}
    }
}

pub(crate) fn nested_at_rule_block_text_from_snapshot(
    rule_snapshot: &CssRuleSnapshot,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<String> {
    let parts = css_rule_snapshot_child_container_parts(rule_snapshot)?;
    match parts.kind {
        CssAtRuleKind::Media
        | CssAtRuleKind::Scope
        | CssAtRuleKind::Supports
        | CssAtRuleKind::Container
        | CssAtRuleKind::Layer => Some(css_rule_snapshot_nested_style_block_text(
            &rule_snapshot.child_rules,
            selector_context,
            nested_grouping_rule_child_selector_context(parts.kind, style_rule_context),
        )),
        _ => None,
    }
}

pub(crate) fn keyframes_name_candidate_matches(name: &str, serialized_name: &str) -> bool {
    css_keyframes_rule_name_from_css_text(&format!("@keyframes {serialized_name} {{}}"))
        .is_some_and(|parsed_name| parsed_name == name)
}

pub(crate) fn css_supports_rule_matches_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSSupportsRule", "matches") {
        return;
    }
    let matches = css_condition_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::Supports)
        .is_some_and(|view| css_supports_condition_text(&view.condition_text));
    rv.set_bool(matches);
}

pub(crate) fn v8_dynamic_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
) -> v8::Local<'s, v8::Value> {
    v8_string(scope, value)
        .map(Into::into)
        .unwrap_or_else(|| v8::String::empty(scope).into())
}

pub(crate) fn css_descriptor_rule_style_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
    interface: &str,
    rule_type: u32,
) {
    if !ensure_css_rule_type_object(scope, args.this(), interface, "style", rule_type) {
        return;
    }
    let Some(css_text) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSStyleDeclaration", "cssText")
    else {
        return;
    };
    let style = if rule_type == CSS_RULE_MARGIN_RULE_TYPE {
        css_margin_rule_style_object(scope, args.this())
    } else if rule_type == CSS_RULE_FONT_FACE_RULE_TYPE {
        css_font_face_rule_style_object(scope, args.this())
    } else if rule_type == CSS_RULE_PAGE_RULE_TYPE {
        css_page_rule_style_object(scope, args.this())
    } else {
        return;
    };
    set_style_object_css_text(scope, style, &css_text);
}

pub(crate) fn sync_nested_at_rule_style_text_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) {
    // New local nested at-rule wrappers are not bound to a live stylesheet yet; seed
    // their projection block from the just-built rule text.
    let Some(parts) = css_at_rule_text_parts_from_object(scope, rule) else {
        return;
    };
    let Some(block_text) = parts.block.as_deref() else {
        return;
    };
    if !matches!(
        parts.kind,
        CssAtRuleKind::Media
            | CssAtRuleKind::Scope
            | CssAtRuleKind::Supports
            | CssAtRuleKind::Container
            | CssAtRuleKind::Layer
    ) {
        return;
    }
    let parent_style_sheet = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let child_context = nested_grouping_rule_child_selector_context(parts.kind, style_rule_context);
    let Some(rule_snapshots) = css_nested_rule_block_snapshots_with_stylo_context(
        scope,
        parent_style_sheet,
        rule,
        block_text,
        css_at_rule_stylo_rule_type(parts.kind),
        child_context,
        false,
    ) else {
        return;
    };
    let block =
        css_rule_snapshot_nested_style_block_text(&rule_snapshots, selector_context, child_context);
    set_private_string(scope, rule, CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT, &block);
}

pub(crate) fn set_style_object_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    css_text: &str,
) {
    let _ = set_lightweight_css_style_css_text(scope, style, css_text);
}
