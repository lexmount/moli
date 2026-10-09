use super::*;

pub(crate) fn css_keyframes_rule_view_from_css_text(
    css_text: &str,
) -> Option<CssKeyframesRuleView> {
    parse_keyframes_rule_view_with_stylo(css_text)
}

pub(crate) fn css_keyframes_rule_name_from_css_text(css_text: &str) -> Option<String> {
    css_keyframes_rule_view_from_css_text(css_text).map(|view| view.name)
}

pub(crate) fn css_grouping_rule_css_rules_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let this = args.this();
    let rules =
        if css_rule_current_stylo_rule_type_from_object(scope, this) == Some(CssRuleType::Style) {
            css_style_rule_rules_array(scope, this)
        } else {
            css_grouping_rule_rules_array(scope, this)
        };
    rv.set(rules.into());
}

pub(crate) fn css_grouping_rule_rules_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(existing) = get_private_value(scope, rule, CSS_AT_RULE_NESTED_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        install_css_rule_list_surface(scope, existing);
        return existing;
    }
    let rules = new_css_rule_list_object(scope);
    if !sync_css_grouping_rule_rules_array_from_live_stylesheet(scope, rule, rules) {
        let parent_style_sheet = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT);
        if !initialize_detached_css_rule_list_from_parent_snapshot(
            scope,
            rules,
            parent_style_sheet,
            rule,
        ) {
            sync_css_grouping_rule_rules_array_from_current_text(scope, rule, rules);
        }
    }
    set_private_value(scope, rule, CSS_AT_RULE_NESTED_RULES_SLOT, rules.into());
    rules
}

pub(crate) fn sync_css_grouping_rule_rules_array_from_current_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
) {
    // This is only the local refresh path after detached cssText updates.
    // Public lazy reads must not parse detached state while a live stylesheet owner exists.
    if css_rule_has_attached_native_binding(scope, rule) {
        return;
    }
    let css_text = css_rule_detached_snapshot_text(scope, rule);
    if let Some(parts) = css_at_rule_text_parts_from_object(scope, rule) {
        let nested_style_text = private_string(scope, rule, CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT);
        let block = if nested_style_text.is_empty() {
            parts.block.as_deref().unwrap_or_default()
        } else {
            nested_style_text.as_str()
        };
        let parent_style_sheet = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        let selector_context = parent_style_sheet
            .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
            .unwrap_or_default();
        let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(rule));
        if style_rule_context != StyleRuleSelectorContext::TopLevel {
            let rule_snapshots = css_nested_rule_block_snapshots_with_stylo_context(
                scope,
                parent_style_sheet,
                rule,
                block,
                css_at_rule_stylo_rule_type(parts.kind),
                style_rule_context,
                false,
            )
            .unwrap_or_default();
            replace_detached_css_rule_list_from_snapshots(
                scope,
                rules,
                rule_snapshots,
                parent_style_sheet,
                Some(rule),
            );
        } else if let Some(rule_snapshots) =
            css_grouping_rule_child_snapshots(&css_text, parts.kind)
        {
            replace_detached_css_rule_list_from_snapshots(
                scope,
                rules,
                rule_snapshots,
                parent_style_sheet,
                Some(rule),
            );
        } else if parts.kind == CssAtRuleKind::Page {
            let snapshots = css_page_rule_view_from_css_text(&css_text)
                .map(|view| {
                    view.child_rules
                        .into_iter()
                        .map(|rule| CssRuleSnapshot {
                            rule_type: CssRuleType::Margin,
                            css_text: rule.css_text,
                            prelude_text: Some(rule.name),
                            selector_text: None,
                            declaration_text: Some(rule.style_text),
                            child_rules: Vec::new(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            replace_detached_css_rule_list_from_snapshots(
                scope,
                rules,
                snapshots,
                parent_style_sheet,
                Some(rule),
            );
        } else {
            let rule_snapshots = parse_css_rule_list_top_level_snapshots_with_selector_context(
                block,
                &selector_context,
            );
            replace_detached_css_rule_list_from_snapshots(
                scope,
                rules,
                rule_snapshots,
                parent_style_sheet,
                Some(rule),
            );
        }
    }
}

pub(crate) fn css_grouping_rule_child_snapshots(
    css_text: &str,
    kind: CssAtRuleKind,
) -> Option<Vec<CssRuleSnapshot>> {
    let rule_type = css_at_rule_stylo_rule_type(kind);
    css_rule_nested_rules_slot_for_stylo_rule_type(rule_type)?;
    let mut rules = parse_stylesheet_rule_snapshots_with_stylo(css_text).into_iter();
    let rule = rules.next()?;
    if rules.next().is_some() || rule.rule_type != rule_type {
        return None;
    }
    Some(rule.child_rules)
}

pub(crate) fn css_keyframes_rule_css_rules_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let this = css_rule_receiver(scope, &args);
    let rules = css_keyframes_rule_rules_array(scope, this);
    rv.set(rules.into());
}

pub(crate) fn css_keyframes_rule_rules_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(existing) = get_private_value(scope, rule, CSS_KEYFRAMES_RULE_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return existing;
    }
    let creation_context = rule
        .get_creation_context(scope)
        .expect("CSSKeyframesRule has a creation context");
    if creation_context == scope.get_current_context() {
        return css_keyframes_rule_rules_array_in_current_context(scope, rule);
    }
    let rules = {
        let target_scope = &mut v8::ContextScope::new(scope, creation_context);
        let rules = css_keyframes_rule_rules_array_in_current_context(target_scope, rule);
        v8::Global::new(target_scope, rules)
    };
    v8::Local::new(scope, &rules)
}

fn css_keyframes_rule_rules_array_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let rules = new_css_rule_list_object(scope);
    if let Some((parent_style_sheet, child_count)) =
        css_rule_live_stylesheet_child_rule_count(scope, rule)
    {
        initialize_attached_css_rule_list(
            scope,
            rules,
            parent_style_sheet,
            Some(rule),
            child_count,
        );
    } else {
        let parent_style_sheet = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT);
        if !initialize_detached_css_rule_list_from_parent_snapshot(
            scope,
            rules,
            parent_style_sheet,
            rule,
        ) {
            let child_snapshots = css_keyframes_rule_detached_child_snapshots(scope, rule);
            replace_detached_css_rule_list_from_snapshots(
                scope,
                rules,
                child_snapshots,
                parent_style_sheet,
                Some(rule),
            );
        }
    }
    set_private_value(scope, rule, CSS_KEYFRAMES_RULE_RULES_SLOT, rules.into());
    rules
}

pub(crate) fn css_keyframes_rule_detached_child_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Vec<CssRuleSnapshot> {
    if let Some(snapshots) = detached_css_rule_child_snapshot_array(scope, rule) {
        return detached_css_rule_snapshot_array_snapshots(scope, snapshots);
    }
    css_keyframes_rule_child_snapshots(&css_rule_detached_snapshot_text(scope, rule))
}

pub(crate) fn css_keyframes_rule_child_snapshots(css_text: &str) -> Vec<CssRuleSnapshot> {
    parse_stylesheet_rule_snapshots_with_stylo(css_text)
        .into_iter()
        .find(|rule| rule.rule_type == CssRuleType::Keyframes)
        .map(|rule| rule.child_rules)
        .unwrap_or_default()
}

pub(crate) fn sync_css_keyframes_rule_rules_array_from_current_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
) {
    // This refreshes an already materialized CSSRuleList from detached wrapper state.
    // Attached lists read native child identity and count directly.
    if css_rule_has_attached_native_binding(scope, rule) {
        return;
    }
    let parent_style_sheet = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let css_text = css_rule_detached_snapshot_text(scope, rule);
    let child_snapshots = css_keyframes_rule_child_snapshots(&css_text);
    replace_detached_css_rule_list_from_snapshots(
        scope,
        rules,
        child_snapshots,
        parent_style_sheet,
        Some(rule),
    );
}

pub(crate) fn css_grouping_rule_insert_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let this = args.this();
    let is_style_rule =
        css_rule_current_stylo_rule_type_from_object(scope, this) == Some(CssRuleType::Style);
    let rules = if is_style_rule {
        css_style_rule_rules_array(scope, this)
    } else {
        css_grouping_rule_rules_array(scope, this)
    };
    let rules_len = css_rule_list_length(scope, rules);
    let Some(parsed) = parse_grouping_rule_insert_rule_args(scope, &args, rules_len) else {
        return;
    };
    let parent_style_sheet = get_private_value(scope, this, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let selector_context = parent_style_sheet
        .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
        .unwrap_or_default();
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(this));
    if let Some(containing_rule_type_bits) =
        stylo_containing_rule_type_bits_for_parent_rule(scope, this)
    {
        let attached_rule = parent_style_sheet.and_then(|sheet| {
            css_rule_attached_native_path(scope, this, sheet).map(|path| (sheet, path))
        });
        if let Some((sheet, path)) = attached_rule {
            if let Err(error) = apply_live_stylesheet_nested_rule_insert_mutation(
                scope,
                rules,
                &parsed.rule,
                parsed.index,
                sheet,
                &path,
                style_rule_context,
                containing_rule_type_bits,
            ) {
                throw_insert_rule_error(scope, error);
                return;
            }
            rv.set(v8::Integer::new(scope, parsed.index as i32).into());
            return;
        }

        if let Err(error) = apply_detached_nested_rule_insert_mutation(
            scope,
            this,
            rules,
            &parsed.rule,
            parsed.index,
            parent_style_sheet,
            &selector_context,
            style_rule_context,
            containing_rule_type_bits,
        ) {
            throw_insert_rule_error(scope, error);
            return;
        }
        if is_style_rule {
            sync_css_style_rule_style_text_from_nested_rules(scope, this, rules);
        } else {
            sync_css_grouping_rule_css_text_from_rules(scope, this, rules);
        }
        rv.set(v8::Integer::new(scope, parsed.index as i32).into());
        return;
    }
    if let Some(style_text) = parse_nested_declarations_insert_rule_text(
        &parsed.rule,
        &selector_context,
        style_rule_context,
    ) {
        let rule =
            build_css_nested_declarations_rule_object(scope, &style_text, parent_style_sheet, this);
        insert_css_rule_list_rule_object(scope, rules, parsed.index, rule);
        if is_style_rule {
            sync_css_style_rule_style_text_from_nested_rules(scope, this, rules);
        } else {
            sync_css_grouping_rule_css_text_from_rules(scope, this, rules);
        }
        rv.set(v8::Integer::new(scope, parsed.index as i32).into());
        return;
    }
    if !css_rule_text_is_insertable_with_selector_context_and_rule_context(
        &parsed.rule,
        &selector_context,
        style_rule_context,
    ) {
        webidl::throw_dom_exception(scope, "SyntaxError", "Invalid CSS rule.");
        return;
    }
    if matches!(
        css_rule_text_order_kind(&parsed.rule),
        CssRuleOrderKind::Import | CssRuleOrderKind::Namespace
    ) {
        webidl::throw_dom_exception(
            scope,
            "HierarchyRequestError",
            "CSS rule cannot be inserted into this grouping rule.",
        );
        return;
    }
    insert_css_rule_list_rule_with_selector_context_and_rule_context(
        scope,
        rules,
        parsed.index,
        &parsed.rule,
        parent_style_sheet,
        Some(this),
        &selector_context,
        style_rule_context,
    );
    if is_style_rule {
        sync_css_style_rule_style_text_from_nested_rules(scope, this, rules);
    } else {
        sync_css_grouping_rule_css_text_from_rules(scope, this, rules);
    }
    rv.set(v8::Integer::new(scope, parsed.index as i32).into());
}

pub(crate) fn css_grouping_rule_delete_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CssGroupingRuleDeleteRuleArgs>(scope, &args) else {
        return;
    };
    let this = args.this();
    let is_style_rule =
        css_rule_current_stylo_rule_type_from_object(scope, this) == Some(CssRuleType::Style);
    let rules = if is_style_rule {
        css_style_rule_rules_array(scope, this)
    } else {
        css_grouping_rule_rules_array(scope, this)
    };
    if parsed.index >= css_rule_list_length(scope, rules) {
        webidl::throw_index_size_error(scope);
        return;
    }
    let parent_style_sheet = get_private_value(scope, this, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let selector_context = parent_style_sheet
        .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
        .unwrap_or_default();
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(this));
    if let Some(containing_rule_type_bits) =
        stylo_containing_rule_type_bits_for_parent_rule(scope, this)
    {
        let attached_rule = parent_style_sheet.and_then(|sheet| {
            css_rule_attached_native_path(scope, this, sheet).map(|path| (sheet, path))
        });
        if let Some((sheet, path)) = attached_rule {
            if let Err(error) = apply_live_stylesheet_nested_rule_delete_mutation(
                scope,
                rules,
                parsed.index,
                sheet,
                &path,
            ) {
                throw_delete_rule_error(scope, error);
                return;
            }
            rv.set_undefined();
            return;
        }

        if let Err(error) = apply_detached_nested_rule_delete_mutation(
            scope,
            this,
            rules,
            parsed.index,
            parent_style_sheet,
            &selector_context,
            style_rule_context,
            containing_rule_type_bits,
        ) {
            throw_delete_rule_error(scope, error);
            return;
        }
        if is_style_rule {
            sync_css_style_rule_style_text_from_nested_rules(scope, this, rules);
        } else {
            sync_css_grouping_rule_css_text_from_rules(scope, this, rules);
        }
        rv.set_undefined();
        return;
    }
    delete_css_rule_list_rule(scope, rules, parsed.index);
    if is_style_rule {
        sync_css_style_rule_style_text_from_nested_rules(scope, this, rules);
    } else {
        sync_css_grouping_rule_css_text_from_rules(scope, this, rules);
    }
    rv.set_undefined();
}

pub(crate) fn css_keyframes_rule_append_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CssKeyframesRuleAppendRuleArgs>(scope, &args) else {
        return;
    };
    let this = css_rule_receiver(scope, &args);
    let rules = css_keyframes_rule_rules_array(scope, this);
    let parent_style_sheet = get_private_value(scope, this, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let index = css_rule_list_length(scope, rules);
    let attached_rule = parent_style_sheet.and_then(|sheet| {
        css_rule_attached_native_path(scope, this, sheet).map(|path| (sheet, path))
    });
    let result = if let Some((sheet, path)) = attached_rule {
        apply_live_stylesheet_keyframe_rule_insert_mutation(
            scope,
            rules,
            &parsed.rule,
            index,
            sheet,
            &path,
        )
    } else {
        let result = apply_detached_keyframe_rule_insert_mutation(
            scope,
            this,
            rules,
            &parsed.rule,
            index,
            parent_style_sheet,
        );
        if result.is_ok() {
            sync_css_keyframes_rule_css_text_from_rules(scope, this, rules);
        }
        result
    };
    match result {
        // Unlike CSSStyleSheet.insertRule, appendRule ignores invalid CSS.
        Ok(()) | Err(CssRuleInsertError::Syntax) => rv.set_undefined(),
        Err(error) => throw_insert_rule_error(scope, error),
    }
}

pub(crate) fn css_keyframes_rule_delete_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CssKeyframesRuleDeleteRuleArgs>(scope, &args) else {
        return;
    };
    let this = css_rule_receiver(scope, &args);
    let rules = css_keyframes_rule_rules_array(scope, this);
    if let Some(index) = find_css_keyframe_rule_index(scope, this, rules, &parsed.key) {
        let parent_style_sheet = get_private_value(scope, this, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        let attached_rule = parent_style_sheet.and_then(|sheet| {
            css_rule_attached_native_path(scope, this, sheet).map(|path| (sheet, path))
        });
        if let Some((sheet, path)) = attached_rule {
            if let Err(error) = apply_live_stylesheet_keyframe_rule_delete_mutation(
                scope, rules, index, sheet, &path,
            ) {
                throw_delete_rule_error(scope, error);
                return;
            }
            rv.set_undefined();
            return;
        }

        if let Err(error) = apply_detached_keyframe_rule_delete_mutation(
            scope,
            this,
            rules,
            index,
            parent_style_sheet,
        ) {
            throw_delete_rule_error(scope, error);
            return;
        }
        sync_css_keyframes_rule_css_text_from_rules(scope, this, rules);
        rv.set_undefined();
        return;
    }
    rv.set_undefined();
}

pub(crate) fn css_keyframes_rule_find_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CssKeyframesRuleFindRuleArgs>(scope, &args) else {
        return;
    };
    let this = css_rule_receiver(scope, &args);
    let rules = css_keyframes_rule_rules_array(scope, this);
    if let Some(index) = find_css_keyframe_rule_index(scope, this, rules, &parsed.key)
        && let Some(rule) = css_rule_list_item(scope, rules, index)
    {
        rv.set(rule.into());
        return;
    }
    rv.set(v8::null(scope).into());
}

pub(crate) fn serialized_grouping_rule_text(
    kind: CssAtRuleKind,
    prelude: &str,
    block: &str,
) -> String {
    let at_keyword = css_at_rule_keyword(kind);
    let header = if prelude.is_empty() {
        format!("@{at_keyword}")
    } else {
        format!("@{at_keyword} {prelude}")
    };
    if let Some(css_text) =
        canonical_grouping_rule_text_with_stylo(&format!("{header} {{ {block} }}"), kind)
    {
        return css_text;
    }
    let nested_rules = top_level_rule_texts_from_stylo_snapshots(block);
    if nested_rules.is_empty() {
        return format!("{header} {{\n}}");
    }
    let nested = nested_rules
        .into_iter()
        .map(|rule| format!("  {}", rule))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{header} {{\n{nested}\n}}")
}

pub(crate) fn nested_grouping_rule_text_from_snapshot(
    rule_snapshot: &CssRuleSnapshot,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<String> {
    let parts = css_rule_snapshot_child_container_parts(rule_snapshot)?;
    let block = nested_at_rule_block_text_from_snapshot(
        rule_snapshot,
        selector_context,
        style_rule_context,
    )?;
    Some(format_nested_grouping_rule_text(
        parts.kind,
        &parts.prelude,
        &block,
    ))
}

pub(crate) fn format_nested_grouping_rule_text(
    kind: CssAtRuleKind,
    prelude: &str,
    block: &str,
) -> String {
    let at_keyword = css_at_rule_keyword(kind);
    let header = if prelude.is_empty() {
        format!("@{at_keyword}")
    } else {
        format!("@{at_keyword} {prelude}")
    };
    if block.is_empty() {
        return format!("{header} {{\n}}");
    }
    let nested = block
        .split('\n')
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{header} {{\n{nested}\n}}")
}

pub(crate) fn serialized_nested_grouping_rule_text(
    kind: CssAtRuleKind,
    prelude: &str,
    block: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> String {
    let child_context = nested_grouping_rule_child_selector_context(kind, style_rule_context);
    let block = if block.contains('{') || block.contains('@') {
        canonical_nested_grouping_rule_block_text_with_stylo(
            kind,
            block,
            selector_context,
            child_context,
        )
        .unwrap_or_else(|| block.to_owned())
    } else {
        block.to_owned()
    };
    format_nested_grouping_rule_text(kind, prelude, &block)
}

pub(crate) fn nested_grouping_rule_child_selector_context(
    kind: CssAtRuleKind,
    style_rule_context: StyleRuleSelectorContext,
) -> StyleRuleSelectorContext {
    if kind == CssAtRuleKind::Scope {
        StyleRuleSelectorContext::Scope
    } else {
        style_rule_context
    }
}

pub(crate) fn canonical_nested_grouping_rule_block_text_with_stylo(
    kind: CssAtRuleKind,
    block: &str,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<String> {
    let rule_type = css_at_rule_stylo_rule_type(kind);
    let mutation = css_nested_rule_block_with_selector_context(
        selector_context,
        block,
        rule_type,
        nested_rule_block_containing_type_bits(rule_type, style_rule_context),
        style_rule_context,
        false,
    )?;
    Some(nested_rule_block_text_from_stylo_mutation(mutation))
}

pub(crate) fn nested_rule_block_containing_type_bits(
    rule_type: CssRuleType,
    style_rule_context: StyleRuleSelectorContext,
) -> u32 {
    let mut bits = rule_type.bit();
    match style_rule_context {
        StyleRuleSelectorContext::TopLevel => {}
        StyleRuleSelectorContext::Nested => {
            bits |= CssRuleType::Style.bit();
        }
        StyleRuleSelectorContext::Scope => {
            bits |= CssRuleType::Scope.bit();
        }
    }
    bits
}

pub(crate) fn serialize_nested_style_rule_css_text_from_block(
    selector: &str,
    block: &str,
) -> String {
    let block = block.trim();
    if block.is_empty() {
        return format!("{selector} {{ }}");
    }
    let children = block
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            if line.starts_with(' ') || line == "}" {
                line.to_owned()
            } else {
                format!("  {line}")
            }
        })
        .collect::<Vec<_>>();
    format!("{selector} {{\n{}\n}}", children.join("\n"))
}

pub(crate) fn css_keyframes_rule_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let this = css_rule_receiver(scope, &args);
    let name = css_keyframes_rule_current_name(scope, this).unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &name));
}

pub(crate) fn css_keyframes_rule_name_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(name) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSKeyframesRule", "name")
    else {
        return;
    };
    let this = css_rule_receiver(scope, &args);
    if apply_live_stylesheet_keyframes_rule_name_mutation(scope, this, &name) {
        return;
    }
    let rules = css_keyframes_rule_rules_array(scope, this);
    let nested = css_rule_list_css_text(scope, rules);
    let name = serialize_keyframes_name(&name);
    let css_text = format!("@keyframes {name} {{ {nested} }}");
    let _ = commit_detached_css_rule_snapshot_text(scope, this, &css_text, false);
}
