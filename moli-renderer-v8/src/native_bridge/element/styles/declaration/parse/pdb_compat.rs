use super::declaration_parser::{
    canonical_unresolved_legacy_color_function, css_color_value_property_requires_stylo_parser,
    css_math_value_property_requires_stylo_parser, cssom_style_entry_requires_structured_parser,
    parse_animation_numeric_property_entries, parse_transition_numeric_property_entries,
};
use super::inline_state::style_entries_equal;
use super::*;

pub(super) fn style_entry_is_pdb_safe(entry: &StyleEntry) -> bool {
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

pub(super) fn inline_style_entry_is_pdb_storage_candidate(entry: &StyleEntry) -> bool {
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

pub(super) fn pdb_mutation_value_for_style_entry(entry: &StyleEntry) -> std::borrow::Cow<'_, str> {
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

pub(super) fn style_entry_affects_property_query(
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

pub(super) fn prefixed_style_entry_is_independent_of_unprefixed_property(
    entry_name: &str,
    property: &str,
) -> bool {
    entry_name.starts_with("-webkit-") && !property.starts_with("-webkit-")
}

pub(super) fn pdb_block_from_style_entries(
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

pub(super) fn set_pdb_block_property_from_style_entry(
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

pub(super) fn style_entries_affecting_property(
    entries: &[StyleEntry],
    property: &str,
) -> Vec<StyleEntry> {
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

pub(super) fn cssom_style_property_write_uses_pdb(name: &str, value: &str) -> bool {
    if moli_css_parse::css_value_is_eof_open_var_function(value) {
        return false;
    }
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

pub(super) fn inline_style_property_write_can_use_pdb_storage(name: &str, value: &str) -> bool {
    cssom_style_property_write_uses_pdb(name, value)
        || cssom_style_property_write_can_use_pdb_storage(name, value)
            && cssom_border_image_property_name(name)
        || inline_box_style_property_can_use_pdb_storage(name)
}

pub(super) fn inline_box_style_property_can_use_pdb_storage(name: &str) -> bool {
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

pub(super) fn cssom_ordinary_longhand_value_can_use_direct_pdb_write(
    name: &str,
    value: &str,
) -> bool {
    let name = canonical_style_property_name(name);
    if !cssom_numeric_longhand_can_skip_legacy_cssom_parser(&name) {
        return false;
    }
    cssom_legacy_parser_value_can_use_pdb_storage(&name, value)
        && style_property_affected_names_with_pdb(&name)
            .is_some_and(|affected_names| affected_names.len() == 1 && affected_names[0] == name)
}

pub(super) fn cssom_legacy_parser_value_can_use_pdb_storage(name: &str, value: &str) -> bool {
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

pub(super) fn cssom_numeric_longhand_can_skip_legacy_cssom_parser(name: &str) -> bool {
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

pub(super) fn cssom_border_image_reset_value_uses_pdb_storage(name: &str, value: &str) -> bool {
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

pub(super) fn cssom_border_image_property_name(name: &str) -> bool {
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

pub(super) fn cssom_style_property_query_uses_pdb(name: &str) -> bool {
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

pub(super) fn cssom_animation_property_query_uses_pdb(name: &str, is_longhand_query: bool) -> bool {
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

pub(super) fn cssom_style_shorthand_query_uses_pdb(name: &str) -> bool {
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
            | "overflow"
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

pub(super) fn cssom_style_property_write_requires_legacy_parser(name: &str, value: &str) -> bool {
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

pub(super) fn stylo_pdb_owns_property(name: &str) -> bool {
    stylo_pdb_entries_for_property(name, "initial", false).is_some()
}

pub(super) fn cssom_border_or_outline_property_write_uses_pdb(name: &str) -> bool {
    cssom_border_property_write_uses_pdb(name)
        || cssom_border_image_property_write_uses_pdb(name)
        || cssom_outline_property_write_uses_pdb(name)
}

pub(super) fn cssom_border_image_property_write_uses_pdb(name: &str) -> bool {
    cssom_border_image_property_name(name)
}

pub(super) fn cssom_structured_property_write_uses_pdb(name: &str) -> bool {
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

pub(super) fn cssom_text_decoration_property_write_uses_pdb(name: &str, _value: &str) -> bool {
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

pub(super) fn cssom_text_emphasis_property_write_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "text-emphasis" | "text-emphasis-color" | "text-emphasis-position" | "text-emphasis-style"
    )
}

pub(super) fn cssom_transition_property_write_uses_pdb(name: &str) -> bool {
    name == "transition" || name.starts_with("transition-")
}

pub(super) fn cssom_webkit_text_stroke_property_write_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "-webkit-text-stroke" | "-webkit-text-stroke-color" | "-webkit-text-stroke-width"
    )
}

pub(super) fn cssom_animation_property_write_uses_pdb(name: &str) -> bool {
    name == "animation"
        || name == "animation-range"
        || name == "animation-timeline"
        || name.starts_with("animation-")
}

pub(super) fn cssom_font_property_write_uses_pdb(name: &str, value: &str) -> bool {
    if name == "font" && font_shorthand_value_requires_legacy_system_font_keyword(value) {
        return false;
    }
    name == "font" || font_shorthand_longhands().contains(&name)
}

pub(super) fn font_shorthand_value_requires_legacy_system_font_keyword(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "caption" | "icon" | "menu" | "message-box" | "small-caption" | "status-bar"
    )
}

pub(super) fn cssom_font_variant_property_write_uses_pdb(name: &str) -> bool {
    name == "font-variant" || font_variant_longhands().contains(&name)
}

pub(super) fn cssom_overflow_property_write_uses_pdb(name: &str) -> bool {
    matches!(name, "overflow" | "overflow-x" | "overflow-y")
}

pub(super) fn cssom_border_property_write_uses_pdb(name: &str) -> bool {
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

pub(super) fn cssom_outline_property_write_uses_pdb(name: &str) -> bool {
    matches!(
        name,
        "outline" | "outline-width" | "outline-style" | "outline-color"
    )
}

pub(super) fn css_value_contains_overlay_keyword(value: &str) -> bool {
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

pub(super) fn stylo_pdb_entries_for_property(
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
    let canonical_unresolved_color = css_color_value_property_requires_stylo_parser(name)
        .then(|| canonical_unresolved_legacy_color_function(value))
        .flatten();
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
        let mut entry_value = entry.value;
        let is_declared_empty_custom_property = entry_value.is_empty()
            && !value.is_empty()
            && moli_css_parse::is_cssom_custom_property_name(&entry_name)
            && block.property_is_declared(&entry_name);
        if entry_name.is_empty()
            || entry_value.is_empty() && !is_declared_empty_custom_property
            || entry.priority != priority
            || !accepted_names
                .iter()
                .any(|accepted| accepted == &entry_name)
        {
            return None;
        }
        if entry_name == name
            && let Some(canonical) = canonical_unresolved_color.as_ref()
        {
            entry_value.clone_from(canonical);
        }
        entries.push(StyleEntry {
            name: entry_name,
            value: entry_value,
            priority: entry.priority,
        });
    }
    Some(ParsedStylePropertyEntries {
        entries,
        affected_names,
    })
}

pub(super) fn css_value_uses_unresolved_cssom_storage(value: &str) -> bool {
    moli_css_parse::css_value_may_contain_var_function(value)
        || moli_css_parse::css_value_may_contain_env_function(value)
}

pub(super) fn parse_pdb_supplemental_entries(
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

pub(super) fn parse_text_decoration_line_supplemental_entries(
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

pub(super) fn normalize_text_decoration_line_compat_value(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    cssom_text_decoration_line_value_is_compat(&value).then_some(value)
}

pub(crate) fn cssom_text_decoration_line_value_is_compat(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "spelling-error" | "grammar-error"
    )
}

pub(super) fn normalize_outline_color_supplemental_value(value: &str) -> Option<String> {
    value
        .trim()
        .eq_ignore_ascii_case("invert")
        .then(|| "invert".to_owned())
}

pub(super) fn parse_overflow_overlay_supplemental_entries(
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

pub(super) fn normalize_overflow_overlay_longhand_value(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    matches!(
        value.as_str(),
        "visible" | "hidden" | "clip" | "scroll" | "auto" | "overlay"
    )
    .then_some(value)
}

pub(super) fn parse_preferred_pdb_supplemental_entries(
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

pub(super) fn parse_animation_timing_function_preferred_supplemental_entries(
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

pub(super) fn text_decoration_standard_longhand_affects_family(name: &str) -> bool {
    matches!(
        name,
        "text-decoration-color"
            | "text-decoration-line"
            | "text-decoration-style"
            | "text-decoration-thickness"
    )
}

pub(super) fn append_unique_name(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|existing| existing == name) {
        names.push(name.to_owned());
    }
}

pub(super) fn cssom_empty_specified_placeholder_property(name: &str) -> bool {
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
