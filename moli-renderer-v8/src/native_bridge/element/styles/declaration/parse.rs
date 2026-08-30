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
    color::{
        ColorFunction,
        component::ColorComponent,
        parsing::{NumberOrAngleComponent, NumberOrPercentageComponent},
    },
    context::QuirksMode,
    properties::{
        PropertyDeclaration, PropertyDeclarationId, PropertyId, SourcePropertyDeclaration,
        parse_one_declaration_into,
    },
    stylesheets::{CssRuleType, Origin, UrlExtraData},
    values::specified::{Color as SpecifiedColor, ColorPropertyValue},
};
use style_traits::{CssString, ParsingMode, ToCss};

mod cssom_mutation;
mod declaration_parser;
mod entry_conversion;
mod inline_state;
mod pdb_compat;
mod property_access;
pub(crate) use cssom_mutation::{
    parse_inline_css_text_with_base, parse_style_property_entries_for_cssom_write,
};
pub(in crate::native_bridge::element::styles) use cssom_mutation::{
    parse_style_property_entries_for_cssom_fallback_write,
    set_inline_style_css_text_with_pdb_storage, set_inline_style_property_with_pdb_storage,
};
pub(crate) use declaration_parser::{
    cssom_style_entry_requires_structured_parser, parse_style_property_entries_with_base,
};
pub(in crate::native_bridge::element::styles) use entry_conversion::{
    set_style_entries_if_changed_with_inline_base_url, set_style_entries_with_inline_base_url,
    style_entries, style_entries_for_style_object,
};
pub(in crate::native_bridge::element::styles) use inline_state::expand_unresolved_box_shorthand_entries_for_mutation;
pub(crate) use inline_state::{
    inline_state_property_priority_with_pdb, inline_state_property_value_with_pdb,
};
pub(crate) use pdb_compat::{
    cssom_style_property_uses_preferred_pdb_supplemental_entries,
    cssom_style_property_write_can_use_pdb_storage, cssom_text_decoration_line_value_is_compat,
    set_pdb_block_property_collecting_entries, style_entry_is_pdb_supplemental_side_entry,
    style_property_affected_names_with_pdb, style_property_mutation_affected_names_with_pdb,
    style_property_mutation_cleanup_names_with_pdb,
};
pub(crate) use property_access::{
    pdb_property_priority_for_cssom_query_with_side_entries,
    pdb_property_value_for_cssom_query_with_side_entries, style_entries_css_text_with_pdb,
    style_entries_property_priority_with_pdb, style_entries_property_value_with_pdb,
};

pub(in crate::native_bridge::element::styles) struct StyleObjectEntries {
    pub(in crate::native_bridge::element::styles) entries: Vec<StyleEntry>,
    pub(in crate::native_bridge::element::styles) base_url: Option<url::Url>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleEntriesPdbQueryCandidate {
    Pdb,
    SupplementalSide(PdbQueryPriority),
}

pub(crate) struct ParsedStylePropertyEntries {
    pub(crate) entries: Vec<StyleEntry>,
    pub(crate) affected_names: Vec<String>,
}

#[derive(Clone, Copy)]
enum CssNumericPropertyRule {
    TimeList { non_negative: bool },
    AnimationDurationList,
    AnimationIterationCountList,
}

#[cfg(test)]
mod tests;
