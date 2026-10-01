mod constructor;
mod detached_mutations;
mod detached_snapshot;
mod font_palette_values;
mod grouping_rule;
mod install;
mod media_list;
mod mutations;
mod native_stylesheet;
mod rule_callbacks;
mod rule_helpers;
mod rule_list;
mod rule_parse;
mod specific_rules;
mod style_sheet;
#[cfg(test)]
mod tests;
mod types;

// Re-export everything from submodules so siblings can cross-reference via `use super::*`
pub(crate) use constructor::*;
pub(crate) use detached_mutations::*;
pub(crate) use detached_snapshot::*;
pub(crate) use grouping_rule::*;
pub(crate) use install::*;
pub(crate) use media_list::*;
pub(crate) use mutations::*;
pub(crate) use native_stylesheet::*;
pub(crate) use rule_callbacks::*;
pub(crate) use rule_helpers::*;
pub(crate) use rule_list::*;
pub(crate) use rule_parse::*;
pub(crate) use specific_rules::*;
pub(crate) use style_sheet::*;
pub use types::*;

// Imports from external crates - available to all submodules via `use super::*`
use super::css_runtime::{css_supports_condition_text, resolved_promise};
use super::*;
use crate::css_custom_function::single_custom_css_function_projection;
use crate::css_style::{parse_css_declaration_list, serialize_css_style_entries};
use crate::detached_css_style::{
    build_lightweight_css_font_face_descriptors, build_lightweight_css_keyframe_style_declaration,
    build_lightweight_css_page_descriptors, build_lightweight_css_rule_style_declaration,
    build_lightweight_css_style_declaration, create_lightweight_css_style_stylo_declaration_block,
    lightweight_css_keyframe_declaration_write_uses_pdb,
    lightweight_css_rule_declaration_write_uses_pdb, lightweight_css_style_css_text,
    lightweight_css_style_has_pdb_side_entries,
    lightweight_css_style_stylo_declaration_block_css_text,
    lightweight_css_style_uses_only_stylo_declaration_block,
    remove_lightweight_css_style_stylo_declaration_block,
    set_lightweight_css_style_change_callback, set_lightweight_css_style_css_text,
    set_lightweight_css_style_css_text_without_notify, set_lightweight_css_style_declaration_block,
    set_lightweight_css_style_stylo_declaration_block_id,
};
use crate::webidl;
use crate::{
    document_runtime::DomHandle,
    dom::native::Node,
    style_engine::media_list::{
        append_media_query_list_medium, delete_media_query_list_medium, media_query_list_items,
        normalize_media_query_list,
    },
    util::{
        call_object_method, get_private_object, get_private_value, global_constructor_prototype,
        serialize_v8_array, set_private_value,
    },
};
use cssparser::{serialize_identifier, serialize_string};
use moli_css_parse::{
    CssCounterStyleRuleView, CssDetachedRuleListMutation, CssFontFaceRuleView,
    CssFontFeatureValueEntryView, CssFontFeatureValuesRuleView, CssImportRuleView,
    CssKeyframesRuleView, CssMarginRuleView, CssNamespaceRuleView, CssNativeStylesheet,
    CssPageRuleView, CssPropertyRuleView, CssRuleInsertError, CssRuleSnapshot,
    delete_detached_keyframe_rule_with_stylo, delete_detached_nested_rule_with_stylo,
    insert_detached_keyframe_rule_with_stylo, insert_detached_nested_rule_with_stylo,
    keyframe_selector_texts_match_with_stylo, native_stylesheet_counter_style_rule_read_with_stylo,
    native_stylesheet_font_face_rule_read_with_stylo,
    native_stylesheet_font_feature_values_rule_read_with_stylo,
    native_stylesheet_import_rule_read_with_stylo, native_stylesheet_margin_rule_read_with_stylo,
    native_stylesheet_namespace_rule_read_with_stylo,
    native_stylesheet_property_rule_read_with_stylo, normalize_keyframe_selector_text_with_stylo,
    normalize_page_selector_text_with_stylo, parse_condition_rule_view_with_stylo,
    parse_constructed_stylesheet_rule_snapshots_with_stylo,
    parse_counter_style_rule_view_with_stylo, parse_font_face_descriptor_block_with_stylo,
    parse_font_face_rule_view_with_stylo, parse_font_feature_values_rule_view_with_stylo,
    parse_import_rule_view_with_stylo, parse_keyframes_rule_view_with_stylo,
    parse_layer_rule_view_with_stylo, parse_namespace_rule_view_with_stylo,
    parse_nested_rule_block_snapshots_with_stylo, parse_page_descriptor_block_with_stylo,
    parse_page_margin_descriptor_block_with_stylo, parse_page_margin_rule_view_with_stylo,
    parse_page_rule_view_with_stylo, parse_property_rule_view_with_stylo,
    parse_stylesheet_rule_snapshot_for_insert_with_stylo,
    parse_stylesheet_rule_snapshots_with_stylo, serialize_component_values_single_line,
};
#[allow(unused_imports)]
use moli_selector::{
    StyleRuleNamespaceContext, StyleRuleSelectorContext,
    canonicalize_cssom_style_rule_selector_text,
};
use moli_webapi_declare::WebApiFunctionTemplate;
use style::stylesheets::CssRuleType;
