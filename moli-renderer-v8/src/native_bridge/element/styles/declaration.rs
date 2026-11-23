mod identity;
mod observation;
mod parse;
mod properties;
mod sizing;
mod style_world;
mod values;

pub(crate) use identity::StyleMode;
pub(super) use identity::style_runtime_and_handle_from_object;
pub(super) use observation::StyleComputationContext;
pub(crate) use observation::{ComputedStyleRead, StyleObservation};
pub(crate) use parse::{
    cssom_style_entry_requires_structured_parser,
    cssom_style_property_uses_preferred_pdb_supplemental_entries,
    cssom_style_property_write_can_use_pdb_storage, cssom_text_decoration_line_value_is_compat,
    inline_state_property_priority_with_pdb, inline_state_property_value_with_pdb,
    parse_inline_css_text_with_base,
    parse_style_property_entries_for_cssom_write as parse_cssom_style_property_entries_for_write,
    parse_style_property_entries_with_base as parse_cssom_style_property_entries_with_base,
    pdb_property_priority_for_cssom_query_with_side_entries,
    pdb_property_value_for_cssom_query_with_side_entries,
    set_pdb_block_property_collecting_entries, style_entries_css_text_with_pdb,
    style_entries_property_priority_with_pdb, style_entries_property_value_with_pdb,
    style_entry_is_pdb_supplemental_side_entry as cssom_style_entry_is_pdb_supplemental_side_entry,
    style_property_affected_names_with_pdb as cssom_style_property_affected_names_with_pdb,
    style_property_mutation_affected_names_with_pdb as cssom_style_property_mutation_affected_names_with_pdb,
    style_property_mutation_cleanup_names_with_pdb as cssom_style_property_mutation_cleanup_names_with_pdb,
};
pub(super) use parse::{
    expand_unresolved_box_shorthand_entries_for_mutation,
    parse_style_property_entries_for_cssom_fallback_write,
    parse_style_property_entries_for_cssom_write, set_inline_style_css_text_with_pdb_storage,
    set_inline_style_property_with_pdb_storage, set_style_entries_if_changed_with_inline_base_url,
    set_style_entries_with_inline_base_url, style_entries, style_entries_for_style_object,
};
pub(super) use properties::{
    all_shorthand_applies_to, animation_shorthand_longhands, css_wide_keyword,
    font_variant_longhands, is_style_intrinsic_name, known_style_property,
    resolve_known_style_property_name, resolve_style_property_name, shorthand_longhands,
    supported_declared_property, text_decoration_shorthand_longhands,
    transition_shorthand_longhands,
};
pub(in crate::native_bridge::element) use style_world::style_base_url;
pub(crate) use values::{
    active_css_animation_transform_value, css_animation_start_applies,
    raw_inline_style_property_value, serialize_animation_range_shorthand,
    serialize_animation_shorthand_from_longhands, serialize_transition_shorthand_from_longhands,
    style_property_value,
};
pub(super) use values::{
    computed_style_applies, style_css_text_for_computed, style_property_count_with_context,
    style_property_index_exists_with_context, style_property_name_at_with_context,
    style_property_names_with_context, style_property_priority,
    style_property_value_for_pseudo_with_context, style_property_value_with_context,
};

fn normalize_css_integer_token(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'+') => (false, &trimmed[1..]),
        Some(b'-') => (true, &trimmed[1..]),
        Some(_) => (false, trimmed),
        None => return None,
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some("0".to_owned());
    }
    if negative {
        Some(format!("-{significant}"))
    } else {
        Some(significant.to_owned())
    }
}
