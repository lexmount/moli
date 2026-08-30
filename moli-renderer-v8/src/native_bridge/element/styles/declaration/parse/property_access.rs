use super::declaration_parser::{
    canonical_unresolved_legacy_color_function, css_color_value_property_requires_stylo_parser,
};
use super::inline_state::unresolved_box_shorthand_longhands;
use super::pdb_compat::{
    css_value_uses_unresolved_cssom_storage, cssom_style_property_query_uses_pdb,
    pdb_block_from_style_entries, style_entry_affects_property_query, style_entry_is_pdb_safe,
    style_entry_is_pdb_supplemental_side_entry, style_property_affected_names_with_pdb,
};
use super::*;

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

pub(super) fn style_entries_pdb_with_supplemental(
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

pub(super) fn style_entries_pdb_property_query_candidate(
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

pub(super) fn style_entries_pdb_supplemental_entry(
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

pub(super) fn style_entries_pdb_for_property_query(
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

pub(super) fn style_entries_pdb_for_property_query_with_supplemental(
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
    let mut value = block.property_value(property)?;
    if value.is_empty() && moli_css_parse::is_cssom_custom_property_name(property) {
        return Some(" ".to_owned());
    }
    if value.is_empty() && !moli_css_parse::is_cssom_custom_property_name(property) {
        return None;
    }
    if css_color_value_property_requires_stylo_parser(property)
        && let Some(canonical) = canonical_unresolved_legacy_color_function(&value)
    {
        value = canonical;
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

pub(super) fn pdb_property_is_declared_for_cssom_query(
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

pub(super) fn pdb_overflow_property_value(
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

pub(super) fn pdb_overflow_property_priority(
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

pub(super) fn overflow_property_query_uses_pdb_supplemental_side_entries(
    property: &str,
    side_entries: &[StyleEntry],
) -> bool {
    matches!(property, "overflow" | "overflow-x" | "overflow-y")
        && side_entries.iter().any(|entry| {
            matches!(entry.name.as_str(), "overflow-x" | "overflow-y")
                && style_entry_is_pdb_supplemental_side_entry(entry)
        })
}

pub(super) fn text_decoration_property_query_uses_pdb_supplemental_side_entries(
    property: &str,
    side_entries: &[StyleEntry],
) -> bool {
    property == "text-decoration"
        && side_entries.iter().any(|entry| {
            entry.name == "text-decoration-line"
                && style_entry_is_pdb_supplemental_side_entry(entry)
        })
}

pub(super) fn pdb_text_decoration_shorthand_value(
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

pub(super) fn pdb_text_decoration_shorthand_priority(
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

pub(super) fn pdb_text_decoration_longhand_entries(
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

pub(super) fn pdb_text_decoration_longhand_entry(
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

pub(super) fn serialize_pdb_text_decoration_shorthand(
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

pub(super) fn text_decoration_component_or_initial<'a>(
    value: &'a str,
    initial: &'static str,
) -> &'a str {
    if value.is_empty() { initial } else { value }
}

pub(super) fn pdb_overflow_longhand_value(
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

pub(super) fn pdb_overflow_longhand_priority(
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

pub(super) fn serialize_font_variant_shorthand_values(values: &[String]) -> Option<String> {
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
