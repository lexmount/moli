use crate::{
    css_style::{
        CssInlineStyleDeclarationState, CssStyleEntry as StyleEntry, canonical_style_property_name,
        mask_compat_property_name, mask_compat_value_is_supported, parse_css_declaration_list,
        stylo_mask_property_name, top_level_comma_separated_component_values,
        webkit_transform_origin_compat_property_name,
        webkit_transform_origin_compat_value_is_supported,
    },
    document_runtime::DomHandle,
    util::get_private_value,
};

use super::super::super::super::JsContextHost;
use super::super::super::{set_reflected_style_attribute_with_inline_base_url, style_string};
use super::super::STYLE_DECLARATION_BASE_URL_SLOT;
use super::properties::{
    all_shorthand_applies_to, animation_shorthand_longhands, box_shorthand_components,
    css_wide_keyword, font_shorthand_longhands, font_variant_longhands, shorthand_longhands,
    supported_declared_property, text_decoration_shorthand_longhands,
    transition_shorthand_longhands,
};
use super::values::{
    normalize_style_value_with_base, normalize_transition_behavior_list,
    normalize_transition_property_list, normalize_transition_timing_function_list,
    parse_transition_shorthand_entries,
};
use style::{
    context::QuirksMode,
    properties::{
        PropertyDeclarationId, PropertyId, SourcePropertyDeclaration, parse_one_declaration_into,
    },
    stylesheets::{CssRuleType, Origin, UrlExtraData},
};
use style_traits::{CssString, ParsingMode};

pub(in crate::native_bridge::element::styles) struct StyleObjectEntries {
    pub(in crate::native_bridge::element::styles) entries: Vec<StyleEntry>,
    pub(in crate::native_bridge::element::styles) base_url: Option<url::Url>,
}

fn inline_style_declaration_state_from_entries(
    entries: &[StyleEntry],
) -> CssInlineStyleDeclarationState {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let mut state = CssInlineStyleDeclarationState {
        entries: entries.to_vec(),
        ..Default::default()
    };
    let mut pdb_entries = Vec::new();
    for entry in entries {
        if style_entry_is_pdb_supplemental_side_entry(entry) {
            state.side_entries.push(entry.clone());
            continue;
        }
        if style_entry_is_pdb_safe(entry) {
            pdb_entries.push(entry.clone());
            continue;
        }
        state.side_entries.push(entry.clone());
    }
    if !pdb_entries.is_empty() {
        state.block = pdb_block_from_style_entries(&pdb_entries).unwrap_or_default();
    }
    state
}

fn inline_style_declaration_state_from_serialized_entries(
    entries: &[StyleEntry],
    css_text: &str,
    base_url: Option<&url::Url>,
) -> CssInlineStyleDeclarationState {
    if !inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(entries) {
        return inline_style_declaration_state_from_css_text(css_text, base_url);
    }
    let mut state = inline_style_declaration_state_from_entries(entries);
    state.refresh_pdb_entries();
    state
}

fn inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(
    entries: &[StyleEntry],
) -> bool {
    let mut has_pdb_entry = false;
    for entry in entries {
        if style_entry_is_pdb_safe(entry) && !style_entry_is_pdb_supplemental_side_entry(entry) {
            has_pdb_entry = true;
            continue;
        }
        if !style_entry_is_pdb_supplemental_side_entry(entry)
            && !inline_serialized_side_entry_can_seed_without_css_text_reparse(entry)
        {
            return false;
        }
    }
    has_pdb_entry
}

fn inline_serialized_side_entry_can_seed_without_css_text_reparse(entry: &StyleEntry) -> bool {
    if entry.value.is_empty() {
        return false;
    }
    let name = canonical_style_property_name(&entry.name);
    if moli_css_parse::is_cssom_custom_property_name(&name) {
        return true;
    }
    if !supported_declared_property(&name) {
        return false;
    }
    if shorthand_longhands(&name).is_some() {
        return false;
    }
    if let Some(affected_names) = style_property_affected_names_with_pdb(&name) {
        return affected_names.len() == 1 && affected_names[0] == name;
    }
    false
}

fn inline_style_declaration_state_from_css_text(
    css_text: &str,
    base_url: Option<&url::Url>,
) -> CssInlineStyleDeclarationState {
    let entries = parse_inline_css_text_with_base(css_text, base_url);
    inline_style_declaration_state_from_entries(&entries)
}

fn inline_style_declaration_state_for_handle(
    runtime: &JsContextHost,
    handle: DomHandle,
    base_url: Option<&url::Url>,
) -> CssInlineStyleDeclarationState {
    if runtime.element_inline_style_csp_state(handle)
        == crate::style_engine::InlineStyleCspState::BlockedAttribute
    {
        return CssInlineStyleDeclarationState::default();
    }
    runtime
        .element_inline_style_declaration_state(handle)
        .cloned()
        .unwrap_or_else(|| {
            inline_style_declaration_state_from_css_text(&style_string(runtime, handle), base_url)
        })
}

fn inline_css_text_pdb_storage_state(css_text: &str) -> Option<CssInlineStyleDeclarationState> {
    if let Some(entries) = inline_css_text_all_adapter_entries(css_text) {
        let mut state = inline_style_declaration_state_from_entries(&entries);
        state.refresh_pdb_entries();
        return Some(state);
    }
    if !inline_css_text_can_seed_plain_pdb_block(css_text) {
        return None;
    }
    let entries = parse_inline_css_text_with_base(css_text, None);
    if entries
        .iter()
        .any(style_entry_is_pdb_supplemental_side_entry)
        || inline_css_text_contains_animation_shorthand(css_text)
    {
        let mut state = inline_style_declaration_state_from_entries(&entries);
        state.refresh_pdb_entries();
        return Some(state);
    }
    Some(CssInlineStyleDeclarationState {
        block: moli_css_parse::parse_declaration_block(css_text),
        ..Default::default()
    })
}

fn inline_css_text_contains_animation_shorthand(css_text: &str) -> bool {
    parse_css_declaration_list(css_text)
        .into_iter()
        .any(|declaration| canonical_style_property_name(&declaration.name) == "animation")
}

fn inline_css_text_can_seed_plain_pdb_block(css_text: &str) -> bool {
    parse_css_declaration_list(css_text)
        .into_iter()
        .all(|declaration| {
            let name = canonical_style_property_name(declaration.name.trim());
            if name.is_empty() {
                return true;
            }
            if declaration.value.is_empty() && cssom_empty_specified_placeholder_property(&name) {
                return false;
            }
            if declaration.value.is_empty() && moli_css_parse::is_cssom_custom_property_name(&name)
            {
                return true;
            }
            if name == "all" {
                return false;
            }
            if css_value_uses_unresolved_cssom_storage(&declaration.value)
                && unresolved_box_shorthand_longhands(&name).is_some()
            {
                return false;
            }
            cssom_style_property_write_can_use_pdb_storage(&name, &declaration.value)
                && parse_style_property_entries_with_pdb(
                    &name,
                    &declaration.value,
                    declaration.priority,
                )
                .is_some()
        })
}

fn inline_css_text_all_adapter_entries(css_text: &str) -> Option<Vec<StyleEntry>> {
    let mut entries = Vec::new();
    let mut has_all = false;
    for declaration in parse_css_declaration_list(css_text) {
        let name = canonical_style_property_name(declaration.name.trim());
        if name.is_empty() {
            continue;
        }
        let parsed = if name == "all" {
            has_all = true;
            parse_style_property_entries_with_base(
                &name,
                &declaration.value,
                declaration.priority,
                None,
            )?
        } else {
            parse_style_property_entries_with_pdb(&name, &declaration.value, declaration.priority)?
        };
        retain_inline_css_text_adapter_entries(&mut entries, &name, &parsed.affected_names);
        entries.extend(parsed.entries);
    }
    has_all.then_some(entries)
}

fn retain_inline_css_text_adapter_entries(
    entries: &mut Vec<StyleEntry>,
    property: &str,
    affected_names: &[String],
) {
    if property == "all" {
        entries.retain(|entry| entry.name != "all" && !all_shorthand_applies_to(&entry.name));
        return;
    }
    entries.retain(|entry| {
        entry.name == "all" || !style_entry_affects_property_query(entry, property, affected_names)
    });
}

fn inline_state_has_unpreservable_side_entries_for_property(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
) -> bool {
    state.side_entries.iter().any(|entry| {
        style_entry_affects_property_query(entry, property, affected_names)
            && !style_entry_is_replaceable_by_pdb_property(entry, property, affected_names)
            && !style_entry_is_preservable_for_pdb_property(entry, property)
    })
}

fn inline_state_has_replaceable_side_entries_for_property(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
) -> bool {
    state
        .side_entries
        .iter()
        .any(|entry| style_entry_is_replaceable_by_pdb_property(entry, property, affected_names))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineStatePdbQueryCandidate {
    Pdb,
    Side,
    SupplementalSide,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PdbQueryPriority {
    Normal,
    Important,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct InlineStatePdbQueryResult {
    candidate: InlineStatePdbQueryCandidate,
    priority: PdbQueryPriority,
}

pub(crate) fn inline_state_property_value_with_pdb(
    state: &CssInlineStyleDeclarationState,
    property: &str,
) -> Option<String> {
    let overflow_supplemental_query =
        overflow_property_query_uses_pdb_supplemental_side_entries(property, &state.side_entries);
    if !cssom_style_property_query_uses_pdb(property) && !overflow_supplemental_query {
        return None;
    }
    let affected_names = style_property_affected_names_with_pdb(property)?;
    if property == "font-variant" {
        return pdb_property_value_for_cssom_query_with_side_entries(
            &state.block,
            property,
            &state.side_entries,
        );
    }
    let text_decoration_supplemental_query =
        text_decoration_property_query_uses_pdb_supplemental_side_entries(
            property,
            &state.side_entries,
        );
    if text_decoration_supplemental_query {
        return pdb_property_value_for_cssom_query_with_side_entries(
            &state.block,
            property,
            &state.side_entries,
        );
    }
    if overflow_supplemental_query {
        return pdb_property_value_for_cssom_query_with_side_entries(
            &state.block,
            property,
            &state.side_entries,
        );
    }
    let query = inline_state_pdb_property_query_candidate(state, property, &affected_names)?;
    match query.candidate {
        InlineStatePdbQueryCandidate::Pdb => pdb_property_value_for_cssom_query_with_side_entries(
            &state.block,
            property,
            &state.side_entries,
        ),
        InlineStatePdbQueryCandidate::SupplementalSide if text_decoration_supplemental_query => {
            pdb_property_value_for_cssom_query_with_side_entries(
                &state.block,
                property,
                &state.side_entries,
            )
        }
        InlineStatePdbQueryCandidate::SupplementalSide => inline_state_pdb_supplemental_side_entry(
            state,
            property,
            &affected_names,
            query.priority,
        )
        .map(|entry| entry.value),
        InlineStatePdbQueryCandidate::Side => None,
    }
}

pub(crate) fn inline_state_property_priority_with_pdb(
    state: &CssInlineStyleDeclarationState,
    property: &str,
) -> Option<bool> {
    let overflow_supplemental_query =
        overflow_property_query_uses_pdb_supplemental_side_entries(property, &state.side_entries);
    if !cssom_style_property_query_uses_pdb(property) && !overflow_supplemental_query {
        return None;
    }
    let affected_names = style_property_affected_names_with_pdb(property)?;
    if property == "font-variant" {
        return pdb_property_priority_for_cssom_query_with_side_entries(
            &state.block,
            property,
            &state.side_entries,
        );
    }
    let text_decoration_supplemental_query =
        text_decoration_property_query_uses_pdb_supplemental_side_entries(
            property,
            &state.side_entries,
        );
    if text_decoration_supplemental_query {
        return pdb_property_priority_for_cssom_query_with_side_entries(
            &state.block,
            property,
            &state.side_entries,
        );
    }
    if overflow_supplemental_query {
        return pdb_property_priority_for_cssom_query_with_side_entries(
            &state.block,
            property,
            &state.side_entries,
        );
    }
    let query = inline_state_pdb_property_query_candidate(state, property, &affected_names)?;
    match query.candidate {
        InlineStatePdbQueryCandidate::Pdb => {
            pdb_property_priority_for_cssom_query_with_side_entries(
                &state.block,
                property,
                &state.side_entries,
            )
        }
        InlineStatePdbQueryCandidate::SupplementalSide if text_decoration_supplemental_query => {
            pdb_property_priority_for_cssom_query_with_side_entries(
                &state.block,
                property,
                &state.side_entries,
            )
        }
        InlineStatePdbQueryCandidate::SupplementalSide => inline_state_pdb_supplemental_side_entry(
            state,
            property,
            &affected_names,
            query.priority,
        )
        .map(|entry| entry.priority),
        InlineStatePdbQueryCandidate::Side => None,
    }
}

fn inline_state_pdb_property_query_candidate(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
) -> Option<InlineStatePdbQueryResult> {
    if state.side_entries.is_empty()
        && !state.entries.iter().any(|entry| entry.name == "all")
        && !state.entries.iter().any(|entry| {
            entry.value.is_empty() || css_value_uses_unresolved_cssom_storage(&entry.value)
        })
    {
        return (!state.block.is_empty()).then_some(InlineStatePdbQueryResult {
            candidate: InlineStatePdbQueryCandidate::Pdb,
            priority: PdbQueryPriority::Normal,
        });
    }

    let mut remaining_side_entries = state.side_entries.clone();
    let mut normal = None;
    let mut important = None;
    for entry in &state.entries {
        let is_side_entry = remaining_side_entries
            .iter()
            .position(|side| style_entries_equal(side, entry))
            .map(|position| {
                remaining_side_entries.remove(position);
            })
            .is_some();
        if !style_entry_affects_property_query(entry, property, affected_names) {
            continue;
        }
        let candidate = if is_side_entry {
            if inline_state_block_contains_entry(state, entry) {
                InlineStatePdbQueryCandidate::Pdb
            } else if style_entry_is_pdb_supplemental_side_entry(entry) {
                InlineStatePdbQueryCandidate::SupplementalSide
            } else {
                InlineStatePdbQueryCandidate::Side
            }
        } else if inline_style_entry_is_pdb_storage_candidate(entry) {
            InlineStatePdbQueryCandidate::Pdb
        } else {
            return None;
        };
        let candidate = InlineStatePdbQueryResult {
            candidate,
            priority: if entry.priority {
                PdbQueryPriority::Important
            } else {
                PdbQueryPriority::Normal
            },
        };
        if entry.priority {
            important = Some(candidate);
        } else {
            normal = Some(candidate);
        }
    }

    if remaining_side_entries.iter().any(|entry| {
        style_entry_affects_property_query(entry, property, affected_names)
            && !style_entry_is_pdb_supplemental_side_entry(entry)
    }) {
        return None;
    }
    match important.or(normal) {
        Some(InlineStatePdbQueryResult {
            candidate: InlineStatePdbQueryCandidate::Side,
            ..
        })
        | None
            if state.block.is_empty() =>
        {
            None
        }
        None => Some(InlineStatePdbQueryResult {
            candidate: InlineStatePdbQueryCandidate::Pdb,
            priority: PdbQueryPriority::Normal,
        }),
        Some(candidate) => Some(candidate),
    }
}

fn inline_state_block_contains_entry(
    state: &CssInlineStyleDeclarationState,
    entry: &StyleEntry,
) -> bool {
    let block_entries = inline_state_block_entries(state);
    block_entries_contains_entry(&block_entries, entry)
}

fn inline_state_block_entries(state: &CssInlineStyleDeclarationState) -> Vec<StyleEntry> {
    state
        .block
        .entries()
        .into_iter()
        .map(StyleEntry::from)
        .collect()
}

fn inline_state_block_entries_for_property_mutation(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
) -> Vec<StyleEntry> {
    inline_state_block_entries(state)
        .into_iter()
        .filter(|entry| style_entry_affects_property_query(entry, property, affected_names))
        .collect()
}

fn block_entries_contains_entry(block_entries: &[StyleEntry], entry: &StyleEntry) -> bool {
    block_entries
        .iter()
        .any(|block_entry| style_entries_equal(block_entry, entry))
}

fn inline_state_pdb_supplemental_side_entry(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
    priority: PdbQueryPriority,
) -> Option<StyleEntry> {
    state
        .side_entries
        .iter()
        .rev()
        .find(|entry| {
            style_entry_affects_property_query(entry, property, affected_names)
                && style_entry_is_pdb_supplemental_side_entry(entry)
                && entry.priority == (priority == PdbQueryPriority::Important)
        })
        .cloned()
}

fn style_entry_is_replaceable_by_pdb_property(
    entry: &StyleEntry,
    property: &str,
    affected_names: &[String],
) -> bool {
    if affected_names.iter().any(|name| name == &entry.name) {
        return true;
    }
    if style_entry_is_preservable_for_pdb_property(entry, property) {
        return false;
    }
    if entry.name == property {
        return true;
    }
    if property == "all" {
        return all_shorthand_applies_to(&entry.name);
    }
    style_property_affected_names_with_pdb(&entry.name).is_some_and(|entry_affected_names| {
        entry_affected_names
            .iter()
            .all(|name| affected_names.iter().any(|affected| affected == name))
    })
}

fn style_entry_is_preservable_for_pdb_property(entry: &StyleEntry, property: &str) -> bool {
    matches!(
        (entry.name.as_str(), property),
        (
            "margin",
            "margin-top" | "margin-right" | "margin-bottom" | "margin-left"
        ) | (
            "padding",
            "padding-top" | "padding-right" | "padding-bottom" | "padding-left"
        ) | (
            "border-width",
            "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width"
        ) | (
            "border-style",
            "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style"
        ) | (
            "border-color",
            "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color"
        ) | (
            "border-width",
            "border-top" | "border-right" | "border-bottom" | "border-left"
        ) | (
            "border-style",
            "border-top" | "border-right" | "border-bottom" | "border-left"
        ) | (
            "border-color",
            "border-top" | "border-right" | "border-bottom" | "border-left"
        ) | (
            "border-top",
            "border-top-width" | "border-top-style" | "border-top-color"
        ) | (
            "border-right",
            "border-right-width" | "border-right-style" | "border-right-color"
        ) | (
            "border-bottom",
            "border-bottom-width" | "border-bottom-style" | "border-bottom-color"
        ) | (
            "border-left",
            "border-left-width" | "border-left-style" | "border-left-color"
        ) | (
            "outline",
            "outline-width" | "outline-style" | "outline-color"
        )
    )
}

fn style_entries_equal(left: &StyleEntry, right: &StyleEntry) -> bool {
    left.name == right.name && left.value == right.value && left.priority == right.priority
}

fn refresh_inline_state_entries_after_pdb_mutation(
    state: &mut CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
    new_entries: impl IntoIterator<Item = StyleEntry>,
    new_side_entries: impl IntoIterator<Item = StyleEntry>,
) {
    let new_entries = new_entries.into_iter().collect::<Vec<_>>();
    let new_side_entries = new_side_entries.into_iter().collect::<Vec<_>>();
    let has_renderer_order_projection =
        state.entries.iter().chain(new_entries.iter()).any(|entry| {
            entry.name.starts_with("--")
                || entry.value.is_empty()
                || css_value_uses_unresolved_cssom_storage(&entry.value)
        });
    let has_all_entry = state.entries.iter().any(|entry| entry.name == "all");
    if state.entries.is_empty()
        && !state.block.is_empty()
        && (!new_side_entries.is_empty() || has_renderer_order_projection)
    {
        state.entries = inline_state_block_entries(state);
    }
    if state.side_entries.is_empty()
        && new_side_entries.is_empty()
        && property != "all"
        && !has_all_entry
        && !has_renderer_order_projection
    {
        state.refresh_pdb_entries();
        return;
    }
    let block_entries = inline_state_block_entries(state);
    let mut retained_affecting_side_entries = state
        .side_entries
        .iter()
        .filter(|entry| {
            style_entry_affects_property_query(entry, property, affected_names)
                && !style_entry_is_replaceable_by_pdb_property(entry, property, affected_names)
                && !block_entries_contains_entry(&block_entries, entry)
        })
        .cloned()
        .collect::<Vec<_>>();
    state.side_entries.retain(|entry| {
        !block_entries_contains_entry(&block_entries, entry)
            && (!style_entry_affects_property_query(entry, property, affected_names)
                || !style_entry_is_replaceable_by_pdb_property(entry, property, affected_names))
    });
    state.entries.retain(|entry| {
        if entry.name == "all" {
            return property != "all";
        }
        if !style_entry_affects_property_query(entry, property, affected_names) {
            return true;
        }
        if style_entry_is_preservable_for_pdb_property(entry, property)
            && !(entry.value.is_empty() && affected_names.iter().any(|name| name == &entry.name))
        {
            return true;
        }
        if let Some(position) = retained_affecting_side_entries
            .iter()
            .position(|side| style_entries_equal(side, entry))
        {
            retained_affecting_side_entries.remove(position);
            return true;
        }
        false
    });
    expand_unresolved_box_shorthand_projection_after_mutation(
        &mut state.entries,
        &mut state.block,
        affected_names,
    );
    state.entries.extend(new_entries);
    state.side_entries.extend(new_side_entries);
}

fn expand_unresolved_box_shorthand_projection_after_mutation(
    entries: &mut Vec<StyleEntry>,
    block: &mut moli_css_parse::CssDeclarationBlock,
    affected_names: &[String],
) {
    let mut expanded = Vec::with_capacity(entries.len());
    for entry in entries.drain(..) {
        if css_value_uses_unresolved_cssom_storage(&entry.value)
            && let Some(longhands) = unresolved_box_shorthand_longhands(&entry.name)
            && longhands
                .iter()
                .any(|longhand| affected_names.iter().any(|affected| affected == longhand))
        {
            expanded.extend(
                longhands
                    .iter()
                    .filter(|longhand| !affected_names.iter().any(|affected| affected == *longhand))
                    .map(|longhand| {
                        let _ = block.set_property_with_projection(
                            longhand,
                            &entry.value,
                            entry.priority,
                        );
                        StyleEntry {
                            name: (*longhand).to_owned(),
                            value: String::new(),
                            priority: entry.priority,
                        }
                    }),
            );
            continue;
        }
        expanded.push(entry);
    }
    *entries = expanded;
}

pub(in crate::native_bridge::element::styles) fn expand_unresolved_box_shorthand_entries_for_mutation(
    entries: &mut Vec<StyleEntry>,
    affected_names: &[String],
) {
    let mut expanded = Vec::with_capacity(entries.len());
    for entry in entries.drain(..) {
        if css_value_uses_unresolved_cssom_storage(&entry.value)
            && let Some(longhands) = unresolved_box_shorthand_longhands(&entry.name)
            && longhands
                .iter()
                .any(|longhand| affected_names.iter().any(|affected| affected == longhand))
        {
            expanded.extend(
                longhands
                    .iter()
                    .filter(|longhand| !affected_names.iter().any(|affected| affected == *longhand))
                    .map(|longhand| StyleEntry {
                        name: (*longhand).to_owned(),
                        value: String::new(),
                        priority: entry.priority,
                    }),
            );
            continue;
        }
        expanded.push(entry);
    }
    *entries = expanded;
}

fn unresolved_box_shorthand_longhands(property: &str) -> Option<&'static [&'static str]> {
    match property {
        "margin" => Some(&["margin-top", "margin-right", "margin-bottom", "margin-left"]),
        "margin-inline" => Some(&["margin-inline-start", "margin-inline-end"]),
        "margin-block" => Some(&["margin-block-start", "margin-block-end"]),
        "padding" => Some(&[
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ]),
        "padding-block" => Some(&["padding-block-start", "padding-block-end"]),
        "padding-inline" => Some(&["padding-inline-start", "padding-inline-end"]),
        "overflow" => Some(&["overflow-x", "overflow-y"]),
        "outline" => Some(&["outline-width", "outline-style", "outline-color"]),
        "text-decoration" => Some(text_decoration_shorthand_longhands()),
        "text-emphasis" => Some(&["text-emphasis-style", "text-emphasis-color"]),
        "font-variant" => Some(font_variant_longhands()),
        "transition" => Some(transition_shorthand_longhands()),
        "animation" => Some(animation_shorthand_longhands()),
        "font" => Some(font_shorthand_longhands()),
        "background" => Some(&[
            "background-image",
            "background-position-x",
            "background-position-y",
            "background-size",
            "background-repeat",
            "background-attachment",
            "background-origin",
            "background-clip",
            "background-color",
        ]),
        "gap" => Some(&["row-gap", "column-gap"]),
        "place-content" => Some(&["align-content", "justify-content"]),
        "border-width" => Some(&[
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ]),
        "border-style" => Some(&[
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ]),
        "border-color" => Some(&[
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
        ]),
        "overscroll-behavior" => Some(&["overscroll-behavior-x", "overscroll-behavior-y"]),
        _ => None,
    }
}

pub(crate) fn parse_inline_css_text_with_base(
    style_text: &str,
    base_url: Option<&url::Url>,
) -> Vec<StyleEntry> {
    let mut entries: Vec<StyleEntry> = Vec::new();
    for declaration in parse_css_declaration_list(style_text) {
        let name = canonical_style_property_name(declaration.name.trim());
        if declaration.value.is_empty() && cssom_empty_specified_placeholder_property(&name) {
            entries.push(StyleEntry {
                name,
                value: String::new(),
                priority: declaration.priority,
            });
            continue;
        }
        if name.is_empty() {
            continue;
        }
        if cssom_style_property_write_uses_pdb(&name, &declaration.value) {
            if let Some(parsed) = parse_style_property_entries_with_pdb(
                &name,
                &declaration.value,
                declaration.priority,
            ) {
                entries.extend(parsed.entries);
            }
            continue;
        }
        if let Some(parsed) = parse_style_property_entries_with_base(
            &name,
            &declaration.value,
            declaration.priority,
            base_url,
        ) {
            entries.extend(parsed.entries);
        }
    }
    entries
}

pub(crate) fn parse_style_property_entries_for_cssom_write(
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    let name = canonical_style_property_name(name);
    if moli_css_parse::is_cssom_custom_property_name(&name) {
        return (!value.is_empty())
            .then(|| parse_style_property_entries_with_pdb(&name, value, priority))
            .flatten();
    }
    if name.starts_with("--") {
        return None;
    }
    if cssom_style_property_write_uses_pdb(&name, value) {
        return parse_style_property_entries_with_pdb(&name, value, priority);
    }
    parse_style_property_entries_with_base(&name, value, priority, base_url)
}

pub(in crate::native_bridge::element::styles) fn parse_style_property_entries_for_cssom_fallback_write(
    entries: &[StyleEntry],
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    if cssom_style_property_write_uses_pdb(name, value) {
        let parsed = parse_style_property_entries_for_cssom_write(name, value, priority, base_url)?;
        if let Some(affected_names) = style_property_mutation_affected_names_with_pdb(name)
            && entries.iter().any(|entry| {
                style_entry_affects_property_query(entry, name, &affected_names)
                    && !style_entry_is_replaceable_by_pdb_property(entry, name, &affected_names)
                    && !style_entry_is_preservable_for_pdb_property(entry, name)
            })
        {
            return parse_style_property_entries_with_base(name, value, priority, base_url);
        }
        return Some(parsed);
    }
    parse_style_property_entries_for_cssom_write(name, value, priority, base_url)
}

pub(in crate::native_bridge::element::styles) fn set_inline_style_property_with_pdb_storage(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    value: &str,
    priority: bool,
) -> Option<bool> {
    if name != "all" && !inline_style_property_write_can_use_pdb_storage(name, value) {
        return None;
    }
    let runtime = unsafe { &*runtime_ptr };
    let existing_inline_base_url = runtime.existing_element_inline_style_base_url(handle);
    let mut state = inline_style_declaration_state_for_handle(
        runtime,
        handle,
        existing_inline_base_url.as_ref(),
    );
    let update_inline_style_base = name == "background-image" && !value.is_empty();
    let inline_base_url = if update_inline_style_base {
        Some(runtime.element_inline_style_base_url(handle))
    } else {
        existing_inline_base_url
    };
    let affected_names = style_property_mutation_affected_names_with_pdb(name)?;
    if inline_state_has_unpreservable_side_entries_for_property(&state, name, &affected_names) {
        return None;
    }
    let has_replaceable_side_entry =
        inline_state_has_replaceable_side_entries_for_property(&state, name, &affected_names);
    let new_entries = if value.is_empty() {
        let mut removed_from_block = false;
        for affected in &affected_names {
            removed_from_block |= state.block.remove_property(affected).changed;
        }
        if !removed_from_block && !has_replaceable_side_entry {
            return Some(false);
        }
        (Vec::new(), Vec::new())
    } else if name == "all" {
        let parsed = parse_style_property_entries_with_base(name, value, priority, None)?;
        if state.block.set_property(name, value, priority)
            == moli_css_parse::CssSetResult::ParseError
        {
            return None;
        }
        let block_entries = state
            .block
            .entries()
            .into_iter()
            .map(StyleEntry::from)
            .collect::<Vec<_>>();
        if block_entries.iter().any(|entry| entry.name == "all") {
            (block_entries, Vec::new())
        } else {
            (parsed.entries, Vec::new())
        }
    } else {
        let parsed = parse_style_property_entries_with_pdb(name, value, priority)?;
        if parsed
            .entries
            .iter()
            .all(style_entry_is_pdb_supplemental_side_entry)
        {
            for affected in &affected_names {
                let _ = state.block.remove_property(affected);
            }
            (parsed.entries.clone(), parsed.entries)
        } else {
            let supplemental_entries = parsed
                .entries
                .iter()
                .filter(|entry| style_entry_is_pdb_supplemental_side_entry(entry))
                .cloned()
                .collect::<Vec<_>>();
            let uses_preferred_supplemental_entries =
                cssom_style_property_uses_preferred_pdb_supplemental_entries(name, value, priority);
            let mut entries = set_pdb_block_property_collecting_entries(
                &mut state.block,
                name,
                value,
                priority,
                &parsed,
                uses_preferred_supplemental_entries,
            )?;
            for affected in style_property_mutation_cleanup_names_with_pdb(name) {
                let _ = state.block.remove_property(&affected);
            }
            if uses_preferred_supplemental_entries {
                entries = parsed.entries.clone();
            } else if css_value_uses_unresolved_cssom_storage(value)
                && (entries.is_empty() || entries.iter().any(|entry| entry.value.is_empty()))
            {
                entries = parsed
                    .entries
                    .iter()
                    .filter(|entry| !style_entry_is_pdb_supplemental_side_entry(entry))
                    .cloned()
                    .collect();
            }
            if entries.is_empty() {
                entries =
                    inline_state_block_entries_for_property_mutation(&state, name, &affected_names);
            }
            if entries.is_empty() {
                (supplemental_entries.clone(), supplemental_entries)
            } else {
                entries.extend(supplemental_entries.iter().cloned());
                (entries, supplemental_entries)
            }
        }
    };
    let (new_entries, new_side_entries) = new_entries;
    refresh_inline_state_entries_after_pdb_mutation(
        &mut state,
        name,
        &affected_names,
        new_entries,
        new_side_entries,
    );
    let css_text = state.css_text();
    let resolution_text = state.style_resolution_text();
    if update_inline_style_base && let Some(inline_base_url) = &inline_base_url {
        unsafe { &mut *runtime_ptr }
            .set_element_inline_style_base_url(handle, inline_base_url.clone());
    }
    if style_string(runtime, handle) == css_text {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.set_element_inline_style_resolution_text(handle, resolution_text);
        runtime.set_element_inline_style_declaration_state(handle, state);
        return Some(false);
    }
    set_reflected_style_attribute_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &css_text,
        inline_base_url.as_ref(),
    );
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.set_element_inline_style_resolution_text(handle, resolution_text);
    runtime.set_element_inline_style_declaration_state(handle, state);
    Some(true)
}

pub(in crate::native_bridge::element::styles) fn set_inline_style_css_text_with_pdb_storage(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    css_text: &str,
) {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let mut state = if let Some(state) = inline_css_text_pdb_storage_state(css_text) {
        state
    } else {
        let runtime = unsafe { &*runtime_ptr };
        let base_url = runtime.element_inline_style_base_url(handle);
        inline_style_declaration_state_from_css_text(css_text, Some(&base_url))
    };
    state.refresh_pdb_entries();
    let css_text = state.css_text();
    let resolution_text = state.style_resolution_text();
    set_reflected_style_attribute_with_inline_base_url(scope, runtime_ptr, handle, &css_text, None);
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.set_element_inline_style_resolution_text(handle, resolution_text);
    runtime.set_element_inline_style_declaration_state(handle, state);
}

pub(crate) fn style_entries_css_text_with_pdb(entries: &[StyleEntry]) -> Option<String> {
    if entries.iter().any(|entry| {
        entry.name == "all"
            || css_value_uses_unresolved_cssom_storage(&entry.value)
                && unresolved_box_shorthand_longhands(&entry.name).is_some()
    }) {
        return None;
    }
    let block = style_entries_pdb_with_supplemental(entries)?;
    let side_entries = entries
        .iter()
        .filter(|entry| style_entry_is_pdb_supplemental_side_entry(entry))
        .cloned()
        .collect::<Vec<_>>();
    crate::css_style::serialize_css_style_entries_with_pdb_block(entries, &side_entries, &block)
}

pub(crate) fn style_entries_property_value_with_pdb(
    entries: &[StyleEntry],
    property: &str,
) -> Option<String> {
    let overflow_supplemental_query =
        overflow_property_query_uses_pdb_supplemental_side_entries(property, entries);
    if !cssom_style_property_query_uses_pdb(property) && !overflow_supplemental_query {
        return None;
    }
    let affected_names = style_property_affected_names_with_pdb(property)?;
    if property == "font-variant" {
        let block = style_entries_pdb_for_property_query_with_supplemental(entries, property)?;
        return pdb_property_value_for_cssom_query_with_side_entries(&block, property, entries);
    }
    let text_decoration_supplemental_query =
        text_decoration_property_query_uses_pdb_supplemental_side_entries(property, entries);
    if text_decoration_supplemental_query {
        let block = style_entries_pdb_for_property_query_with_supplemental(entries, property)?;
        return pdb_property_value_for_cssom_query_with_side_entries(&block, property, entries);
    }
    if overflow_supplemental_query {
        let block = style_entries_pdb_for_property_query_with_supplemental(entries, property)?;
        return pdb_property_value_for_cssom_query_with_side_entries(&block, property, entries);
    }
    let candidate = style_entries_pdb_property_query_candidate(entries, property, &affected_names)?;
    match candidate {
        StyleEntriesPdbQueryCandidate::Pdb => {
            let block = style_entries_pdb_for_property_query(entries, property)?;
            pdb_property_value_for_cssom_query_with_side_entries(&block, property, entries)
        }
        StyleEntriesPdbQueryCandidate::SupplementalSide(_)
            if text_decoration_supplemental_query =>
        {
            let block = style_entries_pdb_for_property_query_with_supplemental(entries, property)?;
            pdb_property_value_for_cssom_query_with_side_entries(&block, property, entries)
        }
        StyleEntriesPdbQueryCandidate::SupplementalSide(priority) => {
            style_entries_pdb_supplemental_entry(entries, property, &affected_names, priority)
                .map(|entry| entry.value)
        }
    }
}

pub(crate) fn style_entries_property_priority_with_pdb(
    entries: &[StyleEntry],
    property: &str,
) -> Option<bool> {
    let overflow_supplemental_query =
        overflow_property_query_uses_pdb_supplemental_side_entries(property, entries);
    if !cssom_style_property_query_uses_pdb(property) && !overflow_supplemental_query {
        return None;
    }
    let affected_names = style_property_affected_names_with_pdb(property)?;
    if property == "font-variant" {
        let block = style_entries_pdb_for_property_query_with_supplemental(entries, property)?;
        return pdb_property_priority_for_cssom_query_with_side_entries(&block, property, entries);
    }
    let text_decoration_supplemental_query =
        text_decoration_property_query_uses_pdb_supplemental_side_entries(property, entries);
    if text_decoration_supplemental_query {
        let block = style_entries_pdb_for_property_query_with_supplemental(entries, property)?;
        return pdb_property_priority_for_cssom_query_with_side_entries(&block, property, entries);
    }
    if overflow_supplemental_query {
        let block = style_entries_pdb_for_property_query_with_supplemental(entries, property)?;
        return pdb_property_priority_for_cssom_query_with_side_entries(&block, property, entries);
    }
    let candidate = style_entries_pdb_property_query_candidate(entries, property, &affected_names)?;
    match candidate {
        StyleEntriesPdbQueryCandidate::Pdb => {
            let block = style_entries_pdb_for_property_query(entries, property)?;
            pdb_property_priority_for_cssom_query_with_side_entries(&block, property, entries)
        }
        StyleEntriesPdbQueryCandidate::SupplementalSide(_)
            if text_decoration_supplemental_query =>
        {
            let block = style_entries_pdb_for_property_query_with_supplemental(entries, property)?;
            pdb_property_priority_for_cssom_query_with_side_entries(&block, property, entries)
        }
        StyleEntriesPdbQueryCandidate::SupplementalSide(priority) => {
            style_entries_pdb_supplemental_entry(entries, property, &affected_names, priority)
                .map(|entry| entry.priority)
        }
    }
}

fn style_entries_pdb_with_supplemental(
    entries: &[StyleEntry],
) -> Option<moli_css_parse::CssDeclarationBlock> {
    let mut pdb_entries = Vec::new();
    for entry in entries {
        if style_entry_is_pdb_safe(entry) && !style_entry_is_pdb_supplemental_side_entry(entry) {
            pdb_entries.push(entry.clone());
        } else if !style_entry_is_pdb_supplemental_side_entry(entry) {
            return None;
        }
    }
    pdb_block_from_style_entries(&pdb_entries)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleEntriesPdbQueryCandidate {
    Pdb,
    SupplementalSide(PdbQueryPriority),
}

fn style_entries_pdb_property_query_candidate(
    entries: &[StyleEntry],
    property: &str,
    affected_names: &[String],
) -> Option<StyleEntriesPdbQueryCandidate> {
    let mut normal = None;
    let mut important = None;
    let mut has_pdb_entry = false;
    for entry in entries {
        if style_entry_is_pdb_safe(entry) && !style_entry_is_pdb_supplemental_side_entry(entry) {
            has_pdb_entry = true;
        }
        if !style_entry_affects_property_query(entry, property, affected_names) {
            continue;
        }
        let candidate = if style_entry_is_pdb_supplemental_side_entry(entry) {
            StyleEntriesPdbQueryCandidate::SupplementalSide(if entry.priority {
                PdbQueryPriority::Important
            } else {
                PdbQueryPriority::Normal
            })
        } else if style_entry_is_pdb_safe(entry) {
            StyleEntriesPdbQueryCandidate::Pdb
        } else {
            return None;
        };
        if entry.priority {
            important = Some(candidate);
        } else {
            normal = Some(candidate);
        }
    }
    important
        .or(normal)
        .or_else(|| has_pdb_entry.then_some(StyleEntriesPdbQueryCandidate::Pdb))
}

fn style_entries_pdb_supplemental_entry(
    entries: &[StyleEntry],
    property: &str,
    affected_names: &[String],
    priority: PdbQueryPriority,
) -> Option<StyleEntry> {
    entries
        .iter()
        .rev()
        .find(|entry| {
            style_entry_affects_property_query(entry, property, affected_names)
                && style_entry_is_pdb_supplemental_side_entry(entry)
                && entry.priority == (priority == PdbQueryPriority::Important)
        })
        .cloned()
}

fn style_entries_pdb_for_property_query(
    entries: &[StyleEntry],
    property: &str,
) -> Option<moli_css_parse::CssDeclarationBlock> {
    let affected_names = style_property_affected_names_with_pdb(property)?;
    let mut pdb_entries = Vec::new();
    for entry in entries {
        if style_entry_is_pdb_safe(entry) && !style_entry_is_pdb_supplemental_side_entry(entry) {
            pdb_entries.push(entry.clone());
            continue;
        }
        if style_entry_affects_property_query(entry, property, &affected_names) {
            return None;
        }
    }
    if pdb_entries.is_empty() {
        return None;
    }
    pdb_block_from_style_entries(&pdb_entries)
}

fn style_entries_pdb_for_property_query_with_supplemental(
    entries: &[StyleEntry],
    property: &str,
) -> Option<moli_css_parse::CssDeclarationBlock> {
    let affected_names = style_property_affected_names_with_pdb(property)?;
    let mut pdb_entries = Vec::new();
    for entry in entries {
        if style_entry_is_pdb_safe(entry) && !style_entry_is_pdb_supplemental_side_entry(entry) {
            pdb_entries.push(entry.clone());
            continue;
        }
        if style_entry_affects_property_query(entry, property, &affected_names)
            && !style_entry_is_pdb_supplemental_side_entry(entry)
        {
            return None;
        }
    }
    pdb_block_from_style_entries(&pdb_entries)
}

pub(crate) fn pdb_property_value_for_cssom_query_with_side_entries(
    block: &moli_css_parse::CssDeclarationBlock,
    property: &str,
    side_entries: &[StyleEntry],
) -> Option<String> {
    if property == "text-decoration"
        && text_decoration_property_query_uses_pdb_supplemental_side_entries(property, side_entries)
    {
        return pdb_text_decoration_shorthand_value(block, side_entries);
    }
    if matches!(property, "overflow" | "overflow-x" | "overflow-y") {
        return pdb_overflow_property_value(block, property, side_entries);
    }
    if !pdb_property_is_declared_for_cssom_query(block, property) {
        return None;
    }
    let value = block.property_value(property)?;
    if value.is_empty() && moli_css_parse::is_cssom_custom_property_name(property) {
        return Some(" ".to_owned());
    }
    if value.is_empty() && !moli_css_parse::is_cssom_custom_property_name(property) {
        return None;
    }
    Some(value)
}

pub(crate) fn pdb_property_priority_for_cssom_query_with_side_entries(
    block: &moli_css_parse::CssDeclarationBlock,
    property: &str,
    side_entries: &[StyleEntry],
) -> Option<bool> {
    if property == "text-decoration"
        && text_decoration_property_query_uses_pdb_supplemental_side_entries(property, side_entries)
    {
        return pdb_text_decoration_shorthand_priority(block, side_entries);
    }
    if matches!(property, "overflow" | "overflow-x" | "overflow-y") {
        return pdb_overflow_property_priority(block, property, side_entries);
    }
    if !pdb_property_is_declared_for_cssom_query(block, property) {
        return None;
    }
    let value = block.property_value(property)?;
    if value.is_empty() && !moli_css_parse::is_cssom_custom_property_name(property) {
        return None;
    }
    Some(block.property_priority(property))
}

fn pdb_property_is_declared_for_cssom_query(
    block: &moli_css_parse::CssDeclarationBlock,
    property: &str,
) -> bool {
    if block.property_is_declared(property) {
        return true;
    }
    if block.entries().iter().any(|entry| entry.name == property) {
        return true;
    }
    let Some(affected_names) = style_property_affected_names_with_pdb(property) else {
        return false;
    };
    let is_longhand = affected_names.len() == 1 && affected_names[0] == property;
    if is_longhand {
        return false;
    }
    affected_names
        .iter()
        .filter(|name| name.as_str() != property)
        .any(|name| block.property_is_declared(name))
}

fn pdb_overflow_property_value(
    block: &moli_css_parse::CssDeclarationBlock,
    property: &str,
    side_entries: &[StyleEntry],
) -> Option<String> {
    match property {
        "overflow-x" | "overflow-y" => pdb_overflow_longhand_value(block, property, side_entries),
        "overflow" => {
            let x = pdb_overflow_longhand_value(block, "overflow-x", side_entries)?;
            let y = pdb_overflow_longhand_value(block, "overflow-y", side_entries)?;
            if x == y {
                Some(x)
            } else {
                Some(format!("{x} {y}"))
            }
        }
        _ => None,
    }
}

fn pdb_overflow_property_priority(
    block: &moli_css_parse::CssDeclarationBlock,
    property: &str,
    side_entries: &[StyleEntry],
) -> Option<bool> {
    match property {
        "overflow-x" | "overflow-y" => {
            pdb_overflow_longhand_priority(block, property, side_entries)
        }
        "overflow" => {
            let x = pdb_overflow_longhand_priority(block, "overflow-x", side_entries)?;
            let y = pdb_overflow_longhand_priority(block, "overflow-y", side_entries)?;
            (x == y).then_some(x)
        }
        _ => None,
    }
}

fn overflow_property_query_uses_pdb_supplemental_side_entries(
    property: &str,
    side_entries: &[StyleEntry],
) -> bool {
    matches!(property, "overflow" | "overflow-x" | "overflow-y")
        && side_entries.iter().any(|entry| {
            matches!(entry.name.as_str(), "overflow-x" | "overflow-y")
                && style_entry_is_pdb_supplemental_side_entry(entry)
        })
}

fn text_decoration_property_query_uses_pdb_supplemental_side_entries(
    property: &str,
    side_entries: &[StyleEntry],
) -> bool {
    property == "text-decoration"
        && side_entries.iter().any(|entry| {
            entry.name == "text-decoration-line"
                && style_entry_is_pdb_supplemental_side_entry(entry)
        })
}

fn pdb_text_decoration_shorthand_value(
    block: &moli_css_parse::CssDeclarationBlock,
    side_entries: &[StyleEntry],
) -> Option<String> {
    let entries = pdb_text_decoration_longhand_entries(block, side_entries)?;
    let css_wide_keywords = entries
        .iter()
        .map(|entry| css_wide_keyword(&entry.value))
        .collect::<Option<Vec<_>>>();
    if entries
        .iter()
        .any(|entry| css_wide_keyword(&entry.value).is_some())
    {
        let keywords = css_wide_keywords?;
        let first = keywords.first()?.clone();
        return keywords
            .iter()
            .all(|keyword| keyword == &first)
            .then_some(first);
    }
    Some(serialize_pdb_text_decoration_shorthand(
        &entries[0].value,
        &entries[1].value,
        &entries[2].value,
        &entries[3].value,
    ))
}

fn pdb_text_decoration_shorthand_priority(
    block: &moli_css_parse::CssDeclarationBlock,
    side_entries: &[StyleEntry],
) -> Option<bool> {
    let entries = pdb_text_decoration_longhand_entries(block, side_entries)?;
    let priority = entries.first()?.priority;
    entries
        .iter()
        .all(|entry| entry.priority == priority)
        .then_some(priority)
}

fn pdb_text_decoration_longhand_entries(
    block: &moli_css_parse::CssDeclarationBlock,
    side_entries: &[StyleEntry],
) -> Option<Vec<StyleEntry>> {
    let mut entries = Vec::new();
    let mut priority = None;
    for longhand in text_decoration_shorthand_longhands() {
        let entry = pdb_text_decoration_longhand_entry(block, side_entries, longhand)?;
        if priority.is_some_and(|current| current != entry.priority) {
            return None;
        }
        priority = Some(entry.priority);
        entries.push(entry);
    }
    Some(entries)
}

fn pdb_text_decoration_longhand_entry(
    block: &moli_css_parse::CssDeclarationBlock,
    side_entries: &[StyleEntry],
    property: &str,
) -> Option<StyleEntry> {
    if let Some(entry) = side_entries
        .iter()
        .rev()
        .find(|entry| entry.name == property && style_entry_is_pdb_supplemental_side_entry(entry))
    {
        return Some(entry.clone());
    }
    if !block.property_is_declared(property) {
        return None;
    }
    let value = block.property_value(property)?;
    (!value.is_empty()).then(|| StyleEntry {
        name: property.to_owned(),
        value,
        priority: block.property_priority(property),
    })
}

fn serialize_pdb_text_decoration_shorthand(
    line: &str,
    thickness: &str,
    style: &str,
    color: &str,
) -> String {
    let line = text_decoration_component_or_initial(line, "none");
    let thickness = text_decoration_component_or_initial(thickness, "auto");
    let style = text_decoration_component_or_initial(style, "solid");
    let color = text_decoration_component_or_initial(color, "currentcolor");
    let defaults =
        line == "none" && thickness == "auto" && style == "solid" && color == "currentcolor";

    let mut values = Vec::new();
    if defaults || line != "none" {
        values.push(line);
    }
    if thickness != "auto" {
        values.push(thickness);
    }
    if style != "solid" {
        values.push(style);
    }
    if !color.eq_ignore_ascii_case("currentcolor") {
        values.push(color);
    }
    values.join(" ")
}

fn text_decoration_component_or_initial<'a>(value: &'a str, initial: &'static str) -> &'a str {
    if value.is_empty() { initial } else { value }
}

fn pdb_overflow_longhand_value(
    block: &moli_css_parse::CssDeclarationBlock,
    property: &str,
    side_entries: &[StyleEntry],
) -> Option<String> {
    if let Some(entry) = side_entries
        .iter()
        .rev()
        .find(|entry| entry.name == property && style_entry_is_pdb_supplemental_side_entry(entry))
    {
        return Some(entry.value.clone());
    }
    if !block.property_is_declared(property) {
        return None;
    }
    let value = block.property_value(property)?;
    (!value.is_empty()).then_some(value)
}

fn pdb_overflow_longhand_priority(
    block: &moli_css_parse::CssDeclarationBlock,
    property: &str,
    side_entries: &[StyleEntry],
) -> Option<bool> {
    if let Some(entry) = side_entries
        .iter()
        .rev()
        .find(|entry| entry.name == property && style_entry_is_pdb_supplemental_side_entry(entry))
    {
        return Some(entry.priority);
    }
    if !block.property_is_declared(property) {
        return None;
    }
    let value = block.property_value(property)?;
    (!value.is_empty()).then(|| block.property_priority(property))
}

fn serialize_font_variant_shorthand_values(values: &[String]) -> Option<String> {
    if values.len() != font_variant_longhands().len() {
        return None;
    }
    if values.iter().any(|value| value.is_empty()) {
        return None;
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let first = values.first()?;
        return values
            .iter()
            .all(|value| value == first)
            .then(|| first.clone());
    }
    let non_normal = values
        .iter()
        .filter(|value| !value.eq_ignore_ascii_case("normal"))
        .collect::<Vec<_>>();
    if non_normal.is_empty() {
        return Some("normal".to_owned());
    }
    if values[0].eq_ignore_ascii_case("none") {
        return (non_normal.len() == 1).then(|| "none".to_owned());
    }
    Some(
        non_normal
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn style_entry_is_pdb_safe(entry: &StyleEntry) -> bool {
    let name = canonical_style_property_name(&entry.name);
    let value = pdb_mutation_value_for_style_entry(entry);
    if value.is_empty() && !moli_css_parse::is_cssom_custom_property_name(&name) {
        return false;
    }
    if entry.name == "all" {
        return parse_style_property_entries_with_pdb(&entry.name, &value, entry.priority)
            .is_some();
    }
    (inline_style_property_write_can_use_pdb_storage(&entry.name, &value)
        || cssom_border_image_reset_value_uses_pdb_storage(&entry.name, &value))
        && parse_style_property_entries_with_pdb(&entry.name, &value, entry.priority).is_some()
}

fn inline_style_entry_is_pdb_storage_candidate(entry: &StyleEntry) -> bool {
    let name = canonical_style_property_name(&entry.name);
    let value = pdb_mutation_value_for_style_entry(entry);
    if value.is_empty() && !moli_css_parse::is_cssom_custom_property_name(&name) {
        return false;
    }
    if style_entry_is_pdb_supplemental_side_entry(entry) {
        return true;
    }
    if entry.name == "all" {
        return parse_style_property_entries_with_pdb(&entry.name, &value, entry.priority)
            .is_some();
    }
    inline_style_property_write_can_use_pdb_storage(&entry.name, &value)
        && parse_style_property_entries_with_pdb(&entry.name, &value, entry.priority).is_some()
}

fn pdb_mutation_value_for_style_entry(entry: &StyleEntry) -> std::borrow::Cow<'_, str> {
    if entry.value.is_empty()
        && moli_css_parse::is_cssom_custom_property_name(&canonical_style_property_name(
            &entry.name,
        ))
    {
        return std::borrow::Cow::Borrowed(" ");
    }
    std::borrow::Cow::Borrowed(&entry.value)
}

pub(crate) fn style_entry_is_pdb_supplemental_side_entry(entry: &StyleEntry) -> bool {
    let Some(parsed) = parse_pdb_supplemental_entries(&entry.name, &entry.value, entry.priority)
    else {
        return false;
    };
    if parsed.entries.len() != 1 || !style_entries_equal(&parsed.entries[0], entry) {
        return false;
    }
    if parse_preferred_pdb_supplemental_entries(&entry.name, &entry.value, entry.priority)
        .is_some_and(|parsed| {
            parsed.entries.len() == 1 && style_entries_equal(&parsed.entries[0], entry)
        })
    {
        return true;
    }
    stylo_pdb_entries_for_property(
        &canonical_style_property_name(&entry.name),
        &entry.value,
        entry.priority,
    )
    .is_none_or(|parsed| parsed.entries.is_empty())
}

pub(crate) fn cssom_style_property_uses_preferred_pdb_supplemental_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> bool {
    let name = canonical_style_property_name(name);
    parse_preferred_pdb_supplemental_entries(&name, value, priority).is_some()
}

fn style_entry_affects_property_query(
    entry: &StyleEntry,
    property: &str,
    affected_names: &[String],
) -> bool {
    if prefixed_style_entry_is_independent_of_unprefixed_property(&entry.name, property) {
        return false;
    }
    if entry.name == property || affected_names.iter().any(|name| name == &entry.name) {
        return true;
    }
    if let Some(entry_affected_names) = style_property_affected_names_with_pdb(&entry.name)
        && entry_affected_names
            .iter()
            .any(|name| name == property || affected_names.iter().any(|affected| affected == name))
    {
        return true;
    }
    shorthand_longhands(&entry.name).is_some_and(|longhands| {
        longhands.iter().any(|longhand| {
            longhand == &property || affected_names.iter().any(|name| name == longhand)
        })
    })
}

fn prefixed_style_entry_is_independent_of_unprefixed_property(
    entry_name: &str,
    property: &str,
) -> bool {
    entry_name.starts_with("-webkit-") && !property.starts_with("-webkit-")
}

fn pdb_block_from_style_entries(
    entries: &[StyleEntry],
) -> Option<moli_css_parse::CssDeclarationBlock> {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let mut block = moli_css_parse::CssDeclarationBlock::default();
    let mut previous_important_entries = Vec::new();
    for entry in entries {
        if !style_entry_is_pdb_safe(entry) || style_entry_is_pdb_supplemental_side_entry(entry) {
            return None;
        }
        let restore_important_entries = if entry.priority {
            Vec::new()
        } else {
            style_entries_affecting_property(&previous_important_entries, &entry.name)
        };
        if !set_pdb_block_property_from_style_entry(&mut block, entry) {
            return None;
        }
        if entry.priority {
            previous_important_entries.push(entry.clone());
        }
        for important_entry in &restore_important_entries {
            if !set_pdb_block_property_from_style_entry(&mut block, important_entry) {
                return None;
            }
        }
    }
    Some(block)
}

fn set_pdb_block_property_from_style_entry(
    block: &mut moli_css_parse::CssDeclarationBlock,
    entry: &StyleEntry,
) -> bool {
    let value = pdb_mutation_value_for_style_entry(entry);
    block
        .set_property_with_projection(&entry.name, &value, entry.priority)
        .set_result
        != moli_css_parse::CssSetResult::ParseError
}

pub(crate) fn set_pdb_block_property_collecting_entries(
    block: &mut moli_css_parse::CssDeclarationBlock,
    name: &str,
    value: &str,
    priority: bool,
    parsed: &ParsedStylePropertyEntries,
    skip_original_projection: bool,
) -> Option<Vec<StyleEntry>> {
    if !skip_original_projection {
        let projection = block.set_property_with_projection(name, value, priority);
        if projection.set_result != moli_css_parse::CssSetResult::ParseError {
            return Some(
                projection
                    .entries
                    .into_iter()
                    .map(StyleEntry::from)
                    .collect(),
            );
        }
    }
    let mut entries = Vec::new();
    for entry in &parsed.entries {
        if style_entry_is_pdb_supplemental_side_entry(entry) {
            continue;
        }
        let projection =
            block.set_property_with_projection(&entry.name, &entry.value, entry.priority);
        if projection.set_result == moli_css_parse::CssSetResult::ParseError {
            return None;
        }
        entries.extend(projection.entries.into_iter().map(StyleEntry::from));
    }
    Some(entries)
}

fn style_entries_affecting_property(entries: &[StyleEntry], property: &str) -> Vec<StyleEntry> {
    let affected_names =
        style_property_mutation_affected_names_with_pdb(property).unwrap_or_default();
    entries
        .iter()
        .filter(|entry| {
            entry.priority && style_entry_affects_property_query(entry, property, &affected_names)
        })
        .cloned()
        .collect()
}

fn cssom_style_property_write_uses_pdb(name: &str, value: &str) -> bool {
    let name = canonical_style_property_name(name);
    if moli_css_parse::is_cssom_custom_property_name(&name) {
        return !value.is_empty() && stylo_pdb_entries_for_property(&name, value, false).is_some();
    }
    if css_value_uses_unresolved_cssom_storage(value) {
        return !cssom_style_property_write_requires_legacy_parser(&name, value)
            && stylo_pdb_entries_for_property(&name, value, false).is_some();
    }
    !cssom_style_property_write_requires_legacy_parser(&name, value)
        || cssom_ordinary_longhand_value_can_use_direct_pdb_write(&name, value)
}

fn inline_style_property_write_can_use_pdb_storage(name: &str, value: &str) -> bool {
    cssom_style_property_write_uses_pdb(name, value)
        || cssom_style_property_write_can_use_pdb_storage(name, value)
            && cssom_border_image_property_name(name)
        || inline_box_style_property_can_use_pdb_storage(name)
}

fn inline_box_style_property_can_use_pdb_storage(name: &str) -> bool {
    matches!(
        name,
        "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "margin-block"
            | "margin-block-start"
            | "margin-block-end"
            | "margin-inline"
            | "margin-inline-start"
            | "margin-inline-end"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "padding-block"
            | "padding-block-start"
            | "padding-block-end"
            | "padding-inline"
            | "padding-inline-start"
            | "padding-inline-end"
    )
}

pub(crate) fn cssom_style_property_write_can_use_pdb_storage(name: &str, value: &str) -> bool {
    if cssom_style_property_write_uses_pdb(name, value) {
        return true;
    }
    let name = canonical_style_property_name(name);
    if cssom_border_image_reset_value_uses_pdb_storage(&name, value) {
        return true;
    }
    cssom_legacy_parser_value_can_use_pdb_storage(&name, value)
}

fn cssom_ordinary_longhand_value_can_use_direct_pdb_write(name: &str, value: &str) -> bool {
    let name = canonical_style_property_name(name);
    if !cssom_numeric_longhand_can_skip_legacy_cssom_parser(&name) {
        return false;
    }
    cssom_legacy_parser_value_can_use_pdb_storage(&name, value)
        && style_property_affected_names_with_pdb(&name)
            .is_some_and(|affected_names| affected_names.len() == 1 && affected_names[0] == name)
}

fn cssom_legacy_parser_value_can_use_pdb_storage(name: &str, value: &str) -> bool {
    (css_math_value_property_requires_stylo_parser(name)
        || css_color_value_property_requires_stylo_parser(name))
        && !name.starts_with("--")
        && !name.starts_with("-webkit-")
        && !moli_css_parse::css_value_may_contain_var_function(value)
        && !moli_css_parse::css_value_may_contain_env_function(value)
        && !(name == "width"
            && value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("anchor-size("))
}

fn cssom_numeric_longhand_can_skip_legacy_cssom_parser(name: &str) -> bool {
    matches!(
        name,
        "bottom"
            | "height"
            | "left"
            | "margin-bottom"
            | "margin-left"
            | "margin-right"
            | "margin-top"
            | "max-height"
            | "max-width"
            | "min-height"
            | "min-width"
            | "padding-bottom"
            | "padding-left"
            | "padding-right"
            | "padding-top"
            | "right"
            | "top"
            | "width"
    )
}

fn cssom_border_image_reset_value_uses_pdb_storage(name: &str, value: &str) -> bool {
    if !cssom_border_image_property_name(name) {
        return false;
    }
    if css_wide_keyword(value).is_some() {
        return true;
    }
    let value = value.trim().to_ascii_lowercase();
    matches!(
        (name, value.as_str()),
        ("border-image", "none")
            | ("border-image-source", "none")
            | ("border-image-slice", "100%")
            | ("border-image-width", "1")
            | ("border-image-outset", "0")
            | ("border-image-repeat", "stretch")
    )
}

fn cssom_border_image_property_name(name: &str) -> bool {
    matches!(
        name,
        "border-image"
            | "border-image-source"
            | "border-image-slice"
            | "border-image-width"
            | "border-image-outset"
            | "border-image-repeat"
    )
}

fn cssom_style_property_query_uses_pdb(name: &str) -> bool {
    let name = canonical_style_property_name(name);
    let Some(affected_names) = style_property_affected_names_with_pdb(&name) else {
        return false;
    };
    if moli_css_parse::is_cssom_custom_property_name(&name) {
        return true;
    }
    let is_longhand_query = affected_names.len() == 1 && affected_names[0] == name;
    cssom_style_property_write_can_use_pdb_storage(&name, "") && is_longhand_query
        || cssom_style_shorthand_query_uses_pdb(&name)
        || cssom_animation_property_query_uses_pdb(&name, is_longhand_query)
}

fn cssom_animation_property_query_uses_pdb(name: &str, is_longhand_query: bool) -> bool {
    if !is_longhand_query {
        return false;
    }
    name == "animation-timeline"
        || name == "animation-range-start"
        || name == "animation-range-end"
        || animation_shorthand_longhands()
            .iter()
            .any(|longhand| longhand == &name)
}

fn cssom_style_shorthand_query_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "background"
            | "background-position"
            | "border"
            | "border-image"
            | "border-color"
            | "border-radius"
            | "border-style"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-width"
            | "flex"
            | "flex-flow"
            | "gap"
            | "grid-column"
            | "list-style"
            | "margin"
            | "margin-block"
            | "margin-inline"
            | "mask"
            | "outline"
            | "overscroll-behavior"
            | "page-break-after"
            | "page-break-before"
            | "page-break-inside"
            | "padding"
            | "place-content"
            | "text-decoration"
            | "text-emphasis"
            | "transition"
            | "white-space"
            | "font"
            | "font-variant"
            | "-webkit-text-stroke"
    )
}

fn cssom_style_property_write_requires_legacy_parser(name: &str, value: &str) -> bool {
    let canonical_name = canonical_style_property_name(name);
    // Stylo's typed transform-origin value cannot preserve whether an authored
    // zero depth was omitted. Chromium serializes `20px 30px` and
    // `20px 30px 0px` differently. The prefixed mask aliases likewise project
    // to canonical declarations and lose their authored CSSOM names and order.
    // Keep only those lossless-storage exceptions on the legacy parser.
    canonical_name.starts_with("-webkit-")
        && (canonical_name == "-webkit-transform-origin"
            || mask_compat_property_name(&canonical_name)
                && !stylo_mask_property_name(&canonical_name)
            || !stylo_pdb_owns_property(&canonical_name))
        || name == "font" && font_shorthand_value_requires_legacy_system_font_keyword(value)
        || name.starts_with("border-")
            && !cssom_border_property_write_uses_pdb(name)
            && !cssom_border_image_property_write_uses_pdb(name)
            && !cssom_structured_property_write_uses_pdb(name)
        || name.starts_with("outline-") && !cssom_outline_property_write_uses_pdb(name)
        || name == "width"
            && value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("anchor-size(")
        || cssom_style_entry_requires_structured_parser(name)
            && !css_value_uses_unresolved_cssom_storage(value)
            && !cssom_border_or_outline_property_write_uses_pdb(name)
            && !cssom_structured_property_write_uses_pdb(name)
            && !cssom_text_decoration_property_write_uses_pdb(name, value)
            && !cssom_text_emphasis_property_write_uses_pdb(name)
            && !cssom_animation_property_write_uses_pdb(name)
            && !cssom_transition_property_write_uses_pdb(name)
            && !cssom_font_property_write_uses_pdb(name, value)
            && !cssom_font_variant_property_write_uses_pdb(name)
            && !cssom_overflow_property_write_uses_pdb(name)
            && !cssom_webkit_text_stroke_property_write_uses_pdb(name)
}

fn stylo_pdb_owns_property(name: &str) -> bool {
    stylo_pdb_entries_for_property(name, "initial", false).is_some()
}

fn cssom_border_or_outline_property_write_uses_pdb(name: &str) -> bool {
    cssom_border_property_write_uses_pdb(name)
        || cssom_border_image_property_write_uses_pdb(name)
        || cssom_outline_property_write_uses_pdb(name)
}

fn cssom_border_image_property_write_uses_pdb(name: &str) -> bool {
    cssom_border_image_property_name(name)
}

fn cssom_structured_property_write_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "accent-color"
            | "align-content"
            | "align-items"
            | "align-self"
            | "alignment-baseline"
            | "background-attachment"
            | "background-blend-mode"
            | "background-color"
            | "background-position"
            | "background-size"
            | "block-size"
            | "box-shadow"
            | "baseline-source"
            | "bookmark-level"
            | "bookmark-state"
            | "border-collapse"
            | "caret-color"
            | "caption-side"
            | "clear"
            | "clip"
            | "color-scheme"
            | "column-rule-width"
            | "column-width"
            | "color"
            | "content"
            | "empty-cells"
            | "forced-color-adjust"
            | "gap"
            | "grid-column"
            | "isolation"
            | "justify-self"
            | "justify-content"
            | "column-gap"
            | "link-parameters"
            | "list-style"
            | "list-style-position"
            | "list-style-type"
            | "letter-spacing"
            | "margin"
            | "margin-block"
            | "margin-block-end"
            | "margin-block-start"
            | "margin-inline"
            | "margin-inline-end"
            | "margin-inline-start"
            | "mix-blend-mode"
            | "opacity"
            | "overscroll-behavior"
            | "overscroll-behavior-block"
            | "overscroll-behavior-inline"
            | "overscroll-behavior-x"
            | "overscroll-behavior-y"
            | "orphans"
            | "padding"
            | "padding-block-end"
            | "padding-block-start"
            | "padding-inline-end"
            | "padding-inline-start"
            | "page-break-after"
            | "page-break-before"
            | "page-break-inside"
            | "place-content"
            | "print-color-adjust"
            | "quotes"
            | "rotate"
            | "row-gap"
            | "scale"
            | "scrollbar-color"
            | "scrollbar-width"
            | "scroll-margin-top"
            | "scroll-padding-bottom"
            | "scroll-snap-align"
            | "shape-margin"
            | "tab-size"
            | "table-layout"
            | "text-indent"
            | "text-shadow"
            | "text-size-adjust"
            | "text-transform"
            | "text-underline-offset"
            | "text-underline-position"
            | "transform"
            | "will-change"
            | "widows"
            | "z-index"
            | "zoom"
    )
}

fn cssom_text_decoration_property_write_uses_pdb(name: &str, _value: &str) -> bool {
    matches!(
        name,
        "text-decoration"
            | "text-decoration-color"
            | "text-decoration-fill"
            | "text-decoration-inset"
            | "text-decoration-line"
            | "text-decoration-skip-ink"
            | "text-decoration-skip-spaces"
            | "text-decoration-stroke"
            | "text-decoration-style"
            | "text-decoration-thickness"
    )
}

fn cssom_text_emphasis_property_write_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "text-emphasis" | "text-emphasis-color" | "text-emphasis-position" | "text-emphasis-style"
    )
}

fn cssom_transition_property_write_uses_pdb(name: &str) -> bool {
    name == "transition" || name.starts_with("transition-")
}

fn cssom_webkit_text_stroke_property_write_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "-webkit-text-stroke" | "-webkit-text-stroke-color" | "-webkit-text-stroke-width"
    )
}

fn cssom_animation_property_write_uses_pdb(name: &str) -> bool {
    name == "animation"
        || name == "animation-range"
        || name == "animation-timeline"
        || name.starts_with("animation-")
}

fn cssom_font_property_write_uses_pdb(name: &str, value: &str) -> bool {
    if name == "font" && font_shorthand_value_requires_legacy_system_font_keyword(value) {
        return false;
    }
    name == "font" || font_shorthand_longhands().contains(&name)
}

fn font_shorthand_value_requires_legacy_system_font_keyword(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "caption" | "icon" | "menu" | "message-box" | "small-caption" | "status-bar"
    )
}

fn cssom_font_variant_property_write_uses_pdb(name: &str) -> bool {
    name == "font-variant" || font_variant_longhands().contains(&name)
}

fn cssom_overflow_property_write_uses_pdb(name: &str) -> bool {
    matches!(name, "overflow" | "overflow-x" | "overflow-y")
}

fn cssom_border_property_write_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "border"
            | "border-color"
            | "border-style"
            | "border-width"
            | "border-radius"
            | "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-right-radius"
            | "border-bottom-left-radius"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "border-block-end-color"
            | "border-block-start-color"
            | "border-inline-end-color"
            | "border-inline-start-color"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
    )
}

fn cssom_outline_property_write_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "outline" | "outline-width" | "outline-style" | "outline-color"
    )
}

fn css_value_contains_overlay_keyword(value: &str) -> bool {
    value
        .split(|ch: char| !matches!(ch, '-' | '_' | '0'..='9' | 'a'..='z' | 'A'..='Z'))
        .any(|token| token.eq_ignore_ascii_case("overlay"))
}

pub(crate) fn parse_style_property_entries_with_pdb(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let name = canonical_style_property_name(name);
    if let Some(parsed) = parse_preferred_pdb_supplemental_entries(&name, value, priority) {
        return Some(parsed);
    }
    if let Some(parsed) = stylo_pdb_entries_for_property(&name, value, priority)
        && !parsed.entries.is_empty()
    {
        return Some(parsed);
    }
    parse_pdb_supplemental_entries(&name, value, priority)
}

fn stylo_pdb_entries_for_property(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    if moli_css_parse::escape_top_level_semicolons(value) != value
        || moli_css_parse::split_important_priority(value).1
    {
        return None;
    }
    let mut block = moli_css_parse::CssDeclarationBlock::default();
    let projection = block.set_property_with_projection(name, value, priority);
    if projection.set_result == moli_css_parse::CssSetResult::ParseError {
        return None;
    }
    let mut accepted_names = projection.stored_names.clone();
    append_unique_name(&mut accepted_names, name);
    let affected_names = projection.affected_names.clone();
    let block_entries =
        if projection.has_unresolved_value || css_value_uses_unresolved_cssom_storage(value) {
            if !block.property_is_declared(name) {
                return None;
            }
            vec![moli_css_parse::CssDeclarationEntry {
                name: name.to_owned(),
                value: block.property_value(name)?,
                priority,
            }]
        } else {
            projection.entries
        };
    let mut entries = Vec::new();
    for entry in block_entries {
        let entry_name = canonical_style_property_name(&entry.name);
        let is_declared_empty_custom_property = entry.value.is_empty()
            && !value.is_empty()
            && moli_css_parse::is_cssom_custom_property_name(&entry_name)
            && block.property_is_declared(&entry_name);
        if entry_name.is_empty()
            || entry.value.is_empty() && !is_declared_empty_custom_property
            || entry.priority != priority
            || !accepted_names
                .iter()
                .any(|accepted| accepted == &entry_name)
        {
            return None;
        }
        entries.push(StyleEntry {
            name: entry_name,
            value: entry.value,
            priority: entry.priority,
        });
    }
    Some(ParsedStylePropertyEntries {
        entries,
        affected_names,
    })
}

fn css_value_uses_unresolved_cssom_storage(value: &str) -> bool {
    moli_css_parse::css_value_may_contain_var_function(value)
        || moli_css_parse::css_value_may_contain_env_function(value)
}

fn parse_pdb_supplemental_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    if let Some(entries) = parse_preferred_pdb_supplemental_entries(name, value, priority) {
        return Some(entries);
    }
    if let Some(entries) = parse_animation_numeric_property_entries(name, value, priority) {
        return Some(entries);
    }
    if let Some(parsed) = parse_overflow_overlay_supplemental_entries(name, value, priority) {
        return Some(parsed);
    }
    if name == "outline-color"
        && let Some(value) = normalize_outline_color_supplemental_value(value)
    {
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            affected_names: vec![name.to_owned()],
        });
    }
    parse_transition_numeric_property_entries(name, value, priority)
}

fn parse_text_decoration_line_supplemental_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    (name == "text-decoration-line")
        .then(|| normalize_text_decoration_line_compat_value(value))
        .flatten()
        .map(|value| ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            affected_names: vec![name.to_owned()],
        })
}

fn normalize_text_decoration_line_compat_value(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    cssom_text_decoration_line_value_is_compat(&value).then_some(value)
}

pub(crate) fn cssom_text_decoration_line_value_is_compat(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "spelling-error" | "grammar-error"
    )
}

fn normalize_outline_color_supplemental_value(value: &str) -> Option<String> {
    value
        .trim()
        .eq_ignore_ascii_case("invert")
        .then(|| "invert".to_owned())
}

fn parse_overflow_overlay_supplemental_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    if !cssom_overflow_property_write_uses_pdb(name) || !css_value_contains_overlay_keyword(value) {
        return None;
    }
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let values = match (name, tokens.as_slice()) {
        ("overflow", [single]) => {
            let value = normalize_overflow_overlay_longhand_value(single)?;
            [value.clone(), value]
        }
        ("overflow", [left, right]) => [
            normalize_overflow_overlay_longhand_value(left)?,
            normalize_overflow_overlay_longhand_value(right)?,
        ],
        ("overflow-x", [single]) => [
            normalize_overflow_overlay_longhand_value(single)?,
            String::new(),
        ],
        ("overflow-y", [single]) => [
            String::new(),
            normalize_overflow_overlay_longhand_value(single)?,
        ],
        _ => return None,
    };
    if !values.iter().any(|value| value == "overlay") {
        return None;
    }
    let mut affected_names = vec![name.to_owned()];
    if name != "overflow-x" {
        affected_names.push("overflow-x".to_owned());
    }
    if name != "overflow-y" {
        affected_names.push("overflow-y".to_owned());
    }
    let entries = ["overflow-x", "overflow-y"]
        .into_iter()
        .zip(values)
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| StyleEntry {
            name: name.to_owned(),
            value,
            priority,
        })
        .collect();
    Some(ParsedStylePropertyEntries {
        entries,
        affected_names,
    })
}

fn normalize_overflow_overlay_longhand_value(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    matches!(
        value.as_str(),
        "visible" | "hidden" | "clip" | "scroll" | "auto" | "overlay"
    )
    .then_some(value)
}

fn parse_preferred_pdb_supplemental_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    if let Some(parsed) = parse_text_decoration_line_supplemental_entries(name, value, priority) {
        return Some(parsed);
    }
    if let Some(parsed) = parse_overflow_overlay_supplemental_entries(name, value, priority) {
        return Some(parsed);
    }
    parse_animation_timing_function_preferred_supplemental_entries(name, value, priority)
}

fn parse_animation_timing_function_preferred_supplemental_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    if !matches!(
        name,
        "animation-timing-function" | "transition-timing-function"
    ) {
        return None;
    }
    let normalized = normalize_transition_timing_function_list(value)?;
    // Stylo owns timing-function acceptance. This adapter may retain a supplemental entry only
    // when Stylo accepted the value but cannot preserve its CSSOM specified serialization.
    let parsed = stylo_pdb_entries_for_property(name, value, priority)?;
    let pdb_round_trips_normalized_value = parsed.entries.len() == 1
        && parsed.entries[0].name == name
        && parsed.entries[0].value == normalized
        && parsed.entries[0].priority == priority;
    if pdb_round_trips_normalized_value {
        return None;
    }
    Some(ParsedStylePropertyEntries {
        entries: vec![StyleEntry {
            name: name.to_owned(),
            value: normalized,
            priority,
        }],
        affected_names: vec![name.to_owned()],
    })
}

pub(crate) fn style_property_affected_names_with_pdb(name: &str) -> Option<Vec<String>> {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let name = canonical_style_property_name(name);
    moli_css_parse::CssDeclarationBlock::affected_names_for_property(&name)
}

pub(crate) fn style_property_mutation_affected_names_with_pdb(name: &str) -> Option<Vec<String>> {
    let name = canonical_style_property_name(name);
    let mut affected_names = style_property_affected_names_with_pdb(&name)?;
    for longhand in style_property_mutation_cleanup_names_with_pdb(&name) {
        append_unique_name(&mut affected_names, &longhand);
    }
    Some(affected_names)
}

pub(crate) fn style_property_mutation_cleanup_names_with_pdb(name: &str) -> Vec<String> {
    let name = canonical_style_property_name(name);
    if name == "text-decoration" || text_decoration_standard_longhand_affects_family(&name) {
        return [
            "text-decoration-fill",
            "text-decoration-inset",
            "text-decoration-skip-ink",
            "text-decoration-skip-spaces",
            "text-decoration-stroke",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
    }
    Vec::new()
}

fn text_decoration_standard_longhand_affects_family(name: &str) -> bool {
    matches!(
        name,
        "text-decoration-color"
            | "text-decoration-line"
            | "text-decoration-style"
            | "text-decoration-thickness"
    )
}

fn append_unique_name(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|existing| existing == name) {
        names.push(name.to_owned());
    }
}

fn cssom_empty_specified_placeholder_property(name: &str) -> bool {
    matches!(
        name,
        "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "margin-inline-start"
            | "margin-inline-end"
            | "margin-block-start"
            | "margin-block-end"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "overscroll-behavior-block"
            | "overscroll-behavior-inline"
            | "overscroll-behavior-x"
            | "overscroll-behavior-y"
    )
}

pub(crate) struct ParsedStylePropertyEntries {
    pub(crate) entries: Vec<StyleEntry>,
    pub(crate) affected_names: Vec<String>,
}

pub(crate) fn parse_style_property_entries_with_base(
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let name = canonical_style_property_name(name);
    if name.starts_with("--") {
        if !moli_css_parse::is_cssom_custom_property_name(&name) {
            return None;
        }
        let value = moli_css_parse::normalize_custom_property_specified_value(value)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value,
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if !supported_declared_property(&name) {
        return None;
    }

    if mask_compat_property_name(&name)
        && !stylo_mask_property_name(&name)
        && !mask_compat_value_is_supported(&name, value)
    {
        return None;
    }
    if webkit_transform_origin_compat_property_name(&name)
        && !webkit_transform_origin_compat_value_is_supported(&name, value)
    {
        return None;
    }
    let property_write_uses_pdb = cssom_border_property_write_uses_pdb(&name)
        || cssom_border_image_property_write_uses_pdb(&name)
        || cssom_outline_property_write_uses_pdb(&name)
        || stylo_mask_property_name(&name)
        || cssom_overflow_property_write_uses_pdb(&name)
        || cssom_animation_property_write_uses_pdb(&name)
        || cssom_text_decoration_property_write_uses_pdb(&name, value)
        || cssom_text_emphasis_property_write_uses_pdb(&name)
        || cssom_font_property_write_uses_pdb(&name, value)
        || cssom_font_variant_property_write_uses_pdb(&name)
        || cssom_transition_property_write_uses_pdb(&name)
        || cssom_webkit_text_stroke_property_write_uses_pdb(&name);
    if property_write_uses_pdb {
        return parse_style_property_entries_with_pdb(&name, value, priority);
    }

    if moli_css_parse::css_value_may_contain_var_function(value) {
        let value = moli_css_parse::normalize_css_variable_specified_value(value)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value,
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if moli_css_parse::css_value_may_contain_env_function(value) {
        let value = normalize_style_value_with_base(&name, value, base_url);
        if value.is_empty() {
            return None;
        }
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value,
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if name == "width"
        && value
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("anchor-size(")
    {
        let value = normalize_style_value_with_base(&name, value, base_url);
        if value.is_empty() {
            return None;
        }
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value,
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if cssom_resolved_base_fallback_write_uses_pdb(&name, value) {
        return parse_style_property_entries_with_pdb(&name, value, priority);
    }

    if name == "background-image" && background_image_value_requires_stylo_parser(value) {
        return parse_strict_style_property_entries(&name, value, priority, base_url);
    }

    if cssom_style_entry_requires_structured_parser(&name) {
        return parse_strict_style_property_entries(&name, value, priority, base_url);
    }

    parse_normalized_style_property_entries(&name, value, priority, base_url)
}

fn cssom_resolved_base_fallback_write_uses_pdb(name: &str, value: &str) -> bool {
    !(css_value_uses_unresolved_cssom_storage(value)
        || moli_css_parse::css_value_may_contain_env_function(value)
        || name == "width"
            && value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("anchor-size("))
        && (cssom_structured_property_write_uses_pdb(name)
            || cssom_ordinary_longhand_value_can_use_direct_pdb_write(name, value))
}

fn parse_strict_style_property_entries(
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    if name == "all" {
        parse_style_property_with_stylo(name, value, base_url)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value: value.trim().to_ascii_lowercase(),
                priority,
            }],
            affected_names: vec![name.to_owned()],
        });
    }

    if name == "animation-timing-function" {
        parse_style_property_with_stylo(name, value, base_url)?;
        let value = normalize_transition_timing_function_list(value)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            affected_names: vec![name.to_owned()],
        });
    }

    if let Some(keyword) = css_wide_keyword(value)
        && let Some(longhands) = shorthand_longhands(name)
    {
        let mut affected_names = Vec::with_capacity(longhands.len() + 1);
        affected_names.push(name.to_owned());
        affected_names.extend(longhands.iter().map(|longhand| (*longhand).to_owned()));
        let entries = affected_names
            .iter()
            .filter(|affected| *affected != name)
            .map(|affected| StyleEntry {
                name: affected.clone(),
                value: keyword.clone(),
                priority,
            })
            .collect();
        return Some(ParsedStylePropertyEntries {
            entries,
            affected_names,
        });
    }

    if name == "transition" {
        let longhand_values = parse_transition_shorthand_entries(value)?;
        let entries = transition_shorthand_longhands()
            .iter()
            .zip(longhand_values)
            .map(|(longhand, values)| StyleEntry {
                name: (*longhand).to_owned(),
                value: values.join(", "),
                priority,
            })
            .collect::<Vec<_>>();
        let mut affected_names = vec![name.to_owned()];
        affected_names.extend(
            transition_shorthand_longhands()
                .iter()
                .map(|longhand| (*longhand).to_owned()),
        );
        return Some(ParsedStylePropertyEntries {
            entries,
            affected_names,
        });
    }

    if let Some(value) = normalize_transition_longhand(name, value) {
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            affected_names: vec![name.to_owned()],
        });
    }

    if let Some(entries) = parse_transition_numeric_property_entries(name, value, priority) {
        return Some(entries);
    }

    if let Some(longhands) = shorthand_longhands(name)
        && matches!(name, "border-color" | "border-style")
    {
        if let Some(keyword) = css_wide_keyword(value) {
            let entries = longhands
                .iter()
                .map(|longhand| StyleEntry {
                    name: (*longhand).to_owned(),
                    value: keyword.clone(),
                    priority,
                })
                .collect();
            let mut affected_names = vec![name.to_owned()];
            affected_names.extend(longhands.iter().map(|longhand| (*longhand).to_owned()));
            return Some(ParsedStylePropertyEntries {
                entries,
                affected_names,
            });
        }
        let value = normalize_style_value_with_base(name, value, base_url);
        if value.is_empty() {
            return None;
        }
        let components = box_shorthand_components(&value)?;
        let entries = longhands
            .iter()
            .zip(components)
            .map(|(longhand, value)| StyleEntry {
                name: (*longhand).to_owned(),
                value,
                priority,
            })
            .collect();
        let mut affected_names = vec![name.to_owned()];
        affected_names.extend(longhands.iter().map(|longhand| (*longhand).to_owned()));
        return Some(ParsedStylePropertyEntries {
            entries,
            affected_names,
        });
    }

    if let Some(mut declarations) = parse_style_property_with_stylo(name, value, base_url) {
        let mut affected_names = style_property_expanded_affected_names_with_pdb(name)?;
        let mut entries = Vec::new();
        for declaration in declarations.drain().declarations {
            let PropertyDeclarationId::Longhand(id) = declaration.id() else {
                continue;
            };
            let mut value = CssString::new();
            declaration.to_css(&mut value).ok()?;
            entries.push(StyleEntry {
                name: id.name().to_owned(),
                value,
                priority,
            });
        }
        if cssom_shorthand_store_should_preserve_property_entry(name) && !affected_names.is_empty()
        {
            affected_names.insert(0, name.to_owned());
            affected_names.dedup();
            if let Some(entry) = cssom_preserved_shorthand_entry(name, value, priority, &entries) {
                if border_shorthand_resets_border_image(name) {
                    affected_names.push("border-image".to_owned());
                    affected_names.dedup();
                }
                return Some(ParsedStylePropertyEntries {
                    entries: vec![entry],
                    affected_names,
                });
            }
        }
        if !entries.is_empty() {
            return Some(ParsedStylePropertyEntries {
                entries,
                affected_names,
            });
        }
    }

    parse_css_numeric_property_entries(name, value, priority)
}

fn parse_transition_numeric_property_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    matches!(name, "transition-delay" | "transition-duration")
        .then(|| parse_css_numeric_property_entries(name, value, priority))
        .flatten()
}

fn parse_animation_numeric_property_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    matches!(
        name,
        "animation-delay" | "animation-duration" | "animation-iteration-count"
    )
    .then(|| parse_css_numeric_property_entries(name, value, priority))
    .flatten()
}

fn cssom_shorthand_store_should_preserve_property_entry(name: &str) -> bool {
    matches!(
        name,
        "border"
            | "background"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-width"
            | "outline"
            | "font"
            | "font-variant"
    )
}

fn cssom_preserved_shorthand_entry(
    name: &str,
    input_value: &str,
    priority: bool,
    longhand_entries: &[StyleEntry],
) -> Option<StyleEntry> {
    let value = css_wide_keyword(input_value)
        .or_else(|| cssom_preserved_shorthand_value(name, longhand_entries))
        .or_else(|| {
            let value = normalize_style_value_with_base(name, input_value, None);
            (!value.is_empty()).then_some(value)
        })?;
    Some(StyleEntry {
        name: name.to_owned(),
        value,
        priority,
    })
}

fn cssom_preserved_shorthand_value(name: &str, longhand_entries: &[StyleEntry]) -> Option<String> {
    if name == "border" {
        return border_shorthand_value_from_longhands(longhand_entries);
    }
    if let Some(prefix) = border_side_shorthand_prefix(name) {
        return border_side_shorthand_value_from_longhands(longhand_entries, prefix);
    }
    if name == "outline" {
        return outline_shorthand_value_from_longhands(longhand_entries);
    }
    if name == "font-variant" {
        return font_variant_shorthand_value_from_longhands(longhand_entries);
    }
    let longhands = shorthand_longhands(name)?;
    let values = longhands
        .iter()
        .map(|longhand| style_entry_value(longhand_entries, longhand))
        .collect::<Option<Vec<_>>>()?;
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let first = values.first()?;
        return values
            .iter()
            .all(|value| value == first)
            .then(|| first.clone());
    }
    compress_box_components(&values)
}

fn border_shorthand_value_from_longhands(entries: &[StyleEntry]) -> Option<String> {
    let top = border_side_shorthand_value_from_longhands(entries, "border-top")?;
    let right = border_side_shorthand_value_from_longhands(entries, "border-right")?;
    let bottom = border_side_shorthand_value_from_longhands(entries, "border-bottom")?;
    let left = border_side_shorthand_value_from_longhands(entries, "border-left")?;
    (top == right && top == bottom && top == left).then_some(top)
}

fn outline_shorthand_value_from_longhands(entries: &[StyleEntry]) -> Option<String> {
    let width = style_entry_value(entries, "outline-width")?;
    let style = style_entry_value(entries, "outline-style")?;
    let color = style_entry_value(entries, "outline-color")?;
    border_side_shorthand_value(width, style, color)
}

fn font_variant_shorthand_value_from_longhands(entries: &[StyleEntry]) -> Option<String> {
    let values = font_variant_longhands()
        .iter()
        .map(|longhand| style_entry_value(entries, longhand).or_else(|| Some("normal".to_owned())))
        .collect::<Option<Vec<_>>>()?;
    serialize_font_variant_shorthand_values(&values)
}

fn border_side_shorthand_value_from_longhands(
    entries: &[StyleEntry],
    prefix: &str,
) -> Option<String> {
    let width = style_entry_value(entries, &format!("{prefix}-width"))?;
    let style = style_entry_value(entries, &format!("{prefix}-style"))?;
    let color = style_entry_value(entries, &format!("{prefix}-color"))?;
    border_side_shorthand_value(width, style, color)
}

fn border_side_shorthand_value(width: String, style: String, color: String) -> Option<String> {
    if [width.as_str(), style.as_str(), color.as_str()]
        .iter()
        .any(|value| css_wide_keyword(value).is_some())
    {
        return (width == style && width == color).then_some(width);
    }
    let mut parts = Vec::new();
    if width != "medium" {
        parts.push(width);
    }
    if style != "none" {
        parts.push(style);
    }
    if color != "currentcolor" {
        parts.push(color);
    }
    Some(parts.join(" "))
}

fn style_entry_value(entries: &[StyleEntry], name: &str) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.value.clone())
}

fn compress_box_components(values: &[String]) -> Option<String> {
    match values {
        [start, end] if start == end => Some(start.clone()),
        [start, end] => Some(format!("{start} {end}")),
        [top, right, bottom, left] if top == right && top == bottom && top == left => {
            Some(top.clone())
        }
        [top, right, bottom, left] if top == bottom && right == left => {
            Some(format!("{top} {right}"))
        }
        [top, right, bottom, left] if right == left => Some(format!("{top} {right} {bottom}")),
        [top, right, bottom, left] => Some(format!("{top} {right} {bottom} {left}")),
        _ => None,
    }
}

fn border_shorthand_resets_border_image(name: &str) -> bool {
    name == "border"
}

fn border_side_shorthand_prefix(name: &str) -> Option<&'static str> {
    match name {
        "border-top" => Some("border-top"),
        "border-right" => Some("border-right"),
        "border-bottom" => Some("border-bottom"),
        "border-left" => Some("border-left"),
        _ => None,
    }
}

fn style_property_expanded_affected_names_with_pdb(name: &str) -> Option<Vec<String>> {
    let mut affected_names = style_property_mutation_affected_names_with_pdb(name)?;
    if shorthand_longhands(name).is_some() {
        affected_names.retain(|affected| affected != name);
    }
    Some(affected_names)
}

fn parse_style_property_with_stylo(
    name: &str,
    value: &str,
    base_url: Option<&url::Url>,
) -> Option<SourcePropertyDeclaration> {
    let base_url = base_url.cloned().unwrap_or_else(about_blank_url);
    let url_data = UrlExtraData::from(base_url);
    let property_id = PropertyId::parse_enabled_for_all_content(name).ok()?;
    let mut declarations = SourcePropertyDeclaration::default();
    parse_one_declaration_into(
        &mut declarations,
        property_id.clone(),
        value,
        Origin::Author,
        &url_data,
        None,
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        CssRuleType::Style,
    )
    .ok()?;
    Some(declarations)
}

fn about_blank_url() -> url::Url {
    url::Url::parse("about:blank").expect("static about:blank URL should parse")
}

fn parse_normalized_style_property_entries(
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    let value = normalize_style_value_with_base(name, value, base_url);
    if value.is_empty() {
        return None;
    }
    if value_mixes_css_wide_keyword(&value) {
        return None;
    }
    if name == "font" {
        let mut affected_names = Vec::with_capacity(font_variant_longhands().len() + 2);
        affected_names.push(name.to_owned());
        affected_names.push("font-variant".to_owned());
        affected_names.extend(
            font_variant_longhands()
                .iter()
                .map(|longhand| (*longhand).to_owned()),
        );
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            affected_names,
        });
    }
    let (entries, affected_names) = if let Some(longhands) = shorthand_longhands(name)
        && let Some(components) = box_shorthand_components(&value)
    {
        let entries = longhands
            .iter()
            .zip(components)
            .map(|(longhand, value)| StyleEntry {
                name: (*longhand).to_owned(),
                value,
                priority,
            })
            .collect();
        let mut affected_names = Vec::with_capacity(longhands.len() + 1);
        affected_names.push(name.to_owned());
        affected_names.extend(longhands.iter().map(|longhand| (*longhand).to_owned()));
        (entries, affected_names)
    } else {
        (
            vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            vec![name.to_owned()],
        )
    };
    Some(ParsedStylePropertyEntries {
        entries,
        affected_names,
    })
}

fn value_mixes_css_wide_keyword(value: &str) -> bool {
    let mut input = cssparser::ParserInput::new(value);
    let mut input = cssparser::Parser::new(&mut input);
    let mut component_count = 0usize;
    let mut has_css_wide_keyword = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            cssparser::Token::WhiteSpace(_) | cssparser::Token::Comment(_) => {}
            cssparser::Token::Ident(ident) => {
                component_count += 1;
                has_css_wide_keyword |= css_wide_keyword(ident.as_ref()).is_some();
            }
            _ => component_count += 1,
        }
    }
    has_css_wide_keyword && component_count > 1
}

pub(crate) fn cssom_style_entry_requires_structured_parser(name: &str) -> bool {
    css_math_value_property_requires_stylo_parser(name)
        || css_color_value_property_requires_stylo_parser(name)
        || name == "all"
        || name == "animation"
        || name.starts_with("animation-")
        || name == "background"
        || name == "background-blend-mode"
        || name == "bookmark-level"
        || name == "bookmark-state"
        || name == "color-scheme"
        || name == "column-rule-width"
        || name == "column-width"
        || name == "content"
        || name == "forced-color-adjust"
        || font_variant_longhands().contains(&name)
        || name == "grid-column"
        || name == "isolation"
        || name == "link-parameters"
        || name == "mix-blend-mode"
        || name == "overscroll-behavior"
        || name.starts_with("overscroll-behavior-")
        || name == "outline"
        || name == "orphans"
        || name == "page-break-after"
        || name == "page-break-before"
        || name == "page-break-inside"
        || name == "print-color-adjust"
        || name == "quotes"
        || name == "scroll-margin-top"
        || name == "scroll-padding-bottom"
        || name == "scroll-snap-align"
        || name == "scrollbar-color"
        || name == "scrollbar-width"
        || name == "shape-margin"
        || name == "text-size-adjust"
        || name == "text-decoration"
        || name.starts_with("text-decoration-")
        || name == "text-emphasis"
        || name.starts_with("text-emphasis-")
        || name == "text-shadow"
        || name == "text-underline-position"
        || name == "text-underline-offset"
        || name == "transition"
        || name.starts_with("transition-")
        || name == "-webkit-text-stroke"
        || name.starts_with("-webkit-text-stroke-")
        || name == "widows"
        || name == "will-change"
        || name == "zoom"
}

fn css_color_value_property_requires_stylo_parser(name: &str) -> bool {
    matches!(
        name,
        "accent-color" | "background-color" | "caret-color" | "color"
    )
}

fn background_image_value_requires_stylo_parser(value: &str) -> bool {
    let lower = value.trim_start().to_ascii_lowercase();
    lower.starts_with("image-set(") || lower.starts_with("-webkit-image-set(")
}

fn css_math_value_property_requires_stylo_parser(name: &str) -> bool {
    matches!(
        name,
        "background-size"
            | "block-size"
            | "bottom"
            | "border"
            | "border-bottom"
            | "border-bottom-width"
            | "border-left"
            | "border-left-width"
            | "border-right"
            | "border-right-width"
            | "border-top"
            | "border-top-width"
            | "border-width"
            | "height"
            | "left"
            | "letter-spacing"
            | "margin"
            | "margin-bottom"
            | "margin-left"
            | "margin-right"
            | "margin-top"
            | "margin-block"
            | "margin-block-end"
            | "margin-block-start"
            | "margin-inline"
            | "margin-inline-end"
            | "margin-inline-start"
            | "max-height"
            | "max-width"
            | "min-height"
            | "min-width"
            | "opacity"
            | "padding"
            | "padding-bottom"
            | "padding-left"
            | "padding-right"
            | "padding-top"
            | "padding-block-end"
            | "padding-block-start"
            | "padding-inline-end"
            | "padding-inline-start"
            | "right"
            | "rotate"
            | "scale"
            | "tab-size"
            | "text-indent"
            | "top"
            | "transform"
            | "width"
            | "z-index"
    )
}

fn parse_css_numeric_property_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    let value = value.trim();
    let supported = match css_numeric_property_rule(name)? {
        CssNumericPropertyRule::TimeList { non_negative } => {
            css_time_list_is_supported(value, non_negative)
        }
        CssNumericPropertyRule::AnimationDurationList => {
            value.eq_ignore_ascii_case("auto") || css_time_list_is_supported(value, true)
        }
        CssNumericPropertyRule::AnimationIterationCountList => {
            css_animation_iteration_count_list_is_supported(value)
        }
    };
    supported.then(|| ParsedStylePropertyEntries {
        entries: vec![StyleEntry {
            name: name.to_owned(),
            value: value.to_owned(),
            priority,
        }],
        affected_names: vec![name.to_owned()],
    })
}

fn normalize_transition_longhand(name: &str, value: &str) -> Option<String> {
    let value = value.trim();
    if css_wide_keyword(value).is_some() {
        return Some(value.to_ascii_lowercase());
    }
    match name {
        "transition-property" => normalize_transition_property_list(value),
        "transition-timing-function" => normalize_transition_timing_function_list(value),
        "transition-behavior" => normalize_transition_behavior_list(value),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum CssNumericPropertyRule {
    TimeList { non_negative: bool },
    AnimationDurationList,
    AnimationIterationCountList,
}

fn css_numeric_property_rule(name: &str) -> Option<CssNumericPropertyRule> {
    Some(match name {
        "animation-delay" | "transition-delay" => CssNumericPropertyRule::TimeList {
            non_negative: false,
        },
        "animation-duration" => CssNumericPropertyRule::AnimationDurationList,
        "transition-duration" => CssNumericPropertyRule::TimeList { non_negative: true },
        "animation-iteration-count" => CssNumericPropertyRule::AnimationIterationCountList,
        _ => return None,
    })
}

fn css_time_list_is_supported(value: &str, non_negative: bool) -> bool {
    top_level_comma_separated_component_values(value)
        .filter(|components| !components.is_empty())
        .is_some_and(|components| {
            components.into_iter().all(|component| {
                let time = moli_css_parse::resolve_css_numeric(
                    &component,
                    moli_css_parse::CssNumericKind::Time,
                    moli_css_parse::CssNumericContext::supports_probe(),
                )
                .and_then(moli_css_parse::CssNumericValue::time_seconds);
                time.is_some() && (!non_negative || time.is_some_and(|seconds| seconds >= 0.0))
            })
        })
}

fn css_animation_iteration_count_list_is_supported(value: &str) -> bool {
    top_level_comma_separated_component_values(value)
        .filter(|components| !components.is_empty())
        .is_some_and(|components| {
            components.into_iter().all(|component| {
                component.eq_ignore_ascii_case("infinite")
                    || moli_css_parse::resolve_css_numeric(
                        &component,
                        moli_css_parse::CssNumericKind::Number,
                        moli_css_parse::CssNumericContext::supports_probe(),
                    )
                    .and_then(moli_css_parse::CssNumericValue::number)
                    .is_some_and(|value| value >= 0.0)
            })
        })
}

pub(in crate::native_bridge::element::styles) fn style_entries(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Vec<StyleEntry> {
    if runtime.element_inline_style_csp_state(handle)
        == crate::style_engine::InlineStyleCspState::BlockedAttribute
    {
        return Vec::new();
    }
    if let Some(state) = runtime.element_inline_style_declaration_state(handle) {
        return state.entries();
    }
    parse_inline_css_text_with_base(&style_string(runtime, handle), None)
}

fn style_entries_with_base(
    runtime: &JsContextHost,
    handle: DomHandle,
    base_url: Option<&url::Url>,
) -> Vec<StyleEntry> {
    if runtime.element_inline_style_csp_state(handle)
        == crate::style_engine::InlineStyleCspState::BlockedAttribute
    {
        return Vec::new();
    }
    if let Some(state) = runtime.element_inline_style_declaration_state(handle) {
        return state.entries();
    }
    parse_inline_css_text_with_base(&style_string(runtime, handle), base_url)
}

pub(in crate::native_bridge::element::styles) fn style_entries_for_style_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    runtime: &JsContextHost,
    handle: DomHandle,
) -> StyleObjectEntries {
    let base_url = get_private_value(scope, style, STYLE_DECLARATION_BASE_URL_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .and_then(|value| url::Url::parse(&value.to_rust_string_lossy(scope)).ok());
    let entries = style_entries_with_base(runtime, handle, base_url.as_ref());
    StyleObjectEntries { entries, base_url }
}

pub(in crate::native_bridge::element::styles) fn set_style_entries_with_inline_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    entries: &[StyleEntry],
    inline_base_url: Option<&url::Url>,
) {
    let state = inline_style_declaration_state_from_entries(entries);
    let css_text = state.css_text();
    let resolution_text = state.style_resolution_text();
    set_reflected_style_attribute_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &css_text,
        inline_base_url,
    );
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.set_element_inline_style_resolution_text(handle, resolution_text);
    runtime.set_element_inline_style_declaration_state(
        handle,
        inline_style_declaration_state_from_serialized_entries(entries, &css_text, inline_base_url),
    );
}

pub(in crate::native_bridge::element::styles) fn set_style_entries_if_changed_with_inline_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    entries: &[StyleEntry],
    inline_base_url: Option<&url::Url>,
) -> bool {
    let state = inline_style_declaration_state_from_entries(entries);
    let css_text = state.css_text();
    let resolution_text = state.style_resolution_text();
    if style_string(unsafe { &*runtime_ptr }, handle) == css_text {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.set_element_inline_style_resolution_text(handle, resolution_text);
        runtime.set_element_inline_style_declaration_state(
            handle,
            inline_style_declaration_state_from_serialized_entries(
                entries,
                &css_text,
                inline_base_url,
            ),
        );
        return false;
    }
    set_reflected_style_attribute_with_inline_base_url(
        scope,
        runtime_ptr,
        handle,
        &css_text,
        inline_base_url,
    );
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.set_element_inline_style_resolution_text(handle, resolution_text);
    runtime.set_element_inline_style_declaration_state(
        handle,
        inline_style_declaration_state_from_serialized_entries(entries, &css_text, inline_base_url),
    );
    true
}

#[cfg(test)]
mod tests {
    use crate::css_style::{CssInlineStyleDeclarationState, CssStyleEntry as StyleEntry};
    use crate::native_bridge::element::styles::declaration::properties::known_style_property;

    use super::{
        animation_shorthand_longhands, font_shorthand_longhands, font_variant_longhands,
        transition_shorthand_longhands,
    };
    use super::{
        cssom_style_property_write_uses_pdb, expand_unresolved_box_shorthand_entries_for_mutation,
        inline_css_text_pdb_storage_state,
        inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse,
        inline_state_block_entries_for_property_mutation,
        inline_state_has_replaceable_side_entries_for_property,
        inline_state_has_unpreservable_side_entries_for_property,
        inline_state_property_priority_with_pdb, inline_state_property_value_with_pdb,
        inline_style_declaration_state_from_css_text, inline_style_declaration_state_from_entries,
        inline_style_declaration_state_from_serialized_entries,
        inline_style_entry_is_pdb_storage_candidate, parse_style_property_entries_for_cssom_write,
        parse_style_property_entries_with_base, parse_style_property_entries_with_pdb,
        pdb_property_priority_for_cssom_query_with_side_entries,
        pdb_property_value_for_cssom_query_with_side_entries,
        refresh_inline_state_entries_after_pdb_mutation, set_pdb_block_property_collecting_entries,
        style_entry_is_pdb_safe, style_entry_is_pdb_supplemental_side_entry,
        style_property_affected_names_with_pdb, style_property_mutation_affected_names_with_pdb,
        stylo_pdb_entries_for_property, unresolved_box_shorthand_longhands,
    };

    fn style_entry(name: &str, value: &str) -> StyleEntry {
        StyleEntry {
            name: name.to_owned(),
            value: value.to_owned(),
            priority: false,
        }
    }

    fn important_style_entry(name: &str, value: &str) -> StyleEntry {
        StyleEntry {
            name: name.to_owned(),
            value: value.to_owned(),
            priority: true,
        }
    }

    #[test]
    fn css_math_properties_reject_invalid_values_with_stylo_parser() {
        for (property, value) in [
            ("transform", "rotate(calc((0.25turn error)))"),
            ("width", "calc(7px * up)"),
            ("width", "calc(5px / 1px)"),
            ("width", "calc(5px * 1px)"),
            ("width", "round(nearest, 1px, 1px, 1px)"),
            ("width", "round(nearest, 1px)"),
            ("width", "calc([])"),
            ("width", "calc( [])"),
        ] {
            assert!(
                parse_style_property_entries_with_base(property, value, false, None).is_none(),
                "{property}: {value} should be rejected"
            );
        }
    }

    #[test]
    fn cssom_write_rejects_mixed_css_wide_keywords() {
        for (property, value) in [
            ("border-spacing", "5px inherit"),
            ("margin", "inherit 5px"),
            ("border-radius", "1px 0 3px inherit"),
            ("overflow", "inherit scroll"),
        ] {
            assert!(
                parse_style_property_entries_for_cssom_write(property, value, false, None)
                    .is_none(),
                "{property}: {value} should reject CSS-wide keyword mixed with ordinary values"
            );
        }

        assert!(
            parse_style_property_entries_for_cssom_write("border-spacing", "inherit", false, None)
                .is_some()
        );
    }

    #[test]
    fn css_math_properties_serialize_through_stylo_parser() {
        let cssom_width = parse_style_property_entries_for_cssom_write(
            "width",
            "calc(10px + 1vmin + 10%)",
            false,
            None,
        )
        .expect("CSSOM width should parse through the PDB value-fragment path");
        assert!(
            cssom_style_property_write_uses_pdb("width", "calc(10px + 1vmin + 10%)"),
            "numeric longhand CSSOM writes should no longer fall back to the renderer base parser"
        );
        assert_eq!(cssom_width.entries.len(), 1);
        assert_eq!(cssom_width.entries[0].name, "width");
        assert_eq!(cssom_width.entries[0].value, "calc(10% + 10px + 1vmin)");
        assert!(style_entry_is_pdb_safe(&cssom_width.entries[0]));

        let cssom_height = parse_style_property_entries_for_cssom_write(
            "height",
            "clamp(1px,2px,3px)",
            false,
            None,
        )
        .expect("CSSOM height should parse through the PDB value-fragment path");
        assert!(cssom_style_property_write_uses_pdb(
            "height",
            "clamp(1px,2px,3px)"
        ));
        assert_eq!(cssom_height.entries.len(), 1);
        assert_eq!(cssom_height.entries[0].name, "height");
        assert_eq!(cssom_height.entries[0].value, "calc(2px)");
        assert!(style_entry_is_pdb_safe(&cssom_height.entries[0]));

        let cssom_margin =
            parse_style_property_entries_for_cssom_write("margin", "1px 2px", false, None)
                .expect("CSSOM margin shorthand should parse through the PDB value-fragment path");
        assert!(
            cssom_style_property_write_uses_pdb("margin", "1px 2px"),
            "physical box shorthand CSSOM writes should no longer fall back to the entries adapter"
        );
        assert_eq!(
            cssom_margin
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [
                ("margin-top", "1px"),
                ("margin-right", "2px"),
                ("margin-bottom", "1px"),
                ("margin-left", "2px")
            ]
        );
        assert!(cssom_margin.entries.iter().all(style_entry_is_pdb_safe));

        let cssom_padding = parse_style_property_entries_for_cssom_write(
            "padding",
            "calc(calc(12px)) 2px",
            false,
            None,
        )
        .expect("CSSOM padding shorthand should parse through the PDB value-fragment path");
        assert!(
            cssom_style_property_write_uses_pdb("padding", "calc(calc(12px)) 2px"),
            "physical padding shorthand CSSOM writes should no longer fall back to the entries adapter"
        );
        assert_eq!(
            cssom_padding
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [
                ("padding-top", "calc(12px)"),
                ("padding-right", "2px"),
                ("padding-bottom", "calc(12px)"),
                ("padding-left", "2px")
            ]
        );
        assert!(cssom_padding.entries.iter().all(style_entry_is_pdb_safe));

        let cssom_margin_top = parse_style_property_entries_for_cssom_write(
            "margin-top",
            "clamp(1px,2px,3px)",
            false,
            None,
        )
        .expect("CSSOM margin-top should parse through the PDB value-fragment path");
        assert!(
            cssom_style_property_write_uses_pdb("margin-top", "clamp(1px,2px,3px)"),
            "physical box longhand CSSOM writes should no longer fall back to the entries adapter"
        );
        assert_eq!(cssom_margin_top.entries.len(), 1);
        assert_eq!(cssom_margin_top.entries[0].name, "margin-top");
        assert_eq!(cssom_margin_top.entries[0].value, "calc(2px)");
        assert!(style_entry_is_pdb_safe(&cssom_margin_top.entries[0]));

        let cssom_margin_block =
            parse_style_property_entries_for_cssom_write("margin-block", "1px 2px", false, None)
                .expect("CSSOM margin-block should parse through the PDB value-fragment path");
        assert!(
            cssom_style_property_write_uses_pdb("margin-block", "1px 2px"),
            "logical box shorthand CSSOM writes should no longer fall back to the entries adapter"
        );
        assert_eq!(
            cssom_margin_block
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [("margin-block-start", "1px"), ("margin-block-end", "2px")]
        );
        assert!(
            cssom_margin_block
                .entries
                .iter()
                .all(style_entry_is_pdb_safe)
        );

        let cssom_margin_inline_start = parse_style_property_entries_for_cssom_write(
            "margin-inline-start",
            "clamp(1px,2px,3px)",
            false,
            None,
        )
        .expect("CSSOM margin-inline-start should parse through the PDB value-fragment path");
        assert!(cssom_style_property_write_uses_pdb(
            "margin-inline-start",
            "clamp(1px,2px,3px)"
        ));
        assert_eq!(cssom_margin_inline_start.entries.len(), 1);
        assert_eq!(
            cssom_margin_inline_start.entries[0].name,
            "margin-inline-start"
        );
        assert_eq!(cssom_margin_inline_start.entries[0].value, "calc(2px)");
        assert!(style_entry_is_pdb_safe(
            &cssom_margin_inline_start.entries[0]
        ));

        let cssom_padding_left = parse_style_property_entries_for_cssom_write(
            "padding-left",
            "calc(calc(12px))",
            false,
            None,
        )
        .expect("CSSOM padding-left should parse through the PDB value-fragment path");
        assert!(
            cssom_style_property_write_uses_pdb("padding-left", "calc(calc(12px))"),
            "physical padding longhand CSSOM writes should no longer fall back to the entries adapter"
        );
        assert_eq!(cssom_padding_left.entries.len(), 1);
        assert_eq!(cssom_padding_left.entries[0].name, "padding-left");
        assert_eq!(cssom_padding_left.entries[0].value, "calc(12px)");
        assert!(style_entry_is_pdb_safe(&cssom_padding_left.entries[0]));

        let cssom_padding_inline_end = parse_style_property_entries_for_cssom_write(
            "padding-inline-end",
            "calc(calc(12px))",
            false,
            None,
        )
        .expect("CSSOM padding-inline-end should parse through the PDB value-fragment path");
        assert!(cssom_style_property_write_uses_pdb(
            "padding-inline-end",
            "calc(calc(12px))"
        ));
        assert_eq!(cssom_padding_inline_end.entries.len(), 1);
        assert_eq!(
            cssom_padding_inline_end.entries[0].name,
            "padding-inline-end"
        );
        assert_eq!(cssom_padding_inline_end.entries[0].value, "calc(12px)");
        assert!(style_entry_is_pdb_safe(
            &cssom_padding_inline_end.entries[0]
        ));

        for (property, value, expected_value) in [
            ("background-size", "10px 20px", "10px 20px"),
            ("block-size", "clamp(1px,2px,3px)", "calc(2px)"),
            ("letter-spacing", "clamp(1px,2px,3px)", "calc(2px)"),
            ("opacity", "0.5", "0.5"),
            ("opacity", "50%", "0.5"),
            ("opacity", "calc(-50% - 50%)", "calc(-100%)"),
            ("opacity", "clamp(50%,80%,70%)", "clamp(50%, 80%, 70%)"),
            ("opacity", "calc(-0.5 - 0.5)", "calc(-1)"),
            ("rotate", "45deg", "45deg"),
            ("scale", "2", "2"),
            ("tab-size", "4", "4"),
            ("text-indent", "calc(calc(12px))", "calc(12px)"),
            ("z-index", "3", "3"),
        ] {
            let parsed = parse_style_property_entries_for_cssom_write(property, value, false, None)
                .unwrap_or_else(|| {
                    panic!("{property}: {value} should parse through the PDB value-fragment path")
                });
            assert!(
                cssom_style_property_write_uses_pdb(property, value),
                "{property}: {value} should no longer fall back to the renderer entries adapter"
            );
            assert_eq!(parsed.entries.len(), 1, "{property}: {value}");
            assert_eq!(parsed.entries[0].name, property, "{property}: {value}");
            assert_eq!(
                parsed.entries[0].value, expected_value,
                "{property}: {value}"
            );
            assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
            assert!(
                parse_style_property_entries_with_pdb(property, value, false).is_some(),
                "{property}: {value} should parse directly through PDB"
            );
        }

        for (property, value) in [
            ("background-size", "1px 2px 3px"),
            ("block-size", "banana"),
            ("letter-spacing", "1px 2px"),
            ("opacity", "banana"),
            ("rotate", "1px"),
            ("scale", "banana"),
            ("tab-size", "-1"),
            ("text-indent", "banana"),
            ("z-index", "1.5"),
        ] {
            assert!(
                parse_style_property_entries_for_cssom_write(property, value, false, None)
                    .is_none(),
                "{property}: {value} should be rejected by the PDB value-fragment path"
            );
            assert!(
                parse_style_property_entries_with_pdb(property, value, false).is_none(),
                "{property}: {value} should be rejected by direct PDB parsing"
            );
        }

        let width = parse_style_property_entries_with_base(
            "width",
            "calc(10px + 1vmin + 10%)",
            false,
            None,
        )
        .expect("valid calc width should parse");
        assert_eq!(width.entries.len(), 1);
        assert_eq!(width.entries[0].name, "width");
        assert_eq!(width.entries[0].value, "calc(10% + 10px + 1vmin)");

        let margin =
            parse_style_property_entries_with_base("margin-top", "clamp(1px,2px,3px)", false, None)
                .expect("valid clamp margin should parse");
        assert_eq!(margin.entries.len(), 1);
        assert_eq!(margin.entries[0].name, "margin-top");
        assert_eq!(margin.entries[0].value, "calc(2px)");

        let margin_shorthand = parse_style_property_entries_with_base("margin", "1px", false, None)
            .expect("valid margin shorthand should parse");
        assert_eq!(margin_shorthand.entries.len(), 4);
        assert_eq!(margin_shorthand.entries[0].name, "margin-top");
        assert_eq!(margin_shorthand.entries[0].value, "1px");
        assert_eq!(margin_shorthand.entries[1].name, "margin-right");
        assert_eq!(margin_shorthand.entries[1].value, "1px");
        assert_eq!(margin_shorthand.entries[2].name, "margin-bottom");
        assert_eq!(margin_shorthand.entries[2].value, "1px");
        assert_eq!(margin_shorthand.entries[3].name, "margin-left");
        assert_eq!(margin_shorthand.entries[3].value, "1px");

        let padding_shorthand =
            parse_style_property_entries_with_base("padding", "calc(calc(12px))", false, None)
                .expect("valid nested calc padding shorthand should parse");
        assert_eq!(padding_shorthand.entries.len(), 4);
        assert_eq!(padding_shorthand.entries[0].name, "padding-top");
        assert_eq!(padding_shorthand.entries[0].value, "calc(12px)");
        assert_eq!(padding_shorthand.entries[1].name, "padding-right");
        assert_eq!(padding_shorthand.entries[1].value, "calc(12px)");
        assert_eq!(padding_shorthand.entries[2].name, "padding-bottom");
        assert_eq!(padding_shorthand.entries[2].value, "calc(12px)");
        assert_eq!(padding_shorthand.entries[3].name, "padding-left");
        assert_eq!(padding_shorthand.entries[3].value, "calc(12px)");

        let border = parse_style_property_entries_with_base(
            "border",
            "calc(calc(10px)) solid pink",
            false,
            None,
        )
        .expect("valid nested calc border shorthand should parse through PDB");
        assert!(border.entries.iter().all(style_entry_is_pdb_safe));
        for (name, value) in [
            ("border-top-width", "calc(10px)"),
            ("border-right-width", "calc(10px)"),
            ("border-bottom-width", "calc(10px)"),
            ("border-left-width", "calc(10px)"),
            ("border-top-style", "solid"),
            ("border-right-style", "solid"),
            ("border-bottom-style", "solid"),
            ("border-left-style", "solid"),
        ] {
            assert!(
                border
                    .entries
                    .iter()
                    .any(|entry| entry.name == name && entry.value == value),
                "border base fallback should materialize PDB entry {name}: {value}"
            );
        }

        let border_width = parse_style_property_entries_with_base(
            "border-top-width",
            "calc(calc(10px))",
            false,
            None,
        )
        .expect("valid nested calc border width longhand should parse through PDB");
        assert!(border_width.entries.iter().all(style_entry_is_pdb_safe));
        assert!(
            border_width
                .entries
                .iter()
                .any(|entry| { entry.name == "border-top-width" && entry.value == "calc(10px)" })
        );

        let border_width_shorthand =
            parse_style_property_entries_with_base("border-width", "calc(calc(12px))", false, None)
                .expect("valid nested calc border-width shorthand should parse through PDB");
        assert!(
            border_width_shorthand
                .entries
                .iter()
                .all(style_entry_is_pdb_safe)
        );
        for longhand in [
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ] {
            assert!(
                border_width_shorthand
                    .entries
                    .iter()
                    .any(|entry| entry.name == longhand && entry.value == "calc(12px)"),
                "border-width base fallback should materialize PDB entry {longhand}"
            );
        }

        let border_top = parse_style_property_entries_with_base(
            "border-top",
            "calc(calc(11px)) solid pink",
            false,
            None,
        )
        .expect("valid nested calc border side shorthand should parse through PDB");
        assert!(border_top.entries.iter().all(style_entry_is_pdb_safe));
        for (name, value) in [
            ("border-top-width", "calc(11px)"),
            ("border-top-style", "solid"),
        ] {
            assert!(
                border_top
                    .entries
                    .iter()
                    .any(|entry| entry.name == name && entry.value == value),
                "border-top base fallback should materialize PDB entry {name}: {value}"
            );
        }

        for (property, value) in [
            ("border", "banana"),
            ("border-width", "1px 2px 3px 4px 5px"),
            ("border-top-width", "1px 2px"),
            ("border-image-width", "banana"),
        ] {
            assert!(
                parse_style_property_entries_with_base(property, value, false, None).is_none(),
                "{property}: {value} should be rejected by the PDB-backed base fallback"
            );
        }
    }

    #[test]
    fn transition_pdb_parser_accepts_dynamic_numeric_longhands() {
        let shorthand = parse_style_property_entries_with_base(
            "transition",
            "display 3s ease-in-out 1s allow-discrete, opacity",
            true,
            None,
        )
        .expect("base parser should route transition shorthand through PDB");
        for longhand in transition_shorthand_longhands() {
            assert!(
                shorthand.affected_names.iter().any(|name| name == longhand),
                "base parser transition should affect {longhand}"
            );
        }
        assert!(
            shorthand.entries.iter().any(|entry| {
                entry.name == "transition-duration" && entry.value == "3s, 0s" && entry.priority
            }),
            "base parser transition should retain PDB longhand projection"
        );
        assert!(
            shorthand.entries.iter().any(|entry| {
                entry.name == "transition-behavior"
                    && entry.value == "allow-discrete, normal"
                    && entry.priority
            }),
            "base parser transition should include behavior longhand projection"
        );

        let duration = parse_style_property_entries_with_pdb(
            "transition-duration",
            "calc(10s + (sign(2cqw - 10px) * 5s))",
            false,
        )
        .expect("dynamic transition-duration should parse for PDB-backed CSSOM");
        assert_eq!(duration.entries.len(), 1);
        assert_eq!(duration.entries[0].name, "transition-duration");
        assert_eq!(
            duration.entries[0].value,
            "calc(10s + (5s * sign(2cqw - 10px)))"
        );

        let timing = parse_style_property_entries_with_pdb(
            "transition-timing-function",
            "steps(calc(2 * sibling-index()), jump-none)",
            false,
        )
        .expect("dynamic transition-timing-function should parse for PDB-backed CSSOM");
        assert_eq!(timing.entries.len(), 1);
        assert_eq!(timing.entries[0].name, "transition-timing-function");
        assert_eq!(
            timing.entries[0].value,
            "steps(calc(2 * sibling-index()), jump-none)"
        );

        let base_duration = parse_style_property_entries_with_base(
            "transition-duration",
            "calc(10s + (sign(2cqw - 10px) * 5s))",
            false,
            None,
        )
        .expect("base parser should route dynamic transition-duration through PDB");
        assert_eq!(base_duration.entries.len(), 1);
        assert_eq!(base_duration.entries[0].name, "transition-duration");
        assert_eq!(
            base_duration.entries[0].value,
            "calc(10s + (5s * sign(2cqw - 10px)))"
        );
        assert!(!style_entry_is_pdb_supplemental_side_entry(
            &base_duration.entries[0]
        ));

        for (property, value) in [
            ("transition", "1s 2s 3s"),
            ("transition-duration", "-2s"),
            ("transition-property", "none, width"),
        ] {
            assert!(
                parse_style_property_entries_with_base(property, value, false, None).is_none(),
                "{property}: {value} should be rejected by the PDB write boundary"
            );
        }
    }

    #[test]
    fn base_parser_routes_remaining_pdb_families_through_pdb() {
        let animation = parse_style_property_entries_with_base(
            "animation",
            "fade paused both reverse 3 1s 2s linear",
            true,
            None,
        )
        .expect("base parser should route animation shorthand through PDB");
        for longhand in animation_shorthand_longhands() {
            assert!(
                animation.affected_names.iter().any(|name| name == longhand),
                "base parser animation should affect {longhand}"
            );
        }
        for reset_only in [
            "animation-timeline",
            "animation-range-start",
            "animation-range-end",
        ] {
            assert!(
                animation
                    .entries
                    .iter()
                    .any(|entry| entry.name == reset_only && entry.priority),
                "base parser animation should keep reset-only {reset_only} entries"
            );
        }

        let dynamic_duration = parse_style_property_entries_with_base(
            "animation-duration",
            "calc(10s + (sign(2cqw - 10px) * 5s))",
            false,
            None,
        )
        .expect("base parser should route dynamic animation-duration through PDB");
        assert_eq!(dynamic_duration.entries.len(), 1);
        assert_eq!(dynamic_duration.entries[0].name, "animation-duration");
        assert_eq!(
            dynamic_duration.entries[0].value,
            "calc(10s + (5s * sign(2cqw - 10px)))"
        );
        assert!(!style_entry_is_pdb_supplemental_side_entry(
            &dynamic_duration.entries[0]
        ));

        let font = parse_style_property_entries_with_base(
            "font",
            "italic small-caps 700 16px / 2 Ahem",
            true,
            None,
        )
        .expect("base parser should route font shorthand through PDB");
        for longhand in font_shorthand_longhands() {
            assert!(
                font.affected_names.iter().any(|name| name == longhand),
                "base parser font should affect {longhand}"
            );
        }
        assert!(style_entry_is_pdb_safe(&StyleEntry {
            name: "font".to_owned(),
            value: "italic small-caps 700 16px / 2 Ahem".to_owned(),
            priority: true,
        }));

        let stroke =
            parse_style_property_entries_with_base("-webkit-text-stroke", "1px red", true, None)
                .expect("base parser should route -webkit-text-stroke through PDB");
        assert_eq!(
            stroke
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
                .collect::<Vec<_>>(),
            [
                ("-webkit-text-stroke-width", "1px", true),
                ("-webkit-text-stroke-color", "red", true),
            ]
        );

        for (property, value) in [
            ("animation", "1s 2s 3s"),
            ("animation-duration", "-1s"),
            ("font", "16px"),
            ("-webkit-text-stroke", "banana red"),
        ] {
            assert!(
                parse_style_property_entries_with_base(property, value, false, None).is_none(),
                "{property}: {value} should be rejected by the PDB write boundary"
            );
        }
    }

    #[test]
    fn transition_shorthand_query_uses_pdb_longhand_state() {
        let mut block = moli_css_parse::parse_declaration_block(
            "transition: display 3s ease-in-out 1s allow-discrete, opacity !important;",
        );
        let projection = block.set_property_with_projection("transition-duration", "4s, 5s", true);
        assert_ne!(
            projection.set_result,
            moli_css_parse::CssSetResult::ParseError
        );

        assert_eq!(
            pdb_property_value_for_cssom_query_with_side_entries(&block, "transition", &[])
                .as_deref(),
            Some("display 4s ease-in-out 1s allow-discrete, opacity 5s")
        );
        assert_eq!(
            pdb_property_priority_for_cssom_query_with_side_entries(&block, "transition", &[]),
            Some(true)
        );
    }

    #[test]
    fn animation_pdb_parser_expands_shorthand_and_reset_only_entries() {
        let parsed = parse_style_property_entries_with_pdb(
            "animation",
            "fade paused both reverse 3 1s 2s linear",
            true,
        )
        .expect("animation shorthand should parse through PDB");
        let entries = parsed
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            [
                ("animation-duration", "1s", true),
                ("animation-timing-function", "linear", true),
                ("animation-delay", "2s", true),
                ("animation-iteration-count", "3", true),
                ("animation-direction", "reverse", true),
                ("animation-fill-mode", "both", true),
                ("animation-play-state", "paused", true),
                ("animation-name", "fade", true),
                ("animation-timeline", "auto", true),
                ("animation-range-start", "normal", true),
                ("animation-range-end", "normal", true)
            ]
        );
        assert_eq!(
            parsed.affected_names,
            [
                "animation",
                "animation-duration",
                "animation-timing-function",
                "animation-delay",
                "animation-iteration-count",
                "animation-direction",
                "animation-fill-mode",
                "animation-play-state",
                "animation-name",
                "animation-timeline",
                "animation-range-start",
                "animation-range-end"
            ]
        );

        let range =
            parse_style_property_entries_with_pdb("animation-range", "entry 10% exit 20%", false)
                .expect("animation-range shorthand should parse through PDB");
        assert_eq!(
            range
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [
                ("animation-range-start", "entry 10%"),
                ("animation-range-end", "exit 20%")
            ]
        );

        let timeline = parse_style_property_entries_with_pdb("animation-timeline", "auto", false)
            .expect("reset-only animation longhands should parse through PDB supplementals");
        assert_eq!(
            timeline
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [("animation-timeline", "auto")]
        );
        let range_start =
            parse_style_property_entries_with_pdb("animation-range-start", "normal", false)
                .expect("animation-range-start reset value should parse through PDB supplementals");
        assert_eq!(range_start.entries[0].value, "normal");
    }

    #[test]
    fn animation_pdb_parser_keeps_only_required_supplemental_side_entries() {
        let animation = style_entry("animation", "fade 1s linear");
        let range = style_entry("animation-range", "entry 10% exit 20%");
        let simple_duration = style_entry("animation-duration", "1s");
        let ordinary_timing = style_entry("animation-timing-function", "linear");
        let dynamic_duration =
            style_entry("animation-duration", "calc(10s + (sign(2cqw - 10px) * 5s))");
        let cssom_timing = style_entry("animation-timing-function", "linear(0, 1)");

        assert!(style_entry_is_pdb_safe(&animation));
        assert!(style_entry_is_pdb_safe(&range));
        assert!(style_entry_is_pdb_safe(&simple_duration));
        assert!(style_entry_is_pdb_safe(&ordinary_timing));
        assert!(
            !style_entry_is_pdb_supplemental_side_entry(&simple_duration),
            "ordinary animation-duration should stay only in the PDB block"
        );
        assert!(
            !style_entry_is_pdb_supplemental_side_entry(&ordinary_timing),
            "ordinary animation-timing-function should stay only in the PDB block"
        );
        assert!(
            !style_entry_is_pdb_supplemental_side_entry(&dynamic_duration),
            "dynamic animation-duration should stay in Stylo's PDB block"
        );
        assert!(
            style_entry_is_pdb_supplemental_side_entry(&cssom_timing),
            "animation-timing-function keeps CSSOM-compatible easing text"
        );

        let state = inline_style_declaration_state_from_entries(&[
            style_entry("animation-name", "fade"),
            dynamic_duration.clone(),
            cssom_timing.clone(),
        ]);
        assert_eq!(
            state
                .side_entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [(cssom_timing.name.as_str(), cssom_timing.value.as_str())]
        );
        assert_eq!(
            state.block.property_value("animation-duration").as_deref(),
            Some("calc(10s + (5s * sign(2cqw - 10px)))")
        );
        assert!(
            state
                .block
                .property_value("animation-timing-function")
                .is_none_or(|value| value.is_empty())
        );

        let ordinary_state = inline_style_declaration_state_from_entries(&[ordinary_timing]);
        assert!(
            ordinary_state.side_entries.is_empty(),
            "ordinary animation timing values should not keep supplemental side storage"
        );
        assert_eq!(
            ordinary_state
                .block
                .property_value("animation-timing-function")
                .as_deref(),
            Some("linear")
        );
    }

    #[test]
    fn font_variant_pdb_query_ignores_unrelated_block_entries() {
        let state = inline_style_declaration_state_from_entries(&[style_entry("color", "red")]);

        assert_eq!(
            inline_state_property_value_with_pdb(&state, "font-variant"),
            None
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "font-variant"),
            None
        );
    }

    #[test]
    fn inline_pdb_query_ignores_unrelated_block_entries() {
        let state = inline_style_declaration_state_from_entries(&[style_entry("color", "red")]);

        assert_eq!(
            inline_state_property_value_with_pdb(&state, "display"),
            None
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "display"),
            None
        );
    }

    #[test]
    fn css_color_properties_use_pdb_entries() {
        for property in ["accent-color", "background-color", "caret-color", "color"] {
            let parsed = parse_style_property_entries_for_cssom_write(
                property,
                "rgb(0 128 0 / 50%)",
                false,
                None,
            )
            .unwrap_or_else(|| panic!("{property} should accept a valid rgb color"));
            assert_eq!(parsed.entries.len(), 1);
            assert_eq!(parsed.entries[0].name, property);
            assert_eq!(parsed.entries[0].value, "rgba(0, 128, 0, 0.5)");
            assert!(
                parse_style_property_entries_with_pdb(property, "rgb(0 128 0 / 50%)", false)
                    .is_some(),
                "{property} should be owned by PDB for CSSOM color writes"
            );
            assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
            assert!(
                parse_style_property_entries_for_cssom_write(
                    property,
                    "rgb(clamp(10, none, 20) 0 0)",
                    false,
                    None,
                )
                .is_none(),
                "{property} should reject invalid math inside rgb()"
            );
        }

        for (property, value) in [("accent-color", "auto"), ("caret-color", "auto")] {
            let parsed = parse_style_property_entries_for_cssom_write(property, value, true, None)
                .unwrap_or_else(|| panic!("{property}: {value} should parse through PDB"));
            assert_eq!(parsed.entries.len(), 1);
            assert_eq!(parsed.entries[0].name, property);
            assert_eq!(parsed.entries[0].value, value);
            assert!(parsed.entries[0].priority);
            assert!(parse_style_property_entries_with_pdb(property, value, true).is_some());
            assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
        }
    }

    #[test]
    fn background_image_serializes_resolution_math_with_stylo_parser() {
        let parsed = parse_style_property_entries_with_base(
            "background-image",
            r#"image-set(url("") calc(1x * NaN))"#,
            false,
            None,
        )
        .expect("valid image-set resolution math should parse");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "background-image");
        assert_eq!(
            parsed.entries[0].value,
            r#"image-set(url("") calc(NaN * 1dppx))"#
        );
    }

    #[test]
    fn pdb_parser_accepts_gap_shorthand_entries() {
        let block = moli_css_parse::parse_declaration_block("gap: 10px 10px;");
        assert_eq!(block.property_value("gap").as_deref(), Some("10px"));

        let parsed = parse_style_property_entries_with_pdb("gap", "10px 10px", false)
            .expect("gap shorthand should parse through PDB");
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].name, "row-gap");
        assert_eq!(parsed.entries[0].value, "10px");
        assert_eq!(parsed.entries[1].name, "column-gap");
        assert_eq!(parsed.entries[1].value, "10px");

        for name in ["row-gap", "column-gap"] {
            let parsed = parse_style_property_entries_for_cssom_write(name, "567px", false, None)
                .unwrap_or_else(|| panic!("{name} should parse through PDB"));
            assert_eq!(parsed.entries.len(), 1);
            assert_eq!(parsed.entries[0].name, name);
            assert_eq!(parsed.entries[0].value, "567px");
            assert!(
                parse_style_property_entries_for_cssom_write(name, "1234", false, None).is_none(),
                "{name} should reject non-zero unitless CSSOM lengths"
            );
        }
    }

    #[test]
    fn inline_css_text_all_adapter_removes_preceding_reset_properties() {
        let state =
            inline_css_text_pdb_storage_state("display: block; all: inherit; padding-left: 1px;")
                .expect("all cssText should build a PDB storage state");

        assert_eq!(
            state
                .entries
                .into_iter()
                .map(|entry| (entry.name, entry.value))
                .collect::<Vec<_>>(),
            [
                ("all".to_owned(), "inherit".to_owned()),
                ("padding-left".to_owned(), "1px".to_owned())
            ]
        );
    }

    #[test]
    fn inline_dynamic_transition_entries_seed_pdb_without_css_text_reparse() {
        let duration = style_entry(
            "transition-duration",
            "calc(10s + (sign(2cqw - 10px) * 5s))",
        );
        let timing = style_entry(
            "transition-timing-function",
            "steps(calc(2 * sibling-index()), jump-none)",
        );
        assert!(
            inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(&[
                duration.clone(),
                timing.clone()
            ])
        );

        let state = inline_style_declaration_state_from_entries(&[duration, timing]);
        assert!(!state.block.is_empty());
        assert!(state.side_entries.is_empty());
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "transition-duration").as_deref(),
            Some("calc(10s + (5s * sign(2cqw - 10px)))")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "transition-timing-function").as_deref(),
            Some("steps(calc(2 * sibling-index()), jump-none)")
        );
    }

    #[test]
    fn inline_pdb_mutation_can_replace_fully_covered_side_entries() {
        let shorthand_state = CssInlineStyleDeclarationState {
            entries: vec![
                style_entry("--token", "value"),
                style_entry("padding-left", "var(--pad)"),
            ],
            side_entries: vec![
                style_entry("--token", "value"),
                style_entry("padding-left", "var(--pad)"),
            ],
            ..Default::default()
        };
        let padding_affected = style_property_affected_names_with_pdb("padding")
            .expect("padding affected names should come from PDB");
        assert!(
            !inline_state_has_unpreservable_side_entries_for_property(
                &shorthand_state,
                "padding",
                &padding_affected,
            ),
            "padding shorthand should be able to replace a legacy padding-left side entry"
        );
        assert!(
            inline_state_has_replaceable_side_entries_for_property(
                &shorthand_state,
                "padding",
                &padding_affected,
            ),
            "empty padding writes should still notice the replaceable side entry"
        );

        let longhand_state = CssInlineStyleDeclarationState {
            entries: vec![style_entry("padding", "var(--pad)")],
            side_entries: vec![style_entry("padding", "var(--pad)")],
            ..Default::default()
        };
        let padding_left_affected = style_property_affected_names_with_pdb("padding-left")
            .expect("padding-left affected names should come from PDB");
        assert!(
            parse_style_property_entries_with_pdb("padding-left", "1px", false).is_some(),
            "padding-left should be writable through PDB"
        );
        assert!(
            !inline_state_has_unpreservable_side_entries_for_property(
                &longhand_state,
                "padding-left",
                &padding_left_affected,
            ),
            "padding-left can preserve a legacy padding shorthand side entry while writing PDB"
        );
        assert!(
            !inline_state_has_replaceable_side_entries_for_property(
                &longhand_state,
                "padding-left",
                &padding_left_affected,
            ),
            "padding-left must not replace a legacy padding shorthand side entry"
        );

        let border_longhand_state = CssInlineStyleDeclarationState {
            entries: vec![style_entry("border-width", "var(--w)")],
            side_entries: vec![style_entry("border-width", "var(--w)")],
            ..Default::default()
        };
        let border_left_width_affected =
            style_property_affected_names_with_pdb("border-left-width")
                .expect("border-left-width affected names should come from PDB");
        assert!(
            parse_style_property_entries_with_pdb("border-left-width", "1px", false).is_some(),
            "border-left-width should be writable through PDB"
        );
        assert!(
            !inline_state_has_unpreservable_side_entries_for_property(
                &border_longhand_state,
                "border-left-width",
                &border_left_width_affected,
            ),
            "border-left-width can preserve a legacy border-width shorthand side entry"
        );
        assert!(
            !inline_state_has_replaceable_side_entries_for_property(
                &border_longhand_state,
                "border-left-width",
                &border_left_width_affected,
            ),
            "border-left-width must not replace a legacy border-width shorthand side entry"
        );

        let border_side_state = CssInlineStyleDeclarationState {
            entries: vec![style_entry("border-top", "var(--top)")],
            side_entries: vec![style_entry("border-top", "var(--top)")],
            ..Default::default()
        };
        let border_top_width_affected = style_property_affected_names_with_pdb("border-top-width")
            .expect("border-top-width affected names should come from PDB");
        assert!(
            parse_style_property_entries_with_pdb("border-top-width", "1px", false).is_some(),
            "border-top-width should be writable through PDB"
        );
        assert!(
            !inline_state_has_unpreservable_side_entries_for_property(
                &border_side_state,
                "border-top-width",
                &border_top_width_affected,
            ),
            "border-top-width can preserve a legacy border-top shorthand side entry"
        );
        assert!(
            !inline_state_has_replaceable_side_entries_for_property(
                &border_side_state,
                "border-top-width",
                &border_top_width_affected,
            ),
            "border-top-width must not replace a legacy border-top shorthand side entry"
        );

        let font_state = CssInlineStyleDeclarationState {
            entries: vec![style_entry("font", "var(--font)")],
            side_entries: vec![style_entry("font", "var(--font)")],
            ..Default::default()
        };
        let font_size_affected = style_property_affected_names_with_pdb("font-size")
            .expect("font-size affected names should come from PDB");
        assert!(
            inline_state_has_unpreservable_side_entries_for_property(
                &font_state,
                "font-size",
                &font_size_affected,
            ),
            "font shorthand partial coverage is still outside the proven adapter boundary"
        );
    }

    #[test]
    fn pdb_affected_names_cover_shorthand_families() {
        fn assert_affected_names_include(property: &str, expected: &[&str]) {
            let affected_names = style_property_affected_names_with_pdb(property)
                .unwrap_or_else(|| panic!("{property} should have PDB affected names"));
            for expected_name in expected {
                assert!(
                    affected_names
                        .iter()
                        .any(|affected_name| affected_name == expected_name),
                    "{property} should affect {expected_name}; got {affected_names:?}"
                );
            }
        }

        let font_affected = style_property_affected_names_with_pdb("font")
            .expect("font should have PDB affected names");
        for expected_name in font_shorthand_longhands()
            .iter()
            .copied()
            .chain(["font", "font-variant"])
        {
            assert!(
                font_affected
                    .iter()
                    .any(|affected| affected == expected_name),
                "font should affect {expected_name}; got {font_affected:?}"
            );
        }

        let font_variant_affected = style_property_affected_names_with_pdb("font-variant")
            .expect("font-variant should have PDB affected names");
        for expected_name in font_variant_longhands()
            .iter()
            .copied()
            .chain(["font-variant"])
        {
            assert!(
                font_variant_affected
                    .iter()
                    .any(|affected| affected == expected_name),
                "font-variant should affect {expected_name}; got {font_variant_affected:?}"
            );
        }

        assert_affected_names_include(
            "animation",
            &[
                "animation",
                "animation-name",
                "animation-duration",
                "animation-timeline",
                "animation-range-start",
                "animation-range-end",
            ],
        );
        assert_affected_names_include(
            "border",
            &[
                "border",
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
                "border-image",
            ],
        );
        assert_affected_names_include(
            "overscroll-behavior",
            &[
                "overscroll-behavior",
                "overscroll-behavior-x",
                "overscroll-behavior-y",
            ],
        );
        assert_eq!(
            style_property_affected_names_with_pdb("overscroll-behavior-block"),
            Some(vec!["overscroll-behavior-block".to_owned()])
        );
        assert_eq!(
            style_property_affected_names_with_pdb("overscroll-behavior-inline"),
            Some(vec!["overscroll-behavior-inline".to_owned()])
        );
    }

    #[test]
    fn inline_state_pdb_queries_use_stored_block_when_it_wins_order() {
        let no_side_state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                "display: block; padding-left: 1px !important;",
            ),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&no_side_state, "display").as_deref(),
            Some("block")
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&no_side_state, "padding-left"),
            Some(true)
        );

        let partial_side_state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("padding-left: 1px;"),
            entries: vec![
                style_entry("padding", "var(--pad)"),
                style_entry("padding-left", "1px"),
            ],
            side_entries: vec![style_entry("padding", "var(--pad)")],
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&partial_side_state, "padding-left").as_deref(),
            Some("1px")
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&partial_side_state, "padding-left"),
            Some(false)
        );

        let important_side_state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("padding-left: 1px;"),
            entries: vec![
                StyleEntry {
                    priority: true,
                    ..style_entry("padding", "var(--pad)")
                },
                style_entry("padding-left", "1px"),
            ],
            side_entries: vec![StyleEntry {
                priority: true,
                ..style_entry("padding", "var(--pad)")
            }],
        };
        assert!(
            inline_state_property_value_with_pdb(&important_side_state, "padding-left").is_none(),
            "important side shorthand must keep the query on the CSSOM adapter path"
        );
        assert!(
            inline_state_property_priority_with_pdb(&important_side_state, "padding-left")
                .is_none(),
            "important side shorthand must keep priority on the CSSOM adapter path"
        );

        let overflow_state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                "overflow-x: hidden; overflow-y: scroll;",
            ),
            ..Default::default()
        };
        assert!(
            inline_state_property_value_with_pdb(&overflow_state, "overflow").is_none(),
            "non-whitelisted shorthand queries must keep their CSSOM adapter semantics"
        );
    }

    #[test]
    fn inline_pdb_mutation_empty_return_uses_mutated_block_entries() {
        let mut state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                "display: block; color: rgb(0 128 0 / 50%);",
            ),
            entries: vec![
                style_entry("display", "block"),
                style_entry("--token", "value"),
                style_entry("color", "red"),
            ],
            side_entries: vec![style_entry("--token", "value")],
        };
        let affected_names = style_property_affected_names_with_pdb("color")
            .expect("color affected names should come from PDB");
        let rebuilt_entries =
            inline_state_block_entries_for_property_mutation(&state, "color", &affected_names);

        assert_eq!(rebuilt_entries.len(), 1);
        assert_eq!(rebuilt_entries[0].name, "color");
        assert_eq!(rebuilt_entries[0].value, "rgba(0, 128, 0, 0.5)");

        refresh_inline_state_entries_after_pdb_mutation(
            &mut state,
            "color",
            &affected_names,
            rebuilt_entries,
            Vec::<StyleEntry>::new(),
        );

        assert_eq!(
            state
                .entries()
                .into_iter()
                .map(|entry| (entry.name, entry.value))
                .collect::<Vec<_>>(),
            [
                ("display".to_owned(), "block".to_owned()),
                ("--token".to_owned(), "value".to_owned()),
                ("color".to_owned(), "rgba(0, 128, 0, 0.5)".to_owned()),
            ]
        );
        assert_eq!(
            state.css_text(),
            "display: block; --token: value; color: rgba(0, 128, 0, 0.5);"
        );
    }

    #[test]
    fn inline_pdb_transition_longhand_mutation_keeps_shorthand_query_with_side_entries() {
        let mut state = inline_style_declaration_state_from_entries(&[
            important_style_entry("transition-property", "display, opacity"),
            important_style_entry("transition-duration", "3s, 0s"),
            important_style_entry("transition-timing-function", "ease-in-out, ease"),
            important_style_entry("transition-delay", "1s, 0s"),
            important_style_entry("transition-behavior", "allow-discrete, normal"),
            style_entry("--token", "value"),
            style_entry("-webkit-transform-origin", "20px 30px"),
        ]);
        assert!(state.block.property_is_declared("transition-property"));
        assert!(state.block.property_is_declared("transition-duration"));
        assert_eq!(state.side_entries.len(), 1);
        assert_eq!(state.side_entries[0].name, "-webkit-transform-origin");
        assert_eq!(state.side_entries[0].value, "20px 30px");

        let affected_names = style_property_affected_names_with_pdb("transition-duration")
            .expect("transition-duration affected names should come from PDB");
        let parsed = parse_style_property_entries_with_pdb("transition-duration", "4s, 5s", true)
            .expect("transition-duration should parse through PDB");
        let entries = set_pdb_block_property_collecting_entries(
            &mut state.block,
            "transition-duration",
            "4s, 5s",
            true,
            &parsed,
            false,
        )
        .expect("transition-duration should update the PDB block");

        refresh_inline_state_entries_after_pdb_mutation(
            &mut state,
            "transition-duration",
            &affected_names,
            entries,
            Vec::<StyleEntry>::new(),
        );

        assert!(state.block.property_is_declared("transition-property"));
        assert!(state.block.property_is_declared("transition-duration"));
        assert_eq!(
            state.block.property_value("transition").as_deref(),
            Some("display 4s ease-in-out 1s allow-discrete, opacity 5s")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "transition").as_deref(),
            Some("display 4s ease-in-out 1s allow-discrete, opacity 5s")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "--token").as_deref(),
            Some("value")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "-webkit-transform-origin").as_deref(),
            None
        );
        assert_eq!(state.side_entries.len(), 1);
        assert_eq!(state.side_entries[0].name, "-webkit-transform-origin");
        assert_eq!(state.side_entries[0].value, "20px 30px");
    }

    #[test]
    fn border_shorthand_is_pdb_write_and_query_safe() {
        let parsed = parse_style_property_entries_with_pdb("border", "1px solid red", true)
            .expect("border shorthand should parse through PDB");
        for longhand in [
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
            "border-top-color",
            "border-right-color",
            "border-bottom-color",
            "border-left-color",
            "border-image-source",
            "border-image-slice",
            "border-image-width",
            "border-image-outset",
            "border-image-repeat",
        ] {
            assert!(
                parsed.entries.iter().any(|entry| entry.name == longhand),
                "border should materialize {longhand} through PDB"
            );
            assert!(
                parsed.affected_names.iter().any(|name| name == longhand),
                "border should affect {longhand}"
            );
        }
        assert!(
            parsed.affected_names.iter().any(|name| name == "border"),
            "border should affect its shorthand query"
        );
        assert!(
            parsed
                .affected_names
                .iter()
                .any(|name| name == "border-image"),
            "border should replace legacy border-image side entries"
        );
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "border".to_owned(),
                value: "1px solid red".to_owned(),
                priority: true,
            }),
            "border shorthand is now owned by Stylo/PDB"
        );

        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("border: 1px solid red !important;"),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "border").as_deref(),
            Some("1px solid red"),
            "border shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "border"),
            Some(true),
            "border shorthand priority should be served from PDB"
        );
    }

    #[test]
    fn border_image_shorthand_is_pdb_write_and_query_safe() {
        let parsed = parse_style_property_entries_with_pdb(
            "border-image",
            r#"url("img.png") 30 / 2 / 1 round"#,
            true,
        )
        .expect("border-image should parse through PDB");
        assert_eq!(
            parsed.affected_names,
            [
                "border-image",
                "border-image-outset",
                "border-image-repeat",
                "border-image-slice",
                "border-image-source",
                "border-image-width",
            ]
        );
        assert_eq!(
            parsed
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
                .collect::<Vec<_>>(),
            [
                ("border-image-outset", "1", true),
                ("border-image-repeat", "round", true),
                ("border-image-slice", "30", true),
                ("border-image-source", r#"url("img.png")"#, true),
                ("border-image-width", "2", true),
            ]
        );
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "border-image".to_owned(),
                value: r#"url("img.png") 30 / 2 / 1 round"#.to_owned(),
                priority: true,
            }),
            "border-image should no longer be a legacy side-table shorthand"
        );
        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                r#"border-image: url("img.png") 30 / 2 / 1 round !important;"#,
            ),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "border-image").as_deref(),
            Some(r#"url("img.png") 30 / 2 / 1 round"#),
            "border-image shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "border-image"),
            Some(true),
            "border-image shorthand priority should be served from PDB"
        );
        for (property, expected) in [
            ("border-image-outset", "1"),
            ("border-image-repeat", "round"),
            ("border-image-slice", "30"),
            ("border-image-source", r#"url("img.png")"#),
            ("border-image-width", "2"),
        ] {
            assert_eq!(
                inline_state_property_value_with_pdb(&state, property).as_deref(),
                Some(expected),
                "border-image longhand projection should be queryable through PDB"
            );
            assert_eq!(
                inline_state_property_priority_with_pdb(&state, property),
                Some(true),
                "border-image longhand projection should retain shorthand priority"
            );
        }
    }

    #[test]
    fn webkit_text_stroke_shorthand_is_pdb_write_and_query_safe() {
        let parsed = parse_style_property_entries_with_pdb("-webkit-text-stroke", "1px red", true)
            .expect("-webkit-text-stroke shorthand should parse through PDB");
        assert_eq!(
            parsed
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
                .collect::<Vec<_>>(),
            [
                ("-webkit-text-stroke-width", "1px", true),
                ("-webkit-text-stroke-color", "red", true),
            ]
        );
        assert_eq!(
            parsed.affected_names,
            [
                "-webkit-text-stroke",
                "-webkit-text-stroke-width",
                "-webkit-text-stroke-color",
            ]
        );
        assert!(style_entry_is_pdb_safe(&StyleEntry {
            name: "-webkit-text-stroke".to_owned(),
            value: "1px red".to_owned(),
            priority: true,
        }));

        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                "-webkit-text-stroke: 1px red !important;",
            ),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "-webkit-text-stroke").as_deref(),
            Some("1px red")
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "-webkit-text-stroke"),
            Some(true)
        );
    }

    #[test]
    fn border_component_shorthands_are_pdb_write_and_query_safe() {
        for (property, value, expected) in [
            (
                "border-color",
                "red blue",
                (
                    "red blue",
                    [
                        "border-top-color",
                        "border-right-color",
                        "border-bottom-color",
                        "border-left-color",
                    ],
                ),
            ),
            (
                "border-style",
                "solid dotted",
                (
                    "solid dotted",
                    [
                        "border-top-style",
                        "border-right-style",
                        "border-bottom-style",
                        "border-left-style",
                    ],
                ),
            ),
            (
                "border-width",
                "1px 2px",
                (
                    "1px 2px",
                    [
                        "border-top-width",
                        "border-right-width",
                        "border-bottom-width",
                        "border-left-width",
                    ],
                ),
            ),
        ] {
            assert!(
                parse_style_property_entries_with_pdb(property, value, true).is_some(),
                "{property} should parse through the Stylo declaration block path"
            );
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: property.to_owned(),
                    value: value.to_owned(),
                    priority: true,
                }),
                "{property} should no longer be a legacy side-table entry"
            );
            let state = CssInlineStyleDeclarationState {
                block: moli_css_parse::parse_declaration_block(&format!(
                    "{property}: {value} !important;"
                )),
                ..Default::default()
            };
            assert_eq!(
                inline_state_property_value_with_pdb(&state, property).as_deref(),
                Some(expected.0),
                "{property} shorthand query should be served from PDB"
            );
            assert_eq!(
                inline_state_property_priority_with_pdb(&state, property),
                Some(true),
                "{property} shorthand priority should be served from PDB"
            );
            let affected = style_property_affected_names_with_pdb(property)
                .expect("border component shorthand affected names should come from PDB");
            let expected_names = expected
                .1
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut expected_affected = vec![property.to_owned()];
            expected_affected.extend(expected_names.iter().cloned());
            assert_eq!(affected, expected_affected);
            for longhand in expected_names {
                assert!(
                    style_entry_is_pdb_safe(&StyleEntry {
                        name: longhand.to_owned(),
                        value: "initial".to_owned(),
                        priority: false,
                    }),
                    "{longhand} should remain PDB-safe after shorthand expansion"
                );
            }
        }
    }

    #[test]
    fn border_side_shorthands_are_pdb_write_and_query_safe() {
        for (property, value, expected_longhands) in [
            (
                "border-top",
                "1px solid red",
                ["border-top-width", "border-top-style", "border-top-color"],
            ),
            (
                "border-right",
                "2px dotted blue",
                [
                    "border-right-width",
                    "border-right-style",
                    "border-right-color",
                ],
            ),
            (
                "border-bottom",
                "3px dashed green",
                [
                    "border-bottom-width",
                    "border-bottom-style",
                    "border-bottom-color",
                ],
            ),
            (
                "border-left",
                "4px double black",
                [
                    "border-left-width",
                    "border-left-style",
                    "border-left-color",
                ],
            ),
        ] {
            let parsed = parse_style_property_entries_with_pdb(property, value, true)
                .expect("border side shorthand should parse through PDB");
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: property.to_owned(),
                    value: value.to_owned(),
                    priority: true,
                }),
                "{property} serialized entries should now seed PDB storage"
            );
            let state = CssInlineStyleDeclarationState {
                block: moli_css_parse::parse_declaration_block(&format!(
                    "{property}: {value} !important;"
                )),
                ..Default::default()
            };
            assert_eq!(
                inline_state_property_value_with_pdb(&state, property).as_deref(),
                Some(value),
                "{property} shorthand query should be served from PDB"
            );
            assert_eq!(
                inline_state_property_priority_with_pdb(&state, property),
                Some(true),
                "{property} shorthand priority should be served from PDB"
            );
            let mut expected_affected = vec![property.to_owned()];
            expected_affected.extend(expected_longhands.iter().map(|name| (*name).to_owned()));
            assert_eq!(parsed.affected_names, expected_affected);
            for longhand in expected_longhands {
                assert!(
                    style_entry_is_pdb_safe(&StyleEntry {
                        name: longhand.to_owned(),
                        value: "initial".to_owned(),
                        priority: false,
                    }),
                    "{longhand} should remain PDB-safe after shorthand expansion"
                );
            }
        }
    }

    #[test]
    fn border_radius_shorthand_is_pdb_write_and_query_safe() {
        let parsed = parse_style_property_entries_with_pdb("border-radius", "1px 2px", true)
            .expect("border-radius should parse through the Stylo declaration block path");
        for longhand in [
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-right-radius",
            "border-bottom-left-radius",
        ] {
            assert!(
                parsed.affected_names.iter().any(|name| name == longhand),
                "border-radius should affect {longhand}"
            );
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: longhand.to_owned(),
                    value: "initial".to_owned(),
                    priority: false,
                }),
                "{longhand} should be writable through PDB"
            );
        }
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "border-radius".to_owned(),
                value: "1px 2px".to_owned(),
                priority: true,
            }),
            "border-radius should no longer be a legacy side-table shorthand"
        );

        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("border-radius: 1px 2px !important;"),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "border-radius").as_deref(),
            Some("1px 2px"),
            "border-radius shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "border-top-left-radius").as_deref(),
            Some("1px"),
            "border-radius longhand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "border-radius"),
            Some(true),
            "border-radius shorthand priority should be served from PDB"
        );
    }

    #[test]
    fn text_decoration_family_is_pdb_write_and_query_safe() {
        let parsed = parse_style_property_entries_with_pdb(
            "text-decoration",
            "overline from-font dotted green",
            true,
        )
        .expect("text-decoration should parse through the Stylo declaration block path");
        let expected_longhands = [
            "text-decoration-color",
            "text-decoration-line",
            "text-decoration-style",
            "text-decoration-thickness",
        ];
        let mut expected_affected = vec!["text-decoration".to_owned()];
        expected_affected.extend(expected_longhands.iter().map(|name| (*name).to_owned()));
        assert_eq!(parsed.affected_names, expected_affected);
        let removal_affected = style_property_mutation_affected_names_with_pdb("text-decoration")
            .expect("text-decoration removal should use PDB affected names");
        for name in [
            "text-decoration-color",
            "text-decoration-line",
            "text-decoration-style",
            "text-decoration-thickness",
            "text-decoration-fill",
            "text-decoration-inset",
            "text-decoration-skip-ink",
            "text-decoration-skip-spaces",
            "text-decoration-stroke",
        ] {
            assert!(
                removal_affected.iter().any(|affected| affected == name),
                "text-decoration removal should affect {name}"
            );
        }
        let line_mutation_affected =
            style_property_mutation_affected_names_with_pdb("text-decoration-line")
                .expect("text-decoration-line mutation should use PDB affected names");
        assert!(
            line_mutation_affected
                .iter()
                .any(|affected| affected == "text-decoration-skip-ink"),
            "text-decoration-line mutation should clear text-decoration-skip-ink"
        );
        for (longhand, value) in [
            ("text-decoration-line", "overline"),
            ("text-decoration-thickness", "from-font"),
            ("text-decoration-style", "dotted"),
            ("text-decoration-color", "green"),
            ("text-decoration-fill", "match-text"),
            ("text-decoration-inset", "0px"),
            ("text-decoration-skip-ink", "all"),
            ("text-decoration-skip-spaces", "start end"),
            ("text-decoration-stroke", "context-fill"),
        ] {
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: longhand.to_owned(),
                    value: value.to_owned(),
                    priority: false,
                }),
                "{longhand} should be writable through PDB"
            );
        }
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "text-decoration".to_owned(),
                value: "overline from-font dotted green".to_owned(),
                priority: true,
            }),
            "text-decoration should no longer be a legacy side-table shorthand"
        );
        for compat_keyword in ["spelling-error", "grammar-error"] {
            let compat_entry = StyleEntry {
                name: "text-decoration-line".to_owned(),
                value: compat_keyword.to_owned(),
                priority: false,
            };
            assert!(
                style_entry_is_pdb_safe(&compat_entry),
                "{compat_keyword} should be accepted by the PDB write boundary"
            );
            assert!(
                style_entry_is_pdb_supplemental_side_entry(&compat_entry),
                "{compat_keyword} stays explicit supplemental until Stylo round-trips it natively"
            );
            let parsed = parse_style_property_entries_for_cssom_write(
                "text-decoration-line",
                compat_keyword,
                false,
                None,
            )
            .expect("compat text-decoration-line keyword should remain accepted");
            assert_eq!(parsed.entries.len(), 1);
            assert_eq!(parsed.entries[0].name, "text-decoration-line");
            assert_eq!(parsed.entries[0].value, compat_keyword);
            assert!(style_entry_is_pdb_supplemental_side_entry(
                &parsed.entries[0]
            ));

            let base_parsed = parse_style_property_entries_with_base(
                "text-decoration-line",
                compat_keyword,
                false,
                None,
            )
            .expect(
                "base parser should route compat text-decoration-line through PDB supplementals",
            );
            assert_eq!(base_parsed.entries.len(), 1);
            assert_eq!(base_parsed.entries[0].name, "text-decoration-line");
            assert_eq!(base_parsed.entries[0].value, compat_keyword);
            assert!(style_entry_is_pdb_supplemental_side_entry(
                &base_parsed.entries[0]
            ));
        }
        let base_normal = parse_style_property_entries_with_base(
            "text-decoration-line",
            "underline overline",
            false,
            None,
        )
        .expect("base parser should route ordinary text-decoration-line through PDB");
        assert_eq!(base_normal.entries.len(), 1);
        assert_eq!(base_normal.entries[0].name, "text-decoration-line");
        assert_eq!(base_normal.entries[0].value, "underline overline");
        assert!(!style_entry_is_pdb_supplemental_side_entry(
            &base_normal.entries[0]
        ));
        assert!(
            parse_style_property_entries_with_base(
                "text-decoration-line",
                "underline underline",
                false,
                None
            )
            .is_none(),
            "base parser should not fall back to renderer text-decoration-line normalization"
        );

        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                "text-decoration: overline from-font dotted green !important; text-decoration-skip-ink: all;",
            ),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "text-decoration").as_deref(),
            Some("overline from-font dotted green"),
            "text-decoration shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "text-decoration-line").as_deref(),
            Some("overline"),
            "text-decoration-line query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "text-decoration-skip-ink").as_deref(),
            Some("all"),
            "text-decoration-skip-ink query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "text-decoration"),
            Some(true),
            "text-decoration shorthand priority should be served from PDB"
        );
    }

    #[test]
    fn text_emphasis_family_is_pdb_write_and_query_safe() {
        let parsed = parse_style_property_entries_with_pdb("text-emphasis", "dot red", true)
            .expect("text-emphasis should parse through the Stylo declaration block path");
        let expected_longhands = ["text-emphasis-style", "text-emphasis-color"];
        let mut expected_affected = vec!["text-emphasis".to_owned()];
        expected_affected.extend(expected_longhands.iter().map(|name| (*name).to_owned()));
        assert_eq!(parsed.affected_names, expected_affected);
        for (longhand, value) in [
            ("text-emphasis-style", "dot"),
            ("text-emphasis-color", "red"),
            ("text-emphasis-position", "over left"),
        ] {
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: longhand.to_owned(),
                    value: value.to_owned(),
                    priority: false,
                }),
                "{longhand} should be writable through PDB"
            );
        }
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "text-emphasis".to_owned(),
                value: "dot red".to_owned(),
                priority: true,
            }),
            "text-emphasis should no longer be a legacy side-table shorthand"
        );
        let base_shorthand =
            parse_style_property_entries_with_base("text-emphasis", "dot red", true, None)
                .expect("base parser should route text-emphasis shorthand through PDB");
        assert_eq!(base_shorthand.affected_names, expected_affected);
        assert_eq!(base_shorthand.entries.len(), 2);
        assert!(
            base_shorthand
                .entries
                .iter()
                .any(|entry| entry.name == "text-emphasis-style" && entry.value == "dot")
        );
        assert!(
            base_shorthand
                .entries
                .iter()
                .any(|entry| entry.name == "text-emphasis-color" && entry.value == "red")
        );
        let base_position = parse_style_property_entries_with_base(
            "text-emphasis-position",
            "over left",
            false,
            None,
        )
        .expect("base parser should route text-emphasis-position through PDB");
        assert_eq!(base_position.entries.len(), 1);
        assert_eq!(base_position.entries[0].name, "text-emphasis-position");
        assert_eq!(base_position.entries[0].value, "over left");
        for (property, value) in [
            ("text-emphasis", "filled open"),
            ("text-emphasis-style", "filled open"),
            ("text-emphasis-position", "left right"),
        ] {
            assert!(
                parse_style_property_entries_with_base(property, value, false, None).is_none(),
                "{property}: {value} should be rejected by the PDB write boundary"
            );
        }

        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                "text-emphasis: dot red !important; text-emphasis-position: over left;",
            ),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "text-emphasis").as_deref(),
            Some("dot red"),
            "text-emphasis shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "text-emphasis-style").as_deref(),
            Some("dot"),
            "text-emphasis-style query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "text-emphasis-position").as_deref(),
            Some("over left"),
            "text-emphasis-position query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "text-emphasis"),
            Some(true),
            "text-emphasis shorthand priority should be served from PDB"
        );
    }

    #[test]
    fn font_variant_family_is_pdb_write_and_query_safe() {
        let alternates = parse_style_property_entries_with_pdb(
            "font-variant-alternates",
            "historical-forms",
            true,
        )
        .expect("font-variant-alternates should parse as a native PDB longhand");
        assert_eq!(alternates.entries.len(), 1);
        assert_eq!(alternates.entries[0].name, "font-variant-alternates");
        assert_eq!(alternates.entries[0].value, "historical-forms");
        assert!(!style_entry_is_pdb_supplemental_side_entry(
            &alternates.entries[0]
        ));

        let parsed = parse_style_property_entries_with_pdb("font-variant", "small-caps", true)
            .expect("font-variant should parse through the Stylo declaration block path");
        assert_eq!(
            parsed.affected_names.first().map(String::as_str),
            Some("font-variant")
        );
        for longhand in [
            "font-variant-ligatures",
            "font-variant-caps",
            "font-variant-alternates",
            "font-variant-numeric",
            "font-variant-east-asian",
            "font-variant-position",
            "font-variant-emoji",
        ] {
            assert!(
                parsed.affected_names.iter().any(|name| name == longhand),
                "font-variant should affect {longhand}"
            );
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: longhand.to_owned(),
                    value: "normal".to_owned(),
                    priority: false,
                }),
                "{longhand} should be writable through PDB"
            );
        }
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "font-variant".to_owned(),
                value: "small-caps".to_owned(),
                priority: true,
            }),
            "font-variant should no longer be a legacy side-table shorthand"
        );

        let base_shorthand =
            parse_style_property_entries_with_base("font-variant", "small-caps", true, None)
                .expect("base parser should route font-variant through PDB");
        for longhand in font_variant_longhands() {
            assert!(
                base_shorthand
                    .affected_names
                    .iter()
                    .any(|name| name == longhand),
                "base parser font-variant should affect {longhand}"
            );
        }
        assert!(
            base_shorthand
                .entries
                .iter()
                .any(|entry| entry.name == "font-variant-caps" && entry.value == "small-caps"),
            "base parser font-variant should retain PDB longhand projection"
        );
        assert!(
            base_shorthand.entries.iter().any(|entry| {
                entry.name == "font-variant-alternates"
                    && entry.value == "normal"
                    && !style_entry_is_pdb_supplemental_side_entry(entry)
            }),
            "base parser font-variant should include native longhand state"
        );

        let base_alternates = parse_style_property_entries_with_base(
            "font-variant-alternates",
            "historical-forms",
            true,
            None,
        )
        .expect("base parser should route font-variant-alternates through native PDB storage");
        assert_eq!(base_alternates.entries.len(), 1);
        assert_eq!(base_alternates.entries[0].name, "font-variant-alternates");
        assert_eq!(base_alternates.entries[0].value, "historical-forms");
        assert!(!style_entry_is_pdb_supplemental_side_entry(
            &base_alternates.entries[0]
        ));

        for (property, value) in [
            ("font-variant", "small-caps small-caps"),
            ("font-variant-caps", "small-caps petite-caps"),
            ("font-variant-position", "sub super"),
        ] {
            assert!(
                parse_style_property_entries_with_base(property, value, false, None).is_none(),
                "{property}: {value} should be rejected by the PDB write boundary"
            );
        }

        let state = inline_style_declaration_state_from_entries(&[
            StyleEntry {
                priority: true,
                ..style_entry("font-variant", "normal")
            },
            StyleEntry {
                priority: true,
                ..style_entry("font-variant-caps", "small-caps")
            },
            StyleEntry {
                priority: true,
                ..style_entry("font-variant-alternates", "historical-forms")
            },
        ]);
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "font-variant").as_deref(),
            Some("small-caps historical-forms"),
            "font-variant shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "font-variant"),
            Some(true),
            "font-variant shorthand priority should be served from PDB"
        );

        let none_state = inline_style_declaration_state_from_entries(&[
            StyleEntry {
                priority: true,
                ..style_entry("font-variant", "normal")
            },
            StyleEntry {
                priority: true,
                ..style_entry("font-variant-ligatures", "none")
            },
        ]);
        assert_eq!(
            inline_state_property_value_with_pdb(&none_state, "font-variant").as_deref(),
            Some("none"),
            "font-variant-ligatures:none should serialize as the standalone shorthand none"
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&none_state, "font-variant-ligatures").as_deref(),
            Some("none"),
            "font-variant-ligatures query should be served from PDB"
        );
    }

    #[test]
    fn font_shorthand_is_pdb_write_and_query_safe() {
        let parsed = parse_style_property_entries_with_pdb(
            "font",
            "italic small-caps 700 16px / 2 Ahem",
            true,
        )
        .expect("font shorthand should parse through the Stylo declaration block path");
        assert_eq!(
            parsed.affected_names.first().map(String::as_str),
            Some("font")
        );
        for longhand in [
            "font-style",
            "font-variant-ligatures",
            "font-variant-caps",
            "font-variant-alternates",
            "font-variant-numeric",
            "font-variant-east-asian",
            "font-variant-position",
            "font-variant-emoji",
            "font-weight",
            "font-stretch",
            "font-size",
            "line-height",
            "font-family",
            "font-kerning",
        ] {
            assert!(
                parsed.affected_names.iter().any(|name| name == longhand),
                "font should affect {longhand}"
            );
        }
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "font".to_owned(),
                value: "italic small-caps 700 16px / 2 Ahem".to_owned(),
                priority: true,
            }),
            "font shorthand should no longer be a legacy side-table shorthand"
        );

        let state = inline_style_declaration_state_from_entries(&[StyleEntry {
            priority: true,
            ..style_entry("font", "italic small-caps 700 16px / 2 Ahem")
        }]);
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "font").as_deref(),
            Some("italic small-caps 700 16px / 2 Ahem"),
            "font shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "font"),
            Some(true),
            "font shorthand priority should be served from PDB"
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "font-size").as_deref(),
            Some("16px"),
            "font-size longhand query should stay on the PDB path"
        );
    }

    #[test]
    fn outline_shorthand_is_pdb_write_and_query_safe() {
        let parsed = parse_style_property_entries_with_pdb("outline", "1px solid red", true)
            .expect("outline should parse through the Stylo declaration block path");
        for longhand in ["outline-color", "outline-style", "outline-width"] {
            assert!(
                parsed.affected_names.iter().any(|name| name == longhand),
                "outline should affect {longhand}"
            );
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: longhand.to_owned(),
                    value: "initial".to_owned(),
                    priority: false,
                }),
                "{longhand} should be writable through PDB"
            );
        }
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "outline".to_owned(),
                value: "1px solid red".to_owned(),
                priority: true,
            }),
            "outline should no longer be a legacy side-table shorthand"
        );
        let outline_color_invert = StyleEntry {
            name: "outline-color".to_owned(),
            value: "invert".to_owned(),
            priority: false,
        };
        assert!(
            style_entry_is_pdb_safe(&outline_color_invert),
            "outline-color: invert should be accepted by the PDB write boundary"
        );
        assert!(
            style_entry_is_pdb_supplemental_side_entry(&outline_color_invert),
            "outline-color: invert should remain an explicit supplemental entry until Stylo owns it natively"
        );

        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("outline: 1px solid red !important;"),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "outline").as_deref(),
            Some("red solid 1px"),
            "outline shorthand query should be served from PDB"
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "outline"),
            Some(true),
            "outline shorthand priority should be served from PDB"
        );

        let supplemental_invert =
            parse_style_property_entries_for_cssom_write("outline-color", "invert", false, None)
                .expect("outline-color: invert should stay accepted on the PDB supplemental path");
        assert_eq!(supplemental_invert.entries.len(), 1);
        assert_eq!(supplemental_invert.entries[0].name, "outline-color");
        assert_eq!(supplemental_invert.entries[0].value, "invert");
        assert!(!supplemental_invert.entries[0].priority);
        assert!(style_entry_is_pdb_supplemental_side_entry(
            &supplemental_invert.entries[0]
        ));

        let base_invert =
            parse_style_property_entries_with_base("outline-color", "invert", false, None)
                .expect("base parser should route outline-color: invert through PDB supplementals");
        assert_eq!(base_invert.entries.len(), 1);
        assert_eq!(base_invert.entries[0].name, "outline-color");
        assert_eq!(base_invert.entries[0].value, "invert");
        assert!(style_entry_is_pdb_supplemental_side_entry(
            &base_invert.entries[0]
        ));
        assert!(
            parse_style_property_entries_with_base("outline-color", "not-a-color", false, None)
                .is_none(),
            "base parser should not fall back to renderer value normalization for invalid outline-color"
        );
    }

    #[test]
    fn css_ui_compat_longhands_use_pdb_for_unprefixed_entries() {
        for (property, value) in [
            ("appearance", "auto"),
            ("-webkit-appearance", "auto"),
            ("backface-visibility", "visible"),
            ("background-clip", "text"),
            ("background-origin", "content-box"),
            ("order", "2"),
            ("transform-style", "preserve-3d"),
            ("user-select", "none"),
            ("-webkit-user-select", "text"),
            ("color-adjust", "economy"),
            ("forced-color-adjust", "preserve-parent-color"),
            ("print-color-adjust", "exact"),
            ("-webkit-text-fill-color", "red"),
        ] {
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: property.to_owned(),
                    value: value.to_owned(),
                    priority: false,
                }),
                "{property} should be owned by Stylo/PDB"
            );
            let parsed = parse_style_property_entries_with_pdb(property, value, false)
                .unwrap_or_else(|| panic!("{property} should parse through PDB"));
            assert!(
                !parsed.entries.is_empty(),
                "{property} should produce PDB entries"
            );
        }

        let property = "-webkit-transform-origin";
        assert!(
            !style_entry_is_pdb_safe(&StyleEntry {
                name: property.to_owned(),
                value: "20px 30px".to_owned(),
                priority: false,
            }),
            "{property} stays on the prefixed compatibility side-table path"
        );
        assert!(
            !known_style_property("-moz-user-select"),
            "-moz-user-select should not stay exposed as a Chromium-compatible side-table property"
        );
        assert!(
            parse_style_property_entries_for_cssom_write("-moz-user-select", "none", false, None)
                .is_none(),
            "-moz-user-select CSSOM writes should be rejected"
        );
    }

    #[test]
    fn webkit_transform_origin_side_entry_uses_pdb_validation_gate() {
        assert!(
            !style_entry_is_pdb_safe(&StyleEntry {
                name: "-webkit-transform-origin".to_owned(),
                value: "20px 30px".to_owned(),
                priority: false,
            }),
            "-webkit-transform-origin stays on the prefixed compatibility side-table path"
        );

        let parsed = parse_style_property_entries_for_cssom_write(
            "-webkit-transform-origin",
            "20px 30px",
            true,
            None,
        )
        .expect("valid transform-origin grammar should pass the compat gate");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "-webkit-transform-origin");
        assert_eq!(parsed.entries[0].value, "20px 30px");
        assert!(parsed.entries[0].priority);

        assert!(
            parse_style_property_entries_for_cssom_write(
                "-webkit-transform-origin",
                "banana",
                false,
                None,
            )
            .is_none(),
            "invalid transform-origin grammar should not fall through to raw side-entry storage"
        );
    }

    #[test]
    fn webkit_text_fill_color_is_owned_by_stylo_pdb() {
        assert!(
            style_entry_is_pdb_safe(&StyleEntry {
                name: "-webkit-text-fill-color".to_owned(),
                value: "red".to_owned(),
                priority: false,
            }),
            "-webkit-text-fill-color should be parsed and retained by Stylo/PDB"
        );

        let parsed = parse_style_property_entries_with_pdb("-webkit-text-fill-color", "red", true)
            .expect("valid color grammar should parse through PDB");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "-webkit-text-fill-color");
        assert_eq!(parsed.entries[0].value, "red");
        assert!(parsed.entries[0].priority);

        let state = inline_style_declaration_state_from_entries(&parsed.entries);
        assert!(state.side_entries.is_empty());
        assert_eq!(
            state
                .block
                .property_value("-webkit-text-fill-color")
                .as_deref(),
            Some("red")
        );

        assert!(
            parse_style_property_entries_with_pdb("-webkit-text-fill-color", "not-a-color", false,)
                .is_none(),
            "invalid color grammar should be rejected by Stylo"
        );
    }

    #[test]
    fn newly_enabled_chromium_properties_use_pdb_capability_routing() {
        for (name, value) in [
            ("-webkit-line-clamp", "2"),
            ("scroll-margin", "1px 2px"),
            ("offset", "none"),
            ("position-try", "--fallback"),
            ("font-synthesis", "weight style small-caps"),
            ("text-wrap", "wrap balance"),
        ] {
            let parsed = parse_style_property_entries_for_cssom_write(name, value, false, None)
                .unwrap_or_else(|| panic!("Stylo/PDB should accept {name}: {value}"));
            assert!(
                !parsed.entries.is_empty(),
                "PDB projection should retain entries for {name}: {value}"
            );
            let state = inline_style_declaration_state_from_entries(&parsed.entries);
            assert!(
                state.side_entries.is_empty(),
                "Stylo-owned property should not use renderer side entries: {name}"
            );
            assert!(
                !state
                    .block
                    .property_value(name)
                    .unwrap_or_default()
                    .is_empty(),
                "Stylo-owned property should serialize through PDB: {name}"
            );
        }
    }

    #[test]
    fn lightweight_standard_property_candidates_parse_through_pdb() {
        for (property, value) in [
            ("aspect-ratio", "1 / 2"),
            ("baseline-shift", "super"),
            ("background-position", "left top"),
            ("background-repeat", "repeat-x"),
            ("border-bottom-color", "red"),
            ("border-bottom-style", "dashed"),
            ("border-left-color", "red"),
            ("border-left-style", "dashed"),
            ("border-right-color", "red"),
            ("border-right-style", "dashed"),
            ("border-top-color", "red"),
            ("border-top-style", "dashed"),
            ("border-block-end-color", "red"),
            ("border-block-start-color", "red"),
            ("border-inline-end-color", "red"),
            ("border-inline-start-color", "red"),
            ("direction", "rtl"),
            ("flex-flow", "column wrap"),
            ("grid-column-start", "span 2"),
            ("grid-column-end", "3"),
            ("justify-self", "safe center"),
            ("perspective", "12px"),
            ("place-content", "center start"),
            ("reading-flow", "grid-order"),
            ("reading-order", "-2"),
            ("word-spacing", "2px"),
            ("writing-mode", "vertical-rl"),
        ] {
            let parsed = parse_style_property_entries_for_cssom_write(property, value, false, None)
                .unwrap_or_else(|| panic!("{property}: {value} should parse through PDB"));
            assert!(
                !parsed.entries.is_empty(),
                "{property}: {value} should produce PDB entries"
            );
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: property.to_owned(),
                    value: value.to_owned(),
                    priority: false,
                }),
                "{property}: {value} should be PDB-safe"
            );
        }

        for (property, value) in [
            ("border-block-start-color", "not-a-color"),
            ("border-inline-end-color", "not-a-color"),
            ("direction", "sideways"),
            ("reading-flow", "auto"),
            ("reading-order", "1.5"),
            ("writing-mode", "horizontal"),
        ] {
            assert!(
                parse_style_property_entries_for_cssom_write(property, value, false, None)
                    .is_none(),
                "{property}: {value} should be rejected by the PDB write boundary"
            );
        }
    }

    #[test]
    fn inline_state_builder_keeps_legacy_shorthands_out_of_pdb_entries() {
        let border = style_entry("border", "1px solid black");
        let border_top = style_entry("border-top", "1px solid black");
        let border_width = style_entry("border-width", "1px");
        let border_radius = style_entry("border-radius", "1px 2px");
        let outline = style_entry("outline", "1px solid red");
        let padding_left = style_entry("padding-left", "1px");
        let text_decoration = style_entry("text-decoration", "overline from-font dotted green");
        let text_emphasis = style_entry("text-emphasis", "dot red");
        let webkit_text_stroke = style_entry("-webkit-text-stroke", "1px red");

        assert!(
            style_entry_is_pdb_safe(&border),
            "border shorthand is now owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&border_width),
            "border component shorthands are now owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&border_top),
            "border side shorthands are now owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&border_radius),
            "border-radius shorthand is now owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&outline),
            "outline shorthand is now owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&text_decoration),
            "text-decoration shorthand is now owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&text_emphasis),
            "text-emphasis shorthand is now owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&style_entry("font-variant", "small-caps")),
            "font-variant shorthand is now owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&webkit_text_stroke),
            "-webkit-text-stroke shorthand is now owned by Stylo/PDB"
        );
        assert!(
            inline_style_entry_is_pdb_storage_candidate(&padding_left),
            "inline PDB state queries still need to recognize direct-storage box longhands"
        );

        let state = inline_style_declaration_state_from_entries(std::slice::from_ref(&border));
        assert!(!state.block.is_empty());
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].name, border.name);
        assert_eq!(state.entries[0].value, border.value);
        assert_eq!(state.entries[0].priority, border.priority);
        assert!(state.side_entries.is_empty());
        assert_eq!(
            state.block.property_value("border").as_deref(),
            Some("1px solid black")
        );
    }

    #[test]
    fn inline_serialized_entries_seed_pdb_state_without_css_text_reparse() {
        assert!(inline_style_entry_is_pdb_storage_candidate(&style_entry(
            "display", "block",
        )));
        assert!(inline_style_entry_is_pdb_storage_candidate(&style_entry(
            "visibility",
            "hidden",
        )));
        let plain = inline_style_declaration_state_from_serialized_entries(
            &[
                style_entry("display", "block"),
                StyleEntry {
                    priority: true,
                    ..style_entry("visibility", "hidden")
                },
            ],
            "display: block; visibility: hidden !important;",
            None,
        );
        assert!(
            plain.entries.is_empty(),
            "pure PDB inline state should not keep ordinary adapter entries"
        );
        assert!(plain.side_entries.is_empty());
        assert_eq!(
            plain.block.property_value("display").as_deref(),
            Some("block")
        );
        assert!(plain.block.property_priority("visibility"));
        assert_eq!(
            plain.css_text(),
            "display: block; visibility: hidden !important;"
        );

        let mixed = inline_style_declaration_state_from_serialized_entries(
            &[
                style_entry("--token", "value"),
                style_entry("visibility", "hidden"),
                style_entry("background-image", r#"url("https://example.test/img.png")"#),
            ],
            r#"--token: value; visibility: hidden; background-image: url("https://example.test/img.png");"#,
            None,
        );
        assert!(mixed.side_entries.is_empty());
        assert!(
            mixed
                .block
                .entries()
                .iter()
                .any(|entry| entry.name == "--token")
        );
        assert_eq!(
            mixed.block.property_value("--token").as_deref(),
            Some("value")
        );
        assert_eq!(
            mixed.block.property_value("visibility").as_deref(),
            Some("hidden")
        );
        assert_eq!(
            mixed.block.property_value("background-image").as_deref(),
            Some(r#"url("https://example.test/img.png")"#)
        );
        assert_eq!(
            mixed
                .entries()
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            ["--token", "visibility", "background-image"]
        );

        let border_entry = style_entry("border", "calc(10px) solid pink");
        assert!(
            inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(
                std::slice::from_ref(&border_entry)
            )
        );
        let border_shorthand = inline_style_declaration_state_from_serialized_entries(
            std::slice::from_ref(&border_entry),
            "border: calc(10px) solid pink;",
            None,
        );
        assert!(border_shorthand.entries.is_empty());
        assert!(border_shorthand.side_entries.is_empty());
        assert_eq!(
            border_shorthand.block.property_value("border").as_deref(),
            Some("calc(10px) solid pink")
        );

        let border_side_entries = [style_entry("border-top", "calc(11px) solid pink")];
        assert!(
            inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(
                &border_side_entries
            )
        );
        let border_side_shorthand = inline_style_declaration_state_from_serialized_entries(
            &border_side_entries,
            "border-top: calc(11px) solid pink;",
            None,
        );
        assert!(!border_side_shorthand.block.is_empty());
        assert!(border_side_shorthand.entries.is_empty());
        assert!(border_side_shorthand.side_entries.is_empty());
        assert_eq!(
            border_side_shorthand
                .block
                .property_value("border-top")
                .as_deref(),
            Some("calc(11px) solid pink")
        );
        assert_eq!(
            border_side_shorthand
                .entries()
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            ["border-top-width", "border-top-style", "border-top-color"]
        );
        assert_eq!(
            border_side_shorthand.css_text(),
            "border-top: calc(11px) solid pink;"
        );
    }

    #[test]
    fn inline_unresolved_and_custom_values_respect_pdb_compat_boundaries() {
        let entries = [
            style_entry("--token", "var(--fallback, value)"),
            style_entry("padding", "var(--pad)"),
            style_entry("width", "env(safe-area-inset-top)"),
        ];
        assert!(
            style_entry_is_pdb_safe(&entries[0]),
            "non-empty custom properties should be owned by Stylo/PDB"
        );
        assert!(
            style_entry_is_pdb_safe(&style_entry("--empty", " ")),
            "whitespace custom property specified values should be owned by Stylo/PDB"
        );
        let parsed_empty_custom =
            parse_style_property_entries_for_cssom_write("--empty", "  ", false, None)
                .expect("whitespace custom property values should stay accepted");
        assert_eq!(parsed_empty_custom.entries[0].value, "");
        assert!(style_entry_is_pdb_safe(&parsed_empty_custom.entries[0]));
        let parsed_custom =
            parse_style_property_entries_with_pdb("--token", "var(--fallback, value)", true)
                .expect("non-empty custom property should parse through Stylo/PDB");
        assert_eq!(
            parsed_custom
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
                .collect::<Vec<_>>(),
            [("--token", "var(--fallback, value)", true)]
        );
        assert!(
            style_entry_is_pdb_safe(&entries[1]),
            "box shorthand unresolved values use PDB plus renderer order projection"
        );
        assert!(
            style_entry_is_pdb_safe(&entries[2]),
            "ordinary unresolved values accepted by Stylo are owned by PDB"
        );
        assert!(
            parse_style_property_entries_for_cssom_write("width", "var(--x ())", false, None)
                .is_none(),
            "invalid var() syntax must still be rejected by the PDB value-fragment parser"
        );
        assert!(
            cssom_style_property_write_uses_pdb("top", "env(test 0 1, green)"),
            "env() indexed syntax should use PDB once Stylo accepts it"
        );
        let parsed_indexed_env =
            parse_style_property_entries_with_pdb("top", "env(test 0 1, green)", false)
                .expect("indexed env() should parse through Stylo/PDB");
        assert_eq!(
            parsed_indexed_env
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [("top", "env(test 0 1, green)")]
        );
        let parsed_padding = parse_style_property_entries_with_pdb("padding", "var(--pad)", false)
            .expect("padding var() should parse through Stylo/PDB");
        assert_eq!(
            parsed_padding
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [("padding", "var(--pad)")]
        );

        let state = inline_style_declaration_state_from_serialized_entries(
            &entries,
            "--token: var(--fallback, value); padding: var(--pad); width: env(safe-area-inset-top);",
            None,
        );
        assert!(state.side_entries.is_empty());
        assert!(
            state
                .block
                .entries()
                .iter()
                .any(|entry| entry.name == "--token")
        );
        assert_eq!(
            state.block.property_value("--token").as_deref(),
            Some("var(--fallback, value)")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "--token").as_deref(),
            Some("var(--fallback, value)")
        );
        assert_eq!(
            state.block.property_value("padding").as_deref(),
            Some("var(--pad)")
        );
        assert_eq!(
            state.block.property_value("width").as_deref(),
            Some("env(safe-area-inset-top)")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "width").as_deref(),
            Some("env(safe-area-inset-top)")
        );
        assert_eq!(
            state.css_text(),
            "--token: var(--fallback, value); padding: var(--pad); width: env(safe-area-inset-top);"
        );

        let empty_custom_state =
            inline_style_declaration_state_from_css_text("--empty:; --space:  ;", None);
        assert!(empty_custom_state.side_entries.is_empty());
        assert!(empty_custom_state.block.property_is_declared("--empty"));
        assert!(empty_custom_state.block.property_is_declared("--space"));
        assert_eq!(
            inline_state_property_value_with_pdb(&empty_custom_state, "--empty").as_deref(),
            Some(" ")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&empty_custom_state, "--space").as_deref(),
            Some(" ")
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&empty_custom_state, "--empty"),
            Some(false)
        );
        assert_eq!(empty_custom_state.css_text(), "--empty: ; --space: ;");

        assert!(
            inline_css_text_pdb_storage_state("margin: var(--prop); margin-top: 10px").is_none()
        );
        assert_eq!(
            inline_style_declaration_state_from_css_text(
                "margin: var(--prop); margin-top: 10px",
                None,
            )
            .css_text(),
            "margin-right: ; margin-bottom: ; margin-left: ; margin-top: 10px;"
        );
        assert!(
            inline_css_text_pdb_storage_state("border-width: var(--width); border-left-width: 3px")
                .is_none()
        );
        assert_eq!(
            inline_style_declaration_state_from_css_text(
                "border-width: var(--width); border-left-width: 3px",
                None,
            )
            .css_text(),
            "border-top-width: ; border-right-width: ; border-bottom-width: ; border-left-width: 3px;"
        );
    }

    #[test]
    fn unresolved_pdb_shorthand_entries_expand_before_longhand_mutation() {
        for (shorthand, mutation_longhand) in [
            ("padding-block", "padding-block-start"),
            ("padding-inline", "padding-inline-end"),
            ("overflow", "overflow-x"),
            ("outline", "outline-color"),
            ("text-decoration", "text-decoration-line"),
            ("text-emphasis", "text-emphasis-color"),
            ("font-variant", "font-variant-caps"),
            ("transition", "transition-duration"),
            ("animation", "animation-name"),
            ("font", "font-size"),
            ("background", "background-color"),
            ("gap", "column-gap"),
            ("place-content", "justify-content"),
        ] {
            let longhands = unresolved_box_shorthand_longhands(shorthand)
                .unwrap_or_else(|| panic!("{shorthand} should expand unresolved storage"));
            assert!(
                longhands.contains(&mutation_longhand),
                "{shorthand} should cover {mutation_longhand}"
            );
            let affected_names = style_property_mutation_affected_names_with_pdb(mutation_longhand)
                .unwrap_or_else(|| panic!("{mutation_longhand} should be PDB-backed"));
            let mut entries = vec![StyleEntry {
                name: shorthand.to_owned(),
                value: "var(--token)".to_owned(),
                priority: true,
            }];

            expand_unresolved_box_shorthand_entries_for_mutation(&mut entries, &affected_names);

            assert!(
                !entries.iter().any(|entry| entry.name == shorthand),
                "{shorthand} should not remain next to a mutated longhand"
            );
            for longhand in longhands {
                let should_keep = !affected_names.iter().any(|affected| affected == longhand);
                assert_eq!(
                    entries.iter().any(|entry| {
                        entry.name == *longhand && entry.value.is_empty() && entry.priority
                    }),
                    should_keep,
                    "{shorthand} placeholder state for {longhand}"
                );
            }
        }
    }

    #[test]
    fn inline_pdb_mutation_keeps_unresolved_shorthand_projection() {
        let mut state = CssInlineStyleDeclarationState::default();
        let affected_names = style_property_affected_names_with_pdb("padding").unwrap();
        let parsed = parse_style_property_entries_with_pdb("padding", "var(--pad)", false)
            .expect("padding var() should parse through Stylo/PDB");
        let projection = state
            .block
            .set_property_with_projection("padding", "var(--pad)", false);
        assert_ne!(
            projection.set_result,
            moli_css_parse::CssSetResult::ParseError
        );
        let mut entries = projection
            .entries
            .into_iter()
            .map(StyleEntry::from)
            .collect::<Vec<_>>();
        if entries.is_empty() || entries.iter().any(|entry| entry.value.is_empty()) {
            entries = parsed.entries.clone();
        }
        refresh_inline_state_entries_after_pdb_mutation(
            &mut state,
            "padding",
            &affected_names,
            entries,
            Vec::new(),
        );

        assert_eq!(
            state.property_names(),
            [
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left"
            ]
        );
        assert_eq!(state.css_text(), "padding: var(--pad);");
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "padding").as_deref(),
            Some("var(--pad)")
        );

        let affected_names = style_property_affected_names_with_pdb("padding-left").unwrap();
        let parsed =
            parse_style_property_entries_with_pdb("padding-left", "calc(calc(1px))", false)
                .expect("padding-left calc() should parse through Stylo/PDB");
        assert_eq!(
            parsed
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [("padding-left", "calc(1px)")]
        );
        let projection =
            state
                .block
                .set_property_with_projection("padding-left", "calc(calc(1px))", false);
        assert_ne!(
            projection.set_result,
            moli_css_parse::CssSetResult::ParseError
        );
        let entries = projection
            .entries
            .into_iter()
            .map(StyleEntry::from)
            .collect::<Vec<_>>();
        refresh_inline_state_entries_after_pdb_mutation(
            &mut state,
            "padding-left",
            &affected_names,
            entries,
            Vec::new(),
        );
        assert_eq!(
            state
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [
                ("padding-top", ""),
                ("padding-right", ""),
                ("padding-bottom", ""),
                ("padding-left", "calc(1px)")
            ]
        );
        assert_eq!(
            state.property_names(),
            [
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left"
            ]
        );
        assert_eq!(
            state.css_text(),
            "padding-top: ; padding-right: ; padding-bottom: ; padding-left: calc(1px);"
        );
        assert_eq!(
            state.style_resolution_text(),
            "padding: var(--pad) var(--pad) var(--pad) calc(1px);"
        );

        let affected_names = style_property_affected_names_with_pdb("padding-left").unwrap();
        let parsed = parse_style_property_entries_with_pdb("padding-left", "2px", true)
            .expect("padding-left important value should parse through Stylo/PDB");
        let entries = set_pdb_block_property_collecting_entries(
            &mut state.block,
            "padding-left",
            "2px",
            true,
            &parsed,
            false,
        )
        .expect("padding-left important value should update PDB");
        refresh_inline_state_entries_after_pdb_mutation(
            &mut state,
            "padding-left",
            &affected_names,
            entries,
            Vec::new(),
        );
        assert_eq!(
            state.css_text(),
            "padding-top: ; padding-right: ; padding-bottom: ; padding-left: 2px !important;"
        );

        let removed = state.block.remove_property("padding-left");
        assert!(removed.changed);
        refresh_inline_state_entries_after_pdb_mutation(
            &mut state,
            "padding-left",
            &affected_names,
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            state.css_text(),
            "padding-top: ; padding-right: ; padding-bottom: ;"
        );
    }

    #[test]
    fn inline_pdb_mutation_materializes_existing_block_before_renderer_projection() {
        let mut state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block("font-size: unset"),
            ..Default::default()
        };
        assert!(state.entries.is_empty());
        assert_eq!(state.css_text(), "font-size: unset;");

        let affected_names = style_property_affected_names_with_pdb("margin-bottom").unwrap();
        let parsed = parse_style_property_entries_with_pdb("margin-bottom", "var(--x)", false)
            .expect("margin-bottom var() should parse through Stylo/PDB");
        let mut entries = set_pdb_block_property_collecting_entries(
            &mut state.block,
            "margin-bottom",
            "var(--x)",
            false,
            &parsed,
            false,
        )
        .expect("margin-bottom var() should update the PDB block");
        if entries.is_empty() || entries.iter().any(|entry| entry.value.is_empty()) {
            entries = parsed.entries.clone();
        }

        refresh_inline_state_entries_after_pdb_mutation(
            &mut state,
            "margin-bottom",
            &affected_names,
            entries,
            Vec::new(),
        );

        assert_eq!(
            state
                .entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            [("font-size", "unset"), ("margin-bottom", "var(--x)")]
        );
        assert_eq!(
            state.css_text(),
            "font-size: unset; margin-bottom: var(--x);"
        );
        assert_eq!(
            state.style_resolution_text(),
            "font-size: unset; margin-bottom: var(--x);"
        );
    }

    #[test]
    fn inline_serialized_border_state_keeps_pdb_query_with_side_entries() {
        let css_text = "border: 1px solid red !important; --token: value; -webkit-transform-origin: 20px 30px;";
        let mut entries = parse_style_property_entries_with_pdb("border", "1px solid red", true)
            .expect("border should parse through PDB")
            .entries;
        entries.push(style_entry("--token", "value"));
        entries.push(style_entry("-webkit-transform-origin", "20px 30px"));
        let state =
            inline_style_declaration_state_from_serialized_entries(&entries, css_text, None);

        assert_eq!(
            state
                .side_entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["-webkit-transform-origin"]
        );
        assert!(
            state
                .block
                .entries()
                .iter()
                .any(|entry| entry.name == "--token")
        );
        assert_eq!(
            state.block.property_value("--token").as_deref(),
            Some("value")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "border").as_deref(),
            Some("1px solid red")
        );
        assert_eq!(
            inline_state_property_priority_with_pdb(&state, "border"),
            Some(true)
        );
    }

    #[test]
    fn structured_properties_use_pdb_entries() {
        for (name, value) in [
            ("appearance", "auto"),
            ("background-image", r#"url("https://example.test/img.png")"#),
            ("background-blend-mode", "multiply"),
            ("color-adjust", "exact"),
            ("color-scheme", "dark only"),
            ("column-rule-width", "0"),
            ("column-width", "0"),
            ("content", "\"x\""),
            ("forced-color-adjust", "preserve-parent-color"),
            ("isolation", "isolate"),
            ("mix-blend-mode", "multiply"),
            ("orphans", "2"),
            ("overscroll-behavior-block", "contain"),
            ("overscroll-behavior-inline", "none"),
            ("overscroll-behavior-x", "contain"),
            ("overscroll-behavior-y", "none"),
            ("print-color-adjust", "exact"),
            ("quotes", "\"a\" \"b\""),
            ("scrollbar-color", "auto"),
            ("scrollbar-width", "thin"),
            ("scroll-margin-top", "0"),
            ("scroll-padding-bottom", "0"),
            ("scroll-snap-align", "start start"),
            ("shape-margin", "0"),
            ("text-shadow", "red 1px 2px 3px"),
            ("text-underline-offset", "1px"),
            ("text-underline-position", "under"),
            ("user-select", "none"),
            ("will-change", "transform"),
            ("widows", "3"),
            ("zoom", "1.5"),
        ] {
            let parsed = parse_style_property_entries_for_cssom_write(name, value, false, None)
                .unwrap_or_else(|| panic!("{name} should parse through PDB"));
            assert_eq!(parsed.entries.len(), 1);
            let expected_name = if name == "color-adjust" {
                "print-color-adjust"
            } else {
                name
            };
            assert_eq!(parsed.entries[0].name, expected_name);
            assert!(parse_style_property_entries_with_pdb(name, value, false).is_some());
            assert!(style_entry_is_pdb_safe(&StyleEntry {
                name: expected_name.to_owned(),
                value: parsed.entries[0].value.clone(),
                priority: false,
            }));
        }

        let snap = parse_style_property_entries_for_cssom_write(
            "scroll-snap-align",
            "start invalid",
            false,
            None,
        );
        assert!(snap.is_none());
    }

    #[test]
    fn legacy_structured_longhands_use_pdb_when_stylo_owns_the_property() {
        for (name, value, expected_name, expected_value) in [
            ("color-scheme", "dark only", "color-scheme", "dark only"),
            ("orphans", "2", "orphans", "2"),
            ("widows", "3", "widows", "3"),
            ("page-break-after", "always", "break-after", "page"),
            ("page-break-before", "avoid", "break-before", "avoid"),
            ("page-break-inside", "avoid", "break-inside", "avoid"),
        ] {
            assert!(
                style_entry_is_pdb_safe(&StyleEntry {
                    name: name.to_owned(),
                    value: value.to_owned(),
                    priority: false,
                }),
                "{name} should be owned by Stylo/PDB"
            );
            let parsed = parse_style_property_entries_for_cssom_write(name, value, false, None)
                .unwrap_or_else(|| panic!("{name} should parse through PDB"));
            assert!(
                parsed
                    .entries
                    .iter()
                    .any(|entry| entry.name == expected_name && entry.value == expected_value),
                "{name} should produce {expected_name}: {expected_value}, got {:?}",
                parsed
                    .entries
                    .iter()
                    .map(|entry| (entry.name.as_str(), entry.value.as_str()))
                    .collect::<Vec<_>>()
            );
        }

        let state = CssInlineStyleDeclarationState {
            block: moli_css_parse::parse_declaration_block(
                "color-scheme: dark only; orphans: 2; widows: 3; \
                 page-break-after: always; page-break-before: left; \
                 page-break-inside: avoid;",
            ),
            ..Default::default()
        };
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "color-scheme").as_deref(),
            Some("dark only")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "orphans").as_deref(),
            Some("2")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "widows").as_deref(),
            Some("3")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "page-break-after").as_deref(),
            Some("always")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "page-break-before").as_deref(),
            Some("left")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "page-break-inside").as_deref(),
            Some("avoid")
        );
    }

    #[test]
    fn overscroll_behavior_family_uses_pdb_entries_and_cssom_folding() {
        let parsed = parse_style_property_entries_for_cssom_write(
            "overscroll-behavior",
            "contain none",
            true,
            None,
        )
        .expect("overscroll-behavior shorthand should parse through PDB");
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].name, "overscroll-behavior-x");
        assert_eq!(parsed.entries[0].value, "contain");
        assert!(parsed.entries[0].priority);
        assert_eq!(parsed.entries[1].name, "overscroll-behavior-y");
        assert_eq!(parsed.entries[1].value, "none");
        assert!(parsed.entries[1].priority);
        assert!(
            parse_style_property_entries_with_pdb("overscroll-behavior", "contain none", true,)
                .is_some()
        );
        assert!(style_entry_is_pdb_safe(&StyleEntry {
            name: "overscroll-behavior".to_owned(),
            value: "contain none".to_owned(),
            priority: true,
        }));

        let state = inline_style_declaration_state_from_entries(&[
            StyleEntry {
                name: "overscroll-behavior-x".to_owned(),
                value: "contain".to_owned(),
                priority: false,
            },
            StyleEntry {
                name: "overscroll-behavior-y".to_owned(),
                value: "contain".to_owned(),
                priority: false,
            },
        ]);
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "overscroll-behavior").as_deref(),
            Some("contain")
        );
        assert_eq!(
            inline_state_property_value_with_pdb(&state, "overscroll-behavior-x").as_deref(),
            Some("contain")
        );
    }

    #[test]
    fn overscroll_behavior_logical_longhands_use_pdb_entries() {
        for (name, value) in [
            ("overscroll-behavior-block", "contain"),
            ("overscroll-behavior-inline", "none"),
        ] {
            let parsed = parse_style_property_entries_for_cssom_write(name, value, true, None)
                .unwrap_or_else(|| panic!("{name} should parse through PDB"));
            assert_eq!(parsed.entries.len(), 1);
            assert_eq!(parsed.entries[0].name, name);
            assert_eq!(parsed.entries[0].value, value);
            assert!(parsed.entries[0].priority);
            assert_eq!(parsed.affected_names, vec![name.to_owned()]);
            assert!(parse_style_property_entries_with_pdb(name, value, true).is_some());
            assert!(style_entry_is_pdb_safe(&StyleEntry {
                name: name.to_owned(),
                value: value.to_owned(),
                priority: true,
            }));

            let state = inline_style_declaration_state_from_entries(&[StyleEntry {
                name: name.to_owned(),
                value: value.to_owned(),
                priority: true,
            }]);
            assert_eq!(
                inline_state_property_value_with_pdb(&state, name).as_deref(),
                Some(value)
            );
            assert_eq!(
                inline_state_property_priority_with_pdb(&state, name),
                Some(true)
            );
        }
    }

    #[test]
    fn text_shadow_cssom_write_uses_pdb_serialization() {
        let parsed = parse_style_property_entries_for_cssom_write(
            "text-shadow",
            "1px 2px 3px red",
            false,
            None,
        )
        .expect("text-shadow should parse through PDB");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "text-shadow");
        assert_eq!(parsed.entries[0].value, "red 1px 2px 3px");
        assert!(
            parse_style_property_entries_with_pdb("text-shadow", "1px 2px 3px red", false,)
                .is_some()
        );
        assert!(style_entry_is_pdb_safe(&parsed.entries[0]));

        let inline =
            parse_style_property_entries_with_base("text-shadow", "1px 2px 3px red", false, None)
                .expect("text-shadow should parse through the strict Stylo path");
        assert_eq!(inline.entries.len(), 1);
        assert_eq!(inline.entries[0].name, "text-shadow");
        assert_eq!(inline.entries[0].value, parsed.entries[0].value);
    }

    #[test]
    fn background_image_image_set_uses_pdb_storage() {
        let value = r#"image-set(url("") calc(1x * NaN))"#;
        let parsed =
            parse_style_property_entries_for_cssom_write("background-image", value, false, None)
                .expect("valid image-set resolution math should parse");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "background-image");
        assert_eq!(
            parsed.entries[0].value,
            r#"image-set(url("") calc(NaN * 1dppx))"#
        );
        assert!(
            cssom_style_property_write_uses_pdb("background-image", value),
            "image-set should use PDB once Stylo mutation exposes the same CSSOM surface"
        );
        assert!(
            style_entry_is_pdb_safe(&parsed.entries[0]),
            "image-set should be promoted to PDB storage after equivalent serialization is proven"
        );
        let pdb = parse_style_property_entries_with_pdb("background-image", value, false)
            .expect("image-set should parse through PDB");
        assert_eq!(pdb.entries.len(), parsed.entries.len());
        assert_eq!(pdb.entries[0].name, parsed.entries[0].name);
        assert_eq!(pdb.entries[0].value, parsed.entries[0].value);
        assert_eq!(pdb.entries[0].priority, parsed.entries[0].priority);
    }

    #[test]
    fn zoom_uses_pdb_for_cssom_compatible_values() {
        for value in ["normal", "100%", "0", "calc(1 - 0.5)"] {
            let parsed = parse_style_property_entries_for_cssom_write("zoom", value, false, None)
                .unwrap_or_else(|| panic!("zoom: {value} should parse through PDB"));
            assert_eq!(parsed.entries.len(), 1);
            assert_eq!(parsed.entries[0].name, "zoom");
            let stylo = stylo_pdb_entries_for_property("zoom", value, false)
                .unwrap_or_else(|| panic!("zoom: {value} should be accepted by Stylo/PDB"));
            assert_eq!(stylo.entries.len(), 1);
            assert_eq!(stylo.entries[0].name, "zoom");
            assert_eq!(stylo.entries[0].value, parsed.entries[0].value);
            assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
            assert!(
                !style_entry_is_pdb_supplemental_side_entry(&parsed.entries[0]),
                "zoom: {value} should stay only in the PDB block"
            );
        }

        let dynamic = parse_style_property_entries_for_cssom_write(
            "zoom",
            "calc(sign(1em - 1px) * 2%)",
            false,
            None,
        )
        .expect("dynamic zoom should stay accepted for CSSOM compat");
        assert_eq!(dynamic.entries.len(), 1);
        assert_eq!(dynamic.entries[0].name, "zoom");
        assert_eq!(dynamic.entries[0].value, "calc(2% * sign(1em - 1px))");
        assert!(!style_entry_is_pdb_supplemental_side_entry(
            &dynamic.entries[0]
        ));
    }

    #[test]
    fn grid_column_shorthand_uses_pdb_entries() {
        let parsed =
            parse_style_property_entries_for_cssom_write("grid-column", "1 / 3", true, None)
                .expect("grid-column should parse through PDB");
        assert_eq!(parsed.affected_names[0], "grid-column");
        assert!(
            parsed
                .affected_names
                .contains(&"grid-column-start".to_owned())
        );
        assert!(
            parsed
                .affected_names
                .contains(&"grid-column-end".to_owned())
        );
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].name, "grid-column-start");
        assert_eq!(parsed.entries[0].value, "1");
        assert!(parsed.entries[0].priority);
        assert_eq!(parsed.entries[1].name, "grid-column-end");
        assert_eq!(parsed.entries[1].value, "3");
        assert!(parsed.entries[1].priority);
        assert!(parse_style_property_entries_with_pdb("grid-column", "1 / 3", true).is_some());
    }

    #[test]
    fn list_style_shorthand_uses_pdb_entries() {
        let parsed =
            parse_style_property_entries_for_cssom_write("list-style", "inside disc", false, None)
                .expect("list-style should parse through PDB");
        assert_eq!(parsed.affected_names[0], "list-style");
        assert_eq!(parsed.entries.len(), 3);
        assert_eq!(parsed.entries[0].name, "list-style-position");
        assert_eq!(parsed.entries[0].value, "inside");
        assert_eq!(parsed.entries[1].name, "list-style-image");
        assert_eq!(parsed.entries[1].value, "none");
        assert_eq!(parsed.entries[2].name, "list-style-type");
        assert_eq!(parsed.entries[2].value, "disc");
        assert!(
            parse_style_property_entries_with_pdb("list-style", "inside disc", false).is_some()
        );
    }

    #[test]
    fn remaining_structured_longhands_use_pdb_entries() {
        for (name, input, expected) in [
            ("alignment-baseline", "alphabetic", "alphabetic"),
            ("background-attachment", "local", "local"),
            ("baseline-source", "first", "first"),
            ("bookmark-level", "1", "1"),
            ("bookmark-state", "closed", "closed"),
            ("border-collapse", "collapse", "collapse"),
            ("caption-side", "bottom", "bottom"),
            ("clear", "both", "both"),
            (
                "clip",
                "rect(0px, 1px, 2px, 3px)",
                "rect(0px, 1px, 2px, 3px)",
            ),
            ("empty-cells", "hide", "hide"),
            (
                "link-parameters",
                "param(--a, orange), param(--b)",
                "param(--a, orange), param(--b)",
            ),
            ("list-style-position", "inside", "inside"),
            ("list-style-type", "upper-alpha", "upper-alpha"),
            ("table-layout", "fixed", "fixed"),
            (
                "text-size-adjust",
                "calc(10% * sibling-index())",
                "calc(10% * sibling-index())",
            ),
            ("text-transform", "uppercase", "uppercase"),
        ] {
            assert!(
                cssom_style_property_write_uses_pdb(name, input),
                "{name} should be PDB-backed for CSSOM writes"
            );
            let parsed = parse_style_property_entries_for_cssom_write(name, input, true, None)
                .unwrap_or_else(|| panic!("{name}: {input} should parse through PDB"));
            assert_eq!(parsed.entries.len(), 1, "{name}: {input}");
            assert_eq!(parsed.entries[0].name, name, "{name}: {input}");
            assert_eq!(parsed.entries[0].value, expected, "{name}: {input}");
            assert!(parsed.entries[0].priority, "{name}: {input}");
            assert_eq!(parsed.affected_names, vec![name.to_owned()]);
            assert!(
                parse_style_property_entries_with_pdb(name, input, true).is_some(),
                "{name}: {input} should parse directly through PDB"
            );
            assert!(
                style_entry_is_pdb_safe(&parsed.entries[0]),
                "{name}: {input} should stay PDB-safe"
            );
        }

        let text_size = parse_style_property_entries_for_cssom_write(
            "text-size-adjust",
            "calc(10% + 5%)",
            false,
            None,
        )
        .expect("static text-size-adjust calc should parse through PDB");
        assert_eq!(text_size.entries[0].value, "calc(15%)");

        let link_empty = parse_style_property_entries_for_cssom_write(
            "link-parameters",
            "param(--a, )",
            false,
            None,
        )
        .expect("empty link-parameters fallback should parse through PDB");
        assert_eq!(link_empty.entries[0].value, "param(--a, )");

        let link_eof = parse_style_property_entries_for_cssom_write(
            "link-parameters",
            "param(--a",
            false,
            None,
        )
        .expect("EOF-recovered link-parameters function should parse through PDB");
        assert_eq!(link_eof.entries[0].value, "param(--a)");

        assert!(
            parse_style_property_entries_for_cssom_write("color", "red; width: 1px", false, None)
                .is_none(),
            "CSSOM value fragments must not be parsed as declaration source"
        );
    }

    #[test]
    fn base_fallback_routes_structured_pdb_properties_through_pdb() {
        fn assert_base_matches_pdb(property: &str, value: &str) {
            let base = parse_style_property_entries_with_base(property, value, true, None)
                .unwrap_or_else(|| {
                    panic!("{property}: {value} should parse through base fallback")
                });
            let direct = parse_style_property_entries_with_pdb(property, value, true)
                .unwrap_or_else(|| panic!("{property}: {value} should parse directly through PDB"));
            assert_eq!(
                base.affected_names, direct.affected_names,
                "{property}: {value} should use PDB affected names in base fallback"
            );
            assert_eq!(
                base.entries
                    .iter()
                    .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
                    .collect::<Vec<_>>(),
                direct
                    .entries
                    .iter()
                    .map(|entry| (entry.name.as_str(), entry.value.as_str(), entry.priority))
                    .collect::<Vec<_>>(),
                "{property}: {value} base fallback output should match direct PDB output"
            );
            assert!(
                base.entries.iter().all(style_entry_is_pdb_safe),
                "{property}: {value} base fallback entries should stay PDB-safe"
            );
        }

        for (property, value) in [
            ("align-content", "first baseline"),
            ("align-items", "first baseline"),
            ("align-self", "first baseline"),
            ("background-size", "calc(10px + 5px) 20px"),
            ("color", "rgb(0 128 0 / 50%)"),
            ("color-scheme", "dark only"),
            ("column-rule-width", "0"),
            ("column-width", "0"),
            ("content", "'string'"),
            ("gap", "10px 10px"),
            ("grid-column", "1 / 3"),
            ("justify-self", "safe center"),
            ("link-parameters", "param(--a"),
            ("list-style", "inside disc"),
            ("orphans", "2"),
            ("overscroll-behavior", "chain chain"),
            ("page-break-after", "always"),
            ("place-content", "center center"),
            ("scroll-margin-top", "0"),
            ("scroll-padding-bottom", "0"),
            ("scroll-snap-align", "start start"),
            ("shape-margin", "0"),
            ("text-shadow", "1px 2px 3px red"),
            ("text-size-adjust", "calc(10% + 5%)"),
            ("will-change", "transform"),
            ("widows", "3"),
            ("width", "calc(10px + 1vmin + 10%)"),
            ("zoom", "calc(1 - 0.5)"),
        ] {
            assert_base_matches_pdb(property, value);
        }

        for (property, value) in [
            ("color", "red; width: 1px"),
            ("column-rule-width", "-1px"),
            ("column-width", "-1px"),
            ("link-parameters", "param(-a)"),
            ("scroll-padding-bottom", "-1px"),
            ("scroll-snap-align", "start invalid"),
            ("shape-margin", "-1px"),
            ("text-size-adjust", "10px"),
            ("width", "calc(5px / 1px)"),
        ] {
            assert!(
                parse_style_property_entries_with_base(property, value, false, None).is_none(),
                "{property}: {value} should be rejected by the PDB-backed base fallback"
            );
        }
    }

    #[test]
    fn remaining_structured_longhands_reject_invalid_values_with_pdb() {
        for (name, value) in [
            ("bookmark-level", "0"),
            ("bookmark-state", "none"),
            ("text-size-adjust", "-100%"),
            ("text-size-adjust", "10px"),
            ("link-parameters", "param(-a)"),
            ("link-parameters", "param(--a red)"),
            ("link-parameters", "param(--a, red) param(--b, blue)"),
        ] {
            assert!(
                parse_style_property_entries_for_cssom_write(name, value, false, None).is_none(),
                "{name}: {value} should be rejected by CSSOM write parsing"
            );
            assert!(
                parse_style_property_entries_with_pdb(name, value, false).is_none(),
                "{name}: {value} should be rejected by direct PDB parsing"
            );
        }
    }

    #[test]
    fn overflow_overlay_uses_pdb_supplemental_cssom_path() {
        assert!(
            parse_style_property_entries_with_base("overflow", "banana", false, None).is_none()
        );

        let parsed =
            parse_style_property_entries_for_cssom_write("overflow-x", "overlay", false, None)
                .expect("overflow overlay should remain accepted for CSSOM compat");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "overflow-x");
        assert_eq!(parsed.entries[0].value, "overlay");
        assert!(style_entry_is_pdb_safe(&parsed.entries[0]));
        assert!(style_entry_is_pdb_supplemental_side_entry(
            &parsed.entries[0]
        ));

        let parsed =
            parse_style_property_entries_for_cssom_write("overflow", "overlay hidden", false, None)
                .expect("overflow shorthand should preserve overlay for CSSOM compat");
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].name, "overflow-x");
        assert_eq!(parsed.entries[0].value, "overlay");
        assert_eq!(parsed.entries[1].name, "overflow-y");
        assert_eq!(parsed.entries[1].value, "hidden");
        assert!(style_entry_is_pdb_supplemental_side_entry(
            &parsed.entries[0]
        ));
        assert!(!style_entry_is_pdb_supplemental_side_entry(
            &parsed.entries[1]
        ));
    }

    #[test]
    fn css_variable_specified_values_bypass_structured_property_expansion() {
        let margin = parse_style_property_entries_with_base("margin", "var(--prop)", true, None)
            .expect("valid var() margin shorthand should parse as specified value");
        assert_eq!(margin.entries.len(), 1);
        assert_eq!(margin.entries[0].name, "margin");
        assert_eq!(margin.entries[0].value, "var(--prop)");
        assert!(margin.entries[0].priority);
        assert_eq!(margin.affected_names, ["margin"]);

        assert!(
            parse_style_property_entries_with_base("width", "var(--x ())", false, None).is_none()
        );
        assert!(
            parse_style_property_entries_with_base("expando", "var(--prop)", false, None).is_none()
        );
    }

    #[test]
    fn custom_property_entries_preserve_empty_specified_value_semantics() {
        let empty = parse_style_property_entries_with_base("--var", "", false, None)
            .expect("empty custom property value should parse");
        assert_eq!(empty.entries[0].name, "--var");
        assert_eq!(empty.entries[0].value, "");

        let whitespace = parse_style_property_entries_with_base("--var", "  ", false, None)
            .expect("whitespace custom property value should parse");
        assert_eq!(whitespace.entries[0].value, "");

        let value = parse_style_property_entries_with_base("--var", " value  ", false, None)
            .expect("non-empty custom property value should parse");
        assert_eq!(value.entries[0].value, "value");

        assert!(
            parse_style_property_entries_for_cssom_write("--var", "a;b", false, None).is_none(),
            "CSSOM custom property values reject bare top-level semicolons"
        );
        let escaped_semicolon =
            parse_style_property_entries_for_cssom_write("--var", r#"a\;b"#, false, None)
                .expect("CSSOM custom property values accept escaped top-level semicolons");
        assert_eq!(escaped_semicolon.entries[0].value, r#"a\;b"#);
        assert!(
            parse_style_property_entries_for_cssom_write("--var", r#"Hello\; world!"#, false, None)
                .is_none(),
            "CSSOM custom property values reject bare priority delimiters"
        );
        let escaped_priority = parse_style_property_entries_for_cssom_write(
            "--var",
            r#"Hello\; world\!"#,
            false,
            None,
        )
        .expect("CSSOM custom property values accept escaped priority delimiters");
        assert_eq!(escaped_priority.entries[0].value, r#"Hello\; world\!"#);

        assert!(parse_style_property_entries_with_base("--", "value", false, None).is_none());
        assert!(
            parse_style_property_entries_with_base("--var name", "value", false, None).is_none()
        );
    }

    #[test]
    fn custom_property_entries_accept_ident_var_reference_names() {
        let parsed = parse_style_property_entries_with_base(
            "--var-with-ident",
            r#"var(ident("--myprop" calc(3 * sign(1em - 1px))), FAIL)"#,
            false,
            None,
        )
        .expect("custom property value should preserve ident() var reference names");
        assert_eq!(parsed.entries[0].name, "--var-with-ident");
        assert_eq!(
            parsed.entries[0].value,
            r#"var(ident("--myprop" calc(3 * sign(1em - 1px))), FAIL)"#
        );
    }

    #[test]
    fn animation_timing_function_parser_preserves_linear_round_trip_percent_precision() {
        let parsed = parse_style_property_entries_with_base(
            "animation-timing-function",
            "linear(0 0%, 1.3 11.111111%, 1 22.222222%, 0.92 33.333333%, 1 44.444444%, 0.99 55.555556%, 1 66.666667%, 1.004 77.777778%, 0.998 88.888889%, 1 100%, 1 100%)",
            false,
            None,
        )
        .expect("linear easing should parse");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].name, "animation-timing-function");
        assert_eq!(
            parsed.entries[0].value,
            "linear(0 0%, 1.3 11.111111%, 1 22.222222%, 0.92 33.333333%, 1 44.444444%, 0.99 55.555556%, 1 66.666667%, 1.004 77.777778%, 0.998 88.888889%, 1 100%, 1 100%)"
        );
    }
}
