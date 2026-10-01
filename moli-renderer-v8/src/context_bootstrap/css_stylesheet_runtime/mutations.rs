pub(crate) use self::apply_live_stylesheet_css_rule_replacement_mutation as apply_live_stylesheet_style_rule_replacement_mutation;
use super::*;

pub(crate) fn apply_stylo_top_level_insert_rule_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
    rule_text: &str,
    index: u32,
) -> Result<(), CssRuleInsertError> {
    insert_css_style_sheet_live_rule(scope, sheet, rule_text, index as usize)?;
    insert_css_rule_list_unmaterialized_rule(scope, rules, index);
    sync_css_style_sheet_change(scope, sheet);
    Ok(())
}

fn sync_css_rule_pdb_style_wrapper_from_declaration_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_object_slot: &'static str,
    declaration_text: &str,
) {
    let Some(style) = get_private_value(scope, rule, style_object_slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    initialize_css_rule_style_object(scope, rule, style, declaration_text);
}

fn sync_css_rule_pdb_style_wrapper_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_object_slot: &'static str,
    snapshot: &CssRuleSnapshot,
) {
    let Some(declaration_text) = snapshot.declaration_text.as_deref() else {
        set_css_rule_stylo_declaration_block_valid(scope, rule, false);
        return;
    };
    sync_css_rule_pdb_style_wrapper_from_declaration_text(
        scope,
        rule,
        style_object_slot,
        declaration_text,
    );
}

pub(crate) fn throw_insert_rule_error(scope: &mut v8::PinScope<'_, '_>, error: CssRuleInsertError) {
    match error {
        CssRuleInsertError::Syntax => {
            webidl::throw_dom_exception(scope, "SyntaxError", "Invalid CSS rule.");
        }
        CssRuleInsertError::IndexSize => {
            webidl::throw_index_size_error(scope);
        }
        CssRuleInsertError::HierarchyRequest => {
            webidl::throw_dom_exception(
                scope,
                "HierarchyRequestError",
                "CSS rule cannot be inserted at this position.",
            );
        }
        CssRuleInsertError::InvalidState => {
            webidl::throw_dom_exception(
                scope,
                "InvalidStateError",
                "@namespace rules cannot be inserted while style rules remain.",
            );
        }
    }
}

pub(crate) fn build_css_rule_object_from_live_stylesheet_typed_rule_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule_type: CssRuleType,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    rule_path: Option<&[usize]>,
) -> Option<v8::Local<'s, v8::Object>> {
    let parent_style_sheet = parent_style_sheet?;
    let rule_path = rule_path?;
    match rule_type {
        CssRuleType::FontFace => css_style_sheet_live_typed_rule_read(
            scope,
            parent_style_sheet,
            rule_path,
            native_stylesheet_font_face_rule_read_with_stylo,
        )
        .map(|view| {
            build_css_font_face_rule_object_from_stylo_view(
                scope,
                view,
                Some(parent_style_sheet),
                parent_rule,
            )
        }),
        CssRuleType::FontFeatureValues => css_style_sheet_live_typed_rule_read(
            scope,
            parent_style_sheet,
            rule_path,
            native_stylesheet_font_feature_values_rule_read_with_stylo,
        )
        .map(|view| {
            build_css_font_feature_values_rule_object_from_stylo_view(
                scope,
                view,
                Some(parent_style_sheet),
                parent_rule,
            )
        }),
        CssRuleType::Property => css_style_sheet_live_typed_rule_read(
            scope,
            parent_style_sheet,
            rule_path,
            native_stylesheet_property_rule_read_with_stylo,
        )
        .map(|view| {
            build_css_property_rule_object_from_stylo_view(
                scope,
                view,
                Some(parent_style_sheet),
                parent_rule,
            )
        }),
        CssRuleType::Margin => css_style_sheet_live_typed_rule_read(
            scope,
            parent_style_sheet,
            rule_path,
            native_stylesheet_margin_rule_read_with_stylo,
        )
        .map(|view| {
            build_css_margin_rule_object_from_stylo_view(
                scope,
                view,
                Some(parent_style_sheet),
                parent_rule,
            )
        }),
        _ => None,
    }
}

pub(crate) fn build_css_rule_object_from_live_stylesheet_rule_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_style_sheet: v8::Local<'s, v8::Object>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    rule_path: &[usize],
) -> Option<v8::Local<'s, v8::Object>> {
    let seed =
        css_style_sheet_live_rule_wrapper_seed_at_path(scope, parent_style_sheet, rule_path)?;
    let rule = match seed {
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::Style {
            selector_text,
            declaration_text,
        } => {
            let selector_context =
                css_style_sheet_selector_namespace_context(scope, parent_style_sheet);
            let style_rule_context =
                css_style_rule_selector_context_for_parent_rule(scope, parent_rule);
            let selector_text = canonicalize_cssom_style_rule_selector_text(
                &selector_text,
                &selector_context.style_rule_namespace_context(),
                style_rule_context,
            )
            .ok()?;
            Some(build_css_style_rule_object(
                scope,
                CssStyleRuleTextParts {
                    css_text: String::new(),
                    selector_text,
                    style_text: declaration_text,
                },
                Some(parent_style_sheet),
                parent_rule,
            ))
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::Keyframe {
            key_text,
            declaration_text,
        } => {
            let key_text = normalize_keyframe_selector_text_with_stylo(&key_text)?;
            Some(build_css_keyframe_rule_object(
                scope,
                CssStyleRuleTextParts {
                    css_text: String::new(),
                    selector_text: key_text,
                    style_text: declaration_text,
                },
                Some(parent_style_sheet),
                parent_rule,
            ))
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::NestedDeclarations {
            declaration_text,
        } => Some(build_css_nested_declarations_rule_object(
            scope,
            &declaration_text,
            Some(parent_style_sheet),
            parent_rule?,
        )),
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::Media { .. } => {
            Some(build_css_generic_at_rule_object_from_stylo_rule_type(
                scope,
                CssRuleType::Media,
                String::new(),
                Some(parent_style_sheet),
                parent_rule,
            ))
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::Page { .. } => {
            Some(build_css_generic_at_rule_object_from_stylo_rule_type(
                scope,
                CssRuleType::Page,
                String::new(),
                Some(parent_style_sheet),
                parent_rule,
            ))
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::GenericAtRule(rule_type) => {
            Some(build_css_generic_at_rule_object_from_stylo_rule_type(
                scope,
                rule_type,
                String::new(),
                Some(parent_style_sheet),
                parent_rule,
            ))
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::TypedAtRule(rule_type) => {
            build_css_rule_object_from_live_stylesheet_typed_rule_path(
                scope,
                rule_type,
                Some(parent_style_sheet),
                parent_rule,
                Some(rule_path),
            )
        }
    }?;
    note_css_rule_wrapper_construction();
    Some(rule)
}

pub(crate) fn apply_stylo_top_level_delete_rule_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
    index: u32,
) -> Result<(), CssRuleInsertError> {
    delete_css_style_sheet_live_rule(scope, sheet, index as usize)?;
    delete_css_rule_list_rule(scope, rules, index);
    Ok(())
}

pub(crate) fn throw_delete_rule_error(scope: &mut v8::PinScope<'_, '_>, error: CssRuleInsertError) {
    match error {
        CssRuleInsertError::IndexSize => webidl::throw_index_size_error(scope),
        CssRuleInsertError::InvalidState => {
            webidl::throw_dom_exception(
                scope,
                "InvalidStateError",
                "@namespace rules cannot be removed while style rules remain.",
            );
        }
        CssRuleInsertError::HierarchyRequest => {
            webidl::throw_dom_exception(
                scope,
                "HierarchyRequestError",
                "CSS rule cannot be deleted from this stylesheet.",
            );
        }
        CssRuleInsertError::Syntax => {
            webidl::throw_dom_exception(scope, "SyntaxError", "Invalid CSS rule.");
        }
    }
}

pub(crate) fn apply_live_stylesheet_nested_rule_insert_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    rule_text: &str,
    index: u32,
    parent_style_sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    style_rule_context: StyleRuleSelectorContext,
    containing_rule_type_bits: u32,
) -> Result<(), CssRuleInsertError> {
    insert_css_style_sheet_live_nested_rule(
        scope,
        parent_style_sheet,
        parent_path,
        rule_text,
        index as usize,
        containing_rule_type_bits,
        stylo_parse_relative_rule_type(style_rule_context),
    )?;
    insert_css_rule_list_unmaterialized_rule(scope, rules, index);
    sync_css_style_sheet_change(scope, parent_style_sheet);
    Ok(())
}

pub(crate) fn apply_live_stylesheet_nested_rule_delete_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    index: u32,
    parent_style_sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
) -> Result<(), CssRuleInsertError> {
    delete_css_style_sheet_live_nested_rule(
        scope,
        parent_style_sheet,
        parent_path,
        index as usize,
    )?;
    delete_css_rule_list_rule(scope, rules, index);
    sync_css_style_sheet_change(scope, parent_style_sheet);
    Ok(())
}

pub(crate) fn apply_live_stylesheet_keyframe_rule_insert_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    rule_text: &str,
    index: u32,
    parent_style_sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
) -> Result<(), CssRuleInsertError> {
    insert_css_style_sheet_live_keyframe_rule(
        scope,
        parent_style_sheet,
        parent_path,
        rule_text,
        index as usize,
    )?;
    insert_css_rule_list_unmaterialized_rule(scope, rules, index);
    sync_css_style_sheet_change(scope, parent_style_sheet);
    Ok(())
}

pub(crate) fn apply_live_stylesheet_keyframe_rule_delete_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rules: v8::Local<'s, v8::Object>,
    index: u32,
    parent_style_sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
) -> Result<(), CssRuleInsertError> {
    delete_css_style_sheet_live_keyframe_rule(
        scope,
        parent_style_sheet,
        parent_path,
        index as usize,
    )?;
    delete_css_rule_list_rule(scope, rules, index);
    sync_css_style_sheet_change(scope, parent_style_sheet);
    Ok(())
}

pub(crate) fn css_style_sheet_live_typed_rule_read<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    read_at_path: impl FnOnce(&CssNativeStylesheet, &[usize]) -> Option<T>,
) -> Option<T> {
    with_css_style_sheet_live_stylesheet(scope, sheet, |native_stylesheet| {
        read_at_path(native_stylesheet, rule_path)
    })
}

pub(crate) fn css_rule_live_stylesheet_typed_rule_read<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    read_at_path: impl FnOnce(&CssNativeStylesheet, &[usize]) -> Option<T>,
) -> Option<T> {
    let parent_style_sheet = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let rule_path = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)?;
    css_style_sheet_live_typed_rule_read(scope, parent_style_sheet, &rule_path, read_at_path)
}

pub(crate) fn css_rule_live_stylesheet_counter_style_rule_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssCounterStyleRuleView> {
    css_rule_live_stylesheet_typed_rule_read(
        scope,
        rule,
        native_stylesheet_counter_style_rule_read_with_stylo,
    )
}

pub(crate) fn css_rule_live_stylesheet_font_face_rule_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssFontFaceRuleView> {
    css_rule_live_stylesheet_typed_rule_read(
        scope,
        rule,
        native_stylesheet_font_face_rule_read_with_stylo,
    )
}

pub(crate) fn css_rule_live_stylesheet_import_rule_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssImportRuleView> {
    css_rule_live_stylesheet_typed_rule_read(
        scope,
        rule,
        native_stylesheet_import_rule_read_with_stylo,
    )
}

pub(crate) fn css_rule_live_stylesheet_namespace_rule_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssNamespaceRuleView> {
    css_rule_live_stylesheet_typed_rule_read(
        scope,
        rule,
        native_stylesheet_namespace_rule_read_with_stylo,
    )
}

pub(crate) fn css_rule_live_stylesheet_margin_rule_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssMarginRuleView> {
    css_rule_live_stylesheet_typed_rule_read(
        scope,
        rule,
        native_stylesheet_margin_rule_read_with_stylo,
    )
}

pub(crate) fn css_rule_live_stylesheet_property_rule_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssPropertyRuleView> {
    css_rule_live_stylesheet_typed_rule_read(
        scope,
        rule,
        native_stylesheet_property_rule_read_with_stylo,
    )
}

pub(crate) fn css_rule_live_stylesheet_font_feature_values_rule_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssFontFeatureValuesRuleView> {
    css_rule_live_stylesheet_typed_rule_read(
        scope,
        rule,
        native_stylesheet_font_feature_values_rule_read_with_stylo,
    )
}

pub(crate) fn css_rule_has_attached_native_binding<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    css_rule_attached_native_path(scope, rule, parent_style_sheet).is_some()
}

pub(crate) fn css_rule_detached_snapshot_typed_view<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parser: impl FnOnce(&str) -> Option<T>,
) -> Option<T> {
    if css_rule_has_attached_native_binding(scope, rule) {
        return None;
    }
    parser(&css_rule_detached_snapshot_text(scope, rule))
}

pub(crate) fn sync_css_rule_object_from_native_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    snapshot: &CssRuleSnapshot,
    selector_context: &CssomSelectorNamespaceContext,
) {
    if snapshot.rule_type == CssRuleType::Style {
        if let (Some(selector_text), Some(style_text)) = (
            snapshot.selector_text.as_deref(),
            style_rule_snapshot_style_text(snapshot),
        ) {
            set_private_string(
                scope,
                rule,
                CSS_STYLE_RULE_SELECTOR_TEXT_SLOT,
                selector_text,
            );
            set_private_string(scope, rule, CSS_STYLE_RULE_STYLE_TEXT_SLOT, &style_text);
        }
    } else if snapshot.rule_type == CssRuleType::NestedDeclarations
        || get_private_value(scope, rule, CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT).is_some()
    {
        set_private_string(
            scope,
            rule,
            CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT,
            &snapshot.css_text,
        );
    } else if get_private_value(scope, rule, CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT).is_some()
        && let Some(kind) = css_at_rule_kind_for_stylo_rule_type(snapshot.rule_type)
    {
        let parent_rule = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        let style_rule_context =
            css_style_rule_selector_context_for_parent_rule(scope, parent_rule);
        if matches!(
            kind,
            CssAtRuleKind::Media
                | CssAtRuleKind::Scope
                | CssAtRuleKind::Supports
                | CssAtRuleKind::Container
                | CssAtRuleKind::Layer
        ) && let Some(block) =
            nested_at_rule_block_text_from_snapshot(snapshot, selector_context, style_rule_context)
        {
            set_private_string(scope, rule, CSS_AT_RULE_NESTED_STYLE_TEXT_SLOT, &block);
        }
    } else if snapshot.rule_type == CssRuleType::Margin
        && let Some(view) = css_rule_live_stylesheet_margin_rule_read(scope, rule)
    {
        sync_css_margin_rule_slots_from_stylo_view(scope, rule, &view);
    } else if snapshot.rule_type == CssRuleType::FontFace {
        sync_css_font_face_rule_style_wrapper_from_native_snapshot(scope, rule, snapshot);
    } else if snapshot.rule_type == CssRuleType::Page {
        sync_css_page_rule_style_wrapper_from_live_stylesheet(scope, rule);
    }
    sync_css_at_rule_wrapper_slots_from_native_snapshot(scope, rule, snapshot);
}

pub(crate) fn sync_css_at_rule_wrapper_slots_from_native_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    snapshot: &CssRuleSnapshot,
) {
    match snapshot.rule_type {
        CssRuleType::Property => {
            if let Some(view) = css_rule_live_stylesheet_property_rule_read(scope, rule) {
                sync_css_property_rule_slots_from_stylo_view(scope, rule, &view);
            }
        }
        CssRuleType::FontFeatureValues => {
            if let Some(view) = css_rule_live_stylesheet_font_feature_values_rule_read(scope, rule)
            {
                sync_css_font_feature_values_rule_slots_from_stylo_view(scope, rule, &view);
            }
        }
        CssRuleType::Import => {
            if let Some(import) = css_rule_live_stylesheet_import_rule_read(scope, rule)
                && let Some(list) = get_private_value(scope, rule, CSS_IMPORT_RULE_MEDIA_LIST_SLOT)
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            {
                set_media_list_from_text(scope, list, &import.media_text, false);
            }
        }
        _ => {}
    }
}

pub(crate) fn css_page_rule_public_mutation_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> (String, String, String) {
    let native_read = css_rule_attached_native_page_read(scope, rule);
    let detached_view = native_read
        .is_none()
        .then(|| css_page_rule_view_from_object(scope, rule))
        .flatten();
    let fallback_parts = (native_read.is_none() && detached_view.is_none())
        .then(|| css_at_rule_text_parts_from_object(scope, rule))
        .flatten();
    let selector_text = native_read
        .as_ref()
        .map(|read| read.selector_text.clone())
        .or_else(|| {
            detached_view
                .as_ref()
                .map(|view| view.selector_text.clone())
        })
        .or_else(|| fallback_parts.as_ref().map(|parts| parts.prelude.clone()))
        .unwrap_or_default();
    let style_text = get_private_value(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|style| lightweight_css_style_object_css_text(scope, style))
        .or_else(|| {
            native_read
                .as_ref()
                .map(|read| read.declaration_text.clone())
        })
        .or_else(|| detached_view.as_ref().map(|view| view.style_text.clone()))
        .or_else(|| {
            fallback_parts
                .as_ref()
                .and_then(|parts| parts.block.clone())
        })
        .unwrap_or_default();
    let nested_rule_text = get_private_value(scope, rule, CSS_AT_RULE_NESTED_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|rules| css_rule_list_current_css_text(scope, rules))
        .or_else(|| {
            detached_view.as_ref().map(|view| {
                view.child_rules
                    .iter()
                    .map(|rule| rule.css_text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        })
        .unwrap_or_default();
    (selector_text, style_text, nested_rule_text)
}

pub(crate) fn css_style_rule_selector_text_from_live_stylesheet_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let selector_text = css_rule_attached_native_style_selector_text(scope, object)?;
    let parent_rule = get_private_value(scope, object, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let style_rule_context = css_style_rule_selector_context_for_parent_rule(scope, parent_rule);
    let selector_context = css_style_rule_selector_namespace_context(scope, object);
    canonicalize_cssom_style_rule_selector_text(
        &selector_text,
        &selector_context.style_rule_namespace_context(),
        style_rule_context,
    )
    .ok()
}

fn freeze_css_rule_wrapper_from_native_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    snapshot: &CssRuleSnapshot,
    child_snapshots: v8::Local<'s, v8::Array>,
    declaration_source: Option<v8::Local<'s, v8::Object>>,
    exposed_parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
) {
    set_css_rule_snapshot_type(scope, rule, Some(snapshot.rule_type));
    set_css_rule_detached_declaration_source(scope, rule, declaration_source);
    if snapshot.rule_type == CssRuleType::Keyframe {
        sync_css_keyframe_rule_object_from_native_snapshot(scope, rule, snapshot);
    } else {
        let selector_context = exposed_parent_style_sheet
            .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
            .unwrap_or_default();
        sync_css_rule_object_from_native_snapshot(scope, rule, snapshot, &selector_context);
    }
    let style_slot = match snapshot.rule_type {
        CssRuleType::Style => Some(CSS_STYLE_RULE_STYLE_OBJECT_SLOT),
        CssRuleType::Keyframe => Some(CSS_KEYFRAME_RULE_STYLE_OBJECT_SLOT),
        CssRuleType::NestedDeclarations => Some(CSS_NESTED_DECLARATIONS_STYLE_OBJECT_SLOT),
        _ => None,
    };
    if let Some(slot) = style_slot {
        sync_css_rule_pdb_style_wrapper_from_snapshot(scope, rule, slot, snapshot);
    }

    let _ = set_detached_css_rule_snapshot_text(scope, rule, &snapshot.css_text);
    set_detached_css_rule_child_snapshot_array(scope, rule, child_snapshots);

    let Some(slot) = css_rule_nested_rules_slot_for_stylo_rule_type(snapshot.rule_type) else {
        return;
    };
    let Some(rules) = get_private_object(scope, rule, slot) else {
        return;
    };
    bind_css_rule_list_to_detached_snapshot_array(
        scope,
        rules,
        exposed_parent_style_sheet,
        Some(rule),
        child_snapshots,
    );
    for (index, child) in css_rule_list_materialized_entries(scope, rules) {
        let Some(entry) = css_rule_list_detached_snapshot_at(scope, rules, index) else {
            debug_assert!(
                false,
                "materialized detached CSSRule must have a native snapshot"
            );
            let _ = freeze_css_rule_wrapper_from_retained_native_binding(scope, child);
            continue;
        };
        let child_snapshots = entry.child_snapshots;
        let declaration_source = entry.declaration_source;
        let child_snapshot = entry.complete_snapshot(scope);
        freeze_css_rule_wrapper_from_native_snapshot(
            scope,
            child,
            &child_snapshot,
            child_snapshots,
            declaration_source,
            exposed_parent_style_sheet,
        );
    }
}

pub(crate) fn freeze_css_rule_wrapper_from_retained_native_binding<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(retained) = css_rule_retained_native_snapshot_for_detach(scope, rule) else {
        return false;
    };
    let parent_style_sheet = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let entry = encode_retained_css_rule_snapshot(scope, retained);
    freeze_css_rule_wrapper_from_native_snapshot(
        scope,
        rule,
        &entry.snapshot,
        entry.child_snapshots,
        entry.declaration_source,
        parent_style_sheet,
    );
    true
}

pub(crate) fn rejected_type_error_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let reason = v8_string(scope, message)?;
    let exception = v8::Exception::type_error(scope, reason);
    let _ = resolver.reject(scope, exception);
    Some(promise.into())
}

pub(crate) fn nested_rule_block_text_from_stylo_mutation(
    mutation: CssDetachedRuleListMutation,
) -> String {
    let mut parts = Vec::new();
    if let Some(declaration_text) = mutation.first_declaration_text {
        let declaration_text = declaration_text.trim();
        if !declaration_text.is_empty() {
            parts.push(declaration_text.to_owned());
        }
    }
    parts.extend(mutation.rules.into_iter().filter_map(|rule| {
        let css_text = rule.css_text.trim();
        (!css_text.is_empty()).then(|| css_text.to_owned())
    }));
    parts.join("\n")
}

pub(crate) fn apply_live_stylesheet_cssom_declaration_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> bool {
    use crate::css_style::CssDeclarationMutation;
    let style = args.this();
    sync_css_rule_stylo_declaration_block_validity_from_style(scope, rule, style);
    if args.length() != 4 || !css_rule_stylo_declaration_block_is_valid(scope, rule) {
        return false;
    }
    let Some(sheet) = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT) else {
        return false;
    };
    let Some(path) = css_rule_attached_native_path(scope, rule, sheet) else {
        return false;
    };
    let Some(value) = args.get(2).to_string(scope) else {
        return false;
    };
    let value = value.to_rust_string_lossy(scope);
    let name = (!args.get(1).is_null()).then(|| args.get(1).to_rust_string_lossy(scope));
    let mutation = match name.as_deref() {
        Some(name) => CssDeclarationMutation::Property {
            name,
            value: &value,
            important: args.get(3).boolean_value(scope),
        },
        None => CssDeclarationMutation::CssText(&value),
    };
    if require_css_style_sheet_live_stylesheet(scope, sheet)
        .mutate_rule_declarations(&path, mutation)
        .is_err()
    {
        return false;
    }
    // The stylesheet parser owns URL, quirks-mode and rule-type context. Keep
    // the CSSOM cache in sync with its result, including rejected declarations.
    let block = css_rule_attached_native_declaration_block(scope, rule)
        .expect("a successful declaration mutation retains its attached rule");
    set_lightweight_css_style_declaration_block(scope, style, block);
    sync_css_style_sheet_change(scope, sheet);
    true
}

pub(crate) fn apply_live_stylesheet_rule_declaration_text_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    kind: CssRulePdbDeclarationKind,
    declaration_text: &str,
) -> bool {
    match kind {
        CssRulePdbDeclarationKind::StyleRule | CssRulePdbDeclarationKind::NestedDeclarations => {
            apply_live_stylesheet_style_like_rule_declaration_block_mutation(
                scope,
                rule,
                kind,
                declaration_text,
            )
        }
        CssRulePdbDeclarationKind::KeyframeRule => {
            apply_live_stylesheet_keyframe_rule_declaration_block_mutation(
                scope,
                rule,
                declaration_text,
            )
        }
    }
}

pub(crate) fn css_style_rule_css_text_reset_can_use_live_stylesheet_declaration_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parsed: &CssStyleRuleTextParts,
) -> bool {
    if css_rule_current_stylo_rule_type_from_object(scope, rule) != Some(CssRuleType::Style) {
        return false;
    }
    if css_style_rule_current_selector(scope, rule).as_deref() != Some(&parsed.selector_text) {
        return false;
    }
    let current_style_text = private_string(scope, rule, CSS_STYLE_RULE_STYLE_TEXT_SLOT);
    if css_style_rule_current_has_nested_rules(scope, rule, &current_style_text) {
        return false;
    }
    css_style_rule_style_text_has_nested_rules_with_stylo_context(scope, rule, &parsed.style_text)
        == Some(false)
}

pub(crate) fn css_keyframe_rule_css_text_reset_can_use_live_stylesheet_declaration_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parsed: &CssStyleRuleTextParts,
) -> bool {
    if css_rule_current_stylo_rule_type_from_object(scope, rule) != Some(CssRuleType::Keyframe) {
        return false;
    }
    let current_key_text = css_rule_attached_native_keyframe_selector_text(scope, rule)
        .unwrap_or_else(|| private_string(scope, rule, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT));
    keyframe_selector_texts_match_with_stylo(&current_key_text, &parsed.selector_text)
}

pub(crate) fn apply_live_stylesheet_style_like_rule_declaration_block_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    kind: CssRulePdbDeclarationKind,
    declaration_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let result = match kind {
        CssRulePdbDeclarationKind::StyleRule => set_css_style_sheet_live_style_rule_declarations(
            scope,
            parent_style_sheet,
            &rule_path,
            declaration_text,
        ),
        CssRulePdbDeclarationKind::NestedDeclarations => {
            set_css_style_sheet_live_nested_declarations_rule_declarations(
                scope,
                parent_style_sheet,
                &rule_path,
                declaration_text,
            )
        }
        CssRulePdbDeclarationKind::KeyframeRule => return false,
    };
    if result.is_err() {
        return false;
    }
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_font_face_rule_descriptor_block_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    descriptor_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_font_face_rule_descriptors(
        scope,
        parent_style_sheet,
        &rule_path,
        descriptor_text,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_page_rule_descriptor_block_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    descriptor_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_page_rule_descriptors(
        scope,
        parent_style_sheet,
        &rule_path,
        descriptor_text,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_page_margin_rule_descriptor_block_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    descriptor_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_page_margin_rule_descriptors(
        scope,
        parent_style_sheet,
        &rule_path,
        descriptor_text,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_keyframe_rule_declaration_block_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    declaration_text: &str,
) -> bool {
    let Some(parent_rule) = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(parent_path) =
        css_rule_path_in_parent_style_sheet(scope, parent_rule, parent_style_sheet)
    else {
        return false;
    };
    let Some(rules) = get_private_value(scope, parent_rule, CSS_KEYFRAMES_RULE_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(index) = css_rule_list_rule_index(scope, rules, rule) else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_keyframe_rule_declarations(
        scope,
        parent_style_sheet,
        &parent_path,
        index as usize,
        declaration_text,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_keyframe_rule_selector_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    selector_text: &str,
) -> bool {
    let Some(parent_rule) = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(parent_path) =
        css_rule_path_in_parent_style_sheet(scope, parent_rule, parent_style_sheet)
    else {
        return false;
    };
    let Some(rules) = get_private_value(scope, parent_rule, CSS_KEYFRAMES_RULE_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(index) = css_rule_list_rule_index(scope, rules, rule) else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_keyframe_rule_selector(
        scope,
        parent_style_sheet,
        &parent_path,
        index as usize,
        selector_text,
    ) else {
        return false;
    };
    set_private_string(scope, rule, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT, selector_text);
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_style_rule_selector_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    selector_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let parent_rule = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let (containing_rule_type_bits, parse_relative_rule_type) =
        if let Some(parent_rule) = parent_rule {
            let Some(containing_rule_type_bits) =
                stylo_containing_rule_type_bits_for_parent_rule(scope, parent_rule)
            else {
                return false;
            };
            let style_rule_context =
                css_style_rule_selector_context_for_parent_rule(scope, Some(parent_rule));
            (
                containing_rule_type_bits,
                stylo_parse_relative_rule_type(style_rule_context),
            )
        } else {
            (0, None)
        };
    let Ok(()) = set_css_style_sheet_live_style_rule_selector(
        scope,
        parent_style_sheet,
        &rule_path,
        selector_text,
        containing_rule_type_bits,
        parse_relative_rule_type,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

fn reset_attached_css_rule_children_after_replacement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parent_style_sheet: v8::Local<'s, v8::Object>,
    rule_type: CssRuleType,
) {
    let Some(slot) = css_rule_nested_rules_slot_for_stylo_rule_type(rule_type) else {
        return;
    };
    let Some(rules) = get_private_object(scope, rule, slot) else {
        return;
    };

    for (_, child) in css_rule_list_materialized_entries(scope, rules) {
        detach_css_rule_from_parent(scope, child);
    }

    let child_count = css_rule_live_stylesheet_child_rule_count(scope, rule)
        .map(|(_, count)| count)
        .unwrap_or(0);
    initialize_attached_css_rule_list(scope, rules, parent_style_sheet, Some(rule), child_count);
}

fn sync_attached_css_rule_root_from_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    seed: crate::live_stylesheet::LiveStylesheetRuleWrapperSeed,
) -> CssRuleType {
    let rule_type = match seed {
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::Style {
            selector_text,
            declaration_text,
        } => {
            set_private_string(
                scope,
                rule,
                CSS_STYLE_RULE_SELECTOR_TEXT_SLOT,
                &selector_text,
            );
            set_private_string(
                scope,
                rule,
                CSS_STYLE_RULE_STYLE_TEXT_SLOT,
                &declaration_text,
            );
            sync_css_rule_pdb_style_wrapper_from_declaration_text(
                scope,
                rule,
                CSS_STYLE_RULE_STYLE_OBJECT_SLOT,
                &declaration_text,
            );
            CssRuleType::Style
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::Keyframe {
            key_text,
            declaration_text,
        } => {
            set_private_string(scope, rule, CSS_KEYFRAME_RULE_KEY_TEXT_SLOT, &key_text);
            set_private_string(
                scope,
                rule,
                CSS_KEYFRAME_RULE_STYLE_TEXT_SLOT,
                &declaration_text,
            );
            sync_css_rule_pdb_style_wrapper_from_declaration_text(
                scope,
                rule,
                CSS_KEYFRAME_RULE_STYLE_OBJECT_SLOT,
                &declaration_text,
            );
            CssRuleType::Keyframe
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::NestedDeclarations {
            declaration_text,
        } => {
            set_private_string(
                scope,
                rule,
                CSS_NESTED_DECLARATIONS_STYLE_TEXT_SLOT,
                &declaration_text,
            );
            sync_css_rule_pdb_style_wrapper_from_declaration_text(
                scope,
                rule,
                CSS_NESTED_DECLARATIONS_STYLE_OBJECT_SLOT,
                &declaration_text,
            );
            CssRuleType::NestedDeclarations
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::Media { media_text } => {
            sync_media_rule_media_list_slot(scope, rule, &media_text);
            CssRuleType::Media
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::Page { declaration_text } => {
            if let Some(style) = get_private_object(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT) {
                set_style_object_css_text_without_notify(scope, style, &declaration_text);
            }
            CssRuleType::Page
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::TypedAtRule(rule_type) => {
            match rule_type {
                CssRuleType::FontFace => {
                    if let Some(view) = css_rule_live_stylesheet_font_face_rule_read(scope, rule) {
                        sync_css_font_face_rule_style_wrapper_from_stylo_view(scope, rule, &view);
                    }
                }
                CssRuleType::FontFeatureValues => {
                    if let Some(view) =
                        css_rule_live_stylesheet_font_feature_values_rule_read(scope, rule)
                    {
                        sync_css_font_feature_values_rule_slots_from_stylo_view(scope, rule, &view);
                    }
                }
                CssRuleType::Margin => {
                    if let Some(view) = css_rule_live_stylesheet_margin_rule_read(scope, rule) {
                        sync_css_margin_rule_slots_from_stylo_view(scope, rule, &view);
                    }
                }
                CssRuleType::Property => {
                    if let Some(view) = css_rule_live_stylesheet_property_rule_read(scope, rule) {
                        sync_css_property_rule_slots_from_stylo_view(scope, rule, &view);
                    }
                }
                _ => {}
            }
            rule_type
        }
        crate::live_stylesheet::LiveStylesheetRuleWrapperSeed::GenericAtRule(rule_type) => {
            if rule_type == CssRuleType::Import
                && let Some(import) = css_rule_live_stylesheet_import_rule_read(scope, rule)
                && let Some(list) = get_private_object(scope, rule, CSS_IMPORT_RULE_MEDIA_LIST_SLOT)
            {
                set_media_list_from_text(scope, list, &import.media_text, false);
            }
            rule_type
        }
    };

    clear_css_rule_detached_snapshot(scope, rule);
    rule_type
}

pub(crate) fn restore_attached_css_rule_wrapper_from_live_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Some(seed) =
        css_style_sheet_live_rule_wrapper_seed_at_path(scope, parent_style_sheet, &rule_path)
    else {
        return false;
    };

    sync_attached_css_rule_root_from_seed(scope, rule, seed);
    true
}

fn sync_attached_css_rule_after_replacement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parent_style_sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
) {
    let Some(seed) =
        css_style_sheet_live_rule_wrapper_seed_at_path(scope, parent_style_sheet, rule_path)
    else {
        return;
    };

    let rule_type = sync_attached_css_rule_root_from_seed(scope, rule, seed);
    reset_attached_css_rule_children_after_replacement(scope, rule, parent_style_sheet, rule_type);
}

pub(crate) fn apply_live_stylesheet_relative_style_rule_replacement_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rule_text: &str,
) -> bool {
    if css_rule_current_stylo_rule_type_from_object(scope, rule) != Some(CssRuleType::Style) {
        return false;
    }
    let Some(parent_rule) = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let style_rule_context =
        css_style_rule_selector_context_for_parent_rule(scope, Some(parent_rule));
    if style_rule_context == StyleRuleSelectorContext::TopLevel {
        return false;
    }
    apply_live_stylesheet_style_rule_replacement_mutation(scope, rule, rule_text)
}

pub(crate) fn apply_live_stylesheet_css_rule_replacement_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rule_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    if rule_path.len() == 1 {
        let index = rule_path[0];
        let Ok(()) = replace_css_style_sheet_live_rule(scope, parent_style_sheet, rule_text, index)
        else {
            return false;
        };
        sync_attached_css_rule_after_replacement(scope, rule, parent_style_sheet, &rule_path);
        sync_css_style_sheet_change(scope, parent_style_sheet);
        return true;
    }

    let Some(parent_rule) = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(containing_rule_type_bits) =
        stylo_containing_rule_type_bits_for_parent_rule(scope, parent_rule)
    else {
        return false;
    };
    let parent_path = &rule_path[..rule_path.len() - 1];
    let index = *rule_path.last().unwrap_or(&0);
    let style_rule_context =
        css_style_rule_selector_context_for_parent_rule(scope, Some(parent_rule));
    let Ok(()) = replace_css_style_sheet_live_nested_rule(
        scope,
        parent_style_sheet,
        parent_path,
        rule_text,
        index,
        containing_rule_type_bits,
        stylo_parse_relative_rule_type(style_rule_context),
    ) else {
        return false;
    };
    sync_attached_css_rule_after_replacement(scope, rule, parent_style_sheet, &rule_path);
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_keyframe_rule_replacement_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rule_text: &str,
) -> bool {
    let Some(parent_rule) = get_private_value(scope, rule, CSS_RULE_PARENT_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(parent_path) =
        css_rule_path_in_parent_style_sheet(scope, parent_rule, parent_style_sheet)
    else {
        return false;
    };
    let Some(rules) = get_private_value(scope, parent_rule, CSS_KEYFRAMES_RULE_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(index) = css_rule_list_rule_index(scope, rules, rule) else {
        return false;
    };
    let Ok(()) = replace_css_style_sheet_live_keyframe_rule(
        scope,
        parent_style_sheet,
        &parent_path,
        rule_text,
        index as usize,
    ) else {
        return false;
    };
    let mut rule_path = parent_path;
    rule_path.push(index as usize);
    sync_attached_css_rule_after_replacement(scope, rule, parent_style_sheet, &rule_path);
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_keyframes_rule_name_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) =
        set_css_style_sheet_live_keyframes_rule_name(scope, parent_style_sheet, &rule_path, name)
    else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_page_rule_selector_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    selector_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_page_rule_selectors(
        scope,
        parent_style_sheet,
        &rule_path,
        selector_text,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn sync_css_keyframe_rule_object_from_native_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    snapshot: &CssRuleSnapshot,
) {
    if let Some(parsed) = keyframe_rule_text_from_snapshot(snapshot) {
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
    }
}

pub(crate) fn sync_css_rule_rules_array_from_live_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
) -> bool {
    let Some((parent_style_sheet, child_count)) =
        css_rule_live_stylesheet_child_rule_count(scope, rule)
    else {
        return false;
    };
    initialize_attached_css_rule_list(scope, rules, parent_style_sheet, Some(rule), child_count);
    true
}

pub(crate) use self::sync_css_rule_rules_array_from_live_stylesheet as sync_css_grouping_rule_rules_array_from_live_stylesheet;

pub(crate) fn apply_live_stylesheet_media_rule_media_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    media_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_media_rule_media(
        scope,
        parent_style_sheet,
        &rule_path,
        media_text,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_import_rule_media_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    media_text: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_import_rule_media(
        scope,
        parent_style_sheet,
        &rule_path,
        media_text,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_font_feature_values_rule_font_family_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    font_families: Vec<style::values::computed::font::FamilyName>,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_font_feature_values_rule_font_family(
        scope,
        parent_style_sheet,
        &rule_path,
        font_families,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_font_feature_values_rule_map_entry_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    group: crate::live_stylesheet::FontFeatureValuesMapGroup,
    name: &str,
    values: &[u32],
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(()) = set_css_style_sheet_live_font_feature_values_rule_map_entry(
        scope,
        parent_style_sheet,
        &rule_path,
        group,
        name,
        values,
    ) else {
        return false;
    };
    sync_css_style_sheet_change(scope, parent_style_sheet);
    true
}

pub(crate) fn apply_live_stylesheet_font_feature_values_rule_map_entry_delete<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    group: crate::live_stylesheet::FontFeatureValuesMapGroup,
    name: &str,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(changed) = delete_css_style_sheet_live_font_feature_values_rule_map_entry(
        scope,
        parent_style_sheet,
        &rule_path,
        group,
        name,
    ) else {
        return false;
    };
    if changed {
        sync_css_style_sheet_change(scope, parent_style_sheet);
    }
    true
}

pub(crate) fn apply_live_stylesheet_font_feature_values_rule_map_clear<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    group: crate::live_stylesheet::FontFeatureValuesMapGroup,
) -> bool {
    let Some(parent_style_sheet) = get_private_value(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let Some(rule_path) = css_rule_path_in_parent_style_sheet(scope, rule, parent_style_sheet)
    else {
        return false;
    };
    let Ok(changed) = clear_css_style_sheet_live_font_feature_values_rule_map(
        scope,
        parent_style_sheet,
        &rule_path,
        group,
    ) else {
        return false;
    };
    if changed {
        sync_css_style_sheet_change(scope, parent_style_sheet);
    }
    true
}

pub(crate) fn sync_css_font_face_rule_style_wrapper_from_native_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    snapshot: &CssRuleSnapshot,
) {
    let Some(style_text) = snapshot.declaration_text.as_deref() else {
        return;
    };
    sync_css_font_face_rule_style_wrapper_from_text(scope, rule, style_text);
}

pub(crate) fn sync_css_font_face_rule_style_wrapper_from_stylo_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rule_view: &CssFontFaceRuleView,
) {
    sync_css_font_face_rule_style_wrapper_from_text(scope, rule, &rule_view.style_text);
}

fn sync_css_font_face_rule_style_wrapper_from_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    style_text: &str,
) {
    let Some(style) = get_private_value(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    set_style_object_css_text_without_notify(scope, style, style_text);
}

pub(crate) fn sync_css_page_rule_style_wrapper_from_live_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) {
    let Some(style) = get_private_object(scope, rule, CSS_AT_RULE_STYLE_OBJECT_SLOT) else {
        return;
    };
    if let Some(read) = css_rule_attached_native_page_read(scope, rule) {
        set_style_object_css_text_without_notify(scope, style, &read.declaration_text);
    }
}

pub(crate) use self::sync_css_rule_rules_array_from_live_stylesheet as sync_css_style_rule_rules_array_from_live_stylesheet;
