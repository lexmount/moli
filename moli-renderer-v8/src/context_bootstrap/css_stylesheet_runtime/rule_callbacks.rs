use super::*;

#[cfg(test)]
thread_local! {
    static CSS_RULE_WRAPPER_CONSTRUCTION_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_css_rule_wrapper_construction_count_for_test() {
    CSS_RULE_WRAPPER_CONSTRUCTION_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn css_rule_wrapper_construction_count_for_test() -> usize {
    CSS_RULE_WRAPPER_CONSTRUCTION_COUNT.with(std::cell::Cell::get)
}

pub(crate) fn note_css_rule_wrapper_construction() {
    #[cfg(test)]
    CSS_RULE_WRAPPER_CONSTRUCTION_COUNT.with(|count| count.set(count.get() + 1));
}

pub(crate) fn insert_css_rule_list_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    index: u32,
    rule: v8::Local<'s, v8::Object>,
) {
    insert_css_rule_list_unmaterialized_rule(scope, rules, index);
    set_css_rule_list_materialized_rule(scope, rules, index, rule);
}

pub(crate) fn css_at_rule_stylo_rule_type(kind: CssAtRuleKind) -> CssRuleType {
    match kind {
        CssAtRuleKind::Import => CssRuleType::Import,
        CssAtRuleKind::Media => CssRuleType::Media,
        CssAtRuleKind::FontFace => CssRuleType::FontFace,
        CssAtRuleKind::FontFeatureValues => CssRuleType::FontFeatureValues,
        CssAtRuleKind::Keyframes => CssRuleType::Keyframes,
        CssAtRuleKind::Page => CssRuleType::Page,
        CssAtRuleKind::Namespace => CssRuleType::Namespace,
        CssAtRuleKind::CounterStyle => CssRuleType::CounterStyle,
        CssAtRuleKind::Supports => CssRuleType::Supports,
        CssAtRuleKind::Layer => CssRuleType::LayerBlock,
        CssAtRuleKind::Container => CssRuleType::Container,
        CssAtRuleKind::Scope => CssRuleType::Scope,
        CssAtRuleKind::StartingStyle => CssRuleType::StartingStyle,
        CssAtRuleKind::Property => CssRuleType::Property,
        CssAtRuleKind::Unknown | CssAtRuleKind::Function => CssRuleType::LayerStatement,
    }
}

pub(crate) fn css_at_rule_kind_for_stylo_rule_type(
    rule_type: CssRuleType,
) -> Option<CssAtRuleKind> {
    match rule_type {
        CssRuleType::Import => Some(CssAtRuleKind::Import),
        CssRuleType::Media => Some(CssAtRuleKind::Media),
        CssRuleType::FontFace => Some(CssAtRuleKind::FontFace),
        CssRuleType::FontFeatureValues => Some(CssAtRuleKind::FontFeatureValues),
        CssRuleType::Keyframes => Some(CssAtRuleKind::Keyframes),
        CssRuleType::Page => Some(CssAtRuleKind::Page),
        CssRuleType::Namespace => Some(CssAtRuleKind::Namespace),
        CssRuleType::CounterStyle => Some(CssAtRuleKind::CounterStyle),
        CssRuleType::Supports => Some(CssAtRuleKind::Supports),
        CssRuleType::LayerBlock | CssRuleType::LayerStatement => Some(CssAtRuleKind::Layer),
        CssRuleType::Container => Some(CssAtRuleKind::Container),
        CssRuleType::Scope => Some(CssAtRuleKind::Scope),
        CssRuleType::StartingStyle => Some(CssAtRuleKind::StartingStyle),
        CssRuleType::Property => Some(CssAtRuleKind::Property),
        _ => None,
    }
}

pub(crate) fn css_rule_snapshot_child_container_parts(
    view: &CssRuleSnapshot,
) -> Option<CssGroupingRuleTextParts> {
    let kind = css_at_rule_kind_for_stylo_rule_type(view.rule_type)?;
    let prelude = match kind {
        CssAtRuleKind::Media
        | CssAtRuleKind::Supports
        | CssAtRuleKind::Scope
        | CssAtRuleKind::Container
        | CssAtRuleKind::Layer
        | CssAtRuleKind::Keyframes => view.prelude_text.clone()?,
        CssAtRuleKind::Page => view.selector_text.clone().unwrap_or_default(),
        CssAtRuleKind::StartingStyle => String::new(),
        _ => return None,
    };
    Some(CssGroupingRuleTextParts { kind, prelude })
}

pub(crate) fn css_rule_snapshot_child_block_text(rule_snapshots: &[CssRuleSnapshot]) -> String {
    rule_snapshots
        .iter()
        .map(|rule| rule.css_text.trim())
        .filter(|css_text| !css_text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn css_rule_nested_rules_slot_for_stylo_rule_type(
    rule_type: CssRuleType,
) -> Option<&'static str> {
    match rule_type {
        CssRuleType::Keyframes => Some(CSS_KEYFRAMES_RULE_RULES_SLOT),
        CssRuleType::Style => Some(CSS_STYLE_RULE_NESTED_RULES_SLOT),
        CssRuleType::Media
        | CssRuleType::Supports
        | CssRuleType::Container
        | CssRuleType::Scope
        | CssRuleType::LayerBlock
        | CssRuleType::StartingStyle
        | CssRuleType::Page => Some(CSS_AT_RULE_NESTED_RULES_SLOT),
        _ => None,
    }
}

pub(crate) fn build_detached_css_rule_object_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: &CssRuleSnapshot,
    child_snapshots: v8::Local<'s, v8::Array>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> v8::Local<'s, v8::Object> {
    note_css_rule_wrapper_construction();

    let rule = if snapshot.rule_type == CssRuleType::NestedDeclarations
        && let Some(parent_rule) = parent_rule
    {
        build_css_nested_declarations_rule_object(
            scope,
            snapshot
                .declaration_text
                .as_deref()
                .unwrap_or(&snapshot.css_text),
            parent_style_sheet,
            parent_rule,
        )
    } else if snapshot.rule_type == CssRuleType::Style
        && let Some(selector_text) = snapshot.selector_text.clone()
    {
        let declaration_text = snapshot.declaration_text.as_deref().unwrap_or_default();
        let child_text = detached_css_rule_snapshot_array_texts(scope, child_snapshots).join("\n");
        let style_text = join_css_rule_blocks(declaration_text, &child_text);
        build_css_style_rule_object_from_stylo_view(
            scope,
            CssStyleRuleTextParts {
                css_text: snapshot.css_text.clone(),
                selector_text,
                style_text,
            },
            declaration_text,
            parent_style_sheet,
            parent_rule,
        )
    } else if snapshot.rule_type == CssRuleType::Keyframe
        && let Some(parsed) = keyframe_rule_text_from_snapshot(snapshot)
    {
        build_css_keyframe_rule_object(scope, parsed, parent_style_sheet, parent_rule)
    } else {
        match snapshot.rule_type {
            CssRuleType::Import
            | CssRuleType::Namespace
            | CssRuleType::Media
            | CssRuleType::Supports
            | CssRuleType::Container
            | CssRuleType::Scope
            | CssRuleType::LayerBlock
            | CssRuleType::LayerStatement
            | CssRuleType::StartingStyle
            | CssRuleType::Keyframes
            | CssRuleType::Page
            | CssRuleType::CounterStyle
            | CssRuleType::Document
            | CssRuleType::FontPaletteValues
            | CssRuleType::PositionTry
            | CssRuleType::CustomMedia
            | CssRuleType::AppearanceBase
            | CssRuleType::ViewTransition => build_css_generic_at_rule_object_from_snapshot(
                scope,
                snapshot,
                parent_style_sheet,
                parent_rule,
                selector_context,
                style_rule_context,
            ),
            CssRuleType::Style
            | CssRuleType::FontFace
            | CssRuleType::Keyframe
            | CssRuleType::Margin
            | CssRuleType::FontFeatureValues
            | CssRuleType::Property
            | CssRuleType::NestedDeclarations => build_css_rule_object_with_rule_context(
                scope,
                &snapshot.css_text,
                parent_style_sheet,
                parent_rule,
                selector_context,
                style_rule_context,
            ),
        }
    };
    let _ = set_detached_css_rule_snapshot_text(scope, rule, &snapshot.css_text);
    set_detached_css_rule_child_snapshot_array(scope, rule, child_snapshots);
    rule
}

pub(crate) fn build_css_generic_at_rule_object_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule_snapshot: &CssRuleSnapshot,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> v8::Local<'s, v8::Object> {
    let object = build_css_generic_at_rule_object_from_stylo_rule_type(
        scope,
        rule_snapshot.rule_type,
        rule_snapshot.css_text.clone(),
        parent_style_sheet,
        parent_rule,
    );
    if style_rule_context != StyleRuleSelectorContext::TopLevel
        && let Some(block) = nested_at_rule_block_text_from_snapshot(
            rule_snapshot,
            selector_context,
            style_rule_context,
        )
    {
        set_private_string(scope, object, CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT, &block);
    }
    object
}

pub(crate) fn build_css_generic_at_rule_object_from_stylo_rule_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stylo_rule_type: CssRuleType,
    css_text: String,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Object> {
    let prototype_name = css_at_rule_prototype_name_for_stylo_rule_type(stylo_rule_type);
    let rule_type = css_rule_type_from_stylo_rule_type(stylo_rule_type);
    let declaration = CssAtRuleDeclaration {
        css_text,
        rule_type,
        parent_rule,
        parent_style_sheet,
    };
    let object = bind_css_at_rule_declaration(
        scope,
        declaration,
        stylo_rule_type == CssRuleType::Keyframes,
    );
    moli_webapi_declare::initialize_web_api_object(scope, object, prototype_name)
        .expect("CSS rule interface identity should initialize");
    if let Some(prototype) = global_constructor_prototype(scope, prototype_name) {
        let _ = object.set_prototype(scope, prototype.into());
    }
    object
}

pub(crate) fn css_rule_list_rule_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    css_rule_list_materialized_entries(scope, rules)
        .into_iter()
        .find_map(|(index, candidate)| candidate.strict_equals(rule.into()).then_some(index))
}

pub(crate) fn sync_local_css_at_rule_wrapper_slots_from_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    css_text: &str,
) {
    set_private_u32(
        scope,
        rule,
        CSS_AT_RULE_TYPE_SLOT,
        css_at_rule_type(css_at_rule_kind(css_text)),
    );
    if get_private_value(scope, rule, CSS_PROPERTY_RULE_NAME_SLOT).is_some() {
        if let Some(view) = parse_property_rule_view_with_stylo(css_text) {
            sync_css_property_rule_slots_from_stylo_view(scope, rule, &view);
        }
    } else if get_private_value(scope, rule, CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT)
        .is_some()
    {
        sync_css_font_feature_values_rule_slots_from_css_text(scope, rule, css_text);
    }

    if let Some(style) = get_private_value(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        && let Some(rule_type) = css_text_single_stylo_rule_type(css_text)
    {
        let style_text = match rule_type {
            CssRuleType::FontFace => css_font_face_rule_style_text_from_css_text(css_text),
            CssRuleType::Page => css_page_rule_style_text_from_css_text(css_text),
            _ => None,
        };
        if let Some(style_text) = style_text {
            set_style_object_css_text_without_notify(scope, style, &style_text);
        }
    }
}

pub(crate) fn stylo_rule_type_for_css_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssRuleType> {
    css_rule_current_stylo_rule_type_from_object(scope, rule)
        .and_then(stylo_containing_rule_type_for_stylo_rule_type)
}

pub(crate) fn delete_css_rule_list_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    index: u32,
) {
    let rules_len = css_rule_list_length(scope, rules);
    for (existing_index, rule) in css_rule_list_materialized_entries(scope, rules) {
        match existing_index.cmp(&index) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => {
                detach_css_rule_from_parent(scope, rule);
                delete_css_rule_list_materialized_rule(scope, rules, existing_index);
            }
            std::cmp::Ordering::Greater => {
                set_css_rule_list_materialized_rule(scope, rules, existing_index - 1, rule);
                delete_css_rule_list_materialized_rule(scope, rules, existing_index);
            }
        }
    }
    set_css_rule_list_length(scope, rules, rules_len - 1);
}

pub(crate) fn detach_css_rule_from_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) {
    // A removed CSSRule remains script-observable and mutable. Freeze native
    // state into detached snapshots before releasing its materialized leases.
    freeze_css_rule_wrapper_from_retained_native_binding(scope, rule);
    release_css_rule_subtree_native_bindings(scope, rule, true);
    set_private_value(
        scope,
        rule,
        CSS_RULE_PARENT_RULE_SLOT,
        v8::null(scope).into(),
    );
    set_private_value(
        scope,
        rule,
        CSS_RULE_PARENT_STYLE_SHEET_SLOT,
        v8::null(scope).into(),
    );
    if let Some(sheet) = get_private_value(scope, rule, CSS_IMPORT_RULE_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_private_value(
            scope,
            sheet,
            CSS_STYLE_SHEET_OWNER_RULE_SLOT,
            v8::null(scope).into(),
        );
    }
}

fn release_css_rule_subtree_native_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    clear_parent_style_sheet: bool,
) {
    let mut pending = vec![root];
    while let Some(rule) = pending.pop() {
        if let Some(rule_type) = css_rule_current_stylo_rule_type_from_object(scope, rule)
            && let Some(slot) = css_rule_nested_rules_slot_for_stylo_rule_type(rule_type)
            && let Some(rules) = get_private_value(scope, rule, slot)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            if clear_parent_style_sheet {
                bind_css_rule_list_to_parent(scope, rules, None, Some(rule));
            }
            for (_, child) in css_rule_list_materialized_entries(scope, rules) {
                pending.push(child);
            }
        }
        detach_css_rule_object_from_native_stylesheet(scope, rule);
        if clear_parent_style_sheet {
            set_private_value(
                scope,
                rule,
                CSS_RULE_PARENT_STYLE_SHEET_SLOT,
                v8::null(scope).into(),
            );
        }
    }
}

pub(crate) fn retire_css_rule_list_for_stylesheet_replacement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
) {
    for (_, rule) in css_rule_list_materialized_entries(scope, rules) {
        freeze_css_rule_wrapper_from_retained_native_binding(scope, rule);
        release_css_rule_subtree_native_bindings(scope, rule, false);
    }
    reset_css_rule_list_materialized_items(scope, rules);
    set_css_rule_list_length(scope, rules, 0);
}

pub(crate) fn css_rule_text_order_kind(css_text: &str) -> CssRuleOrderKind {
    match css_text_single_stylo_rule_type(css_text) {
        Some(CssRuleType::Import) => return CssRuleOrderKind::Import,
        Some(CssRuleType::Namespace) => return CssRuleOrderKind::Namespace,
        Some(_) => return CssRuleOrderKind::Other,
        None => {}
    }
    let trimmed = css_text.trim_start();
    if css_text_starts_with_at_keyword(trimmed, "import") {
        CssRuleOrderKind::Import
    } else if css_text_starts_with_at_keyword(trimmed, "namespace") {
        CssRuleOrderKind::Namespace
    } else {
        CssRuleOrderKind::Other
    }
}

pub(crate) fn ensure_css_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
) -> bool {
    if moli_webapi_declare::implements_interface(scope, object, "CSSRule") {
        return true;
    }
    throw_type_error(
        scope,
        &format!("Failed to get '{member}' on '{interface}': Illegal invocation."),
    );
    false
}

pub(crate) fn ensure_css_rule_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &str,
    member: &str,
    expected_type: u32,
) -> bool {
    if moli_webapi_declare::implements_interface(scope, object, "CSSRule")
        && css_rule_type_from_object(scope, object) == Some(expected_type)
    {
        return true;
    }
    throw_type_error(
        scope,
        &format!("Failed to get '{member}' on '{interface}': Illegal invocation."),
    );
    false
}

pub(crate) fn build_css_rule_object_with_rule_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    css_text: &str,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> v8::Local<'s, v8::Object> {
    if let Some(parsed) = parse_valid_style_rule_text_with_selector_context_and_rule_context(
        css_text,
        selector_context,
        style_rule_context,
    ) {
        return build_css_style_rule_object(scope, parsed, parent_style_sheet, parent_rule);
    }
    if let Some(view) = css_font_face_rule_view_from_css_text(css_text) {
        return build_css_font_face_rule_object_from_stylo_view(
            scope,
            view,
            parent_style_sheet,
            parent_rule,
        );
    }
    if let Some(view) = css_font_feature_values_rule_view_from_css_text(css_text) {
        return build_css_font_feature_values_rule_object_from_stylo_view(
            scope,
            view,
            parent_style_sheet,
            parent_rule,
        );
    }
    if let Some(view) = parse_property_rule_view_with_stylo(css_text) {
        return build_css_property_rule_object_from_stylo_view(
            scope,
            view,
            parent_style_sheet,
            parent_rule,
        );
    }
    if parent_rule_is_page_rule(scope, parent_rule)
        && let Some(view) = css_page_margin_rule_view_from_css_text(css_text)
    {
        return build_css_margin_rule_object_from_stylo_view(
            scope,
            view,
            parent_style_sheet,
            parent_rule,
        );
    }
    let rule_snapshot = (style_rule_context == StyleRuleSelectorContext::TopLevel)
        .then(|| css_text_single_rule_snapshot(css_text))
        .flatten();
    let stylo_rule_type = rule_snapshot.as_ref().map(|view| view.rule_type);
    let rule_kind = stylo_rule_type
        .and_then(css_at_rule_kind_for_stylo_rule_type)
        .unwrap_or_else(|| css_at_rule_kind(css_text));
    let css_text = if style_rule_context != StyleRuleSelectorContext::TopLevel {
        canonical_nested_grouping_rule_text_with_context(
            css_text,
            selector_context,
            style_rule_context,
        )
    } else if let Some(rule_snapshot) = rule_snapshot {
        rule_snapshot.css_text
    } else {
        canonical_css_at_rule_text(css_text, rule_kind)
    };
    let prototype_name = stylo_rule_type
        .or_else(|| css_text_single_stylo_rule_type(&css_text))
        .map(css_at_rule_prototype_name_for_stylo_rule_type)
        .unwrap_or_else(|| css_at_rule_prototype_name(rule_kind));
    let rule_type = stylo_rule_type
        .map(css_rule_type_from_stylo_rule_type)
        .filter(|rule_type| *rule_type != CSS_RULE_UNKNOWN_RULE_TYPE)
        .unwrap_or_else(|| css_at_rule_type(rule_kind));
    let declaration = CssAtRuleDeclaration {
        css_text,
        rule_type,
        parent_rule,
        parent_style_sheet,
    };
    let object =
        bind_css_at_rule_declaration(scope, declaration, rule_kind == CssAtRuleKind::Keyframes);
    moli_webapi_declare::initialize_web_api_object(scope, object, prototype_name)
        .expect("CSS rule interface identity should initialize");
    if let Some(prototype) = global_constructor_prototype(scope, prototype_name) {
        let _ = object.set_prototype(scope, prototype.into());
    }
    if style_rule_context != StyleRuleSelectorContext::TopLevel {
        sync_nested_at_rule_style_text_slot(scope, object, selector_context, style_rule_context);
    }
    object
}

fn bind_css_at_rule_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    declaration: CssAtRuleDeclaration<'s>,
    indexed_keyframes: bool,
) -> v8::Local<'s, v8::Object> {
    if !indexed_keyframes {
        return declaration
            .bind(scope)
            .expect("CSS at-rule declaration should bind");
    }
    let object = new_css_keyframes_rule_object(scope);
    declaration
        .bind_into(scope, object)
        .expect("CSSKeyframesRule declaration should bind into indexed wrapper");
    object
}

pub(crate) fn css_at_rule_kind(css_text: &str) -> CssAtRuleKind {
    css_text_single_stylo_rule_type(css_text)
        .and_then(css_at_rule_kind_for_stylo_rule_type)
        .or_else(|| {
            single_custom_css_function_projection(css_text).map(|_| CssAtRuleKind::Function)
        })
        .unwrap_or(CssAtRuleKind::Unknown)
}

pub(crate) fn css_at_rule_prototype_name_for_stylo_rule_type(
    rule_type: CssRuleType,
) -> &'static str {
    match rule_type {
        CssRuleType::Import => "CSSImportRule",
        CssRuleType::Media => "CSSMediaRule",
        CssRuleType::FontFace => "CSSFontFaceRule",
        CssRuleType::FontFeatureValues => "CSSFontFeatureValuesRule",
        CssRuleType::Keyframes => "CSSKeyframesRule",
        CssRuleType::Page => "CSSPageRule",
        CssRuleType::Namespace => "CSSNamespaceRule",
        CssRuleType::CounterStyle => "CSSCounterStyleRule",
        CssRuleType::Supports => "CSSSupportsRule",
        CssRuleType::LayerBlock => "CSSLayerBlockRule",
        CssRuleType::LayerStatement => "CSSLayerStatementRule",
        CssRuleType::Container => "CSSContainerRule",
        CssRuleType::Scope => "CSSScopeRule",
        _ => "CSSRule",
    }
}

pub(crate) fn css_at_rule_prototype_name(kind: CssAtRuleKind) -> &'static str {
    match kind {
        CssAtRuleKind::Unknown => "CSSRule",
        CssAtRuleKind::Layer => "CSSLayerStatementRule",
        CssAtRuleKind::Import => "CSSImportRule",
        CssAtRuleKind::Media => "CSSMediaRule",
        CssAtRuleKind::Scope => "CSSScopeRule",
        CssAtRuleKind::FontFace => "CSSFontFaceRule",
        CssAtRuleKind::FontFeatureValues => "CSSFontFeatureValuesRule",
        CssAtRuleKind::Keyframes => "CSSKeyframesRule",
        CssAtRuleKind::Page => "CSSPageRule",
        CssAtRuleKind::Namespace => "CSSNamespaceRule",
        CssAtRuleKind::CounterStyle => "CSSCounterStyleRule",
        CssAtRuleKind::Supports => "CSSSupportsRule",
        CssAtRuleKind::Container => "CSSContainerRule",
        CssAtRuleKind::StartingStyle | CssAtRuleKind::Property | CssAtRuleKind::Function => {
            "CSSRule"
        }
    }
}

pub(crate) fn css_at_rule_type(kind: CssAtRuleKind) -> u32 {
    css_rule_type_from_stylo_rule_type(css_at_rule_stylo_rule_type(kind))
}

pub(crate) fn css_at_rule_keyword(kind: CssAtRuleKind) -> &'static str {
    match kind {
        CssAtRuleKind::Unknown => "unknown",
        CssAtRuleKind::Layer => "layer",
        CssAtRuleKind::Import => "import",
        CssAtRuleKind::Media => "media",
        CssAtRuleKind::Scope => "scope",
        CssAtRuleKind::FontFace => "font-face",
        CssAtRuleKind::FontFeatureValues => "font-feature-values",
        CssAtRuleKind::Keyframes => "keyframes",
        CssAtRuleKind::Page => "page",
        CssAtRuleKind::Namespace => "namespace",
        CssAtRuleKind::CounterStyle => "counter-style",
        CssAtRuleKind::Supports => "supports",
        CssAtRuleKind::Container => "container",
        CssAtRuleKind::StartingStyle => "starting-style",
        CssAtRuleKind::Property => "property",
        CssAtRuleKind::Function => "function",
    }
}

pub(crate) fn css_condition_rule_read_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::LiveStylesheetConditionRuleRead> {
    if let Some(read) = css_rule_attached_native_condition_read(scope, object) {
        return Some(read);
    }
    css_rule_detached_snapshot_typed_view(scope, object, parse_condition_rule_view_with_stylo).map(
        |view| crate::live_stylesheet::LiveStylesheetConditionRuleRead {
            rule_type: view.rule_type,
            condition_text: view.condition_text,
            container_name: view.container_name,
            container_query: view.container_query,
            scope_start: view.scope_start,
            scope_end: view.scope_end,
        },
    )
}

pub(crate) fn css_layer_rule_read_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::LiveStylesheetLayerRuleRead> {
    if let Some(read) = css_rule_attached_native_layer_read(scope, object) {
        return Some(read);
    }
    css_rule_detached_snapshot_typed_view(scope, object, parse_layer_rule_view_with_stylo).map(
        |view| crate::live_stylesheet::LiveStylesheetLayerRuleRead {
            rule_type: view.rule_type,
            name: view.name,
            names: view.names,
        },
    )
}

pub(crate) fn css_rule_current_stylo_rule_type_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CssRuleType> {
    if let Some(rule_type) = css_rule_attached_native_rule_type(scope, object) {
        return Some(rule_type);
    }
    if get_private_value(scope, object, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT).is_some() {
        return Some(CssRuleType::Keyframe);
    }
    if get_private_value(scope, object, CSS_MARGIN_RULE_NAME_SLOT).is_some() {
        return Some(CssRuleType::Margin);
    }
    if get_private_value(scope, object, CSS_STYLE_RULE_SELECTOR_TEXT_SLOT).is_some() {
        return Some(CssRuleType::Style);
    }
    if get_private_value(scope, object, CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT).is_some() {
        return Some(CssRuleType::NestedDeclarations);
    }
    if get_private_value(scope, object, CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT).is_some() {
        return Some(CssRuleType::FontFeatureValues);
    }
    if get_private_value(scope, object, CSS_PROPERTY_RULE_NAME_SLOT).is_some() {
        return Some(CssRuleType::Property);
    }
    if let Some(rule_type) =
        private_u32(scope, object, CSS_AT_RULE_TYPE_SLOT).and_then(css_rule_type_to_stylo_rule_type)
    {
        return Some(rule_type);
    }
    None
}

pub(crate) fn css_rule_type_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    if let Some(rule_type) = css_rule_current_stylo_rule_type_from_object(scope, object) {
        let css_rule_type = css_rule_type_from_stylo_rule_type(rule_type);
        if css_rule_type != CSS_RULE_UNKNOWN_RULE_TYPE {
            return Some(css_rule_type);
        }
    }
    moli_webapi_declare::implements_interface(scope, object, "CSSRule").then_some(())?;
    Some(private_u32(scope, object, CSS_AT_RULE_TYPE_SLOT).unwrap_or(CSS_RULE_UNKNOWN_RULE_TYPE))
}

pub(crate) fn css_rule_type_to_stylo_rule_type(rule_type: u32) -> Option<CssRuleType> {
    match rule_type {
        CSS_RULE_STYLE_RULE_TYPE => Some(CssRuleType::Style),
        CSS_RULE_IMPORT_RULE_TYPE => Some(CssRuleType::Import),
        CSS_RULE_MEDIA_RULE_TYPE => Some(CssRuleType::Media),
        CSS_RULE_FONT_FACE_RULE_TYPE => Some(CssRuleType::FontFace),
        CSS_RULE_PAGE_RULE_TYPE => Some(CssRuleType::Page),
        CSS_RULE_KEYFRAMES_RULE_TYPE => Some(CssRuleType::Keyframes),
        CSS_RULE_KEYFRAME_RULE_TYPE => Some(CssRuleType::Keyframe),
        CSS_RULE_MARGIN_RULE_TYPE => Some(CssRuleType::Margin),
        CSS_RULE_NAMESPACE_RULE_TYPE => Some(CssRuleType::Namespace),
        CSS_RULE_COUNTER_STYLE_RULE_TYPE => Some(CssRuleType::CounterStyle),
        CSS_RULE_SUPPORTS_RULE_TYPE => Some(CssRuleType::Supports),
        CSS_RULE_FONT_FEATURE_VALUES_RULE_TYPE => Some(CssRuleType::FontFeatureValues),
        CSS_RULE_CONTAINER_RULE_TYPE => Some(CssRuleType::Container),
        _ => None,
    }
}

pub(crate) fn css_rule_type_from_stylo_rule_type(rule_type: CssRuleType) -> u32 {
    match rule_type {
        CssRuleType::Style => CSS_RULE_STYLE_RULE_TYPE,
        CssRuleType::Import => CSS_RULE_IMPORT_RULE_TYPE,
        CssRuleType::Media => CSS_RULE_MEDIA_RULE_TYPE,
        CssRuleType::FontFace => CSS_RULE_FONT_FACE_RULE_TYPE,
        CssRuleType::Page => CSS_RULE_PAGE_RULE_TYPE,
        CssRuleType::Keyframes => CSS_RULE_KEYFRAMES_RULE_TYPE,
        CssRuleType::Keyframe => CSS_RULE_KEYFRAME_RULE_TYPE,
        CssRuleType::Margin => CSS_RULE_MARGIN_RULE_TYPE,
        CssRuleType::Namespace => CSS_RULE_NAMESPACE_RULE_TYPE,
        CssRuleType::CounterStyle => CSS_RULE_COUNTER_STYLE_RULE_TYPE,
        CssRuleType::Supports => CSS_RULE_SUPPORTS_RULE_TYPE,
        CssRuleType::FontFeatureValues => CSS_RULE_FONT_FEATURE_VALUES_RULE_TYPE,
        CssRuleType::Container => CSS_RULE_CONTAINER_RULE_TYPE,
        _ => CSS_RULE_UNKNOWN_RULE_TYPE,
    }
}

pub(crate) fn css_rule_stylo_declaration_block_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    private_u64(scope, rule, CSS_RULE_STYLO_DECLARATION_BLOCK_ID_SLOT)
}

pub(crate) fn set_css_rule_stylo_declaration_block_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    id: u64,
) {
    set_private_u64(scope, rule, CSS_RULE_STYLO_DECLARATION_BLOCK_ID_SLOT, id);
}

pub(crate) fn set_css_rule_stylo_declaration_block_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    valid: bool,
) {
    let value = v8::Boolean::new(scope, valid);
    set_private_value(
        scope,
        rule,
        CSS_RULE_STYLO_DECLARATION_BLOCK_VALID_SLOT,
        value.into(),
    );
}

pub(crate) fn css_rule_stylo_declaration_block_is_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, rule, CSS_RULE_STYLO_DECLARATION_BLOCK_VALID_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn ensure_css_rule_stylo_declaration_block_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> u64 {
    if let Some(id) = css_rule_stylo_declaration_block_id(scope, rule) {
        return id;
    }
    let id = create_lightweight_css_style_stylo_declaration_block(
        &moli_css_parse::CssDeclarationBlock::default(),
    );
    set_css_rule_stylo_declaration_block_id(scope, rule, id);
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, rule, move || {
        remove_lightweight_css_style_stylo_declaration_block(id)
    });
    id
}

pub(crate) fn attach_css_rule_stylo_declaration_block_to_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    let id = ensure_css_rule_stylo_declaration_block_id(scope, rule);
    let _ = set_lightweight_css_style_stylo_declaration_block_id(scope, style, id);
}

pub(crate) fn css_rule_stylo_declaration_block_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    css_rule_stylo_declaration_block_is_valid(scope, rule)
        .then(|| css_rule_stylo_declaration_block_id(scope, rule))
        .flatten()
        .and_then(lightweight_css_style_stylo_declaration_block_css_text)
}

pub(crate) fn sync_css_rule_stylo_declaration_block_validity_from_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style: v8::Local<'s, v8::Object>,
) {
    let valid = lightweight_css_style_uses_only_stylo_declaration_block(scope, style);
    set_css_rule_stylo_declaration_block_valid(scope, rule, valid);
}

pub(crate) fn seed_css_rule_stylo_declaration_block_from_style_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_text: &str,
    kind: CssRulePdbDeclarationKind,
) {
    let Some(declaration_text) =
        css_rule_declaration_text_for_pdb_for_rule(scope, rule, style_text, kind)
    else {
        set_css_rule_stylo_declaration_block_valid(scope, rule, false);
        return;
    };
    seed_css_rule_stylo_declaration_block_from_declaration_text(
        scope,
        rule,
        &declaration_text,
        kind,
    );
}

pub(crate) fn seed_css_rule_stylo_declaration_block_from_declaration_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    declaration_text: &str,
    kind: CssRulePdbDeclarationKind,
) {
    let id = ensure_css_rule_stylo_declaration_block_id(scope, rule);
    let Some(block) =
        css_rule_pdb_safe_declaration_block_from_declaration_text(declaration_text, kind)
    else {
        set_css_rule_stylo_declaration_block_valid(scope, rule, false);
        return;
    };
    store_lightweight_css_style_stylo_declaration_block(id, &block);
    set_css_rule_stylo_declaration_block_valid(scope, rule, true);
}

pub(crate) fn css_rule_pdb_safe_declaration_block(
    style_text: &str,
    kind: CssRulePdbDeclarationKind,
) -> Option<moli_css_parse::CssDeclarationBlock> {
    let declaration_text = css_rule_declaration_text_for_pdb(style_text, kind)?;
    css_rule_pdb_safe_declaration_block_from_declaration_text(&declaration_text, kind)
}

pub(crate) fn css_rule_pdb_safe_declaration_block_from_declaration_text(
    declaration_text: &str,
    kind: CssRulePdbDeclarationKind,
) -> Option<moli_css_parse::CssDeclarationBlock> {
    if declaration_text.trim().is_empty() {
        return Some(moli_css_parse::CssDeclarationBlock::default());
    }
    let entries = parse_css_declaration_list(declaration_text);
    if entries.is_empty() {
        return None;
    }
    let mut block = moli_css_parse::CssDeclarationBlock::default();
    for entry in entries {
        let pdb_safe = match kind {
            CssRulePdbDeclarationKind::StyleRule
            | CssRulePdbDeclarationKind::NestedDeclarations => {
                lightweight_css_rule_declaration_write_uses_pdb(&entry.name, &entry.value)
            }
            CssRulePdbDeclarationKind::KeyframeRule => {
                lightweight_css_keyframe_declaration_write_uses_pdb(&entry.name, &entry.value)
            }
        };
        if !pdb_safe {
            return None;
        }
        let name = moli_css_parse::canonical_style_property_name(&entry.name);
        let projection = block.set_property_with_projection(&name, &entry.value, entry.priority);
        if projection.set_result == moli_css_parse::CssSetResult::ParseError {
            return None;
        }
    }
    Some(block)
}

pub(crate) fn css_rule_declaration_text_for_pdb(
    style_text: &str,
    kind: CssRulePdbDeclarationKind,
) -> Option<String> {
    match kind {
        CssRulePdbDeclarationKind::StyleRule => {
            css_style_rule_first_declaration_text_with_stylo(style_text)
        }
        CssRulePdbDeclarationKind::KeyframeRule | CssRulePdbDeclarationKind::NestedDeclarations => {
            Some(style_text.to_owned())
        }
    }
}

pub(crate) fn css_rule_declaration_text_for_pdb_for_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_text: &str,
    kind: CssRulePdbDeclarationKind,
) -> Option<String> {
    match kind {
        CssRulePdbDeclarationKind::StyleRule => {
            css_style_rule_first_declaration_text_with_stylo_context(scope, rule, style_text)
        }
        CssRulePdbDeclarationKind::KeyframeRule | CssRulePdbDeclarationKind::NestedDeclarations => {
            Some(style_text.to_owned())
        }
    }
}

pub(crate) fn css_rule_serialized_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> String {
    css_rule_current_css_text(scope, rule)
}

pub(crate) fn css_rule_type_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSRule", "type") {
        return;
    }
    let value = css_rule_type_from_object(scope, args.this()).unwrap_or(CSS_RULE_UNKNOWN_RULE_TYPE);
    rv.set(v8::Integer::new_from_unsigned(scope, value).into());
}

pub(crate) fn css_rule_is_at_rule_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, rule, CSS_AT_RULE_TYPE_SLOT).is_some()
        || get_private_value(scope, rule, CSS_FONT_FEATURE_VALUES_RULE_FONT_FAMILY_SLOT).is_some()
}

pub(crate) fn css_rule_parent_rule_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSRule", "parentRule") {
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), CSS_RULE_PARENT_RULE_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(crate) fn css_rule_child_snapshots_from_stylo_stylesheet_context(
    parent_stylesheet_rule_texts: &[String],
    rule_type: CssRuleType,
    css_text: &str,
    constructed: bool,
) -> Option<Vec<CssRuleSnapshot>> {
    let stylesheet_text = parent_stylesheet_rule_texts.join(" ");
    let rule_snapshots = parse_top_level_rule_snapshots_with_stylo(&stylesheet_text, constructed);
    css_rule_child_snapshots_from_snapshots(&rule_snapshots, rule_type, css_text.trim())
}

pub(crate) fn css_rule_child_snapshots_from_snapshots(
    rule_snapshots: &[CssRuleSnapshot],
    rule_type: CssRuleType,
    css_text: &str,
) -> Option<Vec<CssRuleSnapshot>> {
    for rule_snapshot in rule_snapshots {
        if rule_snapshot.rule_type == rule_type && rule_snapshot.css_text == css_text {
            return Some(rule_snapshot.child_rules.clone());
        }
        if let Some(child_rules) =
            css_rule_child_snapshots_from_snapshots(&rule_snapshot.child_rules, rule_type, css_text)
        {
            return Some(child_rules);
        }
    }
    None
}

pub(crate) fn join_css_rule_blocks(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        (first, "") => first.to_owned(),
        ("", second) => second.to_owned(),
        (first, second) => format!("{first} {second}"),
    }
}

pub(crate) fn css_rule_snapshot_nested_style_block_text(
    rule_snapshots: &[CssRuleSnapshot],
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> String {
    rule_snapshots
        .iter()
        .filter_map(|rule_snapshot| {
            let text = css_rule_snapshot_nested_style_block_item_text(
                rule_snapshot,
                selector_context,
                style_rule_context,
            )?;
            (!text.is_empty()).then_some(text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn css_rule_snapshot_nested_style_block_item_text(
    rule_snapshot: &CssRuleSnapshot,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
) -> Option<String> {
    let text = match rule_snapshot.rule_type {
        CssRuleType::NestedDeclarations => rule_snapshot
            .declaration_text
            .as_deref()
            .unwrap_or(&rule_snapshot.css_text)
            .trim()
            .to_owned(),
        CssRuleType::Style => {
            style_rule_text_from_snapshot(rule_snapshot, selector_context, style_rule_context)?
                .css_text
        }
        CssRuleType::Media
        | CssRuleType::Scope
        | CssRuleType::Supports
        | CssRuleType::Container
        | CssRuleType::LayerBlock => nested_grouping_rule_text_from_snapshot(
            rule_snapshot,
            selector_context,
            style_rule_context,
        )?,
        _ => rule_snapshot.css_text.trim().to_owned(),
    };
    Some(text)
}

pub(crate) fn css_rule_list_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
) -> String {
    css_rule_list_css_texts(scope, rules).join(" ")
}

pub(crate) fn css_rule_list_current_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
) -> String {
    css_rule_list_current_css_texts(scope, rules).join(" ")
}

pub(crate) fn css_rule_current_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> String {
    css_rule_css_text_from_materialized_wrapper(scope, rule)
        .or_else(|| {
            css_rule_attached_native_style_has_child_rules(scope, rule)
                .filter(|has_child_rules| *has_child_rules)
                .and_then(|_| css_rule_attached_native_css_text(scope, rule))
        })
        .or_else(|| css_rule_css_text_from_stylo_declaration_block(scope, rule))
        .or_else(|| css_rule_attached_native_css_text(scope, rule))
        .unwrap_or_else(|| css_rule_detached_snapshot_text(scope, rule))
}

fn css_rule_css_text_from_materialized_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if !css_rule_materialized_subtree_has_pdb_side_entries(scope, rule) {
        return None;
    }

    if let Some(style) = get_private_object(scope, rule, CSS_STYLE_RULE_STYLE_OBJECT_SLOT) {
        let selector = css_style_rule_current_selector(scope, rule)?;
        let declarations = lightweight_css_style_object_css_text(scope, style);
        if let Some(rules) = get_private_object(scope, rule, CSS_STYLE_RULE_NESTED_RULES_SLOT) {
            let nested = css_rule_list_current_css_texts(scope, rules).join("\n");
            let block = join_css_rule_blocks(&declarations, &nested);
            return Some(serialize_nested_style_rule_css_text_from_block(
                &selector, &block,
            ));
        }
        let selector_context = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
            .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
            .unwrap_or_default();
        return Some(serialize_style_rule_css_text_with_context(
            &selector,
            &declarations,
            &selector_context,
        ));
    }

    if let Some(style) = get_private_object(scope, rule, CSS_KEYFRAME_RULE_STYLE_OBJECT_SLOT) {
        let key_text = css_rule_attached_native_keyframe_selector_text(scope, rule)
            .unwrap_or_else(|| private_string(scope, rule, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT));
        let declarations = lightweight_css_style_object_css_text(scope, style);
        return Some(format!("{key_text} {{ {declarations} }}"));
    }

    if let Some(style) = get_private_object(scope, rule, CSS_NESTED_DECLARATIONS_STYLE_OBJECT_SLOT)
    {
        return Some(lightweight_css_style_object_css_text(scope, style));
    }

    if let Some(rules) = get_private_object(scope, rule, CSS_KEYFRAMES_RULE_RULES_SLOT) {
        let name = css_keyframes_rule_current_name(scope, rule)?;
        let nested = css_rule_list_current_css_text(scope, rules);
        return Some(format!("@keyframes {name} {{ {nested} }}"));
    }

    if let Some(rules) = get_private_object(scope, rule, CSS_AT_RULE_NESTED_RULES_SLOT) {
        let parts = css_grouping_rule_text_parts(scope, rule)?;
        let nested = css_rule_list_current_css_text(scope, rules);
        if parts.kind == CssAtRuleKind::Page {
            let selector = normalize_page_selector_text_with_stylo(&parts.prelude)
                .unwrap_or_else(|| parts.prelude.clone());
            let style_text = get_private_object(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT)
                .map(|style| lightweight_css_style_object_css_text(scope, style))
                .unwrap_or_else(|| css_at_rule_current_style_text(scope, rule));
            let block = join_css_rule_blocks(&style_text, &nested);
            return Some(serialize_page_rule_text(&selector, &block));
        }
        let selector_context = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
            .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
            .unwrap_or_default();
        let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, Some(rule));
        return Some(
            if style_rule_context == StyleRuleSelectorContext::TopLevel {
                serialized_grouping_rule_text(parts.kind, &parts.prelude, &nested)
            } else {
                serialized_nested_grouping_rule_text(
                    parts.kind,
                    &parts.prelude,
                    &nested,
                    &selector_context,
                    style_rule_context,
                )
            },
        );
    }

    None
}

fn css_rule_materialized_subtree_has_pdb_side_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> bool {
    let mut pending = vec![rule];
    while let Some(rule) = pending.pop() {
        for slot in [
            CSS_STYLE_RULE_STYLE_OBJECT_SLOT,
            CSS_KEYFRAME_RULE_STYLE_OBJECT_SLOT,
            CSS_NESTED_DECLARATIONS_STYLE_OBJECT_SLOT,
            CSS_AT_RULE_STYLE_OBJECT_SLOT,
        ] {
            if get_private_object(scope, rule, slot)
                .is_some_and(|style| lightweight_css_style_has_pdb_side_entries(scope, style))
            {
                return true;
            }
        }

        for slot in [
            CSS_STYLE_RULE_NESTED_RULES_SLOT,
            CSS_KEYFRAMES_RULE_RULES_SLOT,
            CSS_AT_RULE_NESTED_RULES_SLOT,
        ] {
            let Some(rules) = get_private_object(scope, rule, slot) else {
                continue;
            };
            pending.extend(
                css_rule_list_materialized_entries(scope, rules)
                    .into_iter()
                    .map(|(_, rule)| rule),
            );
        }
    }
    false
}

pub(crate) fn css_condition_rule_condition_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSConditionRule", "conditionText") {
        return;
    }
    let condition_text = css_condition_rule_read_from_object(scope, args.this())
        .map(|view| view.condition_text)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &condition_text));
}

pub(crate) fn css_container_rule_container_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let name = css_condition_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::Container)
        .and_then(|view| view.container_name)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &name));
}

pub(crate) fn css_container_rule_container_query_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let query = css_condition_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::Container)
        .and_then(|view| view.container_query)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &query));
}

pub(crate) fn css_layer_block_rule_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSLayerBlockRule", "name") {
        return;
    }
    let name = css_layer_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::LayerBlock)
        .and_then(|view| view.name)
        .unwrap_or_default();
    rv.set(v8_dynamic_string_value(scope, &name));
}

pub(crate) fn css_layer_statement_rule_name_list_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSLayerStatementRule", "nameList") {
        return;
    }
    let names = css_layer_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::LayerStatement)
        .map(|view| view.names)
        .unwrap_or_default();
    rv.set(v8_string_array(scope, &names).into());
}

pub(crate) fn css_scope_rule_start_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSScopeRule", "start") {
        return;
    }
    let start = css_condition_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::Scope)
        .and_then(|view| view.scope_start);
    match start {
        Some(start) => rv.set(v8_dynamic_string_value(scope, &start)),
        None => rv.set(v8::null(scope).into()),
    }
}

pub(crate) fn css_scope_rule_end_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSScopeRule", "end") {
        return;
    }
    let end = css_condition_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::Scope)
        .and_then(|view| view.scope_end);
    match end {
        Some(end) => rv.set(v8_dynamic_string_value(scope, &end)),
        None => rv.set(v8::null(scope).into()),
    }
}

pub(crate) fn css_at_rule_current_style_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> String {
    if let Some(style) = get_private_object(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT) {
        return lightweight_css_style_object_css_text(scope, style);
    }
    if let Some(declaration_text) = css_rule_attached_native_at_rule_declaration_text(scope, rule) {
        return declaration_text;
    }
    let Some(parts) = css_at_rule_text_parts_from_object(scope, rule) else {
        return String::new();
    };
    match parts.kind {
        CssAtRuleKind::FontFace => css_font_face_rule_style_text_from_object(scope, rule),
        CssAtRuleKind::Page => {
            css_page_rule_read_from_object(scope, rule).map(|read| read.declaration_text)
        }
        _ => parts.block,
    }
    .unwrap_or_default()
}
