use super::*;

fn set_css_style_sheet_live_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    stylesheet: crate::live_stylesheet::LiveStylesheetRef,
) {
    if css_style_sheet_id(scope, sheet) == Some(stylesheet.id()) {
        return;
    }
    let host_ptr = context_host_ptr_from_global_bridge(scope)
        .expect("live CSSStyleSheet must belong to a page context");
    let host = unsafe { &*host_ptr };
    if let Some(lease_id) = private_u64(scope, sheet, CSS_STYLE_SHEET_WRAPPER_LEASE_ID_SLOT)
        .and_then(crate::live_stylesheet::StylesheetWrapperLeaseId::from_raw)
        && host.replace_live_stylesheet_wrapper_lease(lease_id, Some(stylesheet.clone()))
    {
        set_private_u64(scope, sheet, CSS_STYLE_SHEET_ID_SLOT, stylesheet.id().get());
        return;
    }

    let (lease_id, lease) = host.create_live_stylesheet_wrapper_lease(stylesheet.clone());
    set_private_u64(scope, sheet, CSS_STYLE_SHEET_ID_SLOT, stylesheet.id().get());
    set_private_u64(
        scope,
        sheet,
        CSS_STYLE_SHEET_WRAPPER_LEASE_ID_SLOT,
        lease_id.get(),
    );
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, sheet, move || {
        lease.borrow_mut().take();
    });
}

pub(crate) fn bind_css_style_sheet_to_live_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    stylesheet: crate::live_stylesheet::LiveStylesheetRef,
) {
    if css_style_sheet_id(scope, sheet) == Some(stylesheet.id()) {
        return;
    }
    let rules = css_style_sheet_rules_array(scope, sheet);
    if css_style_sheet_id(scope, sheet).is_some() {
        retire_css_rule_list_for_stylesheet_replacement(scope, rules);
    }
    let rule_count = stylesheet.top_level_rule_count();
    set_css_style_sheet_live_stylesheet(scope, sheet, stylesheet);
    initialize_attached_css_rule_list(scope, rules, sheet, None, rule_count);
}

pub(crate) fn css_style_sheet_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::StylesheetId> {
    private_u64(scope, sheet, CSS_STYLE_SHEET_ID_SLOT)
        .and_then(crate::live_stylesheet::StylesheetId::from_raw)
}

pub(crate) fn css_style_sheet_live_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
    let id = css_style_sheet_id(scope, sheet)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.live_stylesheet(id)
}

pub(crate) fn bind_css_rule_object_to_native_stylesheet_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    sheet: v8::Local<'s, v8::Object>,
    path: Vec<usize>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(stylesheet) = css_style_sheet_live_stylesheet(scope, sheet) else {
        return false;
    };
    let existing_id = private_u64(scope, rule, CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT)
        .and_then(crate::live_stylesheet::StylesheetRuleWrapperLeaseId::from_raw);
    let Some((id, lease)) =
        unsafe { &*host_ptr }.bind_live_stylesheet_rule_wrapper(existing_id, &stylesheet, path)
    else {
        return false;
    };
    set_private_u64(scope, rule, CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT, id.get());
    if let Some(lease) = lease {
        crate::v8_finalizer::track_context_owned_v8_finalizer(scope, rule, move || {
            lease.borrow_mut().take();
        });
    }
    // Attached rules read from the native lease. Keep a full cssText snapshot
    // only after the rule is detached and must outlive that native binding.
    clear_css_rule_detached_snapshot(scope, rule);
    true
}

pub(crate) fn detach_css_rule_object_from_native_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) {
    let Some(id) = private_u64(scope, rule, CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT)
        .and_then(crate::live_stylesheet::StylesheetRuleWrapperLeaseId::from_raw)
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &*host_ptr }.release_live_stylesheet_rule_wrapper(id);
    set_private_u64(scope, rule, CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT, 0);
}

pub(crate) fn css_rule_attached_native_css_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.css_text())
}

pub(crate) fn css_rule_attached_native_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<moli_css_parse::CssDeclarationBlock> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.declaration_block())?
}

fn with_css_rule_attached_native_binding<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    read: impl FnOnce(&crate::live_stylesheet::StylesheetRuleWrapperBinding) -> R,
) -> Option<R> {
    let parent_style_sheet = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)?;
    let stylesheet_id = css_style_sheet_id(scope, parent_style_sheet)?;
    let lease_id = private_u64(scope, rule, CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT)
        .and_then(crate::live_stylesheet::StylesheetRuleWrapperLeaseId::from_raw)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.with_attached_live_stylesheet_rule_wrapper(lease_id, stylesheet_id, read)
}

pub(crate) fn css_rule_attached_native_rule_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssRuleType> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.rule_type())
}

pub(crate) fn css_rule_attached_native_style_selector_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.style_selector_text())?
}

pub(crate) fn css_rule_attached_native_keyframe_selector_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.keyframe_selector_text())?
}

pub(crate) fn css_rule_attached_native_style_has_child_rules<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.style_has_child_rules())?
}

pub(crate) fn css_rule_attached_native_grouping_prelude<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<(CssRuleType, String)> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.grouping_prelude())?
}

pub(crate) fn css_rule_attached_native_at_rule_declaration_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    with_css_rule_attached_native_binding(scope, rule, |binding| {
        binding.at_rule_declaration_text()
    })?
}

pub(crate) fn css_rule_attached_native_condition_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::LiveStylesheetConditionRuleRead> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.condition_rule_read())?
}

pub(crate) fn css_rule_attached_native_layer_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::LiveStylesheetLayerRuleRead> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.layer_rule_read())?
}

pub(crate) fn css_rule_attached_native_font_palette_values_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    property: moli_css_parse::CssFontPaletteValuesProperty,
) -> Option<String> {
    with_css_rule_attached_native_binding(scope, rule, |binding| {
        binding.font_palette_values_property(property)
    })?
}

pub(crate) fn css_rule_attached_native_keyframes_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.keyframes_name())?
}

pub(crate) fn css_rule_attached_native_page_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::LiveStylesheetPageRuleRead> {
    with_css_rule_attached_native_binding(scope, rule, |binding| binding.page_rule_read())?
}

pub(crate) fn css_rule_attached_native_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    sheet: v8::Local<'s, v8::Object>,
) -> Option<Vec<usize>> {
    let stylesheet_id = css_style_sheet_id(scope, sheet)?;
    let lease_id = private_u64(scope, rule, CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT)
        .and_then(crate::live_stylesheet::StylesheetRuleWrapperLeaseId::from_raw)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.attached_live_stylesheet_rule_wrapper_path(lease_id, stylesheet_id)
}

pub(crate) fn css_rule_live_stylesheet_child_rule_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, usize)> {
    let sheet = get_private_object(scope, rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)?;
    let path = css_rule_attached_native_path(scope, rule, sheet)?;
    let count = css_style_sheet_live_stylesheet(scope, sheet)?.child_rule_count_at_path(&path)?;
    Some((sheet, count))
}

pub(crate) fn css_style_sheet_live_rule_wrapper_seed_at_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    path: &[usize],
) -> Option<crate::live_stylesheet::LiveStylesheetRuleWrapperSeed> {
    css_style_sheet_live_stylesheet(scope, sheet)?.rule_wrapper_seed_at_path(path)
}

pub(crate) fn css_rule_retained_native_snapshot_for_detach<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<crate::live_stylesheet::RetainedStylesheetRuleSnapshot> {
    let lease_id = private_u64(scope, rule, CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT)
        .and_then(crate::live_stylesheet::StylesheetRuleWrapperLeaseId::from_raw)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.retained_live_stylesheet_rule_wrapper_snapshot_for_detach(lease_id)
}

pub(crate) fn replace_css_style_sheet_live_stylesheet_from_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    css_text: &str,
) -> Option<()> {
    if let Some(stylesheet) = css_style_sheet_live_stylesheet(scope, sheet) {
        stylesheet.replace_from_text(css_text);
        let rules = css_style_sheet_rules_array(scope, sheet);
        retire_css_rule_list_for_stylesheet_replacement(scope, rules);
        initialize_attached_css_rule_list(
            scope,
            rules,
            sheet,
            None,
            stylesheet.top_level_rule_count(),
        );
        return Some(());
    }
    let is_constructed = css_style_sheet_is_constructed(scope, sheet);
    let constructor_document = css_style_sheet_constructor_document_handle(scope, sheet);
    let owner_handle = css_style_sheet_owner_node(scope, sheet).and_then(|owner| {
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, owner)
            .ok()
            .map(|(_, handle)| handle)
    });
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &*host_ptr };
    let document = constructor_document
        .or_else(|| {
            owner_handle.and_then(|owner| {
                host.dom_host().node(owner).and_then(|node| {
                    node.is_document()
                        .then_some(owner)
                        .or_else(|| node.owner_document())
                })
            })
        })
        .unwrap_or_else(|| host.document_handle());
    let base_url = css_style_sheet_base_url(scope, sheet)
        .unwrap_or_else(|| host.document_base_url_for_handle(document));
    let allow_import_rules = if is_constructed {
        style::stylesheets::AllowImportRules::No
    } else {
        style::stylesheets::AllowImportRules::Yes
    };
    let stylesheet = host.create_live_stylesheet(document, css_text, base_url, allow_import_rules);
    let rule_count = stylesheet.top_level_rule_count();
    set_css_style_sheet_live_stylesheet(scope, sheet, stylesheet);
    let rules = css_style_sheet_rules_array(scope, sheet);
    initialize_attached_css_rule_list(scope, rules, sheet, None, rule_count);
    Some(())
}

pub(crate) fn require_css_style_sheet_live_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> crate::live_stylesheet::LiveStylesheetRef {
    css_style_sheet_live_stylesheet(scope, sheet)
        .expect("every initialized CSSStyleSheet must retain a LiveStylesheet")
}

pub(crate) fn insert_css_style_sheet_live_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_text: &str,
    index: usize,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).insert_rule(rule_text, index)
}

pub(crate) fn delete_css_style_sheet_live_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    index: usize,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).delete_rule(index)
}

pub(crate) fn replace_css_style_sheet_live_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_text: &str,
    index: usize,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).replace_rule(rule_text, index)
}

pub(crate) fn with_css_style_sheet_live_stylesheet<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    f: impl FnOnce(&CssNativeStylesheet) -> R,
) -> R {
    let stylesheet = require_css_style_sheet_live_stylesheet(scope, sheet);
    let native_stylesheet = stylesheet.stylesheet();
    f(&native_stylesheet)
}

pub(crate) fn insert_css_style_sheet_live_nested_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    rule_text: &str,
    index: usize,
    containing_rule_type_bits: u32,
    parse_relative_rule_type: Option<CssRuleType>,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).insert_nested_rule(
        parent_path,
        rule_text,
        index,
        containing_rule_type_bits,
        parse_relative_rule_type,
    )
}

pub(crate) fn replace_css_style_sheet_live_nested_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    rule_text: &str,
    index: usize,
    containing_rule_type_bits: u32,
    parse_relative_rule_type: Option<CssRuleType>,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).replace_nested_rule(
        parent_path,
        rule_text,
        index,
        containing_rule_type_bits,
        parse_relative_rule_type,
    )
}

pub(crate) fn delete_css_style_sheet_live_nested_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    index: usize,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).delete_nested_rule(parent_path, index)
}

pub(crate) fn insert_css_style_sheet_live_keyframe_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    rule_text: &str,
    index: usize,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).insert_keyframe_rule(
        parent_path,
        rule_text,
        index,
    )
}

pub(crate) fn replace_css_style_sheet_live_keyframe_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    rule_text: &str,
    index: usize,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).replace_keyframe_rule(
        parent_path,
        rule_text,
        index,
    )
}

pub(crate) fn delete_css_style_sheet_live_keyframe_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    index: usize,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).delete_keyframe_rule(parent_path, index)
}

pub(crate) fn set_css_style_sheet_live_media_rule_media<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    media_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_media_rule_media(rule_path, media_text)
}

pub(crate) fn set_css_style_sheet_live_import_rule_media<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    media_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_import_rule_media(rule_path, media_text)
}

pub(crate) fn set_css_style_sheet_live_font_feature_values_rule_font_family<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    font_families: Vec<style::values::computed::font::FamilyName>,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_font_feature_values_rule_font_family(rule_path, font_families)
}

pub(crate) fn set_css_style_sheet_live_font_feature_values_rule_map_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    group: crate::live_stylesheet::FontFeatureValuesMapGroup,
    name: &str,
    values: &[u32],
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_font_feature_values_rule_map_entry(rule_path, group, name, values)
}

pub(crate) fn delete_css_style_sheet_live_font_feature_values_rule_map_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    group: crate::live_stylesheet::FontFeatureValuesMapGroup,
    name: &str,
) -> Result<bool, CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .delete_font_feature_values_rule_map_entry(rule_path, group, name)
}

pub(crate) fn clear_css_style_sheet_live_font_feature_values_rule_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    group: crate::live_stylesheet::FontFeatureValuesMapGroup,
) -> Result<bool, CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .clear_font_feature_values_rule_map(rule_path, group)
}

pub(crate) fn set_css_style_sheet_live_style_rule_declarations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    declaration_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_style_rule_declarations(rule_path, declaration_text)
}

pub(crate) fn set_css_style_sheet_live_style_rule_selector<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    selector_text: &str,
    containing_rule_type_bits: u32,
    parse_relative_rule_type: Option<CssRuleType>,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).set_style_rule_selector(
        rule_path,
        selector_text,
        containing_rule_type_bits,
        parse_relative_rule_type,
    )
}

pub(crate) fn set_css_style_sheet_live_font_face_rule_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    descriptor_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_font_face_rule_descriptors(rule_path, descriptor_text)
}

pub(crate) fn set_css_style_sheet_live_page_rule_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    descriptor_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_page_rule_descriptors(rule_path, descriptor_text)
}

pub(crate) fn set_css_style_sheet_live_page_rule_selectors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    selector_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_page_rule_selectors(rule_path, selector_text)
}

pub(crate) fn set_css_style_sheet_live_page_margin_rule_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    descriptor_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_page_margin_rule_descriptors(rule_path, descriptor_text)
}

pub(crate) fn set_css_style_sheet_live_nested_declarations_rule_declarations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    declaration_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet)
        .set_nested_declarations_rule_declarations(rule_path, declaration_text)
}

pub(crate) fn set_css_style_sheet_live_keyframe_rule_declarations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    index: usize,
    declaration_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).set_keyframe_rule_declarations(
        parent_path,
        index,
        declaration_text,
    )
}

pub(crate) fn set_css_style_sheet_live_keyframe_rule_selector<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    parent_path: &[usize],
    index: usize,
    selector_text: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).set_keyframe_rule_selector(
        parent_path,
        index,
        selector_text,
    )
}

pub(crate) fn set_css_style_sheet_live_keyframes_rule_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    rule_path: &[usize],
    name: &str,
) -> Result<(), CssRuleInsertError> {
    require_css_style_sheet_live_stylesheet(scope, sheet).set_keyframes_rule_name(rule_path, name)
}
