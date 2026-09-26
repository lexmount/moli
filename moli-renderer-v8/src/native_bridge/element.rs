use super::super::{
    context_bootstrap::{
        bridge_descriptor::{RuntimeInstallGroups, SpecializedTemplateInstaller},
        selection_value_for_window,
    },
    util::{
        callback_data_index_value, callback_data_item, get_private_value, set_private_value,
        v8_string, v8str,
    },
};
use super::node::{
    current_or_live_delegate_node_arg_handle, element_name_for_owner_document, node_is_document,
    node_is_element, node_runtime_and_handle_from_object,
    node_runtime_and_handle_from_object_or_detached, node_text_content_getter_function,
    require_element_getter_receiver, require_element_setter_receiver,
    set_text_content_in_reaction_scope, throw_incompatible_getter_receiver,
    throw_incompatible_method_receiver, throw_incompatible_setter_receiver,
};
use super::{JsContextHost, wrapped_handle_value};
use crate::{custom_elements, document_runtime::DomHandle, web_api_interfaces, webidl};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiTemplateValue};

mod activation;
mod anchors;
mod animations;
mod attributes;
mod canvas;
mod class_list;
mod content;
mod dataset;
mod details_dialog;
mod event_handlers;
mod events;
mod focus;
mod forms;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FormLookupWork {
    pub traversals: u64,
    pub enumerations: u64,
    pub inspected_nodes: u64,
}

#[cfg(test)]
std::thread_local! {
    static FORM_LOOKUP_WORK: std::cell::Cell<FormLookupWork> = const { std::cell::Cell::new(FormLookupWork { traversals: 0, enumerations: 0, inspected_nodes: 0 }) };
}

#[cfg(test)]
pub(crate) fn take_form_lookup_work_for_test() -> FormLookupWork {
    FORM_LOOKUP_WORK.with(|work| work.replace(FormLookupWork::default()))
}

#[cfg(test)]
fn record_form_lookup_traversal_for_test() {
    FORM_LOOKUP_WORK.with(|work| {
        let mut current = work.get();
        current.traversals += 1;
        work.set(current);
    });
}

#[cfg(test)]
fn record_form_lookup_enumeration_for_test() {
    FORM_LOOKUP_WORK.with(|work| {
        let mut current = work.get();
        current.enumerations += 1;
        work.set(current);
    });
}

#[cfg(test)]
fn record_form_lookup_node_for_test() {
    FORM_LOOKUP_WORK.with(|work| {
        let mut current = work.get();
        current.inspected_nodes += 1;
        work.set(current);
    });
}
mod geometry;
mod global_attributes;
mod html_elements;
mod images;
mod media;
mod pointer_capture;
mod popover;
mod query;
mod reflection;
mod rendered_state;
mod script_execution;
mod shadow_dom;
mod shared;
mod state_callbacks;
mod styles;
mod toggle_event;
mod trusted_types;

pub(crate) use script_execution::{
    inline_script_source_for_execution, prepare_inline_classic_frame_script_job_for_execution,
};
pub(in crate::native_bridge) use trusted_types::{
    TrustedAttributeSetter, trusted_attribute_string_value, trusted_attribute_value_string,
};
use trusted_types::{
    TrustedHtmlSink, TrustedScriptElementSink, trusted_html_sink_string,
    trusted_script_element_sink_string, trusted_script_url_sink_string,
};
pub(crate) use trusted_types::{
    prepare_trusted_script_text, set_svg_animated_string_base_value,
    trusted_attribute_type_name_for_names, trusted_property_type_name_for_names,
};

pub(crate) use forms::{
    autocomplete_field_name, autofill_related_form_control_elements, control_has_datalist_ancestor,
    form_associated_form_owner, form_control_elements, form_data_control_elements,
    is_valid_submit_button, submit_form_with_submit_event,
};
pub(in crate::native_bridge) use forms::{form_named_control_matches, form_named_image_matches};
#[cfg(test)]
pub(crate) use styles::iframe_width_attribute_viewport_width;
pub(crate) use styles::{
    ComputedStyleTargetContext, STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT,
    STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT, STYLE_DECLARATION_READ_DOCUMENT_SLOT,
    STYLE_DECLARATION_SCREEN_HEIGHT_SLOT, STYLE_DECLARATION_SCREEN_WIDTH_SLOT,
    STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT, STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT,
    STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT, STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT,
    StyleObservation, computed_style_target_context, iframe_handle_viewport,
    marker_pseudo_element_is_generated_for_document_snapshot, style_viewport_for_document,
};
mod stylesheets;
mod template_install;
mod tree_mutation;
mod url_attributes;

mod base_prototypes;
use self::base_prototypes::*;
mod resource_elements;
use self::resource_elements::*;
mod form_controls;
use self::form_controls::*;
mod embedded_elements;
use self::embedded_elements::*;
mod media_elements;
use self::media_elements::*;
mod text_elements;
use self::text_elements::*;
mod form_methods;
use self::form_methods::*;
mod table_elements;
use self::table_elements::*;
mod shadow_template;
pub(in crate::native_bridge) use self::base_prototypes::BODY_LEGACY_PROTOTYPE_ACCESSORS;
pub(crate) use self::base_prototypes::{
    computed_style_property_for_handle, install_html_select_element_prototype_bindings,
};
use self::shadow_template::*;

pub(crate) use animations::{
    dispatch_animation_start_scan, queue_animation_start_for_listener_target,
};
pub(crate) use geometry::{
    compute_mock_client_rect, compute_mock_intersection_client_rect,
    compute_mock_intersection_scrollport_client_rect, scroll_node_into_view_if_needed,
};
pub(crate) use media::{install_text_track_template_bindings, resort_text_track_cues_for_cue};
pub(crate) use shadow_dom::{
    clear_shadow_root_adopted_style_sheets, css_module_sheet_for_url,
    element_internals_form_value_for_target,
    element_internals_validation_message_for_target_handle,
    element_internals_validity_for_target_handle, element_internals_will_validate_for_handle,
    ensure_shadow_root_adopted_style_sheets_initialized,
};
pub(crate) use styles::{
    ComputedStyleRead, StyleMode, active_css_animation_transform_value,
    computed_style_properties_for_inspector_handle,
    computed_style_property_values_for_document_snapshot, css_animation_start_applies,
    cssom_style_entry_requires_structured_parser,
    cssom_style_property_mutation_affected_names_with_pdb,
    cssom_style_property_mutation_cleanup_names_with_pdb,
    cssom_style_property_uses_preferred_pdb_supplemental_entries,
    cssom_style_property_write_can_use_pdb_storage, cssom_text_decoration_line_value_is_compat,
    parse_cssom_style_property_entries_for_write, parse_cssom_style_property_entries_with_base,
    parse_inline_css_text_with_base, pdb_property_priority_for_cssom_query_with_side_entries,
    pdb_property_value_for_cssom_query_with_side_entries, raw_inline_style_property_value,
    serialize_animation_range_shorthand, serialize_animation_shorthand_from_longhands,
    serialize_transition_shorthand_from_longhands, set_pdb_block_property_collecting_entries,
    style_entries_css_text_with_pdb, style_entries_property_priority_with_pdb,
    style_entries_property_value_with_pdb, style_property_value,
};

use super::document::{
    clear_detached_iframe_cached_context, clear_detached_iframe_cached_context_for_handle,
    detached_iframe_content_document, detached_iframe_content_window,
    detached_shadow_root_selection_value, node_shadow_root_element_from_point_callback,
    node_shadow_root_elements_from_point_callback,
};
pub(crate) use activation::perform_clipboard_key_default_action;
pub(crate) use activation::{
    NamedHyperlinkPopup, SpecialBrowsingContextTarget, navigate_existing_browsing_context_target,
    navigate_iframe_target,
};
pub(crate) use activation::{
    activate_default_submit_button_via_keyboard, activate_handle_after_pointer_release,
    activate_handle_via_click, activate_handle_via_click_with_detail_and_modifiers,
    activate_handle_via_synthetic_click, dispatched_click_activation_target,
    finish_legacy_activation_for_dispatched_click, perform_auxiliary_link_default_action,
    perform_click_default_action_for_dispatched_event, perform_drop_default_action,
    prepare_legacy_activation_for_dispatched_click, replace_contenteditable_selection,
    scroll_to_document_fragment_target, scroll_to_url_fragment_or_top,
    select_contenteditable_contents,
};
pub(crate) use activation::{document_copy_command_supported, run_document_copy_command};
pub(super) use activation::{
    input_show_picker_callback, node_click_callback, select_show_picker_callback,
};
pub(super) use anchors::{
    anchor_text_getter_function, anchor_text_setter_function, anchor_to_string_callback,
    area_to_string_callback,
};
pub(crate) use attributes::mutate_live_element_attribute_for_inspector;
pub(super) use attributes::{
    bridge_get_attribute_callback, bridge_remove_attribute_callback, bridge_set_attribute_callback,
    node_get_attribute_callback, node_get_attribute_names_callback,
    node_get_attribute_node_callback, node_get_attribute_node_ns_callback,
    node_get_attribute_ns_callback, node_has_attribute_callback, node_has_attribute_ns_callback,
    node_has_attributes_callback, node_remove_attribute_callback,
    node_remove_attribute_node_callback, node_remove_attribute_ns_callback,
    node_set_attribute_callback, node_set_attribute_node_callback, node_set_attribute_ns_callback,
    node_toggle_attribute_callback,
};
pub(crate) use canvas::{
    canvas_dimension_value, canvas_get_context_callback, canvas_to_data_url_callback,
    canvas_transfer_control_to_offscreen_callback,
};
pub(crate) use canvas::{
    html_canvas_height_getter_callback, html_canvas_height_setter_callback,
    html_canvas_width_getter_callback, html_canvas_width_setter_callback,
};
pub(crate) use class_list::install_dom_token_list_prototype_bindings;
pub(super) use class_list::{
    build_dom_token_list_wrapper_template, html_rel_list_getter_function,
    html_rel_list_setter_function, iframe_sandbox_getter_function, iframe_sandbox_setter_function,
    link_sizes_getter_function, link_sizes_setter_function, output_html_for_getter_function,
    output_html_for_setter_function, svg_rel_list_setter_function,
};
pub(super) use content::{
    node_direct_text_content, node_get_html_callback, node_inner_html_getter_function,
    node_inner_html_setter_function, node_inner_text_getter_function,
    node_inner_text_setter_function, node_outer_html_getter_function,
    node_outer_html_setter_function, node_outer_text_getter_function,
    node_outer_text_setter_function, node_set_html_unsafe_callback,
    set_inner_text_in_reaction_scope, title_text_getter_function, title_text_setter_function,
};
pub(super) use dataset::{build_dom_string_map_wrapper_template, node_dataset_getter_function};
use details_dialog::{
    close_dialog_element, closed_details_ancestors_to_reveal, details_open_getter_function,
    details_open_setter_function, dialog_close_callback, dialog_open_getter_function,
    dialog_open_setter_function, dialog_request_close_callback, dialog_return_value_getter_function,
    dialog_return_value_setter_function, dialog_show_callback, dialog_show_modal_callback,
    perform_summary_click_default_action,
};
pub(crate) use details_dialog::{
    queue_details_toggle_event_for_attribute_change, queue_parser_details_toggle_event,
    queue_parser_details_toggle_events_in_subtree,
};
use event_handlers::install_body_or_frameset_window_event_handler_accessors;
use event_handlers::install_global_event_handler_template_bindings as install_global_event_handler_templates_for_owner;
use event_handlers::install_node_event_handler_template_bindings;
pub(crate) use event_handlers::{
    GlobalEventHandlerOwner, body_or_frameset_reflects_window_event_type,
    canonical_event_handler_event_type, compile_body_window_event_attribute,
    event_handler_content_attribute_name, initialize_parser_inserted_body_window_event_handlers,
    resolve_window_event_handler_content_attribute,
    node_event_handler_getter_function, node_event_handler_setter_function,
    legacy_lenient_this_event_handler,
    compile_window_event_attribute_handler,
    ParserAddedBodyWindowHandlers,
};
pub(in crate::native_bridge::element) use events::construct_event;
pub(crate) use events::construct_focus_event;
pub(crate) use events::{
    NodePublicEventDispatchOutcome, TextEditInputType, TouchEventPoint, construct_clipboard_event,
    construct_command_event, construct_drag_event, construct_input_event, construct_interest_event,
    construct_keyboard_event, construct_mouse_event_with_detail_and_modifiers,
    construct_mouse_event_with_modifiers, construct_mouse_event_with_related_target_and_modifiers,
    construct_pointer_event, construct_pointer_event_with_modifiers,
    construct_pointer_event_with_related_target,
    construct_pointer_event_with_related_target_and_modifiers, construct_simple_event,
    construct_submit_event, construct_toggle_event, construct_touch_event,
    construct_touch_event_with_points, construct_wheel_event, dispatch_beforeinput,
    dispatch_public_event,
};
use events::{construct_click_event, construct_click_event_with_detail_and_modifiers};
pub(crate) use focus::{
    contenteditable_editing_host, contenteditable_editing_host_in_dom, focus_element,
    focus_live_element_for_inspector, perform_access_key_default_action_for_dispatched_event,
    perform_hover_interest_default_action_for_dispatched_event, perform_mouse_focus_default_action,
    perform_tab_focus_default_action_for_dispatched_event, post_parse_autofocus_is_pending,
    process_post_parse_autofocus, reset_document_navigation_focus,
    reset_focus_from_previous_handle, reset_focus_from_previous_handle_with_previous_focus_within,
    schedule_focus_blur_if_needed, update_focus,
};
use focus::{is_disabled_form_control, is_focusable};
pub(super) use focus::{node_blur_callback, node_focus_callback};
pub(super) use forms::{
    button_command_for_element_getter_function, button_command_for_element_setter_function,
    button_command_getter_function, button_command_setter_function,
    button_disabled_getter_function, button_disabled_setter_function,
    button_form_action_getter_function, button_form_action_setter_function,
    button_form_enctype_getter_function, button_form_enctype_setter_function,
    button_form_method_getter_function, button_form_method_setter_function,
    button_form_no_validate_getter_function, button_form_no_validate_setter_function,
    button_form_target_getter_function, button_form_target_setter_function,
    button_interest_for_element_getter_function, button_interest_for_element_setter_function,
    button_popover_target_action_getter_function, button_popover_target_action_setter_function,
    button_popover_target_element_getter_function, button_popover_target_element_setter_function,
    button_type_getter_function, button_type_setter_function, button_value_getter_function,
    button_value_setter_function, control_check_validity_callback, control_labels_getter_function,
    control_report_validity_callback, control_set_custom_validity_callback,
    control_validation_message_getter_function, control_validity_getter_function,
    control_will_validate_getter_function, datalist_options_getter_function,
    fieldset_disabled_getter_function, fieldset_disabled_setter_function,
    fieldset_elements_getter_function, fieldset_type_getter_function,
    form_accept_charset_getter_function, form_accept_charset_setter_function,
    form_action_getter_function, form_action_setter_function, form_associated_form_getter_function,
    form_autocomplete_getter_function, form_autocomplete_setter_function,
    form_check_validity_callback, form_elements_getter_function, form_encoding_getter_function,
    form_encoding_setter_function, form_enctype_getter_function, form_enctype_setter_function,
    form_length_getter_function, form_method_getter_function, form_method_setter_function,
    form_name_getter_function, form_name_setter_function, form_no_validate_getter_function,
    form_no_validate_setter_function, form_report_validity_callback, form_request_submit_callback,
    form_reset_callback, form_submit_callback, form_target_getter_function,
    form_target_setter_function, input_accept_getter_function, input_accept_setter_function,
    input_alt_getter_function, input_alt_setter_function, input_autocomplete_getter_function,
    input_autocomplete_setter_function, input_checked_getter_function,
    input_checked_setter_function, input_default_checked_getter_function,
    input_default_checked_setter_function, input_default_value_getter_function,
    input_default_value_setter_function, input_dir_name_getter_function,
    input_dir_name_setter_function, input_disabled_getter_function, input_disabled_setter_function,
    input_files_getter_function, input_files_setter_function, input_form_action_getter_function,
    input_form_action_setter_function, input_form_enctype_getter_function,
    input_form_enctype_setter_function, input_form_method_getter_function,
    input_form_method_setter_function, input_form_no_validate_getter_function,
    input_form_no_validate_setter_function, input_form_target_getter_function,
    input_form_target_setter_function, input_height_getter_function, input_height_setter_function,
    input_indeterminate_getter_function, input_indeterminate_setter_function,
    input_list_getter_function, input_max_getter_function, input_max_length_getter_function,
    input_max_length_setter_function, input_max_setter_function, input_min_getter_function,
    input_min_length_getter_function, input_min_length_setter_function, input_min_setter_function,
    input_multiple_getter_function, input_multiple_setter_function, input_pattern_getter_function,
    input_pattern_setter_function, input_placeholder_getter_function,
    input_placeholder_setter_function, input_read_only_getter_function,
    input_read_only_setter_function, input_required_getter_function,
    input_required_setter_function, input_size_getter_function, input_size_setter_function,
    input_src_getter_function, input_src_setter_function, input_step_down_callback,
    input_step_getter_function, input_step_setter_function, input_step_up_callback,
    input_type_getter_function, input_type_setter_function, input_value_as_date_getter_function,
    input_value_as_date_setter_function, input_value_as_number_getter_function,
    input_value_as_number_setter_function, input_value_getter_function,
    input_value_setter_function, input_width_getter_function, input_width_setter_function,
    label_activation_control_handle, label_control_getter_function, label_control_handle,
    label_form_getter_function, label_html_for_getter_function, label_html_for_setter_function,
    label_receives_programmatic_focus, legend_form_getter_function, meter_high_getter_function,
    meter_high_setter_function, meter_low_getter_function, meter_low_setter_function,
    meter_max_getter_function, meter_max_setter_function, meter_min_getter_function,
    meter_min_setter_function, meter_optimum_getter_function, meter_optimum_setter_function,
    meter_value_getter_function, meter_value_setter_function,
    option_default_selected_getter_function, option_default_selected_setter_function,
    option_disabled_getter_function, option_disabled_setter_function, option_form_getter_function,
    option_index_getter_function, option_label_getter_function, option_label_setter_function,
    option_selected_getter_function, option_selected_setter_function, option_text_getter_function,
    option_text_setter_function, option_value_getter_function, option_value_setter_function,
    output_default_value_getter_function, output_default_value_setter_function,
    output_type_getter_function, output_value_getter_function, output_value_setter_function,
    progress_max_getter_function, progress_max_setter_function, progress_position_getter_function,
    progress_value_getter_function, progress_value_setter_function, select_add_callback,
    select_autocomplete_getter_function, select_autocomplete_setter_function,
    select_disabled_getter_function, select_disabled_setter_function, select_item_callback,
    select_length_getter_function, select_length_setter_function, select_multiple_getter_function,
    select_multiple_setter_function, select_named_item_callback, select_options_getter_function,
    select_remove_callback, select_required_getter_function, select_required_setter_function,
    select_selected_index_getter_function, select_selected_index_setter_function,
    select_selected_options_getter_function, select_size_getter_function,
    select_size_setter_function, select_value_getter_function, select_value_setter_function,
    set_select_indexed_option, text_control_select_callback,
    text_control_selection_direction_getter_function,
    text_control_selection_direction_setter_function, text_control_selection_end_getter_function,
    text_control_selection_end_setter_function, text_control_selection_start_getter_function,
    text_control_selection_start_setter_function, text_control_set_range_text_callback,
    text_control_set_selection_range_callback, textarea_autocomplete_getter_function,
    textarea_autocomplete_setter_function, textarea_cols_getter_function,
    textarea_cols_setter_function, textarea_default_value_getter_function,
    textarea_default_value_setter_function, textarea_dir_name_getter_function,
    textarea_dir_name_setter_function, textarea_disabled_getter_function,
    textarea_disabled_setter_function, textarea_max_length_getter_function,
    textarea_max_length_setter_function, textarea_min_length_getter_function,
    textarea_min_length_setter_function, textarea_placeholder_getter_function,
    textarea_placeholder_setter_function, textarea_read_only_getter_function,
    textarea_read_only_setter_function, textarea_required_getter_function,
    textarea_required_setter_function, textarea_rows_getter_function,
    textarea_rows_setter_function, textarea_text_length_getter_function,
    textarea_type_getter_function, textarea_value_getter_function, textarea_value_setter_function,
    textarea_wrap_getter_function, textarea_wrap_setter_function,
};
pub(crate) use forms::{
    cache_input_files_from_selected_files, form_control_is_effectively_disabled,
    input_files_for_object,
};
pub(crate) use forms::{control_label_handles, v8_pattern_is_usable};
pub(crate) use forms::{
    dispatch_text_control_event, is_text_control, perform_implicit_submission_from_control,
    queue_text_control_document_selection_change_event, queue_text_control_selection_change_event,
    replace_text_control_selection, text_control_set_selection_range_internal,
    text_control_set_selection_range_with_direction_internal, text_control_value,
};
pub(in crate::native_bridge) use forms::{
    resize_select_options, select_add_insertion_point, select_options_resize_target,
};
use rendered_state::{node_check_visibility_callback, node_current_css_zoom_getter_function};

pub use geometry::ClientRect;
#[cfg(test)]
pub(crate) use geometry::observable_scrollbar_hit_test;
pub(crate) use geometry::{
    element_is_inert_for_hit_testing, InputSurfaceHit, apply_scroll_observable_effects, input_surface_hit_test,
    observable_caret_position, observable_deep_hit_test, observable_document_metrics,
    observable_event_offset, observable_geometry_batch, observable_geometry_query,
    observable_hit_test, observable_hit_test_all, observable_input_hit_test,
    observable_sources_with_fragments, perform_scrollbar_scroll_default_action,
    perform_wheel_scroll_default_action, published_geometry_batch, queue_scroll_observable_effects,
    scroll_node_into_view_at_center, scroll_node_into_view_at_start,
};
pub(super) use geometry::{
    node_client_height_getter_function, node_client_left_getter_function,
    node_client_top_getter_function, node_client_width_getter_function,
    node_get_bounding_client_rect_callback, node_get_client_rects_callback,
    node_offset_height_getter_function, node_offset_left_getter_function,
    node_offset_parent_getter_function, node_offset_top_getter_function,
    node_offset_width_getter_function, node_scroll_by_callback, node_scroll_height_getter_function,
    node_scroll_into_view_callback, node_scroll_into_view_if_needed_callback,
    node_scroll_left_getter_function, node_scroll_left_setter_function, node_scroll_to_callback,
    node_scroll_top_getter_function, node_scroll_top_setter_function,
    node_scroll_width_getter_function,
};
use global_attributes::canonical_fetch_priority_value;
pub(super) use global_attributes::{
    anchor_target_getter_function, anchor_target_setter_function, area_no_href_setter_function,
    area_target_getter_function, area_target_setter_function, base_target_getter_function,
    base_target_setter_function, canonical_cross_origin_value, canonical_dir_value,
    canonical_loading_value, canonical_preload_value, canonical_referrer_policy_value,
    dom_string_reflection_getter_function, dom_string_reflection_setter_function,
    html_align_getter_function, html_align_setter_function, html_alt_getter_function,
    html_as_getter_function, html_bg_color_getter_function, html_border_getter_function,
    html_charset_getter_function, html_cite_getter_function, html_color_getter_function,
    html_compact_getter_function, html_compact_setter_function, html_coords_getter_function,
    html_date_time_getter_function, html_decoding_getter_function, html_download_getter_function,
    html_fetch_priority_getter_function, html_frame_border_getter_function,
    html_height_getter_function, html_hreflang_getter_function, html_hspace_getter_function,
    html_label_getter_function, html_long_desc_getter_function, html_lowsrc_getter_function,
    html_margin_height_getter_function, html_margin_width_getter_function,
    html_media_getter_function, html_name_getter_function, html_name_setter_function,
    html_no_href_getter_function, html_no_resize_getter_function, html_no_resize_setter_function,
    html_no_shade_getter_function, html_no_shade_setter_function, html_ping_getter_function,
    html_rel_getter_function, html_rel_setter_function, html_scrolling_getter_function,
    html_shape_getter_function, html_size_getter_function, html_sizes_getter_function,
    html_true_speed_getter_function, html_true_speed_setter_function, html_type_getter_function,
    html_use_map_getter_function, html_value_getter_function, html_value_type_getter_function,
    html_version_getter_function, html_vspace_getter_function, html_width_getter_function,
    image_decoding_setter_function, image_long_desc_setter_function, image_lowsrc_setter_function,
    link_target_getter_function, link_target_setter_function, node_access_key_getter_function,
    node_access_key_label_getter_function, node_access_key_setter_function,
    node_allow_fullscreen_getter_function, node_allow_fullscreen_setter_function,
    node_autocapitalize_getter_function, node_autocapitalize_setter_function,
    node_autocorrect_getter_function, node_autocorrect_setter_function,
    node_autofocus_getter_function, node_autofocus_setter_function,
    node_content_editable_getter_function, node_content_editable_setter_function,
    node_credentialless_getter_function, node_credentialless_setter_function,
    node_dir_getter_function, node_dir_setter_function, node_draggable_getter_function,
    node_draggable_setter_function, node_enter_key_hint_getter_function,
    node_enter_key_hint_setter_function, node_focus_group_getter_function,
    node_focus_group_setter_function, node_focus_group_start_getter_function,
    node_focus_group_start_setter_function, node_hidden_getter_function,
    node_hidden_setter_function, node_inert_getter_function, node_inert_setter_function,
    node_input_mode_getter_function, node_input_mode_setter_function,
    node_is_content_editable_getter_function, node_lang_getter_function, node_lang_setter_function,
    node_spellcheck_getter_function, node_spellcheck_setter_function,
    node_tab_index_getter_function, node_tab_index_setter_function, node_title_getter_function,
    node_title_setter_function, node_translate_getter_function, node_translate_setter_function,
    node_writing_suggestions_getter_function, node_writing_suggestions_setter_function,
    null_to_empty_dom_string_reflection_getter_function,
    null_to_empty_dom_string_reflection_setter_function, object_archive_getter_function,
    object_code_base_getter_function, object_code_getter_function,
    object_code_type_getter_function, object_data_getter_function, object_declare_getter_function,
    object_declare_setter_function, object_standby_getter_function, pre_width_getter_function,
    pre_width_setter_function, source_height_getter_function, source_height_setter_function,
    source_width_getter_function, source_width_setter_function, table_cell_abbr_getter_function,
    table_cell_axis_getter_function, table_cell_headers_getter_function,
    table_cell_no_wrap_getter_function, table_cell_no_wrap_setter_function,
    table_cell_scope_getter_function, table_ch_getter_function, table_ch_off_getter_function,
    table_v_align_getter_function, unsigned_long_reflection_setter_function,
    usv_string_reflection_setter_function,
};
use html_elements::{
    body_background_getter_function, body_background_setter_function, li_value_getter_function,
    li_value_setter_function, meta_content_getter_function, meta_content_setter_function,
    meta_http_equiv_getter_function, meta_http_equiv_setter_function, ol_reversed_getter_function,
    ol_reversed_setter_function, ol_start_getter_function, ol_start_setter_function,
    ol_type_getter_function, ol_type_setter_function, optgroup_disabled_getter_function,
    optgroup_disabled_setter_function, table_caption_getter_function,
    table_caption_setter_function, table_cell_col_span_getter_function,
    table_cell_col_span_setter_function, table_cell_index_getter_function,
    table_cell_row_span_getter_function, table_cell_row_span_setter_function,
    table_col_span_getter_function, table_col_span_setter_function, table_create_caption_callback,
    table_create_t_body_callback, table_create_t_foot_callback, table_create_t_head_callback,
    table_delete_caption_callback, table_delete_row_callback, table_delete_t_foot_callback,
    table_delete_t_head_callback, table_insert_row_callback, table_row_cells_getter_function,
    table_row_delete_cell_callback, table_row_index_getter_function,
    table_row_insert_cell_callback, table_rows_getter_function, table_section_delete_row_callback,
    table_section_insert_row_callback, table_section_row_index_getter_function,
    table_section_rows_getter_function, table_t_bodies_getter_function,
    table_t_foot_getter_function, table_t_foot_setter_function, table_t_head_getter_function,
    table_t_head_setter_function, track_default_getter_function, track_default_setter_function,
    track_kind_getter_function, track_kind_setter_function, track_ready_state_getter_function,
    track_src_getter_function, track_src_setter_function, track_srclang_getter_function,
    track_srclang_setter_function,
};
use html_elements::{
    marquee_loop_getter_function, marquee_loop_setter_function,
    marquee_scroll_amount_getter_function, marquee_scroll_amount_setter_function,
    marquee_scroll_delay_getter_function, marquee_scroll_delay_setter_function,
};
pub(crate) use images::{
    apply_authorized_image_load_event_in_context, apply_image_attribute_mutation_plan,
    image_intrinsic_dimensions, image_selected_request_key, image_selected_source,
    plan_image_attribute_mutation, queue_image_load_event_after_document_adoption,
    queue_image_load_event_for_loading_change, queue_image_load_event_if_needed,
    queue_image_load_event_if_needed_with_initiator, queue_image_load_network_terminal_followup,
    queue_revealed_lazy_image_loads, reset_image_load_dispatch,
};
pub(in crate::native_bridge) use images::{
    image_complete_getter_function, image_current_src_getter_function, image_decode_callback,
    image_height_getter_function, image_height_setter_function, image_is_map_getter_function,
    image_is_map_setter_function, image_natural_height_getter_function,
    image_natural_width_getter_function, image_width_getter_function, image_width_setter_function,
    image_x_getter_function, image_y_getter_function,
};
pub(in crate::native_bridge) use media::queue_text_track_load_if_source;
pub(crate) use media::{
    MediaLoadEventPhase, apply_default_text_track_mode_for_track, apply_text_track_load_task,
    dispatch_media_load_event_phase, dispatch_media_seek_completion, dispatch_media_seeking_event,
    dispatch_text_track_list_event, queue_default_text_track_mode_if_needed,
    queue_media_canplay_after_text_tracks, queue_media_load_if_needed,
    queue_media_load_if_source_or_loading_change, queue_media_load_network_terminal_followup,
    queue_revealed_lazy_media_loads, queue_text_track_load_if_needed,
    queue_text_track_terminal_followup,
};
pub(super) use media::{
    apply_default_text_track_modes_for_media, media_add_text_track_callback,
    media_autoplay_getter_function, media_autoplay_setter_function, media_buffered_getter_function,
    media_can_play_type_callback, media_controls_getter_function, media_controls_setter_function,
    media_cross_origin_getter_function, media_cross_origin_setter_function,
    media_current_time_getter_function, media_current_time_setter_function,
    media_default_muted_getter_function, media_default_muted_setter_function,
    media_duration_getter_function, media_ended_getter_function, media_error_getter_function,
    media_height_getter_function, media_height_setter_function, media_load_callback,
    media_loading_getter_function, media_loading_setter_function, media_loop_getter_function,
    media_loop_setter_function, media_muted_getter_function, media_muted_setter_function,
    media_network_state_getter_function, media_pause_callback, media_paused_getter_function,
    media_play_callback, media_playback_rate_getter_function, media_playback_rate_setter_function,
    media_played_getter_function, media_plays_inline_getter_function,
    media_plays_inline_setter_function, media_poster_getter_function, media_poster_setter_function,
    media_preload_getter_function, media_preload_setter_function,
    media_ready_state_getter_function, media_seekable_getter_function,
    media_seeking_getter_function, media_src_getter_function, media_src_setter_function,
    media_text_tracks_getter_function, media_video_height_getter_function,
    media_video_width_getter_function, media_volume_getter_function, media_volume_setter_function,
    media_width_getter_function, media_width_setter_function, refresh_media_active_text_track_cues,
    track_ready_state_for_handle, track_text_track_getter_function,
};
use pointer_capture::{
    node_has_pointer_capture_callback, node_release_pointer_capture_callback,
    node_set_pointer_capture_callback,
};
pub(super) use popover::{
    dispatch_popover_hide_events, dispatch_popover_show_events, dispatch_popover_toggle_events,
};
pub(crate) use popover::{
    dispatch_popover_removal_events, handle_popover_attribute_change,
    perform_popover_invoker_default_action,
};
pub(super) use popover::{
    node_hide_popover_callback, node_popover_getter_function, node_popover_setter_function,
    node_show_popover_callback, node_toggle_popover_callback,
};
pub(super) use query::{
    node_closest_callback, node_get_elements_by_class_name_callback,
    node_get_elements_by_name_callback, node_get_elements_by_tag_name_callback,
    node_get_elements_by_tag_name_ns_callback, node_matches_callback,
    node_query_selector_all_callback, node_query_selector_callback,
};
pub(super) use reflection::set_reflected_attribute;
use reflection::{
    CrossOriginReflection, DomStringReflection, ElementReflectionInterface,
    NullToEmptyDomStringReflection, UnsignedLongReflection, UsvStringReflection,
    attribute_property_getter_from_object_or_detached,
    boolean_attribute_property_getter_from_object_or_detached,
    nullable_attribute_property_getter_from_object_or_detached, parse_non_negative_dimension,
    property_dom_string_value, property_string_value, property_usv_string_value,
    remove_reflected_attribute, set_attribute_property_on_object_or_detached,
    set_boolean_attribute_property_on_object_or_detached,
    set_dom_string_attribute_property_on_object, set_dom_string_attribute_property_utf16_on_object,
    set_nullable_dom_string_attribute_property_on_object, set_reflected_boolean_attribute,
    set_reflected_style_attribute_with_inline_base_url,
    set_usv_string_attribute_property_on_object,
};
pub(crate) use shadow_dom::install_element_internals_template_bindings;
pub(super) use shadow_dom::{
    element_attach_internals_callback, element_attach_shadow_callback,
    element_shadow_root_getter_function, node_slot_getter_function, node_slot_setter_function,
    shadow_root_init_from_attach_shadow_value,
};
use shadow_dom::{
    shadow_root_active_element_getter_function, shadow_root_adopted_style_sheets_getter_function,
    shadow_root_adopted_style_sheets_setter_function, shadow_root_clonable_getter_function,
    shadow_root_delegates_focus_getter_function, shadow_root_host_getter_function,
    shadow_root_mode_getter_function, shadow_root_reference_target_getter_function,
    shadow_root_reference_target_setter_function, shadow_root_serializable_getter_function,
    shadow_root_slot_assignment_getter_function, shadow_root_style_sheets_getter_function,
    slot_assign_callback, slot_assigned_elements_callback, slot_assigned_nodes_callback,
    slot_assigned_slot_getter_function, slot_name_getter_function, slot_name_setter_function,
    template_content_getter_function, template_shadow_root_adopted_style_sheets_getter_function,
    template_shadow_root_adopted_style_sheets_setter_function,
    template_shadow_root_clonable_getter_function, template_shadow_root_clonable_setter_function,
    template_shadow_root_custom_element_registry_getter_function,
    template_shadow_root_custom_element_registry_setter_function,
    template_shadow_root_delegates_focus_getter_function,
    template_shadow_root_delegates_focus_setter_function,
    template_shadow_root_mode_getter_function, template_shadow_root_mode_setter_function,
    template_shadow_root_serializable_getter_function,
    template_shadow_root_serializable_setter_function,
    template_shadow_root_slot_assignment_getter_function,
    template_shadow_root_slot_assignment_setter_function,
};
pub(super) use shared::element_attribute;
use shared::{element_attribute_names, element_has_attribute, style_string};
pub(super) use state_callbacks::{
    bridge_set_checked_state_callback, bridge_set_indeterminate_state_callback,
    bridge_set_input_value_callback, bridge_set_selected_state_callback,
};
pub(super) use styles::{
    build_style_wrapper_template, node_style_getter_function, node_style_setter_function,
};
pub(crate) use styles::{
    computed_style_property_names_from_object, computed_style_property_value_from_object,
    cssom_style_entry_is_pdb_supplemental_side_entry, cssom_style_property_affected_names_with_pdb,
    is_live_style_declaration_object, live_style_named_property_value,
    set_live_style_named_property_value, style_css_text_getter_callback,
    style_css_text_setter_callback, style_get_property_priority_callback,
    style_get_property_value_callback, style_item_callback, style_length_getter_callback,
    style_remove_property_callback, style_set_property_callback,
};
pub(crate) use stylesheets::{
    detach_cached_style_sheet_for_element, detach_cached_style_sheet_if_live_stylesheet_changed,
    style_sheet_for_element, style_sheet_getter_function, sync_cached_style_sheet_media_from_owner,
};

use stylesheets::{
    link_disabled_getter_function, link_disabled_setter_function, style_blocking_getter_function,
    style_blocking_setter_function, style_disabled_getter_function, style_disabled_setter_function,
};
pub(super) use template_install::{
    install_specialized_instance_properties, install_specialized_template,
};
pub(super) use tree_mutation::{
    node_insert_adjacent_element_callback, node_insert_adjacent_html_callback,
    node_insert_adjacent_node_callback, node_insert_adjacent_text_callback,
};
use url_attributes::{
    disconnected_iframe_can_materialize_detached_content, iframe_has_inactive_child_context,
    iframe_is_in_own_child_document, iframe_is_inside_its_own_child_context_document,
    parsed_url_like_attribute, resolve_url_like_attribute, set_resolved_url_attribute,
    should_block_dangling_markup_subresource,
};
pub(in crate::native_bridge) use url_attributes::{
    iframe_uses_detached_content_cache, parse_url_with_document_query_encoding,
};
pub(super) use url_attributes::{
    live_frame_owner_content_window_for_handle, update_iframe_snapshot_navigation,
};

pub(in crate::native_bridge) fn set_live_element_attribute_appending_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    value: &str,
) -> bool {
    clear_detached_iframe_context_before_navigation_attribute_change(
        scope,
        runtime_ptr,
        handle,
        name,
        Some(value),
    );
    let image_plan =
        plan_image_attribute_mutation(unsafe { &*runtime_ptr }, handle, name, Some(value));
    let runtime = unsafe { &mut *runtime_ptr };
    let did_set = runtime.set_attribute_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        name,
        value,
    );
    if did_set && name.eq_ignore_ascii_case("style") {
        runtime.set_element_inline_style_current_base_url(handle);
    }
    if did_set {
        apply_image_attribute_mutation_plan(scope, runtime_ptr, image_plan);
        if name.eq_ignore_ascii_case("loading") {
            queue_image_load_event_for_loading_change(scope, runtime_ptr, handle);
        }
        queue_media_load_if_source_or_loading_change(scope, runtime_ptr, handle, name);
        queue_text_track_load_if_source(scope, runtime_ptr, handle, name);
    }
    crate::context_bootstrap::reset_html_canvas_backing_store_for_dimension_assignment(
        scope,
        runtime_ptr,
        handle,
        None,
        name,
    );
    did_set
}

pub(in crate::native_bridge) fn set_live_element_attribute_utf16_units_appending_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    value: &str,
    units: Vec<u16>,
) -> bool {
    clear_detached_iframe_context_before_navigation_attribute_change(
        scope,
        runtime_ptr,
        handle,
        name,
        Some(value),
    );
    let image_plan =
        plan_image_attribute_mutation(unsafe { &*runtime_ptr }, handle, name, Some(value));
    let runtime = unsafe { &mut *runtime_ptr };
    let did_set = runtime.set_attribute_utf16_units_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        name,
        value,
        units,
    );
    if did_set && name.eq_ignore_ascii_case("style") {
        runtime.set_element_inline_style_current_base_url(handle);
    }
    if did_set {
        apply_image_attribute_mutation_plan(scope, runtime_ptr, image_plan);
        if name.eq_ignore_ascii_case("loading") {
            queue_image_load_event_for_loading_change(scope, runtime_ptr, handle);
        }
        queue_media_load_if_source_or_loading_change(scope, runtime_ptr, handle, name);
        queue_text_track_load_if_source(scope, runtime_ptr, handle, name);
    }
    crate::context_bootstrap::reset_html_canvas_backing_store_for_dimension_assignment(
        scope,
        runtime_ptr,
        handle,
        None,
        name,
    );
    did_set
}

pub(in crate::native_bridge) fn set_live_element_attribute_ns_appending_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    namespace: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
    qualified_name: &str,
    value: &str,
) -> bool {
    if namespace.is_none() {
        clear_detached_iframe_context_before_navigation_attribute_change(
            scope,
            runtime_ptr,
            handle,
            local_name,
            Some(value),
        );
    }
    let image_plan = if namespace.is_none() {
        plan_image_attribute_mutation(unsafe { &*runtime_ptr }, handle, local_name, Some(value))
    } else {
        Default::default()
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let did_set = runtime.set_attribute_ns_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        namespace,
        prefix,
        local_name,
        qualified_name,
        value,
    );
    if did_set && namespace.is_none() && local_name.eq_ignore_ascii_case("style") {
        runtime.set_element_inline_style_current_base_url(handle);
    }
    if did_set && namespace.is_none() {
        apply_image_attribute_mutation_plan(scope, runtime_ptr, image_plan);
        if local_name.eq_ignore_ascii_case("loading") {
            queue_image_load_event_for_loading_change(scope, runtime_ptr, handle);
        }
        queue_media_load_if_source_or_loading_change(scope, runtime_ptr, handle, local_name);
        queue_text_track_load_if_source(scope, runtime_ptr, handle, local_name);
    }
    crate::context_bootstrap::reset_html_canvas_backing_store_for_dimension_assignment(
        scope,
        runtime_ptr,
        handle,
        namespace,
        local_name,
    );
    did_set
}

pub(in crate::native_bridge) fn remove_live_element_attribute_appending_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
) -> bool {
    clear_detached_iframe_context_before_navigation_attribute_change(
        scope,
        runtime_ptr,
        handle,
        name,
        None,
    );
    let image_plan = plan_image_attribute_mutation(unsafe { &*runtime_ptr }, handle, name, None);
    let runtime = unsafe { &mut *runtime_ptr };
    let did_remove = runtime.remove_attribute_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        name,
    );
    if did_remove {
        crate::context_bootstrap::reset_html_canvas_backing_store_for_dimension_assignment(
            scope,
            runtime_ptr,
            handle,
            None,
            name,
        );
    }
    if did_remove {
        apply_image_attribute_mutation_plan(scope, runtime_ptr, image_plan);
        if name.eq_ignore_ascii_case("loading") {
            queue_image_load_event_for_loading_change(scope, runtime_ptr, handle);
        }
        queue_media_load_if_source_or_loading_change(scope, runtime_ptr, handle, name);
        queue_text_track_load_if_source(scope, runtime_ptr, handle, name);
    }
    did_remove
}

fn clear_detached_iframe_context_before_navigation_attribute_change(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    next_value: Option<&str>,
) {
    if !name.eq_ignore_ascii_case("src") && !name.eq_ignore_ascii_case("srcdoc") {
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    if !iframe_uses_detached_content_cache(runtime, handle) {
        return;
    }
    let current_value = runtime.dom_host().get_attribute(handle, name);
    let changes = match next_value {
        Some(next_value) => current_value.as_deref() != Some(next_value),
        None => current_value.is_some(),
    };
    if changes {
        clear_detached_iframe_cached_context_for_handle(scope, runtime_ptr, handle);
    }
}

pub(in crate::native_bridge) fn remove_live_element_attribute_ns_appending_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    namespace: Option<&str>,
    local_name: &str,
) -> bool {
    if namespace.is_none() {
        clear_detached_iframe_context_before_navigation_attribute_change(
            scope,
            runtime_ptr,
            handle,
            local_name,
            None,
        );
    }
    let image_plan = if namespace.is_none() {
        plan_image_attribute_mutation(unsafe { &*runtime_ptr }, handle, local_name, None)
    } else {
        Default::default()
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let did_remove = runtime.remove_attribute_ns_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        namespace,
        local_name,
    );
    if did_remove {
        crate::context_bootstrap::reset_html_canvas_backing_store_for_dimension_assignment(
            scope,
            runtime_ptr,
            handle,
            namespace,
            local_name,
        );
    }
    if did_remove && namespace.is_none() {
        apply_image_attribute_mutation_plan(scope, runtime_ptr, image_plan);
        if local_name.eq_ignore_ascii_case("loading") {
            queue_image_load_event_for_loading_change(scope, runtime_ptr, handle);
        }
        queue_media_load_if_source_or_loading_change(scope, runtime_ptr, handle, local_name);
        queue_text_track_load_if_source(scope, runtime_ptr, handle, local_name);
    }
    did_remove
}

pub(crate) fn element_attribute_for_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
) -> Option<String> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) else {
        return None;
    };
    element_attribute(unsafe { &*runtime_ptr }, handle, name)
}

fn element_id_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "id") else {
        rv.set_null();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "id").unwrap_or_default();
    set_element_string_return_value(scope, &mut rv, &value);
}

fn element_heading_offset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "headingOffset")
    else {
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .map(|element| element.heading_offset())
        .unwrap_or(0);
    rv.set_uint32(value);
}

fn element_heading_offset_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_setter_receiver(scope, args.this(), "headingOffset")
    else {
        return;
    };
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        args.get(0),
        webidl::Context::member("Element", "headingOffset"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let value = if value <= i32::MAX as u32 { value } else { 0 };
    set_reflected_attribute(
        scope,
        runtime_ptr,
        handle,
        "headingoffset",
        &value.to_string(),
    );
    rv.set_undefined();
}

fn element_heading_reset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "headingReset")
    else {
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.heading_reset());
    rv.set_bool(value);
}

fn element_heading_reset_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_setter_receiver(scope, args.this(), "headingReset")
    else {
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        "headingreset",
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}

fn element_id_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if element_setter_receiver(scope, args.this(), "id").is_none() {
        return;
    }
    set_dom_string_attribute_property_utf16_on_object(scope, args.this(), "id", args.get(0));
    rv.set_undefined();
}

fn element_class_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "className")
    else {
        rv.set_null();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "class").unwrap_or_default();
    set_element_string_return_value(scope, &mut rv, &value);
}

fn element_class_name_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if element_setter_receiver(scope, args.this(), "className").is_none() {
        return;
    }
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "class",
        args.get(0),
        "Element",
        "className",
    );
    rv.set_undefined();
}

fn set_element_string_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    value: &str,
) {
    match v8_string(scope, value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn element_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_getter_receiver(scope, "Element", member);
        return None;
    };
    if !require_element_getter_receiver(scope, unsafe { &*runtime_ptr }, handle, member) {
        return None;
    }
    Some((runtime_ptr, handle))
}

fn element_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_setter_receiver(scope, "Element", member);
        return None;
    };
    if !require_element_setter_receiver(scope, unsafe { &*runtime_ptr }, handle, member) {
        return None;
    }
    Some((runtime_ptr, handle))
}

pub(in crate::native_bridge::element) fn html_element_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    local_name: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_getter_receiver(scope, interface, member);
        return None;
    };
    if !unsafe { &*runtime_ptr }
        .dom_host()
        .is_html_element_named(handle, local_name)
    {
        throw_incompatible_getter_receiver(scope, interface, member);
        return None;
    }
    Some((runtime_ptr, handle))
}

pub(in crate::native_bridge::element) fn html_element_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
    local_name: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_setter_receiver(scope, interface, member);
        return None;
    };
    if !unsafe { &*runtime_ptr }
        .dom_host()
        .is_html_element_named(handle, local_name)
    {
        throw_incompatible_setter_receiver(scope, interface, member);
        return None;
    }
    Some((runtime_ptr, handle))
}

pub(in crate::native_bridge::element) fn html_media_element_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_getter_receiver(scope, "HTMLMediaElement", member);
        return None;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "audio")
        && !runtime.dom_host().is_html_element_named(handle, "video")
    {
        throw_incompatible_getter_receiver(scope, "HTMLMediaElement", member);
        return None;
    }
    Some((runtime_ptr, handle))
}

pub(in crate::native_bridge::element) fn html_media_element_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_setter_receiver(scope, "HTMLMediaElement", member);
        return None;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "audio")
        && !runtime.dom_host().is_html_element_named(handle, "video")
    {
        throw_incompatible_setter_receiver(scope, "HTMLMediaElement", member);
        return None;
    }
    Some((runtime_ptr, handle))
}

pub(in crate::native_bridge::element) fn html_media_element_method_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    method: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_method_receiver(scope, "HTMLMediaElement", method);
        return None;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "audio")
        && !runtime.dom_host().is_html_element_named(handle, "video")
    {
        throw_incompatible_method_receiver(scope, "HTMLMediaElement", method);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn document_element_or_shadow_root_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_getter_receiver(scope, "Element", member);
        return None;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_document(runtime, handle)
        && !node_is_element(runtime, handle)
        && !runtime.dom_host().is_shadow_root(handle)
    {
        throw_incompatible_getter_receiver(scope, "Element", member);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn element_tag_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "tagName") else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(node) = runtime.dom_host().node(handle) else {
        throw_incompatible_getter_receiver(scope, "Element", "tagName");
        rv.set_undefined();
        return;
    };
    let name = element_name_for_owner_document(runtime, handle).unwrap_or_else(|| node.node_name());
    set_element_string_return_value(scope, &mut rv, &name);
}

fn element_local_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "localName")
    else {
        rv.set_undefined();
        return;
    };
    let Some(node) = unsafe { &*runtime_ptr }.dom_host().node(handle) else {
        throw_incompatible_getter_receiver(scope, "Element", "localName");
        rv.set_undefined();
        return;
    };
    let Some(local_name) = node.local_name() else {
        throw_incompatible_getter_receiver(scope, "Element", "localName");
        rv.set_undefined();
        return;
    };
    set_element_string_return_value(scope, &mut rv, local_name);
}

fn element_namespace_uri_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "namespaceURI")
    else {
        rv.set_null();
        return;
    };
    let Some(node) = unsafe { &*runtime_ptr }.dom_host().node(handle) else {
        throw_incompatible_getter_receiver(scope, "Element", "namespaceURI");
        rv.set_null();
        return;
    };
    let Some(namespace) = node.namespace() else {
        rv.set_null();
        return;
    };
    set_element_string_return_value(scope, &mut rv, namespace);
}

fn element_prefix_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "prefix") else {
        rv.set_null();
        return;
    };
    let Some(node) = unsafe { &*runtime_ptr }.dom_host().node(handle) else {
        throw_incompatible_getter_receiver(scope, "Element", "prefix");
        rv.set_null();
        return;
    };
    let Some(prefix) = node.prefix() else {
        rv.set_null();
        return;
    };
    set_element_string_return_value(scope, &mut rv, prefix);
}

fn element_class_list_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "classList")
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    match runtime
        .native_bridge_mut()
        .wrap_class_list(scope, runtime_ptr, handle)
    {
        Some(class_list) => rv.set(class_list.into()),
        None => rv.set_null(),
    }
}

fn element_class_list_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_setter_receiver(scope, args.this(), "classList")
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), "Element", "classList") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "class", &value);
    rv.set_undefined();
}

fn element_part_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_getter_receiver(scope, args.this(), "part") else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    match runtime
        .native_bridge_mut()
        .wrap_part_list(scope, runtime_ptr, handle)
    {
        Some(part_list) => rv.set(part_list.into()),
        None => rv.set_null(),
    }
}

fn element_part_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_setter_receiver(scope, args.this(), "part") else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), "Element", "part") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "part", &value);
    rv.set_undefined();
}

fn element_attributes_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, _handle)) = element_getter_receiver(scope, args.this(), "attributes")
    else {
        rv.set_undefined();
        return;
    };
    let wrapper = super::document::live_named_node_map_wrapper(scope, runtime_ptr, args.this());
    rv.set(wrapper.into());
}

fn element_custom_element_registry_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_element_or_shadow_root_getter_receiver(
        scope,
        args.this(),
        "customElementRegistry",
    ) else {
        rv.set_undefined();
        return;
    };
    match unsafe { &mut *runtime_ptr }.custom_element_registry_value_for_handle(scope, handle) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

pub(crate) fn install_global_event_handler_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let owner = match interface_name {
        "Document" => GlobalEventHandlerOwner::Document,
        "HTMLElement" | "SVGElement" | "MathMLElement" => GlobalEventHandlerOwner::Element,
        _ => return,
    };
    install_global_event_handler_templates_for_owner(scope, template, owner);
}

pub(crate) fn install_element_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    macro_rules! install {
        ($($declaration:ident),+ $(,)?) => {{
            $($declaration::initialize_prototype_template(scope, prototype);)+
        }};
    }

    match interface_name {
        "Element" => {
            install!(
                ElementAriaStringReflectionDeclaration,
                ElementAriaElementReflectionDeclaration,
                ElementPrototypeReflectionDeclaration,
                ElementPrototypeQueryAndAttributeMethodsDeclaration,
                ExtendedElementPrototypeMethodsDeclaration,
                ElementGeometryPrototypeDeclaration,
                ElementStylePrototypeDeclaration,
            );
            install_node_event_handler_template_bindings(
                scope,
                prototype,
                event_handlers::ELEMENT_FULLSCREEN_EVENT_HANDLER_PROPERTIES,
            );
        }
        "Document" => install!(DocumentCustomElementRegistryPrototypeDeclaration),
        "HTMLElement" => install!(
            ElementStylePrototypeDeclaration,
            HtmlOrForeignElementPrototypeDeclaration,
            HtmlElementStandardPrototypeDeclaration,
            HtmlElementActionPrototypeDeclaration,
            HtmlElementPopoverPrototypeDeclaration,
            HtmlElementGeometryPrototypeDeclaration,
        ),
        "SVGElement" => install!(
            ElementStylePrototypeDeclaration,
            HtmlOrForeignElementPrototypeDeclaration,
            SvgElementFocusPrototypeDeclaration,
        ),
        "MathMLElement" => install!(
            ElementStylePrototypeDeclaration,
            HtmlOrForeignElementPrototypeDeclaration,
            MathMlElementFocusPrototypeDeclaration,
        ),
        _ => {}
    }

    if let Some(interface) = HTML_ALIGN_REFLECTION_INTERFACES
        .iter()
        .copied()
        .find(|interface| interface.name() == interface_name)
    {
        install_html_align_template_binding(scope, prototype, interface);
    }
    if HTML_COMPACT_REFLECTION_INTERFACES.contains(&interface_name) {
        install!(HtmlCompactPrototypeDeclaration);
    }
    if let Some(interface) = HTML_NAME_REFLECTION_INTERFACES
        .iter()
        .copied()
        .find(|interface| interface.name() == interface_name)
    {
        install_html_name_template_binding(scope, prototype, interface);
    }
    if HTML_FORM_OWNER_REFLECTION_INTERFACES
        .iter()
        .any(|interface| interface.name() == interface_name)
    {
        install!(HtmlFormOwnerPrototypeDeclaration);
    }
    if matches!(
        interface_name,
        "HTMLButtonElement"
            | "HTMLInputElement"
            | "HTMLMeterElement"
            | "HTMLOutputElement"
            | "HTMLProgressElement"
            | "HTMLSelectElement"
            | "HTMLTextAreaElement"
    ) {
        install!(LabelableElementPrototypeDeclaration);
    }
    if matches!(interface_name, "HTMLInputElement" | "HTMLTextAreaElement") {
        install!(
            TextControlPrototypeMethodsDeclaration,
            TextControlSelectionPrototypeDeclaration,
        );
    }
    if matches!(
        interface_name,
        "HTMLButtonElement"
            | "HTMLFieldSetElement"
            | "HTMLInputElement"
            | "HTMLObjectElement"
            | "HTMLOutputElement"
            | "HTMLSelectElement"
            | "HTMLTextAreaElement"
    ) {
        install!(
            FormControlValidationPrototypeAccessorsDeclaration,
            FormControlValidationPrototypeMethodsDeclaration,
        );
    }

    match interface_name {
        "HTMLLIElement" => install!(HtmlLiElementValuePrototypeDeclaration),
        "HTMLOListElement" => install!(HtmlOListElementPrototypeDeclaration),
        "HTMLUListElement" => install!(HtmlUListElementPrototypeDeclaration),
        "HTMLBodyElement" => {
            install_body_or_frameset_window_event_handler_accessors(scope, prototype);
            install!(HtmlBodyElementLegacyPrototypeDeclaration);
        }
        "HTMLFrameSetElement" => {
            install_body_or_frameset_window_event_handler_accessors(scope, prototype);
            install!(HtmlFrameSetElementLegacyPrototypeDeclaration);
        }
        "HTMLHRElement" => install!(HtmlHrElementLegacyPrototypeDeclaration),
        "HTMLFontElement" => install!(HtmlFontElementLegacyPrototypeDeclaration),
        "HTMLMarqueeElement" => install!(HtmlMarqueeElementLegacyPrototypeDeclaration),
        "HTMLScriptElement" => install!(HtmlScriptElementPrototypeDeclaration),
        "SVGScriptElement" => install!(SvgScriptElementPrototypeDeclaration),
        "SVGImageElement" => install!(SvgImageElementPrototypeDeclaration),
        "SVGAElement" => install!(SvgAElementRelListPrototypeDeclaration),
        "HTMLStyleElement" => install!(HtmlStyleElementPrototypeDeclaration),
        "SVGStyleElement" => install!(SvgStyleElementPrototypeDeclaration),
        "HTMLTableElement" => install!(HtmlTableElementPrototypeDeclaration),
        "HTMLHtmlElement" => install!(HtmlHtmlElementPrototypeDeclaration),
        "HTMLAnchorElement" => {
            install!(
                HtmlAnchorElementUrlPrototypeDeclaration,
                HtmlAnchorElementTargetPrototypeDeclaration,
                HtmlAnchorElementPrototypeDeclaration,
            );
            install_html_rel_template_bindings(
                scope,
                prototype,
                ElementReflectionInterface::HtmlAnchorElement,
            );
        }
        "HTMLAreaElement" => {
            install!(
                HtmlAreaElementUrlPrototypeDeclaration,
                HtmlAreaElementTargetPrototypeDeclaration,
                HtmlAreaElementReferrerPolicyPrototypeDeclaration,
                HtmlAreaElementPrototypeDeclaration,
            );
            install_html_rel_template_bindings(
                scope,
                prototype,
                ElementReflectionInterface::HtmlAreaElement,
            );
        }
        "HTMLFrameElement" => install!(HtmlFrameElementLegacyPrototypeDeclaration),
        "HTMLIFrameElement" => install!(HtmlIFrameElementPrototypeDeclaration),
        "HTMLSourceElement" => install!(HtmlSourceElementUrlPrototypeDeclaration),
        "HTMLEmbedElement" => install!(HtmlEmbedElementUrlPrototypeDeclaration),
        "HTMLBaseElement" => install!(
            HtmlBaseElementUrlPrototypeDeclaration,
            HtmlBaseElementTargetPrototypeDeclaration,
        ),
        "HTMLLinkElement" => {
            install!(
                HtmlLinkElementUrlPrototypeDeclaration,
                HtmlLinkElementTargetPrototypeDeclaration,
            );
            install_html_rel_template_bindings(
                scope,
                prototype,
                ElementReflectionInterface::HtmlLinkElement,
            );
        }
        "HTMLMetaElement" => install!(
            HtmlMetaElementPrototypeDeclaration,
            HtmlMetaElementMediaPrototypeDeclaration,
        ),
        "HTMLFieldSetElement" => install!(HtmlFieldSetElementPrototypeDeclaration),
        "HTMLDataListElement" => install!(HtmlDataListElementPrototypeDeclaration),
        "HTMLLegendElement" => install!(HtmlLegendElementPrototypeDeclaration),
        "HTMLMeterElement" => install!(HtmlMeterElementPrototypeDeclaration),
        "HTMLProgressElement" => install!(HtmlProgressElementPrototypeDeclaration),
        "HTMLButtonElement" => install!(HtmlButtonElementValuePrototypeDeclaration),
        "HTMLInputElement" => install!(
            HtmlInputElementValuePrototypeDeclaration,
            HtmlInputElementPrototypeMethodsDeclaration,
        ),
        "HTMLOutputElement" => install!(HtmlOutputElementValuePrototypeDeclaration),
        "HTMLTextAreaElement" => install!(
            HtmlTextAreaElementValuePrototypeDeclaration,
            HtmlTextAreaElementPrototypeDeclaration,
        ),
        "HTMLTitleElement" => install!(HtmlTitleElementTextPrototypeDeclaration),
        "HTMLDetailsElement" => install!(HtmlDetailsElementPrototypeDeclaration),
        "HTMLDialogElement" => install!(HtmlDialogElementPrototypeDeclaration),
        "HTMLQuoteElement" => install!(HtmlQuoteElementPrototypeDeclaration),
        "HTMLModElement" => install!(HtmlModElementPrototypeDeclaration),
        "HTMLTimeElement" => install!(HtmlTimeElementPrototypeDeclaration),
        "HTMLPreElement" => install!(HtmlPreElementPrototypeDeclaration),
        "HTMLBRElement" => install!(HtmlBrElementPrototypeDeclaration),
        "HTMLOptGroupElement" => install!(
            HtmlOptGroupElementDisabledPrototypeDeclaration,
            HtmlOptGroupElementLabelPrototypeDeclaration,
        ),
        "HTMLOptionElement" => install!(
            HtmlOptionElementValuePrototypeDeclaration,
            HtmlOptionElementStatePrototypeDeclaration,
            HtmlOptionElementLabelPrototypeDeclaration,
            HtmlOptionElementTextPrototypeDeclaration,
        ),
        "HTMLDataElement" => install!(HtmlDataElementValuePrototypeDeclaration),
        "HTMLParamElement" => install!(HtmlParamElementPrototypeDeclaration),
        "HTMLObjectElement" => install!(HtmlObjectElementPrototypeDeclaration),
        "HTMLLabelElement" => install!(HtmlLabelElementPrototypeDeclaration),
        "HTMLFormElement" => {
            install_html_form_element_prototype_bindings(scope, prototype);
        }
        "HTMLMediaElement" => install!(
            HtmlMediaElementPrototypeDeclaration,
            HtmlMediaElementPrototypeMethodsDeclaration,
        ),
        "HTMLVideoElement" => install!(HtmlVideoElementPrototypeDeclaration),
        "HTMLImageElement" => install!(
            HtmlImageElementUrlPrototypeDeclaration,
            HtmlImageElementPrototypeMethodsDeclaration,
        ),
        "HTMLSelectElement" => install!(HtmlSelectElementPrototypeMethodsDeclaration),
        "HTMLTableSectionElement" => install!(
            HtmlTableSectionElementPrototypeDeclaration,
            HtmlTableSectionElementPrototypeMethodsDeclaration,
            HtmlTableSectionElementLegacyPrototypeDeclaration,
        ),
        "HTMLTableRowElement" => install!(
            HtmlTableRowElementPrototypeDeclaration,
            HtmlTableRowElementPrototypeMethodsDeclaration,
            HtmlTableRowElementLegacyPrototypeDeclaration,
        ),
        "HTMLTableColElement" => install!(HtmlTableColElementLegacyPrototypeDeclaration),
        "HTMLTableCellElement" => install!(
            HtmlTableCellElementPrototypeDeclaration,
            HtmlTableCellElementLegacyPrototypeDeclaration,
        ),
        "ShadowRoot" => install!(ShadowRootPrototypeReflectionDeclaration),
        "Text" => install!(TextPrototypeReflectionDeclaration),
        "HTMLTrackElement" => install!(HtmlTrackElementPrototypeDeclaration),
        "HTMLSlotElement" => install!(HtmlSlotElementPrototypeDeclaration),
        "HTMLTemplateElement" => install!(HtmlTemplateElementPrototypeDeclaration),
        _ => {}
    }
}

fn shadow_root_get_selection_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this()) else {
        let Ok((runtime_ptr, handle)) =
            node_runtime_and_handle_from_object_or_detached(scope, args.this())
        else {
            rv.set(v8::null(scope).into());
            return;
        };
        if !unsafe { &*runtime_ptr }.dom_host().is_shadow_root(handle) {
            rv.set(v8::null(scope).into());
            return;
        }
        match detached_shadow_root_selection_value(scope, args.this()) {
            Some(selection) => rv.set(selection),
            None => rv.set(v8::null(scope).into()),
        }
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if !runtime.dom_host().is_shadow_root(handle) {
        rv.set(v8::null(scope).into());
        return;
    }
    let Some(document_handle) = runtime.dom_host().owner_document_handle(handle) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(document) =
        runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, document_handle)
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(default_view) = document.get(scope, v8str(scope, "defaultView").into()) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Ok(window) = v8::Local::<v8::Object>::try_from(default_view) else {
        rv.set(v8::null(scope).into());
        return;
    };
    match selection_value_for_window(scope, window) {
        Some(selection) => rv.set(selection.into()),
        None => rv.set(v8::null(scope).into()),
    }
}

fn aria_attribute_name_from_data(
    scope: &mut v8::PinScope<'_, '_>,
    data: v8::Local<'_, v8::Value>,
) -> Option<String> {
    Some(data.to_string(scope)?.to_rust_string_lossy(scope))
}

fn aria_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(attribute) = aria_attribute_name_from_data(scope, args.data()) else {
        return;
    };
    nullable_attribute_property_getter_from_object_or_detached(scope, args.this(), &attribute, rv);
}

fn aria_string_attribute_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = aria_attribute_name_from_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    set_nullable_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        &attribute,
        args.get(0),
        "Element",
        "ARIA reflection",
    );
    rv.set_undefined();
}

fn aria_element_reference_array_cache_slot(attribute: &str) -> String {
    format!("__moliAriaElementReferenceArrayCache:{attribute}")
}

fn aria_element_reference_is_singular(attribute: &str) -> bool {
    attribute == "aria-activedescendant"
}

struct AriaElementReferenceValue<'s> {
    object: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
}

impl<'s> webidl::WebIdlConverter<'s> for AriaElementReferenceValue<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        _context: webidl::Context,
        _options: &Self::Options,
    ) -> std::result::Result<Self, webidl::WebIdlError> {
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| {
            webidl::WebIdlError::custom_message("ARIA element references must be Elements.")
        })?;
        let (runtime_ptr, handle) = node_runtime_and_handle_from_object_or_detached(scope, object)
            .map_err(|_| {
                webidl::WebIdlError::custom_message("ARIA element references must be Elements.")
            })?;
        if !node_is_element(unsafe { &*runtime_ptr }, handle) {
            return Err(webidl::WebIdlError::custom_message(
                "ARIA element references must be Elements.",
            ));
        }
        Ok(Self {
            object,
            runtime_ptr,
            handle,
        })
    }
}

fn aria_element_reference_handle_for_owner(
    scope: &mut v8::PinScope<'_, '_>,
    owner_runtime_ptr: *mut JsContextHost,
    reference: AriaElementReferenceValue<'_>,
) -> Option<DomHandle> {
    current_or_live_delegate_node_arg_handle(scope, owner_runtime_ptr, reference.object.into())
        .filter(|handle| node_is_element(unsafe { &*owner_runtime_ptr }, *handle))
        .or_else(|| (reference.runtime_ptr == owner_runtime_ptr).then_some(reference.handle))
}

fn element_reference_is_in_valid_scope(
    runtime: &JsContextHost,
    owner: DomHandle,
    candidate: DomHandle,
) -> bool {
    if !node_is_element(runtime, candidate) {
        return false;
    }
    let Some(candidate_root) = runtime.dom_host().root_node_handle(candidate) else {
        return false;
    };
    let Some(mut owner_root) = runtime.dom_host().root_node_handle(owner) else {
        return false;
    };
    loop {
        if candidate_root == owner_root {
            return true;
        }
        if !runtime.dom_host().is_shadow_root(owner_root) {
            return false;
        }
        let Some(host) = runtime.dom_host().shadow_root_host(owner_root) else {
            return false;
        };
        let Some(next_root) = runtime.dom_host().root_node_handle(host) else {
            return false;
        };
        owner_root = next_root;
    }
}

fn element_by_id_including_disconnected(
    runtime: &JsContextHost,
    owner: DomHandle,
    id: &str,
) -> Option<DomHandle> {
    if id.is_empty() {
        return None;
    }
    let root = runtime.dom_host().root_node_handle(owner)?;
    let mut stack = runtime
        .dom_host()
        .child_handles_reversed(root)
        .collect::<Vec<_>>();
    while let Some(candidate) = stack.pop() {
        if node_is_element(runtime, candidate)
            && runtime.dom_host().get_attribute(candidate, "id").as_deref() == Some(id)
        {
            return Some(candidate);
        }
        stack.extend(runtime.dom_host().child_handles_reversed(candidate));
    }
    None
}

pub(in crate::native_bridge::element) fn reflected_element_attribute_handle(
    runtime: &JsContextHost,
    owner: DomHandle,
    attribute: &str,
) -> Option<DomHandle> {
    let candidate = match runtime
        .dom_host()
        .explicit_element_references(owner, attribute)
    {
        Some(references) => references.into_iter().next()?,
        None => {
            let id = runtime.dom_host().get_attribute(owner, attribute)?;
            element_by_id_including_disconnected(runtime, owner, &id)?
        }
    };
    if !element_reference_is_in_valid_scope(runtime, owner, candidate) {
        return None;
    }
    runtime
        .dom_host()
        .resolve_reference_target_chain(candidate)
        .map(|_| candidate)
}

pub(in crate::native_bridge::element) fn resolved_reflected_element_attribute_handle(
    runtime: &JsContextHost,
    owner: DomHandle,
    attribute: &str,
) -> Option<DomHandle> {
    reflected_element_attribute_handle(runtime, owner, attribute)
        .and_then(|candidate| runtime.dom_host().resolve_reference_target_chain(candidate))
}

fn aria_element_reference_content_handles(
    runtime: &JsContextHost,
    owner: DomHandle,
    attribute: &str,
) -> Option<Vec<DomHandle>> {
    let value = runtime.dom_host().get_attribute(owner, attribute)?;
    if aria_element_reference_is_singular(attribute) {
        return Some(
            element_by_id_including_disconnected(runtime, owner, &value)
                .into_iter()
                .collect(),
        );
    }
    Some(
        value
            .split([' ', '\t', '\n', '\r', '\u{000c}'])
            .filter(|token| !token.is_empty())
            .filter_map(|token| element_by_id_including_disconnected(runtime, owner, token))
            .collect(),
    )
}

fn aria_element_reference_handles(
    runtime: &JsContextHost,
    owner: DomHandle,
    attribute: &str,
) -> Option<Vec<DomHandle>> {
    match runtime
        .dom_host()
        .explicit_element_references(owner, attribute)
    {
        Some(references) => Some(
            references
                .into_iter()
                .filter(|candidate| element_reference_is_in_valid_scope(runtime, owner, *candidate))
                .collect(),
        ),
        None => aria_element_reference_content_handles(runtime, owner, attribute),
    }
}

fn aria_element_reference_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Value>> {
    wrapped_handle_value(scope, runtime_ptr, handle)
}

fn aria_element_reference_array_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handles: Vec<DomHandle>,
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
    handles
        .into_iter()
        .map(|handle| aria_element_reference_value(scope, runtime_ptr, handle))
        .collect()
}

fn aria_cached_frozen_element_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &str,
    values: &[v8::Local<'s, v8::Value>],
) -> v8::Local<'s, v8::Array> {
    let slot = aria_element_reference_array_cache_slot(attribute);
    if let Some(cached) = get_private_value(scope, receiver, &slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        && cached.length() as usize == values.len()
    {
        let mut equal = true;
        for (index, expected) in values.iter().copied().enumerate() {
            if !cached
                .get_index(scope, index as u32)
                .is_some_and(|actual| actual.strict_equals(expected))
            {
                equal = false;
                break;
            }
        }
        if equal {
            return cached;
        }
    }
    let array = v8::Array::new_with_elements(scope, values);
    let _ = array.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    set_private_value(scope, receiver, &slot, array.into());
    array
}

fn clear_aria_element_reference_array_cache(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
    attribute: &str,
) {
    let slot = aria_element_reference_array_cache_slot(attribute);
    let undefined = v8::undefined(scope);
    set_private_value(scope, receiver, &slot, undefined.into());
}

fn aria_element_reference_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = aria_attribute_name_from_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let Some((runtime_ptr, owner)) = element_getter_receiver(scope, args.this(), &attribute) else {
        return;
    };
    let Some(handles) = aria_element_reference_handles(unsafe { &*runtime_ptr }, owner, &attribute)
    else {
        clear_aria_element_reference_array_cache(scope, args.this(), &attribute);
        rv.set_null();
        return;
    };
    if aria_element_reference_is_singular(&attribute) {
        match handles
            .into_iter()
            .next()
            .and_then(|handle| aria_element_reference_value(scope, runtime_ptr, handle))
        {
            Some(value) => rv.set(value),
            None => rv.set_null(),
        }
        return;
    }
    let Some(values) = aria_element_reference_array_values(scope, runtime_ptr, handles) else {
        rv.set_null();
        return;
    };
    let array = aria_cached_frozen_element_array(scope, args.this(), &attribute, &values);
    rv.set(array.into());
}

fn set_explicit_aria_element_references(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    owner: DomHandle,
    attribute: &str,
    handles: Vec<DomHandle>,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = unsafe { &mut *runtime_ptr }.set_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            owner,
            attribute,
            "",
        );
        let _ = unsafe { &mut *runtime_ptr }
            .dom_host_mut()
            .set_explicit_element_references(owner, attribute, handles);
    });
}

fn clear_explicit_aria_element_references(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    owner: DomHandle,
    attribute: &str,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = unsafe { &mut *runtime_ptr }.remove_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            owner,
            attribute,
        );
    });
}

fn converted_aria_element_reference<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<AriaElementReferenceValue<'s>> {
    match webidl::convert::<AriaElementReferenceValue<'s>>(
        scope,
        value,
        webidl::Context::argument("Element ARIA element reflection", 1),
    ) {
        Ok(reference) => Some(reference),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn aria_element_reference_attribute_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = aria_attribute_name_from_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some((runtime_ptr, owner)) = element_setter_receiver(scope, args.this(), &attribute) else {
        return;
    };
    let value = args.get(0);
    if value.is_null_or_undefined() {
        clear_explicit_aria_element_references(scope, runtime_ptr, owner, &attribute);
        rv.set_undefined();
        return;
    }
    if aria_element_reference_is_singular(&attribute) {
        let Some(reference) = converted_aria_element_reference(scope, value) else {
            return;
        };
        let handles = aria_element_reference_handle_for_owner(scope, runtime_ptr, reference)
            .into_iter()
            .collect();
        set_explicit_aria_element_references(scope, runtime_ptr, owner, &attribute, handles);
        rv.set_undefined();
        return;
    }
    let sequence = match webidl::convert::<webidl::Sequence<AriaElementReferenceValue<'s>>>(
        scope,
        value,
        webidl::Context::argument("Element ARIA element reflection", 1),
    ) {
        Ok(sequence) => sequence,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let mut handles = Vec::with_capacity(sequence.0.len());
    for reference in sequence.0 {
        if let Some(handle) = aria_element_reference_handle_for_owner(scope, runtime_ptr, reference)
            && !handles.contains(&handle)
        {
            handles.push(handle);
        }
    }
    set_explicit_aria_element_references(scope, runtime_ptr, owner, &attribute, handles);
    rv.set_undefined();
}

pub(crate) use geometry::read_client_rects;

pub(in crate::native_bridge) use forms::{PlannedFormNavigation, apply_planned_form_navigation};
