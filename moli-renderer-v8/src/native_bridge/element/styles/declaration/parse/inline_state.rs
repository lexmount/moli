use super::cssom_mutation::{parse_inline_css_text_with_base, parse_inline_declaration_entries};
use super::declaration_parser::parse_style_property_entries_with_base;
use super::pdb_compat::{
    css_value_uses_unresolved_cssom_storage, cssom_empty_specified_placeholder_property,
    cssom_style_property_query_uses_pdb, cssom_style_property_write_can_use_pdb_storage,
    inline_style_entry_is_pdb_storage_candidate, parse_pdb_style_property,
    parse_style_property_entries_with_pdb, pdb_block_from_native_style_entries,
    style_entry_affects_property_query, style_entry_is_pdb_safe,
    style_entry_is_pdb_supplemental_side_entry, style_property_affected_names_with_pdb,
};
use super::property_access::{
    overflow_property_query_uses_pdb_supplemental_side_entries,
    pdb_property_priority_for_cssom_query_with_side_entries,
    pdb_property_value_for_cssom_query_with_side_entries,
    text_decoration_property_query_uses_pdb_supplemental_side_entries,
};
use super::*;

pub(super) fn inline_style_declaration_state_from_entries(
    entries: &[StyleEntry],
) -> CssInlineStyleDeclarationState {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let mut side_entries = Vec::new();
    let mut pdb_entries = Vec::new();
    for entry in entries {
        if style_entry_is_pdb_supplemental_side_entry(entry) {
            side_entries.push(entry.clone());
            continue;
        }
        if style_entry_is_pdb_safe(entry) {
            pdb_entries.push(entry);
            continue;
        }
        side_entries.push(entry.clone());
    }
    let block = pdb_block_from_native_style_entries(&pdb_entries).unwrap_or_default();
    let compatibility = (!entries.is_empty()).then(|| {
        Box::new(crate::css_style::InlineStyleCompatibility {
            entries: entries.to_vec(),
            side_entries,
        })
    });
    CssInlineStyleDeclarationState {
        block,
        compatibility,
        ..Default::default()
    }
}

pub(super) fn inline_style_declaration_state_from_serialized_entries(
    mut state: CssInlineStyleDeclarationState,
    css_text: &str,
    base_url: Option<&url::Url>,
) -> CssInlineStyleDeclarationState {
    if !inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(&state) {
        return inline_style_declaration_state_from_css_text(css_text, base_url);
    }
    state.refresh_pdb_entries();
    state
}

pub(super) fn inline_serialized_entries_can_seed_pdb_state_without_css_text_reparse(
    state: &CssInlineStyleDeclarationState,
) -> bool {
    state.source_entries().len() > state.side_entries().len()
        && state.side_entries().iter().all(|entry| {
            style_entry_is_pdb_supplemental_side_entry(entry)
                || inline_serialized_side_entry_can_seed_without_css_text_reparse(entry)
        })
}

pub(super) fn inline_serialized_side_entry_can_seed_without_css_text_reparse(
    entry: &StyleEntry,
) -> bool {
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

pub(super) fn inline_style_declaration_state_from_css_text(
    css_text: &str,
    base_url: Option<&url::Url>,
) -> CssInlineStyleDeclarationState {
    let entries = parse_inline_css_text_with_base(css_text, base_url);
    inline_style_declaration_state_from_entries(&entries)
}

pub(super) fn inline_css_text_pdb_storage_state(
    css_text: &str,
    declarations: &[StyleEntry],
    source_preserved: bool,
) -> Option<CssInlineStyleDeclarationState> {
    if declarations
        .iter()
        .any(|declaration| declaration.name.eq_ignore_ascii_case("all"))
    {
        let entries = inline_css_text_all_adapter_entries(declarations)?;
        let mut state = inline_style_declaration_state_from_entries(&entries);
        state.refresh_pdb_entries();
        return Some(state);
    }
    let mut block = style::properties::PropertyDeclarationBlock::new();
    // Escapes and compatibility normalization can change the token stream.
    let mut use_original_text = !source_preserved || css_text.contains('\\');
    let mut project_entries = false;
    for declaration in declarations {
        let name = canonical_style_property_name(declaration.name.trim());
        let value = &declaration.value;
        if name.is_empty() {
            continue;
        }
        if value.is_empty() && moli_css_parse::is_cssom_custom_property_name(&name) {
            use_original_text = true;
            continue;
        }
        if name == "all"
            || (value.is_empty() && cssom_empty_specified_placeholder_property(&name))
            || (css_value_uses_unresolved_cssom_storage(value)
                && unresolved_box_shorthand_longhands(&name).is_some())
            || !cssom_style_property_write_can_use_pdb_storage(&name, value)
        {
            return None;
        }
        let (parsed, native) = parse_pdb_style_property(&name, value, declaration.priority)?;
        project_entries |= name == "animation"
            || !name.eq_ignore_ascii_case(declaration.name.trim())
            || parsed
                .entries
                .iter()
                .any(style_entry_is_pdb_supplemental_side_entry);
        if let Some(native) = native {
            if !use_original_text && !project_entries {
                native.append_to_declaration_list(&mut block);
            }
        } else {
            use_original_text = true;
        }
    }
    if project_entries {
        let entries = parse_inline_declaration_entries(declarations, None);
        return Some(inline_style_declaration_state_from_entries(&entries));
    }
    let block = if use_original_text {
        moli_css_parse::parse_declaration_block(css_text)
    } else {
        moli_css_parse::CssDeclarationBlock::new(block)
    };
    Some(CssInlineStyleDeclarationState {
        block,
        ..Default::default()
    })
}

fn inline_css_text_all_adapter_entries(declarations: &[StyleEntry]) -> Option<Vec<StyleEntry>> {
    let mut entries = Vec::new();
    let mut has_all = false;
    for declaration in declarations {
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

pub(super) fn retain_inline_css_text_adapter_entries(
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

pub(super) fn inline_state_has_unpreservable_side_entries_for_property(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
) -> bool {
    state.side_entries().iter().any(|entry| {
        style_entry_affects_property_query(entry, property, affected_names)
            && !style_entry_is_replaceable_by_pdb_property(entry, property, affected_names)
            && !style_entry_is_preservable_for_pdb_property(entry, property)
    })
}

pub(super) fn inline_state_has_replaceable_side_entries_for_property(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
) -> bool {
    state
        .side_entries()
        .iter()
        .any(|entry| style_entry_is_replaceable_by_pdb_property(entry, property, affected_names))
}

pub(crate) fn inline_state_property_value_with_pdb(
    state: &CssInlineStyleDeclarationState,
    property: &str,
) -> Option<String> {
    let overflow_supplemental_query =
        overflow_property_query_uses_pdb_supplemental_side_entries(property, state.side_entries());
    if !cssom_style_property_query_uses_pdb(property) && !overflow_supplemental_query {
        return None;
    }
    let affected_names = style_property_affected_names_with_pdb(property)?;
    if property == "font-variant" {
        return pdb_property_value_for_cssom_query_with_side_entries(
            &state.block,
            property,
            state.side_entries(),
        );
    }
    let text_decoration_supplemental_query =
        text_decoration_property_query_uses_pdb_supplemental_side_entries(
            property,
            state.side_entries(),
        );
    if text_decoration_supplemental_query {
        return pdb_property_value_for_cssom_query_with_side_entries(
            &state.block,
            property,
            state.side_entries(),
        );
    }
    if overflow_supplemental_query {
        return pdb_property_value_for_cssom_query_with_side_entries(
            &state.block,
            property,
            state.side_entries(),
        );
    }
    let query = inline_state_pdb_property_query_candidate(state, property, &affected_names)?;
    match query.candidate {
        InlineStatePdbQueryCandidate::Pdb => pdb_property_value_for_cssom_query_with_side_entries(
            &state.block,
            property,
            state.side_entries(),
        ),
        InlineStatePdbQueryCandidate::SupplementalSide if text_decoration_supplemental_query => {
            pdb_property_value_for_cssom_query_with_side_entries(
                &state.block,
                property,
                state.side_entries(),
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
        overflow_property_query_uses_pdb_supplemental_side_entries(property, state.side_entries());
    if !cssom_style_property_query_uses_pdb(property) && !overflow_supplemental_query {
        return None;
    }
    let affected_names = style_property_affected_names_with_pdb(property)?;
    if property == "font-variant" {
        return pdb_property_priority_for_cssom_query_with_side_entries(
            &state.block,
            property,
            state.side_entries(),
        );
    }
    let text_decoration_supplemental_query =
        text_decoration_property_query_uses_pdb_supplemental_side_entries(
            property,
            state.side_entries(),
        );
    if text_decoration_supplemental_query {
        return pdb_property_priority_for_cssom_query_with_side_entries(
            &state.block,
            property,
            state.side_entries(),
        );
    }
    if overflow_supplemental_query {
        return pdb_property_priority_for_cssom_query_with_side_entries(
            &state.block,
            property,
            state.side_entries(),
        );
    }
    let query = inline_state_pdb_property_query_candidate(state, property, &affected_names)?;
    match query.candidate {
        InlineStatePdbQueryCandidate::Pdb => {
            pdb_property_priority_for_cssom_query_with_side_entries(
                &state.block,
                property,
                state.side_entries(),
            )
        }
        InlineStatePdbQueryCandidate::SupplementalSide if text_decoration_supplemental_query => {
            pdb_property_priority_for_cssom_query_with_side_entries(
                &state.block,
                property,
                state.side_entries(),
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

pub(super) fn inline_state_pdb_property_query_candidate(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
) -> Option<InlineStatePdbQueryResult> {
    if state.side_entries().is_empty()
        && !state
            .source_entries()
            .iter()
            .any(|entry| entry.name == "all")
        && !state.source_entries().iter().any(|entry| {
            entry.value.is_empty() || css_value_uses_unresolved_cssom_storage(&entry.value)
        })
    {
        return (!state.block.is_empty()).then_some(InlineStatePdbQueryResult {
            candidate: InlineStatePdbQueryCandidate::Pdb,
            priority: PdbQueryPriority::Normal,
        });
    }

    let mut remaining_side_entries = state.side_entries().to_vec();
    let mut normal = None;
    let mut important = None;
    for entry in state.source_entries() {
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

pub(super) fn inline_state_block_contains_entry(
    state: &CssInlineStyleDeclarationState,
    entry: &StyleEntry,
) -> bool {
    let block_entries = inline_state_block_entries(state);
    block_entries_contains_entry(&block_entries, entry)
}

pub(super) fn inline_state_block_entries(
    state: &CssInlineStyleDeclarationState,
) -> Vec<StyleEntry> {
    state
        .block
        .entries()
        .into_iter()
        .map(StyleEntry::from)
        .collect()
}

pub(super) fn inline_state_block_entries_for_property_mutation(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
) -> Vec<StyleEntry> {
    inline_state_block_entries(state)
        .into_iter()
        .filter(|entry| style_entry_affects_property_query(entry, property, affected_names))
        .collect()
}

pub(super) fn block_entries_contains_entry(
    block_entries: &[StyleEntry],
    entry: &StyleEntry,
) -> bool {
    block_entries
        .iter()
        .any(|block_entry| style_entries_equal(block_entry, entry))
}

pub(super) fn inline_state_pdb_supplemental_side_entry(
    state: &CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
    priority: PdbQueryPriority,
) -> Option<StyleEntry> {
    state
        .side_entries()
        .iter()
        .rev()
        .find(|entry| {
            style_entry_affects_property_query(entry, property, affected_names)
                && style_entry_is_pdb_supplemental_side_entry(entry)
                && entry.priority == (priority == PdbQueryPriority::Important)
        })
        .cloned()
}

pub(super) fn style_entry_is_replaceable_by_pdb_property(
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

pub(super) fn style_entry_is_preservable_for_pdb_property(
    entry: &StyleEntry,
    property: &str,
) -> bool {
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

pub(super) fn style_entries_equal(left: &StyleEntry, right: &StyleEntry) -> bool {
    left.name == right.name && left.value == right.value && left.priority == right.priority
}

pub(super) fn refresh_inline_state_entries_after_pdb_mutation(
    state: &mut CssInlineStyleDeclarationState,
    property: &str,
    affected_names: &[String],
    new_entries: impl IntoIterator<Item = StyleEntry>,
    new_side_entries: impl IntoIterator<Item = StyleEntry>,
) {
    let new_entries = new_entries.into_iter().collect::<Vec<_>>();
    let new_side_entries = new_side_entries.into_iter().collect::<Vec<_>>();
    let has_renderer_order_projection = state
        .source_entries()
        .iter()
        .chain(new_entries.iter())
        .any(|entry| {
            entry.name.starts_with("--")
                || entry.value.is_empty()
                || css_value_uses_unresolved_cssom_storage(&entry.value)
        });
    let has_all_entry = state
        .source_entries()
        .iter()
        .any(|entry| entry.name == "all");
    if state.side_entries().is_empty()
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
        .side_entries()
        .iter()
        .filter(|entry| {
            style_entry_affects_property_query(entry, property, affected_names)
                && !style_entry_is_replaceable_by_pdb_property(entry, property, affected_names)
                && !block_entries_contains_entry(&block_entries, entry)
        })
        .cloned()
        .collect::<Vec<_>>();
    let compatibility = state.compatibility.get_or_insert_with(Box::default);
    compatibility.side_entries.retain(|entry| {
        !block_entries_contains_entry(&block_entries, entry)
            && (!style_entry_affects_property_query(entry, property, affected_names)
                || !style_entry_is_replaceable_by_pdb_property(entry, property, affected_names))
    });
    if compatibility.entries.is_empty()
        && !state.block.is_empty()
        && (!new_side_entries.is_empty() || has_renderer_order_projection)
    {
        compatibility.entries = block_entries;
    }
    compatibility.entries.retain(|entry| {
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
        &mut compatibility.entries,
        &mut state.block,
        affected_names,
    );
    compatibility.entries.extend(new_entries);
    compatibility.side_entries.extend(new_side_entries);
    if compatibility.entries.is_empty() && compatibility.side_entries.is_empty() {
        state.compatibility = None;
    }
}

pub(super) fn expand_unresolved_box_shorthand_projection_after_mutation(
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

pub(super) fn unresolved_box_shorthand_longhands(
    property: &str,
) -> Option<&'static [&'static str]> {
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
