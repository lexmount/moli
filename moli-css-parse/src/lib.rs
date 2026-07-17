//! Shared CSS parsing primitives used by moli renderer and CDP layers.
//!
//! Centralises shared declaration / `@font-face` / top-level-rule parsers and
//! the small set of string helpers around them.

mod color;
mod declaration;
mod font_face;
mod math;
mod root_margin;
mod stylo_stylesheet;
mod transform;
mod util;
mod value;

pub use color::{
    css_system_color_srgb, parse_css_color_scheme_flags, parse_css_color_to_opaque_srgb_hex,
    parse_css_color_to_srgb_bytes,
};
pub use declaration::{CssDeclaration, DeclarationParseOptions, parse_declaration_list};
pub use font_face::{
    CssFontFace, font_load_query_contains_css_wide_keyword, font_load_query_family,
    normalize_font_face_src, parse_font_faces,
};
pub use math::{
    ContainerQueryLengthContext, CssNumericContext, CssNumericKind, CssNumericValue, UnitlessAngle,
    UnitlessLength, balanced_function_len, css_number_value_is_supported,
    css_numeric_value_is_supported, css_time_value_is_supported, number_len, parse_angle_degrees,
    parse_number, parse_px_length, resolve_css_number, resolve_css_numeric,
    resolve_length_percentage, resolve_time_seconds, starts_with_supported_math_function,
};
pub use root_margin::{normalize_root_margin, root_margin_components};
pub use style::moli_declaration_block::{
    CssDeclarationBlock, CssDeclarationEntry, CssMutationProjection, CssRemoveResult, CssSetResult,
    parse_declaration_block,
};
pub use stylo_stylesheet::{
    CssConditionRuleView, CssCounterStyleRuleView, CssDetachedRuleListMutation,
    CssFontFaceDescriptorEntryView, CssFontFaceRuleView, CssFontFeatureValueEntryView,
    CssFontFeatureValuesRuleView, CssImportRuleView, CssKeyframesRuleView, CssLayerRuleView,
    CssMarginRuleView, CssNamespaceRuleView, CssNativeStylesheet, CssPageDescriptorEntryView,
    CssPageRuleView, CssParsedRuleText, CssPropertyRuleView, CssRuleInsertError, CssRuleSnapshot,
    css_rule_snapshot_from_native_with_stylo, delete_detached_keyframe_rule_with_stylo,
    delete_detached_nested_rule_with_stylo, font_face_descriptor_property_names_with_stylo,
    insert_detached_keyframe_rule_with_stylo, insert_detached_nested_rule_with_stylo,
    keyframe_rule_snapshot_from_native_with_stylo, keyframe_selector_texts_match_with_stylo,
    native_stylesheet_counter_style_rule_read_with_stylo, native_stylesheet_css_text_with_stylo,
    native_stylesheet_font_face_rule_read_with_stylo,
    native_stylesheet_font_feature_values_rule_read_with_stylo,
    native_stylesheet_import_rule_read_with_stylo, native_stylesheet_margin_rule_read_with_stylo,
    native_stylesheet_namespace_rule_read_with_stylo,
    native_stylesheet_property_rule_read_with_stylo, normalize_keyframe_selector_text_with_stylo,
    normalize_page_selector_text_with_stylo, page_descriptor_property_names_with_stylo,
    parse_condition_rule_view_with_stylo, parse_constructed_stylesheet_rule_snapshots_with_stylo,
    parse_counter_style_rule_view_with_stylo, parse_font_face_cssom_rule_with_stylo_context,
    parse_font_face_descriptor_block_with_stylo, parse_font_face_descriptor_entry_with_stylo,
    parse_font_face_rule_view_with_stylo, parse_font_feature_values_rule_view_with_stylo,
    parse_import_rule_view_with_stylo, parse_keyframes_rule_view_with_stylo,
    parse_layer_rule_view_with_stylo, parse_namespace_rule_view_with_stylo,
    parse_nested_rule_block_snapshots_with_stylo, parse_page_descriptor_block_with_stylo,
    parse_page_descriptor_entries_with_stylo, parse_page_margin_descriptor_block_with_stylo,
    parse_page_margin_rule_view_with_stylo, parse_page_rule_view_with_stylo,
    parse_property_rule_view_with_stylo, parse_stylesheet_rule_snapshot_for_insert_with_stylo,
    parse_stylesheet_rule_snapshots_with_stylo, parse_stylesheet_rule_texts_with_stylo,
    refresh_native_stylesheet_namespaces_after_cssom_mutation,
};
pub use transform::{CssTransformFunction, parse_transform_function_list};
pub use util::{
    camel_case_style_property_name, camel_to_kebab, canonical_style_property_identifier,
    canonical_style_property_name, decapitalize_ascii_head, escape_top_level_semicolons,
    is_cssom_custom_property_name, serialize_style_property_name, split_important_priority,
    unescape_top_level_semicolons, unquote_css_string,
};
pub use value::{
    css_declaration_value_has_valid_env_functions, css_value_may_contain_env_function,
    css_value_may_contain_var_function, normalize_css_variable_specified_value,
    normalize_cssom_component_value_serialization, normalize_custom_property_specified_value,
    serialize_component_values_single_line,
};

/// Returns the non-custom CSS properties that the pinned Stylo world currently
/// enables in an ordinary author style rule. Callers must install their Stylo
/// preferences before taking this snapshot.
pub fn stylo_enabled_style_rule_property_names() -> Vec<&'static str> {
    use style::stylesheets::{CssRuleType, CssRuleTypes};

    style::properties::NonCustomPropertyId::iter()
        .filter(|property| {
            property.to_property_id().enabled_for_all_content()
                && property.allowed_in_rule(CssRuleTypes::from(CssRuleType::Style))
        })
        .map(style::properties::NonCustomPropertyId::name)
        .collect()
}

/// Whether the pinned Stylo world currently enables this exact CSS name in an
/// ordinary author style rule.
pub fn stylo_style_rule_property_name_is_enabled(name: &str) -> bool {
    use style::{
        properties::PropertyId,
        stylesheets::{CssRuleType, CssRuleTypes},
    };

    let Ok(PropertyId::NonCustom(property)) = PropertyId::parse_enabled_for_all_content(name)
    else {
        return false;
    };
    property.allowed_in_rule(CssRuleTypes::from(CssRuleType::Style))
}

#[cfg(test)]
mod property_surface_tests {
    use super::{
        stylo_enabled_style_rule_property_names, stylo_style_rule_property_name_is_enabled,
    };

    #[test]
    fn style_rule_property_surface_excludes_page_only_descriptors() {
        let names = stylo_enabled_style_rule_property_names();

        assert!(names.contains(&"color"));
        assert!(names.contains(&"margin"));
        assert!(!names.contains(&"size"));
        assert!(!names.contains(&"page-orientation"));
        assert!(stylo_style_rule_property_name_is_enabled("color"));
        assert!(!stylo_style_rule_property_name_is_enabled("size"));
        assert!(!stylo_style_rule_property_name_is_enabled(
            "moli-not-a-property"
        ));
    }
}
