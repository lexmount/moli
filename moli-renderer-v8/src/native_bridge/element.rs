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
use super::JsContextHost;
use super::node::{
    element_name_for_owner_document, node_is_document, node_is_element,
    node_runtime_and_handle_from_object, node_runtime_and_handle_from_object_or_detached,
    node_text_content_getter_function, require_element_getter_receiver,
    require_element_setter_receiver, set_text_content_in_reaction_scope,
    throw_incompatible_getter_receiver, throw_incompatible_method_receiver,
    throw_incompatible_setter_receiver,
};
use crate::document_runtime::DomHandle;
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
use trusted_types::{
    TrustedScriptElementSink, trusted_script_element_sink_string, trusted_script_url_sink_string,
};

pub(crate) use forms::{
    autocomplete_field_name, autofill_related_form_control_elements, form_associated_form_owner,
    form_control_elements, form_data_control_elements, is_valid_submit_button,
};
#[cfg(test)]
pub(crate) use styles::iframe_width_attribute_viewport_width;
pub(crate) use styles::{
    ComputedStyleReadScope, ComputedStyleTargetContext,
    STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT, STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT,
    STYLE_DECLARATION_READ_DOCUMENT_SLOT, STYLE_DECLARATION_SCREEN_HEIGHT_SLOT,
    STYLE_DECLARATION_SCREEN_WIDTH_SLOT, STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT,
    STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT, STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT,
    STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT, computed_style_target_context, iframe_handle_viewport,
    marker_pseudo_element_is_generated_for_document_snapshot,
};
mod stylesheets;
mod template_install;
mod tree_mutation;
mod url_attributes;

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
    StyleMode, active_css_animation_transform_value,
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
use activation::navigate_form_target_browsing_context;
pub(crate) use activation::{
    SpecialBrowsingContextTarget, navigate_existing_browsing_context_target,
    navigate_named_iframe_target,
};
pub(crate) use activation::{
    activate_handle_via_click, activate_handle_via_click_with_detail_and_modifiers,
    activate_handle_via_synthetic_click, dispatched_click_activation_target,
    finish_legacy_activation_for_dispatched_click,
    perform_click_default_action_for_dispatched_event, perform_drop_default_action,
    prepare_legacy_activation_for_dispatched_click, replace_contenteditable_selection,
    scroll_to_url_fragment_or_top, select_contenteditable_contents,
};
pub(super) use activation::{input_show_picker_callback, node_click_callback};
use activation::{
    queue_deferred_named_iframe_target_navigation_from_document,
    queue_deferred_named_iframe_target_request,
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
    canvas_get_context_callback, canvas_to_data_url_callback,
    canvas_transfer_control_to_offscreen_callback,
};
pub(crate) use canvas::{
    html_canvas_height_getter_callback, html_canvas_height_setter_callback,
    html_canvas_width_getter_callback, html_canvas_width_setter_callback,
};
pub(crate) use class_list::install_dom_token_list_prototype_bindings;
pub(super) use class_list::{
    build_dom_token_list_wrapper_template, html_rel_list_getter_function,
    html_rel_list_setter_function,
};
pub(super) use content::{
    node_direct_text_content, node_get_html_callback, node_inner_html_getter_function,
    node_inner_html_setter_function, node_inner_text_getter_function,
    node_inner_text_setter_function, node_outer_html_getter_function,
    node_outer_html_setter_function, node_outer_text_getter_function,
    node_outer_text_setter_function, node_set_html_unsafe_callback, title_text_getter_function,
    title_text_setter_function,
};
pub(super) use dataset::{build_dom_string_map_wrapper_template, node_dataset_getter_function};
use details_dialog::{
    close_dialog_element, details_open_getter_function, details_open_setter_function,
    dialog_close_callback, dialog_open_getter_function, dialog_open_setter_function,
    dialog_return_value_getter_function, dialog_return_value_setter_function, dialog_show_callback,
    dialog_show_modal_callback, perform_summary_click_default_action,
};
pub(crate) use details_dialog::{
    queue_details_toggle_event_for_attribute_change, queue_parser_details_toggle_event,
    queue_parser_details_toggle_events_in_subtree,
};
pub(crate) use event_handlers::compile_window_body_onmessageerror_attribute;
use event_handlers::install_global_event_handler_template_bindings as install_global_event_handler_templates_for_owner;
pub(crate) use event_handlers::{
    EventAttributeHandlerScope, GlobalEventHandlerOwner, compile_event_attribute_handler_for_owner,
    initialize_parser_inserted_body_window_event_handlers,
};
use event_handlers::{
    body_onerror_getter_function, body_onerror_setter_function, body_onload_getter_function,
    body_onload_setter_function, body_onmessageerror_getter_function,
    body_onmessageerror_setter_function,
};
pub(in crate::native_bridge::element) use events::construct_event;
pub(crate) use events::{
    NodePublicEventDispatchOutcome, TouchEventPoint, construct_command_event, construct_drag_event,
    construct_interest_event, construct_keyboard_event,
    construct_mouse_event_with_detail_and_modifiers, construct_mouse_event_with_modifiers,
    construct_mouse_event_with_related_target_and_modifiers, construct_pointer_event,
    construct_pointer_event_with_modifiers, construct_pointer_event_with_related_target,
    construct_pointer_event_with_related_target_and_modifiers, construct_simple_event,
    construct_submit_event, construct_toggle_event, construct_touch_event,
    construct_touch_event_with_points, construct_wheel_event, dispatch_public_event,
};
use events::{
    construct_click_event, construct_click_event_with_detail_and_modifiers, construct_focus_event,
};
pub(crate) use focus::{
    contenteditable_editing_host, focus_element, focus_live_element_for_inspector,
    perform_access_key_default_action_for_dispatched_event,
    perform_hover_interest_default_action_for_dispatched_event, perform_mouse_focus_default_action,
    perform_tab_focus_default_action_for_dispatched_event, post_parse_autofocus_is_pending,
    process_post_parse_autofocus, reset_focus_from_previous_handle,
    reset_focus_from_previous_handle_with_previous_focus_within, schedule_focus_blur_if_needed,
    update_focus,
};
use focus::{is_disabled_form_control, is_focusable};
pub(super) use focus::{node_blur_callback, node_focus_callback};
pub(in crate::native_bridge) use forms::{
    FormAssociatedResetCallbackTiming, reset_form_default_action, resize_select_options,
    select_add_insertion_point, select_options_resize_target,
};
pub(crate) use forms::{
    align_event_constructor_function_realm_with_constructor,
    align_event_constructor_function_realm_with_target,
};
pub(super) use forms::{
    button_command_for_element_getter_function, button_command_for_element_setter_function,
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
    button_value_setter_function, control_check_validity_callback, control_label_handles,
    control_labels_getter_function, control_report_validity_callback,
    control_set_custom_validity_callback, control_validation_message_getter_function,
    control_validity_getter_function, control_will_validate_getter_function,
    datalist_options_getter_function, fieldset_disabled_getter_function,
    fieldset_disabled_setter_function, fieldset_elements_getter_function,
    fieldset_type_getter_function, form_accept_charset_getter_function,
    form_accept_charset_setter_function, form_action_getter_function, form_action_setter_function,
    form_associated_form_getter_function, form_autocomplete_getter_function,
    form_autocomplete_setter_function, form_check_validity_callback, form_elements_getter_function,
    form_encoding_getter_function, form_encoding_setter_function, form_enctype_getter_function,
    form_enctype_setter_function, form_length_getter_function, form_method_getter_function,
    form_method_setter_function, form_name_getter_function, form_name_setter_function,
    form_no_validate_getter_function, form_no_validate_setter_function,
    form_report_validity_callback, form_request_submit_callback, form_reset_callback,
    form_submit_callback, form_target_getter_function, form_target_setter_function,
    input_accept_getter_function, input_accept_setter_function, input_alt_getter_function,
    input_alt_setter_function, input_autocomplete_getter_function,
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
    set_select_indexed_option, submit_form_with_submit_event, text_control_select_callback,
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
};
pub(crate) use forms::{
    char_offset_to_byte_index, dispatch_text_control_event, is_text_control,
    queue_text_control_document_selection_change_event, replace_text_control_selection,
    text_control_set_selection_range_internal,
    text_control_set_selection_range_with_direction_internal, text_control_value,
};
use rendered_state::node_check_visibility_callback;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLFormElement", enumerable)]
struct HtmlFormElementTemplateMethodsDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),

    #[webapi(method = "requestSubmit", length = 1, callback = form_request_submit_callback)]
    request_submit: (),
    #[webapi(method, length = 0, callback = form_submit_callback)]
    submit: (),
    #[webapi(method, length = 0, callback = form_reset_callback)]
    reset: (),
    #[webapi(method = "checkValidity", length = 0, callback = form_check_validity_callback)]
    check_validity: (),
    #[webapi(method = "reportValidity", length = 0, callback = form_report_validity_callback)]
    report_validity: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLSelectElement")]
struct HtmlSelectElementIndexedPropertiesDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

pub(crate) fn install_html_form_element_prototype_bindings(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<'_, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    HtmlFormElementTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(crate) fn install_html_select_element_prototype_bindings(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<'_, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    HtmlSelectElementIndexedPropertiesDeclaration::initialize_prototype_template(scope, proto);
}

pub use geometry::ClientRect;
pub(crate) use geometry::{
    apply_scroll_observable_effects, observable_bounding_client_rect, observable_caret_position,
    observable_deep_hit_test, observable_event_offset, observable_geometry_batch,
    observable_hit_test_all, observable_input_hit_test, observable_scroll_adjusted_client_rect,
    observable_sources_with_fragments, perform_wheel_scroll_default_action,
    queue_scroll_observable_effects, scroll_node_into_view_at_start,
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
pub(super) use global_attributes::{
    anchor_target_getter_function, anchor_target_setter_function, area_no_href_setter_function,
    area_target_getter_function, area_target_setter_function, base_target_getter_function,
    base_target_setter_function, canonical_cross_origin_value, canonical_loading_value,
    canonical_preload_value, canonical_referrer_policy_value,
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
    html_no_href_getter_function, html_no_shade_getter_function, html_no_shade_setter_function,
    html_ping_getter_function, html_rel_getter_function, html_rel_setter_function,
    html_scrolling_getter_function, html_shape_getter_function, html_size_getter_function,
    html_sizes_getter_function, html_type_getter_function, html_use_map_getter_function,
    html_value_getter_function, html_value_type_getter_function, html_version_getter_function,
    html_vspace_getter_function, html_width_getter_function, image_decoding_setter_function,
    image_long_desc_setter_function, image_lowsrc_setter_function, link_target_getter_function,
    link_target_setter_function, node_access_key_getter_function,
    node_access_key_label_getter_function, node_access_key_setter_function,
    node_allow_fullscreen_getter_function, node_allow_fullscreen_setter_function,
    node_autocapitalize_getter_function, node_autocapitalize_setter_function,
    node_autofocus_getter_function, node_autofocus_setter_function,
    node_content_editable_getter_function, node_content_editable_setter_function,
    node_credentialless_getter_function, node_credentialless_setter_function,
    node_dir_getter_function, node_dir_setter_function, node_draggable_getter_function,
    node_draggable_setter_function, node_enter_key_hint_getter_function,
    node_enter_key_hint_setter_function, node_hidden_getter_function, node_hidden_setter_function,
    node_input_mode_getter_function, node_input_mode_setter_function,
    node_is_content_editable_getter_function, node_lang_getter_function, node_lang_setter_function,
    node_sandbox_getter_function, node_sandbox_setter_function, node_spellcheck_getter_function,
    node_spellcheck_setter_function, node_tab_index_getter_function,
    node_tab_index_setter_function, node_title_getter_function, node_title_setter_function,
    node_translate_getter_function, node_translate_setter_function,
    null_to_empty_dom_string_reflection_setter_function, object_archive_getter_function,
    object_code_base_getter_function, object_code_getter_function,
    object_code_type_getter_function, object_data_getter_function, object_declare_getter_function,
    object_declare_setter_function, object_standby_getter_function, pre_width_getter_function,
    pre_width_setter_function, source_height_getter_function, source_height_setter_function,
    source_width_getter_function, source_width_setter_function, table_cell_abbr_getter_function,
    table_cell_axis_getter_function, table_cell_headers_getter_function,
    table_cell_no_wrap_getter_function, table_cell_no_wrap_setter_function,
    table_cell_scope_getter_function, table_ch_getter_function, table_ch_off_getter_function,
    table_col_span_getter_function, table_col_span_setter_function, table_v_align_getter_function,
    unsigned_long_reflection_setter_function, usv_string_reflection_setter_function,
};
pub(in crate::native_bridge) const BODY_LEGACY_PROTOTYPE_ACCESSORS: &[&str] = &[
    "onload",
    "onmessageerror",
    "text",
    "link",
    "vLink",
    "aLink",
    "background",
];

use html_elements::{
    body_a_link_getter_function, body_a_link_setter_function, body_background_getter_function,
    body_background_setter_function, body_link_getter_function, body_link_setter_function,
    body_text_getter_function, body_text_setter_function, body_v_link_getter_function,
    body_v_link_setter_function, li_value_getter_function, li_value_setter_function,
    meta_content_getter_function, meta_content_setter_function, meta_http_equiv_getter_function,
    meta_http_equiv_setter_function, ol_reversed_getter_function, ol_reversed_setter_function,
    ol_start_getter_function, ol_start_setter_function, ol_type_getter_function,
    ol_type_setter_function, optgroup_disabled_getter_function, optgroup_disabled_setter_function,
    table_caption_getter_function, table_caption_setter_function,
    table_cell_col_span_getter_function, table_cell_col_span_setter_function,
    table_cell_index_getter_function, table_cell_row_span_getter_function,
    table_cell_row_span_setter_function, table_create_caption_callback,
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
    media_autoplay_getter_function, media_autoplay_setter_function, media_can_play_type_callback,
    media_controls_getter_function, media_controls_setter_function,
    media_cross_origin_getter_function, media_cross_origin_setter_function,
    media_current_time_getter_function, media_current_time_setter_function,
    media_default_muted_getter_function, media_default_muted_setter_function,
    media_duration_getter_function, media_ended_getter_function, media_height_getter_function,
    media_height_setter_function, media_load_callback, media_loading_getter_function,
    media_loading_setter_function, media_loop_getter_function, media_loop_setter_function,
    media_muted_getter_function, media_muted_setter_function, media_network_state_getter_function,
    media_pause_callback, media_paused_getter_function, media_play_callback,
    media_playback_rate_getter_function, media_playback_rate_setter_function,
    media_plays_inline_getter_function, media_plays_inline_setter_function,
    media_poster_getter_function, media_poster_setter_function, media_preload_getter_function,
    media_preload_setter_function, media_ready_state_getter_function,
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
pub(crate) use popover::{dispatch_popover_removal_events, perform_popover_invoker_default_action};
pub(super) use popover::{dispatch_popover_show_events, dispatch_popover_toggle_events};
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
use reflection::{
    CrossOriginReflection, DomStringReflection, ElementReflectionInterface,
    NullToEmptyDomStringReflection, UnsignedLongReflection, UsvStringReflection,
    attribute_property_getter_from_object_or_detached,
    boolean_attribute_property_getter_from_object_or_detached,
    nullable_attribute_property_getter_from_object_or_detached, parse_non_negative_dimension,
    property_dom_string_value, property_string_value, property_usv_string_value,
    remove_reflected_attribute, set_attribute_property_on_object_or_detached,
    set_boolean_attribute_property_on_object_or_detached,
    set_dom_string_attribute_property_on_object,
    set_nullable_dom_string_attribute_property_on_object, set_reflected_attribute,
    set_reflected_boolean_attribute, set_reflected_style_attribute_with_inline_base_url,
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
use shared::{element_attribute, element_attribute_names, element_has_attribute, style_string};
pub(super) use state_callbacks::{
    bridge_set_checked_state_callback, bridge_set_indeterminate_state_callback,
    bridge_set_input_value_callback, bridge_set_selected_state_callback,
};
pub(super) use styles::{
    build_style_wrapper_template, node_style_getter_function, node_style_setter_function,
};
pub(crate) use styles::{
    computed_style_property_is_shorthand, computed_style_property_names_from_object,
    computed_style_property_value_from_object, cssom_style_entry_is_pdb_supplemental_side_entry,
    cssom_style_property_affected_names_with_pdb, is_live_style_declaration_object,
    live_style_named_property_value, set_live_style_named_property_value,
    style_css_text_getter_callback, style_css_text_setter_callback,
    style_get_property_priority_callback, style_get_property_value_callback, style_item_callback,
    style_length_getter_callback, style_remove_property_callback, style_set_property_callback,
};
pub(crate) use stylesheets::{
    detach_cached_style_sheet_for_element, detach_cached_style_sheet_if_live_stylesheet_changed,
    style_sheet_for_element, style_sheet_getter_function, sync_cached_style_sheet_media_from_owner,
};
use stylesheets::{
    link_disabled_getter_function, link_disabled_setter_function, style_blocking_getter_function,
    style_blocking_setter_function, style_disabled_getter_function, style_disabled_setter_function,
    style_type_getter_function, style_type_setter_function,
};
pub(super) use template_install::{
    install_specialized_instance_properties, install_specialized_template,
};
pub(super) use tree_mutation::{
    node_insert_adjacent_element_callback, node_insert_adjacent_html_callback,
    node_insert_adjacent_node_callback, node_insert_adjacent_text_callback,
};
pub(super) use url_attributes::update_iframe_snapshot_navigation;
use url_attributes::{
    default_port_for_scheme, disconnected_iframe_can_materialize_detached_content,
    iframe_has_inactive_child_context, iframe_is_in_own_child_document,
    iframe_is_inside_its_own_child_context_document, iframe_uses_detached_content_cache,
    normalize_url_default_port, parsed_url_like_attribute, resolve_url_like_attribute,
    set_resolved_url_attribute,
};

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Element")]
struct ElementPrototypeReflectionDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = element_id_getter_function,
        setter = element_id_setter_function
    )]
    id: (),
    #[webapi(
        accessor_property = "className",
        enumerable,
        getter = element_class_name_getter_function,
        setter = element_class_name_setter_function
    )]
    class_name: (),
    #[webapi(accessor_property = "tagName", enumerable, getter = element_tag_name_getter_function)]
    tag_name: (),
    #[webapi(accessor_property = "localName", enumerable, getter = element_local_name_getter_function)]
    local_name: (),
    #[webapi(
        accessor_property = "namespaceURI",
        enumerable,
        getter = element_namespace_uri_getter_function
    )]
    namespace_uri: (),
    #[webapi(accessor_property, enumerable, getter = element_prefix_getter_function)]
    prefix: (),
    #[webapi(
        accessor_property = "innerHTML",
        enumerable,
        getter = node_inner_html_getter_function,
        setter = node_inner_html_setter_function
    )]
    inner_html: (),
    #[webapi(
        accessor_property = "outerHTML",
        enumerable,
        getter = node_outer_html_getter_function,
        setter = node_outer_html_setter_function
    )]
    outer_html: (),
    #[webapi(
        accessor_property = "classList",
        enumerable,
        getter = element_class_list_getter_function,
        setter = element_class_list_setter_function
    )]
    class_list: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = element_part_getter_function,
        setter = element_part_setter_function
    )]
    part: (),
    #[webapi(accessor_property, enumerable, getter = element_attributes_getter_function)]
    attributes: (),
    #[webapi(
        accessor_property = "customElementRegistry",
        enumerable,
        getter = element_custom_element_registry_getter_function
    )]
    custom_element_registry: (),
    #[webapi(method, callback = node_scroll_to_callback)]
    scroll: (),
    #[webapi(method = "scrollTo", callback = node_scroll_to_callback)]
    scroll_to: (),
    #[webapi(method = "scrollBy", callback = node_scroll_by_callback)]
    scroll_by: (),
    #[webapi(method = "scrollIntoView", callback = node_scroll_into_view_callback)]
    scroll_into_view: (),
    #[webapi(method, length = 1, enumerable, callback = node_matches_callback)]
    matches: (),
    #[webapi(
        method = "webkitMatchesSelector",
        length = 1,
        enumerable,
        callback = node_matches_callback
    )]
    webkit_matches_selector: (),
    #[webapi(
        method = "setPointerCapture",
        length = 1,
        enumerable,
        callback = node_set_pointer_capture_callback
    )]
    set_pointer_capture: (),
    #[webapi(
        method = "releasePointerCapture",
        length = 1,
        enumerable,
        callback = node_release_pointer_capture_callback
    )]
    release_pointer_capture: (),
    #[webapi(
        method = "hasPointerCapture",
        length = 1,
        enumerable,
        callback = node_has_pointer_capture_callback
    )]
    has_pointer_capture: (),
    #[webapi(
        method = "requestPointerLock",
        length = 0,
        enumerable,
        callback = super::pointer_lock::element_request_pointer_lock_callback
    )]
    request_pointer_lock: (),
    #[webapi(accessor_property = "shadowRoot", enumerable, getter = element_shadow_root_getter_function)]
    shadow_root: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_slot_getter_function,
        setter = node_slot_setter_function
    )]
    slot: (),
    #[webapi(accessor_property = "assignedSlot", enumerable, getter = slot_assigned_slot_getter_function)]
    assigned_slot: (),
    #[webapi(
        method = "attachShadow",
        length = 1,
        enumerable,
        callback = element_attach_shadow_callback
    )]
    attach_shadow: (),
    #[webapi(
        method = "attachInternals",
        callback = element_attach_internals_callback
    )]
    attach_internals: (),
    #[webapi(method = "getHTML", callback = node_get_html_callback)]
    get_html: (),
    #[webapi(method = "setHTMLUnsafe", length = 1, callback = node_set_html_unsafe_callback)]
    set_html_unsafe: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Element")]
struct ElementPrototypeQueryAndAttributeMethodsDeclaration {
    #[webapi(
        method,
        length = 0,
        enumerable,
        callback = node_get_bounding_client_rect_callback
    )]
    get_bounding_client_rect: (),
    #[webapi(
        method,
        length = 0,
        enumerable,
        callback = node_get_client_rects_callback
    )]
    get_client_rects: (),
    #[webapi(method, length = 1, enumerable, callback = node_has_attribute_callback)]
    has_attribute: (),
    #[webapi(
        method = "hasAttributeNS",
        length = 2,
        enumerable,
        callback = node_has_attribute_ns_callback
    )]
    has_attribute_ns: (),
    #[webapi(method, length = 1, enumerable, callback = node_get_attribute_callback)]
    get_attribute: (),
    #[webapi(
        method = "getAttributeNS",
        length = 2,
        enumerable,
        callback = node_get_attribute_ns_callback
    )]
    get_attribute_ns: (),
    #[webapi(method, length = 2, enumerable, callback = node_set_attribute_callback)]
    set_attribute: (),
    #[webapi(
        method = "setAttributeNS",
        length = 3,
        enumerable,
        callback = node_set_attribute_ns_callback
    )]
    set_attribute_ns: (),
    #[webapi(method, length = 1, enumerable, callback = node_remove_attribute_callback)]
    remove_attribute: (),
    #[webapi(
        method = "removeAttributeNS",
        length = 2,
        enumerable,
        callback = node_remove_attribute_ns_callback
    )]
    remove_attribute_ns: (),
    #[webapi(method, length = 1, enumerable, callback = node_closest_callback)]
    closest: (),
    #[webapi(
        method,
        length = 1,
        enumerable,
        callback = node_get_elements_by_tag_name_callback
    )]
    get_elements_by_tag_name: (),
    #[webapi(
        method = "getElementsByTagNameNS",
        length = 2,
        enumerable,
        callback = node_get_elements_by_tag_name_ns_callback
    )]
    get_elements_by_tag_name_ns: (),
    #[webapi(
        method,
        length = 1,
        enumerable,
        callback = node_get_elements_by_class_name_callback
    )]
    get_elements_by_class_name: (),
    #[webapi(method, length = 1, callback = node_get_elements_by_name_callback)]
    get_elements_by_name: (),
    #[webapi(
        method,
        length = 0,
        enumerable,
        callback = node_get_attribute_names_callback
    )]
    get_attribute_names: (),
    #[webapi(method, length = 0, enumerable, callback = node_has_attributes_callback)]
    has_attributes: (),
    #[webapi(method, length = 1, enumerable, callback = node_toggle_attribute_callback)]
    toggle_attribute: (),
    #[webapi(
        method = "checkVisibility",
        length = 0,
        enumerable,
        callback = node_check_visibility_callback
    )]
    check_visibility: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Element", enumerable)]
struct ExtendedElementPrototypeMethodsDeclaration {
    #[webapi(method, length = 1, callback = node_get_attribute_node_callback)]
    get_attribute_node: (),
    #[webapi(
        method = "getAttributeNodeNS",
        length = 2,
        callback = node_get_attribute_node_ns_callback
    )]
    get_attribute_node_ns: (),
    #[webapi(method, length = 1, callback = node_set_attribute_node_callback)]
    set_attribute_node: (),
    #[webapi(
        method = "setAttributeNodeNS",
        length = 1,
        callback = node_set_attribute_node_callback
    )]
    set_attribute_node_ns: (),
    #[webapi(method, length = 1, callback = node_remove_attribute_node_callback)]
    remove_attribute_node: (),
    #[webapi(method, length = 2, callback = node_insert_adjacent_element_callback)]
    insert_adjacent_element: (),
    #[webapi(method, length = 2, callback = node_insert_adjacent_text_callback)]
    insert_adjacent_text: (),
    #[webapi(
        method = "insertAdjacentHTML",
        length = 2,
        callback = node_insert_adjacent_html_callback
    )]
    insert_adjacent_html: (),
    #[webapi(
        method = "__moliInsertAdjacentNode",
        length = 0,
        callback = node_insert_adjacent_node_callback
    )]
    moli_insert_adjacent_node: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Element")]
struct ElementGeometryPrototypeDeclaration {
    #[webapi(
        accessor_property = "clientWidth",
        enumerable,
        getter = node_client_width_getter_function
    )]
    client_width: (),
    #[webapi(
        accessor_property = "clientHeight",
        enumerable,
        getter = node_client_height_getter_function
    )]
    client_height: (),
    #[webapi(
        accessor_property = "clientTop",
        enumerable,
        getter = node_client_top_getter_function
    )]
    client_top: (),
    #[webapi(
        accessor_property = "clientLeft",
        enumerable,
        getter = node_client_left_getter_function
    )]
    client_left: (),
    #[webapi(
        accessor_property = "scrollWidth",
        enumerable,
        getter = node_scroll_width_getter_function
    )]
    scroll_width: (),
    #[webapi(
        accessor_property = "scrollHeight",
        enumerable,
        getter = node_scroll_height_getter_function
    )]
    scroll_height: (),
    #[webapi(
        accessor_property = "scrollTop",
        enumerable,
        getter = node_scroll_top_getter_function,
        setter = node_scroll_top_setter_function
    )]
    scroll_top: (),
    #[webapi(
        accessor_property = "scrollLeft",
        enumerable,
        getter = node_scroll_left_getter_function,
        setter = node_scroll_left_setter_function
    )]
    scroll_left: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLElement")]
struct HtmlElementGeometryPrototypeDeclaration {
    #[webapi(
        accessor_property = "offsetWidth",
        enumerable,
        getter = node_offset_width_getter_function
    )]
    offset_width: (),
    #[webapi(
        accessor_property = "offsetHeight",
        enumerable,
        getter = node_offset_height_getter_function
    )]
    offset_height: (),
    #[webapi(
        accessor_property = "offsetParent",
        enumerable,
        getter = node_offset_parent_getter_function
    )]
    offset_parent: (),
    #[webapi(
        accessor_property = "offsetTop",
        enumerable,
        getter = node_offset_top_getter_function
    )]
    offset_top: (),
    #[webapi(
        accessor_property = "offsetLeft",
        enumerable,
        getter = node_offset_left_getter_function
    )]
    offset_left: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Element", enumerable)]
struct ElementAriaStringReflectionDeclaration {
    #[webapi(
        accessor_property,
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "role")
    )]
    role: (),
    #[webapi(
        accessor_property = "ariaAtomic",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-atomic")
    )]
    aria_atomic: (),
    #[webapi(
        accessor_property = "ariaAutoComplete",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-autocomplete")
    )]
    aria_auto_complete: (),
    #[webapi(
        accessor_property = "ariaBrailleLabel",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-braillelabel")
    )]
    aria_braille_label: (),
    #[webapi(
        accessor_property = "ariaBrailleRoleDescription",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-brailleroledescription")
    )]
    aria_braille_role_description: (),
    #[webapi(
        accessor_property = "ariaBusy",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-busy")
    )]
    aria_busy: (),
    #[webapi(
        accessor_property = "ariaChecked",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-checked")
    )]
    aria_checked: (),
    #[webapi(
        accessor_property = "ariaColCount",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-colcount")
    )]
    aria_col_count: (),
    #[webapi(
        accessor_property = "ariaColIndex",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-colindex")
    )]
    aria_col_index: (),
    #[webapi(
        accessor_property = "ariaColSpan",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-colspan")
    )]
    aria_col_span: (),
    #[webapi(
        accessor_property = "ariaCurrent",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-current")
    )]
    aria_current: (),
    #[webapi(
        accessor_property = "ariaDisabled",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-disabled")
    )]
    aria_disabled: (),
    #[webapi(
        accessor_property = "ariaExpanded",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-expanded")
    )]
    aria_expanded: (),
    #[webapi(
        accessor_property = "ariaHasPopup",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-haspopup")
    )]
    aria_has_popup: (),
    #[webapi(
        accessor_property = "ariaHidden",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-hidden")
    )]
    aria_hidden: (),
    #[webapi(
        accessor_property = "ariaInvalid",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-invalid")
    )]
    aria_invalid: (),
    #[webapi(
        accessor_property = "ariaKeyShortcuts",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-keyshortcuts")
    )]
    aria_key_shortcuts: (),
    #[webapi(
        accessor_property = "ariaLabel",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-label")
    )]
    aria_label: (),
    #[webapi(
        accessor_property = "ariaLevel",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-level")
    )]
    aria_level: (),
    #[webapi(
        accessor_property = "ariaLive",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-live")
    )]
    aria_live: (),
    #[webapi(
        accessor_property = "ariaModal",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-modal")
    )]
    aria_modal: (),
    #[webapi(
        accessor_property = "ariaMultiLine",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-multiline")
    )]
    aria_multi_line: (),
    #[webapi(
        accessor_property = "ariaMultiSelectable",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-multiselectable")
    )]
    aria_multi_selectable: (),
    #[webapi(
        accessor_property = "ariaOrientation",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-orientation")
    )]
    aria_orientation: (),
    #[webapi(
        accessor_property = "ariaPlaceholder",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-placeholder")
    )]
    aria_placeholder: (),
    #[webapi(
        accessor_property = "ariaPosInSet",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-posinset")
    )]
    aria_pos_in_set: (),
    #[webapi(
        accessor_property = "ariaPressed",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-pressed")
    )]
    aria_pressed: (),
    #[webapi(
        accessor_property = "ariaReadOnly",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-readonly")
    )]
    aria_read_only: (),
    #[webapi(
        accessor_property = "ariaRelevant",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-relevant")
    )]
    aria_relevant: (),
    #[webapi(
        accessor_property = "ariaRequired",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-required")
    )]
    aria_required: (),
    #[webapi(
        accessor_property = "ariaRoleDescription",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-roledescription")
    )]
    aria_role_description: (),
    #[webapi(
        accessor_property = "ariaRowCount",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-rowcount")
    )]
    aria_row_count: (),
    #[webapi(
        accessor_property = "ariaRowIndex",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-rowindex")
    )]
    aria_row_index: (),
    #[webapi(
        accessor_property = "ariaRowSpan",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-rowspan")
    )]
    aria_row_span: (),
    #[webapi(
        accessor_property = "ariaSelected",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-selected")
    )]
    aria_selected: (),
    #[webapi(
        accessor_property = "ariaSetSize",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-setsize")
    )]
    aria_set_size: (),
    #[webapi(
        accessor_property = "ariaSort",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-sort")
    )]
    aria_sort: (),
    #[webapi(
        accessor_property = "ariaValueMax",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-valuemax")
    )]
    aria_value_max: (),
    #[webapi(
        accessor_property = "ariaValueMin",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-valuemin")
    )]
    aria_value_min: (),
    #[webapi(
        accessor_property = "ariaValueNow",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-valuenow")
    )]
    aria_value_now: (),
    #[webapi(
        accessor_property = "ariaValueText",
        getter = aria_attribute_getter_callback,
        setter = aria_string_attribute_setter_callback,
        data = v8str(scope, "aria-valuetext")
    )]
    aria_value_text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Element", enumerable)]
struct ElementAriaElementReflectionDeclaration {
    #[webapi(
        accessor_property = "ariaActiveDescendantElement",
        getter = aria_element_reference_attribute_getter_callback,
        setter = aria_element_reference_attribute_setter_callback,
        data = v8str(scope, "aria-activedescendant")
    )]
    aria_active_descendant_element: (),
    #[webapi(
        accessor_property = "ariaControlsElements",
        getter = aria_element_reference_attribute_getter_callback,
        setter = aria_element_reference_attribute_setter_callback,
        data = v8str(scope, "aria-controls")
    )]
    aria_controls_elements: (),
    #[webapi(
        accessor_property = "ariaDescribedByElements",
        getter = aria_element_reference_attribute_getter_callback,
        setter = aria_element_reference_attribute_setter_callback,
        data = v8str(scope, "aria-describedby")
    )]
    aria_described_by_elements: (),
    #[webapi(
        accessor_property = "ariaDetailsElements",
        getter = aria_element_reference_attribute_getter_callback,
        setter = aria_element_reference_attribute_setter_callback,
        data = v8str(scope, "aria-details")
    )]
    aria_details_elements: (),
    #[webapi(
        accessor_property = "ariaErrorMessageElements",
        getter = aria_element_reference_attribute_getter_callback,
        setter = aria_element_reference_attribute_setter_callback,
        data = v8str(scope, "aria-errormessage")
    )]
    aria_error_message_elements: (),
    #[webapi(
        accessor_property = "ariaFlowToElements",
        getter = aria_element_reference_attribute_getter_callback,
        setter = aria_element_reference_attribute_setter_callback,
        data = v8str(scope, "aria-flowto")
    )]
    aria_flow_to_elements: (),
    #[webapi(
        accessor_property = "ariaLabelledByElements",
        getter = aria_element_reference_attribute_getter_callback,
        setter = aria_element_reference_attribute_setter_callback,
        data = v8str(scope, "aria-labelledby")
    )]
    aria_labelled_by_elements: (),
    #[webapi(
        accessor_property = "ariaOwnsElements",
        getter = aria_element_reference_attribute_getter_callback,
        setter = aria_element_reference_attribute_setter_callback,
        data = v8str(scope, "aria-owns")
    )]
    aria_owns_elements: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document")]
struct DocumentCustomElementRegistryPrototypeDeclaration {
    #[webapi(
        accessor_property = "customElementRegistry",
        enumerable,
        getter = element_custom_element_registry_getter_function
    )]
    custom_element_registry: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Element")]
struct ElementStylePrototypeDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_style_getter_function,
        setter = node_style_setter_function
    )]
    style: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLElement")]
struct HtmlElementStandardPrototypeDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_title_getter_function,
        setter = node_title_setter_function
    )]
    title: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_lang_getter_function,
        setter = node_lang_setter_function
    )]
    lang: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_autocapitalize_getter_function,
        setter = node_autocapitalize_setter_function
    )]
    autocapitalize: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_translate_getter_function,
        setter = node_translate_setter_function
    )]
    translate: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_dir_getter_function,
        setter = node_dir_setter_function
    )]
    dir: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_hidden_getter_function,
        setter = node_hidden_setter_function
    )]
    hidden: (),
    #[webapi(
        accessor_property = "accessKey",
        enumerable,
        getter = node_access_key_getter_function,
        setter = node_access_key_setter_function
    )]
    access_key: (),
    #[webapi(
        accessor_property = "accessKeyLabel",
        enumerable,
        getter = node_access_key_label_getter_function
    )]
    access_key_label: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_draggable_getter_function,
        setter = node_draggable_setter_function
    )]
    draggable: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_spellcheck_getter_function,
        setter = node_spellcheck_setter_function
    )]
    spellcheck: (),
    #[webapi(
        accessor_property = "contentEditable",
        enumerable,
        getter = node_content_editable_getter_function,
        setter = node_content_editable_setter_function
    )]
    content_editable: (),
    #[webapi(
        accessor_property = "enterKeyHint",
        enumerable,
        getter = node_enter_key_hint_getter_function,
        setter = node_enter_key_hint_setter_function
    )]
    enter_key_hint: (),
    #[webapi(
        accessor_property = "isContentEditable",
        enumerable,
        getter = node_is_content_editable_getter_function
    )]
    is_content_editable: (),
    #[webapi(
        accessor_property = "inputMode",
        enumerable,
        getter = node_input_mode_getter_function,
        setter = node_input_mode_setter_function
    )]
    input_mode: (),
    #[webapi(
        accessor_property = "innerText",
        enumerable,
        getter = node_inner_text_getter_function,
        setter = node_inner_text_setter_function
    )]
    inner_text: (),
    #[webapi(
        accessor_property = "outerText",
        enumerable,
        getter = node_outer_text_getter_function,
        setter = node_outer_text_setter_function
    )]
    outer_text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLElement")]
struct HtmlElementActionPrototypeDeclaration {
    #[webapi(method, length = 0, enumerable, callback = node_focus_callback)]
    focus: (),
    #[webapi(method, length = 0, enumerable, callback = node_blur_callback)]
    blur: (),
    #[webapi(method, length = 0, enumerable, callback = node_click_callback)]
    click: (),
    #[webapi(method, length = 0, enumerable, callback = node_show_popover_callback)]
    show_popover: (),
    #[webapi(method, length = 0, enumerable, callback = node_hide_popover_callback)]
    hide_popover: (),
    #[webapi(method, length = 0, enumerable, callback = node_toggle_popover_callback)]
    toggle_popover: (),
    #[webapi(
        method,
        length = 0,
        callback = node_scroll_into_view_if_needed_callback
    )]
    scroll_into_view_if_needed: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOrForeignElement")]
struct HtmlOrForeignElementPrototypeDeclaration {
    #[webapi(accessor_property, enumerable, getter = node_dataset_getter_function)]
    dataset: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_nonce_getter_function,
        setter = node_nonce_setter_function
    )]
    nonce: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_autofocus_getter_function,
        setter = node_autofocus_setter_function
    )]
    autofocus: (),
    #[webapi(
        accessor_property = "tabIndex",
        enumerable,
        getter = node_tab_index_getter_function,
        setter = node_tab_index_setter_function
    )]
    tab_index: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLElement")]
struct HtmlElementPopoverPrototypeDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_popover_getter_function,
        setter = node_popover_setter_function
    )]
    popover: (),
}

fn install_html_align_template_binding<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: ElementReflectionInterface,
) {
    let getter = v8::FunctionTemplate::builder(html_align_getter_function)
        .length(0)
        .build(scope);
    getter.set_class_name(v8str(scope, "get align"));
    let setter_data = interface
        .to_v8_template_value(scope)
        .expect("Element reflection interface must convert to V8 template data");
    let setter = v8::FunctionTemplate::builder(html_align_setter_function)
        .data(setter_data)
        .length(1)
        .build(scope);
    setter.set_class_name(v8str(scope, "set align"));
    prototype.set_accessor_property(
        v8str(scope, "align").into(),
        Some(getter),
        Some(setter),
        v8::PropertyAttribute::NONE,
    );
}

const HTML_ALIGN_REFLECTION_INTERFACES: &[ElementReflectionInterface] = &[
    ElementReflectionInterface::HtmlDivElement,
    ElementReflectionInterface::HtmlHeadingElement,
    ElementReflectionInterface::HtmlParagraphElement,
    ElementReflectionInterface::HtmlHrElement,
    ElementReflectionInterface::HtmlImageElement,
    ElementReflectionInterface::HtmlObjectElement,
    ElementReflectionInterface::HtmlIFrameElement,
    ElementReflectionInterface::HtmlEmbedElement,
    ElementReflectionInterface::HtmlLegendElement,
    ElementReflectionInterface::HtmlTableCaptionElement,
    ElementReflectionInterface::HtmlTableElement,
    ElementReflectionInterface::HtmlTableSectionElement,
    ElementReflectionInterface::HtmlTableRowElement,
    ElementReflectionInterface::HtmlTableColElement,
    ElementReflectionInterface::HtmlTableCellElement,
    ElementReflectionInterface::HtmlInputElement,
];

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Object", enumerable)]
struct HtmlCompactPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_compact_getter_function,
        setter = html_compact_setter_function
    )]
    compact: (),
}

const HTML_COMPACT_REFLECTION_INTERFACES: &[&str] = &[
    "HTMLDirectoryElement",
    "HTMLDListElement",
    "HTMLMenuElement",
    "HTMLOListElement",
    "HTMLUListElement",
];

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLLIElement", enumerable)]
struct HtmlLiElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = li_value_getter_function,
        setter = li_value_setter_function
    )]
    value: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LiType
    )]
    r#type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOListElement", enumerable)]
struct HtmlOListElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = ol_start_getter_function,
        setter = ol_start_setter_function
    )]
    start: (),
    #[webapi(
        accessor_property,
        getter = ol_reversed_getter_function,
        setter = ol_reversed_setter_function
    )]
    reversed: (),
    #[webapi(
        accessor_property,
        getter = ol_type_getter_function,
        setter = ol_type_setter_function
    )]
    r#type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLUListElement", enumerable)]
struct HtmlUListElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::UlType
    )]
    r#type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Object", enumerable)]
struct HtmlBodyOrFrameSetEventHandlersPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = body_onload_getter_function,
        setter = body_onload_setter_function
    )]
    onload: (),
    #[webapi(
        accessor_property,
        getter = body_onmessageerror_getter_function,
        setter = body_onmessageerror_setter_function
    )]
    onmessageerror: (),
    #[webapi(
        accessor_property,
        getter = body_onerror_getter_function,
        setter = body_onerror_setter_function
    )]
    onerror: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLBodyElement", enumerable)]
struct HtmlBodyElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = body_text_getter_function,
        setter = body_text_setter_function
    )]
    text: (),
    #[webapi(
        accessor_property,
        getter = body_link_getter_function,
        setter = body_link_setter_function
    )]
    link: (),
    #[webapi(
        accessor_property = "vLink",
        getter = body_v_link_getter_function,
        setter = body_v_link_setter_function
    )]
    v_link: (),
    #[webapi(
        accessor_property = "aLink",
        getter = body_a_link_getter_function,
        setter = body_a_link_setter_function
    )]
    a_link: (),
    #[webapi(
        accessor_property,
        getter = body_background_getter_function,
        setter = body_background_setter_function
    )]
    background: (),
    #[webapi(
        accessor_property = "bgColor",
        getter = html_bg_color_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::BodyBgColor
    )]
    bg_color: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLHRElement", enumerable)]
struct HtmlHrElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_size_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::HrSize
    )]
    size: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::HrWidth
    )]
    width: (),
    #[webapi(
        accessor_property = "noShade",
        getter = html_no_shade_getter_function,
        setter = html_no_shade_setter_function
    )]
    no_shade: (),
    #[webapi(
        accessor_property,
        getter = html_color_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::HrColor
    )]
    color: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLFontElement", enumerable)]
struct HtmlFontElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_size_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::FontSize
    )]
    size: (),
    #[webapi(
        accessor_property,
        getter = html_color_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::FontColor
    )]
    color: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLMarqueeElement", enumerable)]
struct HtmlMarqueeElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property = "loop",
        getter = marquee_loop_getter_function,
        setter = marquee_loop_setter_function
    )]
    loop_: (),
    #[webapi(
        accessor_property = "scrollAmount",
        getter = marquee_scroll_amount_getter_function,
        setter = marquee_scroll_amount_setter_function
    )]
    scroll_amount: (),
    #[webapi(
        accessor_property = "scrollDelay",
        getter = marquee_scroll_delay_getter_function,
        setter = marquee_scroll_delay_setter_function
    )]
    scroll_delay: (),
    #[webapi(
        accessor_property,
        getter = html_height_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::MarqueeHeight
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::MarqueeWidth
    )]
    width: (),
    #[webapi(
        accessor_property = "bgColor",
        getter = html_bg_color_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::MarqueeBgColor
    )]
    bg_color: (),
    #[webapi(
        accessor_property,
        getter = html_hspace_getter_function,
        setter = unsigned_long_reflection_setter_function,
        setter_data = UnsignedLongReflection::MarqueeHspace
    )]
    hspace: (),
    #[webapi(
        accessor_property,
        getter = html_vspace_getter_function,
        setter = unsigned_long_reflection_setter_function,
        setter_data = UnsignedLongReflection::MarqueeVspace
    )]
    vspace: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableElement")]
struct HtmlTableElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableWidth
    )]
    width: (),
    #[webapi(
        accessor_property = "bgColor",
        enumerable,
        getter = html_bg_color_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::TableBgColor
    )]
    bg_color: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = html_border_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableBorder
    )]
    border: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = table_caption_getter_function,
        setter = table_caption_setter_function
    )]
    caption: (),
    #[webapi(
        accessor_property = "tHead",
        enumerable,
        getter = table_t_head_getter_function,
        setter = table_t_head_setter_function
    )]
    t_head: (),
    #[webapi(
        accessor_property = "tFoot",
        enumerable,
        getter = table_t_foot_getter_function,
        setter = table_t_foot_setter_function
    )]
    t_foot: (),
    #[webapi(accessor_property, enumerable, getter = table_rows_getter_function)]
    rows: (),
    #[webapi(accessor_property = "tBodies", enumerable, getter = table_t_bodies_getter_function)]
    t_bodies: (),
    #[webapi(method = "createCaption", callback = table_create_caption_callback)]
    create_caption: (),
    #[webapi(method = "deleteCaption", callback = table_delete_caption_callback)]
    delete_caption: (),
    #[webapi(method = "createTHead", callback = table_create_t_head_callback)]
    create_t_head: (),
    #[webapi(method = "deleteTHead", callback = table_delete_t_head_callback)]
    delete_t_head: (),
    #[webapi(method = "createTFoot", callback = table_create_t_foot_callback)]
    create_t_foot: (),
    #[webapi(method = "deleteTFoot", callback = table_delete_t_foot_callback)]
    delete_t_foot: (),
    #[webapi(method = "createTBody", callback = table_create_t_body_callback)]
    create_t_body: (),
    #[webapi(method = "insertRow", length = 1, callback = table_insert_row_callback)]
    insert_row: (),
    #[webapi(method = "deleteRow", length = 1, callback = table_delete_row_callback)]
    delete_row: (),
}

fn anchor_url_string_function_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    project: impl FnOnce(&url::Url) -> String,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        rv.set_empty_string();
        return;
    };
    let value = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href")
        .map(|url| project(&url))
        .unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn anchor_href_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href");
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn anchor_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    hyperlink_href_setter_function(scope, args, "HTMLAnchorElement", &mut rv);
}

fn area_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    hyperlink_href_setter_function(scope, args, "HTMLAreaElement", &mut rv);
}

fn base_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    hyperlink_href_setter_function(scope, args, "HTMLBaseElement", &mut rv);
}

fn link_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    hyperlink_href_setter_function(scope, args, "HTMLLinkElement", &mut rv);
}

fn hyperlink_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    owner: &'static str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), owner, "href") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "href", &value);
    rv.set_undefined();
}

fn html_referrer_policy_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value =
        element_attribute(unsafe { &*runtime_ptr }, handle, "referrerpolicy").unwrap_or_default();
    let Some(value) = v8_string(scope, canonical_referrer_policy_value(&value)) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn html_cross_origin_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    match element_attribute(unsafe { &*runtime_ptr }, handle, "crossorigin") {
        Some(value) => {
            let Some(value) = v8_string(scope, canonical_cross_origin_value(&value)) else {
                rv.set_null();
                return;
            };
            rv.set(value.into());
        }
        None => rv.set_null(),
    }
}

fn set_html_cross_origin_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    if value.is_null() || value.is_undefined() {
        remove_reflected_attribute(scope, runtime_ptr, handle, "crossorigin");
        return;
    }
    let Some(value) = property_dom_string_value(scope, value, owner, "crossOrigin") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "crossorigin", &value);
}

fn html_cross_origin_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(descriptor) =
        CrossOriginReflection::descriptor_from_callback_data(scope, args.data())
    {
        set_html_cross_origin_for_receiver(scope, args.this(), args.get(0), descriptor.interface);
    }
    rv.set_undefined();
}

fn html_loading_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "loading").unwrap_or_default();
    let Some(value) = v8_string(scope, canonical_loading_value(&value)) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn image_loading_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "loading")
        .unwrap_or_else(|| "eager".to_owned());
    let Some(value) = v8_string(scope, &value) else {
        rv.set_empty_string();
        return;
    };
    rv.set(value.into());
}

fn set_html_loading_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
) -> Option<(*mut JsContextHost, DomHandle, String)> {
    let value = property_dom_string_value(scope, value, owner, "loading")?;
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return None;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "loading", &value);
    Some((runtime_ptr, handle, value))
}

fn image_loading_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some((runtime_ptr, handle, value)) =
        set_html_loading_for_receiver(scope, args.this(), args.get(0), "HTMLImageElement")
        && !value.trim().eq_ignore_ascii_case("lazy")
    {
        queue_image_load_event_for_loading_change(scope, runtime_ptr, handle);
    }
    rv.set_undefined();
}

fn anchor_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "type", rv);
}

fn anchor_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "type",
        args.get(0),
        "HTMLAnchorElement",
        "type",
    );
    rv.set_undefined();
}

fn anchor_host_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| {
            url.host_str()
                .map(|host| {
                    url.port()
                        .map(|port| format!("{host}:{port}"))
                        .unwrap_or_else(|| host.to_owned())
                })
                .unwrap_or_default()
        },
        rv,
    );
}

fn anchor_host_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    let applied = if let Some((host, port)) = value
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()))
    {
        if url.set_host(Some(host)).is_err() {
            false
        } else {
            let port = port.parse::<u16>().ok();
            if default_port_for_scheme(url.scheme()) == port {
                url.set_port(None).is_ok()
            } else {
                url.set_port(port).is_ok()
            }
        }
    } else {
        url.set_host(Some(&value)).is_ok()
    };
    if applied {
        normalize_url_default_port(&mut url);
        set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    }
    rv.set_undefined();
}

fn anchor_hostname_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| url.host_str().unwrap_or_default().to_owned(),
        rv,
    );
}

fn anchor_hostname_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    if url.set_host(Some(&value)).is_ok() {
        normalize_url_default_port(&mut url);
        set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    }
    rv.set_undefined();
}

fn anchor_port_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| url.port().map(|port| port.to_string()).unwrap_or_default(),
        rv,
    );
}

fn anchor_port_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    let value = args.get(0);
    let applied = if value.is_null_or_undefined() {
        url.set_port(None).is_ok()
    } else if let Some(value) = property_string_value(scope, value) {
        if value.is_empty() {
            url.set_port(None).is_ok()
        } else if let Ok(port) = value.parse::<u16>() {
            if default_port_for_scheme(url.scheme()) == Some(port) {
                url.set_port(None).is_ok()
            } else {
                url.set_port(Some(port)).is_ok()
            }
        } else {
            url.set_port(None).is_ok()
        }
    } else {
        false
    };
    if applied {
        normalize_url_default_port(&mut url);
        set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    }
    rv.set_undefined();
}

fn anchor_pathname_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(scope, args.this(), |url| url.path().to_owned(), rv);
}

fn anchor_pathname_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    if value.starts_with('/') {
        url.set_path(&value);
    } else {
        url.set_path(&format!("/{value}"));
    }
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_search_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| {
            url.query()
                .map(|query| format!("?{query}"))
                .unwrap_or_default()
        },
        rv,
    );
}

fn anchor_search_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    if value.is_empty() {
        url.set_query(None);
    } else {
        url.set_query(Some(value.trim_start_matches('?')));
    }
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_hash_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| {
            url.fragment()
                .map(|fragment| format!("#{fragment}"))
                .unwrap_or_default()
        },
        rv,
    );
}

fn anchor_hash_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    if value.is_empty() {
        url.set_fragment(None);
    } else {
        url.set_fragment(Some(value.trim_start_matches('#')));
    }
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_origin_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(scope, args.this(), moli_url::origin_ascii_serialization, rv);
}

fn anchor_protocol_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(scope, args.this(), |url| format!("{}:", url.scheme()), rv);
}

fn anchor_protocol_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    let scheme = value.trim_end_matches(':');
    if url.set_scheme(scheme).is_ok() {
        normalize_url_default_port(&mut url);
        set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    }
    rv.set_undefined();
}

fn anchor_username_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(scope, args.this(), |url| url.username().to_owned(), rv);
}

fn anchor_username_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    if !anchor_url_can_have_userinfo(&url) {
        return;
    }
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    if url.set_username(&value).is_ok() {
        set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    }
    rv.set_undefined();
}

fn anchor_password_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| url.password().unwrap_or("").to_owned(),
        rv,
    );
}

fn anchor_password_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(mut url) = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href") else {
        return;
    };
    if !anchor_url_can_have_userinfo(&url) {
        return;
    }
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    let password = if value.is_empty() {
        None
    } else {
        Some(value.as_str())
    };
    if url.set_password(password).is_ok() {
        set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    }
    rv.set_undefined();
}

fn anchor_url_can_have_userinfo(url: &url::Url) -> bool {
    !url.cannot_be_a_base() && url.host().is_some()
}

fn reflected_url_attribute_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    name: &str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        if let Some(value) = v8_string(scope, "") {
            rv.set(value.into());
        } else {
            rv.set_null();
        }
        return;
    };
    let value = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, name);
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn script_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    reflected_url_attribute_getter_function(scope, args.this(), "src", rv);
}

fn script_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = trusted_script_url_sink_string(scope, runtime_ptr, args.get(0)) else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
    rv.set_undefined();
}

fn script_dom_string_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &str,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), name, rv);
}

fn script_dom_string_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &str,
    property: &'static str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        name,
        args.get(0),
        "HTMLScriptElement",
        property,
    );
    rv.set_undefined();
}

fn script_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_dom_string_getter_function(scope, args, "type", rv);
}

fn script_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_dom_string_setter_function(scope, args, "type", "type", &mut rv);
}

fn svg_script_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "type",
        args.get(0),
        "SVGScriptElement",
        "type",
    );
    rv.set_undefined();
}

fn node_nonce_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .and_then(crate::dom::native::Element::cryptographic_nonce)
        .map(str::to_owned)
        .or_else(|| runtime.dom_host().get_attribute(handle, "nonce"))
        .unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn node_nonce_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = property_dom_string_value(scope, args.get(0), "Element", "nonce") else {
        return;
    };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_cryptographic_nonce(handle, Some(value));
    rv.set_undefined();
}

fn script_async_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(crate::dom::native::Element::script_async);
    rv.set_bool(value);
}

fn script_async_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let value = args.get(0).boolean_value(scope);
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_script_async(scope, runtime_ptr, handle, value);
    rv.set_undefined();
}

fn script_text_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = node_direct_text_content(unsafe { &*runtime_ptr }, handle).unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn script_source_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    sink: TrustedScriptElementSink,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_element_setter_receiver(
        scope,
        args.this(),
        "HTMLScriptElement",
        sink.api_name(),
        "script",
    ) else {
        return;
    };
    let Some(text) = trusted_script_element_sink_string(scope, runtime_ptr, args.get(0), sink)
    else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_script_text_internal_slot(handle, &text);
    let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &text);
    rv.set_undefined();
}

fn script_text_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_source_setter_function(scope, args, TrustedScriptElementSink::Text, rv);
}

fn script_inner_text_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if html_element_getter_receiver(
        scope,
        args.this(),
        "HTMLScriptElement",
        "innerText",
        "script",
    )
    .is_none()
    {
        return;
    }
    node_inner_text_getter_function(scope, args, rv);
}

fn script_inner_text_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_source_setter_function(scope, args, TrustedScriptElementSink::InnerText, rv);
}

fn script_text_content_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if html_element_getter_receiver(
        scope,
        args.this(),
        "HTMLScriptElement",
        "textContent",
        "script",
    )
    .is_none()
    {
        return;
    }
    node_text_content_getter_function(scope, args, rv);
}

fn script_text_content_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_source_setter_function(scope, args, TrustedScriptElementSink::TextContent, rv);
}

fn script_boolean_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &str,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), name, rv);
}

fn script_boolean_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        name,
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}

fn script_defer_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_boolean_getter_function(scope, args, "defer", rv);
}

fn script_defer_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_boolean_setter_function(scope, args, "defer", &mut rv);
}

fn script_no_module_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_boolean_getter_function(scope, args, "nomodule", rv);
}

fn script_no_module_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_boolean_setter_function(scope, args, "nomodule", &mut rv);
}

fn script_integrity_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_dom_string_getter_function(scope, args, "integrity", rv);
}

fn script_integrity_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_dom_string_setter_function(scope, args, "integrity", "integrity", &mut rv);
}

fn script_event_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_dom_string_getter_function(scope, args, "event", rv);
}

fn script_event_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_dom_string_setter_function(scope, args, "event", "event", &mut rv);
}

fn script_html_for_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_dom_string_getter_function(scope, args, "for", rv);
}

fn script_html_for_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_dom_string_setter_function(scope, args, "for", "htmlFor", &mut rv);
}

fn image_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    reflected_url_attribute_getter_function(scope, args.this(), "src", rv);
}

fn image_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), "HTMLImageElement", "src")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
    rv.set_undefined();
}

fn generic_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    reflected_url_attribute_getter_function(scope, args.this(), "src", rv);
}

fn source_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    generic_src_setter_function(scope, args, "HTMLSourceElement", &mut rv);
}

fn embed_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    generic_src_setter_function(scope, args, "HTMLEmbedElement", &mut rv);
}

fn frame_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    generic_src_setter_function(scope, args, "HTMLFrameElement", &mut rv);
}

fn generic_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    owner: &'static str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), owner, "src") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
    rv.set_undefined();
}

fn image_srcset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "srcset", rv);
}

fn image_srcset_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "srcset",
        args.get(0),
        "HTMLImageElement",
        "srcset",
    );
    rv.set_undefined();
}

fn source_srcset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "srcset", rv);
}

fn source_srcset_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "srcset",
        args.get(0),
        "HTMLSourceElement",
        "srcset",
    );
    rv.set_undefined();
}

fn iframe_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    reflected_url_attribute_getter_function(scope, args.this(), "src", rv);
}

fn iframe_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), "HTMLIFrameElement", "src")
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if iframe_uses_detached_content_cache(runtime, handle)
        || !runtime.dom_host().is_connected(handle)
    {
        set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
        clear_detached_iframe_cached_context(scope, receiver);
    } else {
        update_iframe_snapshot_navigation(scope, runtime_ptr, handle, &value);
    }
    rv.set_undefined();
}

fn iframe_srcdoc_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "srcdoc").unwrap_or_default();
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn iframe_srcdoc_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), "HTMLIFrameElement", "srcdoc")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "srcdoc", &value);
    let runtime = unsafe { &*runtime_ptr };
    if iframe_uses_detached_content_cache(runtime, handle)
        || !runtime.dom_host().is_connected(handle)
    {
        clear_detached_iframe_cached_context(scope, receiver);
    }
    rv.set_undefined();
}

fn iframe_content_document_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        rv.set_null();
        return;
    };
    if iframe_is_inside_its_own_child_context_document(scope, runtime_ptr, handle) {
        rv.set_null();
        return;
    }
    if iframe_has_inactive_child_context(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    if iframe_is_in_own_child_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    if iframe_uses_detached_content_cache(runtime, handle)
        || !runtime.dom_host().is_connected(handle)
    {
        if iframe_uses_detached_content_cache(runtime, handle)
            || disconnected_iframe_can_materialize_detached_content(runtime, handle)
        {
            match detached_iframe_content_document(scope, receiver) {
                Some(document) => rv.set(document.into()),
                None => rv.set_null(),
            }
        } else {
            rv.set_null();
        }
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.refresh_child_browsing_context(scope, handle);
    if !runtime.child_browsing_context_is_same_origin_with_top(handle) {
        rv.set_null();
        return;
    }
    let window = runtime.child_browsing_context_window_wrapper(scope, handle);
    if let Some(window) = window {
        runtime.set_cached_detached_iframe_content_window(scope, handle, window);
        if let Some(document) = window
            .get(scope, v8str(scope, "document").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            runtime.set_cached_detached_iframe_content_document(scope, handle, document);
            rv.set(document.into());
            return;
        }
    }
    match runtime.child_browsing_context_document_wrapper(scope, handle) {
        Some(document) => {
            runtime.set_cached_detached_iframe_content_document(scope, handle, document);
            rv.set(document.into());
        }
        None => rv.set_null(),
    }
}

fn iframe_content_window_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        rv.set_null();
        return;
    };
    if iframe_is_inside_its_own_child_context_document(scope, runtime_ptr, handle) {
        rv.set_null();
        return;
    }
    if iframe_has_inactive_child_context(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    if iframe_is_in_own_child_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    if iframe_uses_detached_content_cache(runtime, handle)
        || !runtime.dom_host().is_connected(handle)
    {
        if iframe_uses_detached_content_cache(runtime, handle)
            || disconnected_iframe_can_materialize_detached_content(runtime, handle)
        {
            match detached_iframe_content_window(scope, receiver) {
                Some(window) => rv.set(window.into()),
                None => rv.set_null(),
            }
        } else {
            rv.set_null();
        }
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.refresh_child_browsing_context(scope, handle);
    let exposes_same_origin_wrapper =
        runtime.child_browsing_context_is_same_origin_with_top(handle);
    let window = runtime.child_browsing_context_window_proxy_for_top(scope, handle);
    if window.is_some() {
        runtime.mark_child_browsing_context_window_wrapper_exposed_to_top(handle);
    }
    if exposes_same_origin_wrapper && window.is_some() {
        runtime.request_child_frame_realm_materialization(handle);
    }
    match window {
        Some(window) => {
            if runtime.child_browsing_context_is_same_origin_with_top(handle) {
                runtime.set_cached_detached_iframe_content_window(scope, handle, window);
            }
            rv.set(window.into());
        }
        None => rv.set_null(),
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLScriptElement", enumerable)]
struct HtmlScriptElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "innerText",
        getter = script_inner_text_getter_function,
        setter = script_inner_text_setter_function
    )]
    inner_text: (),
    #[webapi(
        accessor_property = "textContent",
        getter = script_text_content_getter_function,
        setter = script_text_content_setter_function
    )]
    text_content: (),
    #[webapi(
        accessor_property = "crossOrigin",
        getter = html_cross_origin_getter_function,
        setter = html_cross_origin_setter_function,
        setter_data = CrossOriginReflection::Script
    )]
    cross_origin: (),
    #[webapi(
        accessor_property,
        getter = script_src_getter_function,
        setter = script_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property = "fetchPriority",
        getter = html_fetch_priority_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::ScriptFetchPriority
    )]
    fetch_priority: (),
    #[webapi(
        accessor_property,
        getter = html_charset_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ScriptCharset
    )]
    charset: (),
    #[webapi(
        accessor_property = "type",
        getter = script_type_getter_function,
        setter = script_type_setter_function
    )]
    r#type: (),
    #[webapi(
        accessor_property = "async",
        getter = script_async_getter_function,
        setter = script_async_setter_function
    )]
    r#async: (),
    #[webapi(
        accessor_property,
        getter = script_text_getter_function,
        setter = script_text_setter_function
    )]
    text: (),
    #[webapi(
        accessor_property,
        getter = script_defer_getter_function,
        setter = script_defer_setter_function
    )]
    defer: (),
    #[webapi(
        accessor_property,
        getter = script_no_module_getter_function,
        setter = script_no_module_setter_function
    )]
    no_module: (),
    #[webapi(
        accessor_property,
        getter = script_integrity_getter_function,
        setter = script_integrity_setter_function
    )]
    integrity: (),
    #[webapi(
        accessor_property,
        getter = script_event_getter_function,
        setter = script_event_setter_function
    )]
    event: (),
    #[webapi(
        accessor_property,
        getter = script_html_for_getter_function,
        setter = script_html_for_setter_function
    )]
    html_for: (),
    #[webapi(
        accessor_property,
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ScriptReferrerPolicy
    )]
    referrer_policy: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGScriptElement", enumerable)]
struct SvgScriptElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "type",
        getter = script_type_getter_function,
        setter = svg_script_type_setter_function
    )]
    r#type: (),
    #[webapi(
        accessor_property = "async",
        getter = script_async_getter_function,
        setter = script_async_setter_function
    )]
    r#async: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLAnchorElement", enumerable)]
struct HtmlAnchorElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = anchor_href_getter_function,
        setter = anchor_href_setter_function
    )]
    href: (),
    #[webapi(
        accessor_property,
        getter = anchor_host_getter_function,
        setter = anchor_host_setter_function
    )]
    host: (),
    #[webapi(
        accessor_property,
        getter = anchor_hostname_getter_function,
        setter = anchor_hostname_setter_function
    )]
    hostname: (),
    #[webapi(
        accessor_property,
        getter = anchor_port_getter_function,
        setter = anchor_port_setter_function
    )]
    port: (),
    #[webapi(
        accessor_property,
        getter = anchor_pathname_getter_function,
        setter = anchor_pathname_setter_function
    )]
    pathname: (),
    #[webapi(
        accessor_property,
        getter = anchor_search_getter_function,
        setter = anchor_search_setter_function
    )]
    search: (),
    #[webapi(
        accessor_property,
        getter = anchor_hash_getter_function,
        setter = anchor_hash_setter_function
    )]
    hash: (),
    #[webapi(accessor_property, getter = anchor_origin_getter_function)]
    origin: (),
    #[webapi(
        accessor_property,
        getter = anchor_protocol_getter_function,
        setter = anchor_protocol_setter_function
    )]
    protocol: (),
    #[webapi(
        accessor_property,
        getter = anchor_username_getter_function,
        setter = anchor_username_setter_function
    )]
    username: (),
    #[webapi(
        accessor_property,
        getter = anchor_password_getter_function,
        setter = anchor_password_setter_function
    )]
    password: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLAreaElement", enumerable)]
struct HtmlAreaElementUrlPrototypeDeclaration {
    #[webapi(method = "toString", length = 0, callback = area_to_string_callback)]
    to_string: (),
    #[webapi(
        accessor_property,
        getter = anchor_href_getter_function,
        setter = area_href_setter_function
    )]
    href: (),
    #[webapi(
        accessor_property,
        getter = anchor_host_getter_function,
        setter = anchor_host_setter_function
    )]
    host: (),
    #[webapi(
        accessor_property,
        getter = anchor_hostname_getter_function,
        setter = anchor_hostname_setter_function
    )]
    hostname: (),
    #[webapi(
        accessor_property,
        getter = anchor_port_getter_function,
        setter = anchor_port_setter_function
    )]
    port: (),
    #[webapi(
        accessor_property,
        getter = anchor_pathname_getter_function,
        setter = anchor_pathname_setter_function
    )]
    pathname: (),
    #[webapi(
        accessor_property,
        getter = anchor_search_getter_function,
        setter = anchor_search_setter_function
    )]
    search: (),
    #[webapi(
        accessor_property,
        getter = anchor_hash_getter_function,
        setter = anchor_hash_setter_function
    )]
    hash: (),
    #[webapi(accessor_property, getter = anchor_origin_getter_function)]
    origin: (),
    #[webapi(
        accessor_property,
        getter = anchor_protocol_getter_function,
        setter = anchor_protocol_setter_function
    )]
    protocol: (),
    #[webapi(
        accessor_property,
        getter = anchor_username_getter_function,
        setter = anchor_username_setter_function
    )]
    username: (),
    #[webapi(
        accessor_property,
        getter = anchor_password_getter_function,
        setter = anchor_password_setter_function
    )]
    password: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLImageElement", enumerable)]
struct HtmlImageElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = image_width_getter_function,
        setter = image_width_setter_function
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = image_height_getter_function,
        setter = image_height_setter_function
    )]
    height: (),
    #[webapi(accessor_property = "naturalWidth", getter = image_natural_width_getter_function)]
    natural_width: (),
    #[webapi(accessor_property = "naturalHeight", getter = image_natural_height_getter_function)]
    natural_height: (),
    #[webapi(
        accessor_property = "isMap",
        getter = image_is_map_getter_function,
        setter = image_is_map_setter_function
    )]
    is_map: (),
    #[webapi(accessor_property, getter = image_complete_getter_function)]
    complete: (),
    #[webapi(accessor_property = "currentSrc", getter = image_current_src_getter_function)]
    current_src: (),
    #[webapi(
        accessor_property,
        getter = html_sizes_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ImageSizes
    )]
    sizes: (),
    #[webapi(
        accessor_property = "crossOrigin",
        getter = html_cross_origin_getter_function,
        setter = html_cross_origin_setter_function,
        setter_data = CrossOriginReflection::Image
    )]
    cross_origin: (),
    #[webapi(
        accessor_property,
        getter = html_alt_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ImageAlt
    )]
    alt: (),
    #[webapi(
        accessor_property,
        getter = image_src_getter_function,
        setter = image_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = image_srcset_getter_function,
        setter = image_srcset_setter_function
    )]
    srcset: (),
    #[webapi(
        accessor_property = "fetchPriority",
        getter = html_fetch_priority_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::ImageFetchPriority
    )]
    fetch_priority: (),
    #[webapi(
        accessor_property = "useMap",
        getter = html_use_map_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ImageUseMap
    )]
    use_map: (),
    #[webapi(
        accessor_property,
        getter = image_loading_getter_function,
        setter = image_loading_setter_function
    )]
    loading: (),
    #[webapi(
        accessor_property = "referrerPolicy",
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ImageReferrerPolicy
    )]
    referrer_policy: (),
    #[webapi(
        accessor_property = "longDesc",
        getter = html_long_desc_getter_function,
        setter = image_long_desc_setter_function
    )]
    long_desc: (),
    #[webapi(
        accessor_property,
        getter = html_lowsrc_getter_function,
        setter = image_lowsrc_setter_function
    )]
    lowsrc: (),
    #[webapi(
        accessor_property,
        getter = html_decoding_getter_function,
        setter = image_decoding_setter_function
    )]
    decoding: (),
    #[webapi(
        accessor_property,
        getter = html_border_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::ImageBorder
    )]
    border: (),
    #[webapi(
        accessor_property,
        getter = html_hspace_getter_function,
        setter = unsigned_long_reflection_setter_function,
        setter_data = UnsignedLongReflection::ImageHspace
    )]
    hspace: (),
    #[webapi(
        accessor_property,
        getter = html_vspace_getter_function,
        setter = unsigned_long_reflection_setter_function,
        setter_data = UnsignedLongReflection::ImageVspace
    )]
    vspace: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLSourceElement", enumerable)]
struct HtmlSourceElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = generic_src_getter_function,
        setter = source_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = html_sizes_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::SourceSizes
    )]
    sizes: (),
    #[webapi(
        accessor_property,
        getter = source_width_getter_function,
        setter = source_width_setter_function
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = source_height_getter_function,
        setter = source_height_setter_function
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = source_srcset_getter_function,
        setter = source_srcset_setter_function
    )]
    srcset: (),
    #[webapi(
        accessor_property,
        getter = html_media_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::SourceMedia
    )]
    media: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::SourceType
    )]
    r#type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLEmbedElement", enumerable)]
struct HtmlEmbedElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = generic_src_getter_function,
        setter = embed_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::EmbedWidth
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = html_height_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::EmbedHeight
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::EmbedType
    )]
    r#type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLFrameElement", enumerable)]
struct HtmlFrameElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = generic_src_getter_function,
        setter = frame_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = html_scrolling_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::FrameScrolling
    )]
    scrolling: (),
    #[webapi(
        accessor_property = "frameBorder",
        getter = html_frame_border_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::FrameFrameBorder
    )]
    frame_border: (),
    #[webapi(
        accessor_property = "longDesc",
        getter = html_long_desc_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::FrameLongDesc
    )]
    long_desc: (),
    #[webapi(
        accessor_property = "marginHeight",
        getter = html_margin_height_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::FrameMarginHeight
    )]
    margin_height: (),
    #[webapi(
        accessor_property = "marginWidth",
        getter = html_margin_width_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::FrameMarginWidth
    )]
    margin_width: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLIFrameElement", enumerable)]
struct HtmlIFrameElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = iframe_src_getter_function,
        setter = iframe_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = iframe_srcdoc_getter_function,
        setter = iframe_srcdoc_setter_function
    )]
    srcdoc: (),
    #[webapi(
        accessor_property,
        getter = html_loading_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeLoading
    )]
    loading: (),
    #[webapi(
        accessor_property,
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeReferrerPolicy
    )]
    referrer_policy: (),
    #[webapi(
        accessor_property,
        getter = node_sandbox_getter_function,
        setter = node_sandbox_setter_function
    )]
    sandbox: (),
    #[webapi(
        accessor_property = "allowFullscreen",
        getter = node_allow_fullscreen_getter_function,
        setter = node_allow_fullscreen_setter_function
    )]
    allow_fullscreen: (),
    #[webapi(
        accessor_property,
        getter = node_credentialless_getter_function,
        setter = node_credentialless_setter_function
    )]
    credentialless: (),
    #[webapi(
        accessor_property,
        getter = html_scrolling_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeScrolling
    )]
    scrolling: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeWidth
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = html_height_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeHeight
    )]
    height: (),
    #[webapi(
        accessor_property = "frameBorder",
        getter = html_frame_border_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeFrameBorder
    )]
    frame_border: (),
    #[webapi(
        accessor_property = "longDesc",
        getter = html_long_desc_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::IframeLongDesc
    )]
    long_desc: (),
    #[webapi(
        accessor_property = "marginHeight",
        getter = html_margin_height_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::IframeMarginHeight
    )]
    margin_height: (),
    #[webapi(
        accessor_property = "marginWidth",
        getter = html_margin_width_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::IframeMarginWidth
    )]
    margin_width: (),
    #[webapi(accessor_property, getter = iframe_content_document_getter_function)]
    content_document: (),
    #[webapi(accessor_property, getter = iframe_content_window_getter_function)]
    content_window: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLBaseElement", enumerable)]
struct HtmlBaseElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = anchor_href_getter_function,
        setter = base_href_setter_function
    )]
    href: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLLinkElement", enumerable)]
struct HtmlLinkElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = link_disabled_getter_function,
        setter = link_disabled_setter_function
    )]
    disabled: (),
    #[webapi(
        accessor_property = "crossOrigin",
        getter = html_cross_origin_getter_function,
        setter = html_cross_origin_setter_function,
        setter_data = CrossOriginReflection::Link
    )]
    cross_origin: (),
    #[webapi(
        accessor_property,
        getter = anchor_href_getter_function,
        setter = link_href_setter_function
    )]
    href: (),
    #[webapi(
        accessor_property = "fetchPriority",
        getter = html_fetch_priority_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::LinkFetchPriority
    )]
    fetch_priority: (),
    #[webapi(
        accessor_property = "referrerPolicy",
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkReferrerPolicy
    )]
    referrer_policy: (),
    #[webapi(
        accessor_property,
        getter = html_as_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkAs
    )]
    r#as: (),
    #[webapi(
        accessor_property,
        getter = html_hreflang_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkHreflang
    )]
    hreflang: (),
    #[webapi(
        accessor_property,
        getter = html_charset_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkCharset
    )]
    charset: (),
    #[webapi(
        accessor_property,
        getter = html_media_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkMedia
    )]
    media: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLStyleElement", enumerable)]
struct HtmlStyleElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = style_type_getter_function,
        setter = style_type_setter_function
    )]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = html_media_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::StyleMedia
    )]
    media: (),
    #[webapi(
        accessor_property,
        getter = style_blocking_getter_function,
        setter = style_blocking_setter_function
    )]
    blocking: (),
    #[webapi(
        accessor_property,
        getter = style_disabled_getter_function,
        setter = style_disabled_setter_function
    )]
    disabled: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SVGStyleElement", enumerable)]
struct SvgStyleElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = svg_style_dom_string_getter_function,
        setter = svg_style_dom_string_setter_function,
        data = callback_data_index_value(scope, 0)
    )]
    media: (),
    #[webapi(
        accessor_property,
        getter = svg_style_dom_string_getter_function,
        setter = svg_style_dom_string_setter_function,
        data = callback_data_index_value(scope, 1)
    )]
    title: (),
    #[webapi(
        accessor_property,
        getter = svg_style_dom_string_getter_function,
        setter = svg_style_dom_string_setter_function,
        data = callback_data_index_value(scope, 2)
    )]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = svg_style_disabled_getter_function,
        setter = svg_style_disabled_setter_function
    )]
    disabled: (),
}

const SVG_STYLE_DOM_STRING_ATTRIBUTES: &[&str] = &["media", "title", "type"];

fn is_svg_style_element(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(|element| {
            element.namespace() == "http://www.w3.org/2000/svg" && element.local_name() == "style"
        })
}

fn svg_style_element_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_getter_receiver(scope, "SVGStyleElement", member);
        return None;
    };
    if !is_svg_style_element(unsafe { &*runtime_ptr }, handle) {
        throw_incompatible_getter_receiver(scope, "SVGStyleElement", member);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn svg_style_element_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_setter_receiver(scope, "SVGStyleElement", member);
        return None;
    };
    if !is_svg_style_element(unsafe { &*runtime_ptr }, handle) {
        throw_incompatible_setter_receiver(scope, "SVGStyleElement", member);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn svg_style_dom_string_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        SVG_STYLE_DOM_STRING_ATTRIBUTES,
        "SVGStyleElement DOMString attributes",
    ) else {
        return;
    };
    let Some((runtime_ptr, handle)) =
        svg_style_element_getter_receiver(scope, args.this(), attribute)
    else {
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, attribute).unwrap_or_default();
    set_element_string_return_value(scope, &mut rv, &value);
}

fn svg_style_dom_string_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        SVG_STYLE_DOM_STRING_ATTRIBUTES,
        "SVGStyleElement DOMString attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let Some((runtime_ptr, handle)) =
        svg_style_element_setter_receiver(scope, args.this(), attribute)
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), "SVGStyleElement", attribute)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value);
    rv.set_undefined();
}

fn svg_style_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if svg_style_element_getter_receiver(scope, args.this(), "disabled").is_none() {
        return;
    }
    style_disabled_getter_function(scope, args, rv);
}

fn svg_style_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if svg_style_element_setter_receiver(scope, args.this(), "disabled").is_none() {
        return;
    }
    style_disabled_setter_function(scope, args, rv);
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLDetailsElement", enumerable)]
struct HtmlDetailsElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = details_open_getter_function,
        setter = details_open_setter_function
    )]
    open: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLDialogElement", enumerable)]
struct HtmlDialogElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dialog_open_getter_function,
        setter = dialog_open_setter_function
    )]
    open: (),
    #[webapi(
        accessor_property,
        getter = dialog_return_value_getter_function,
        setter = dialog_return_value_setter_function
    )]
    return_value: (),
    #[webapi(method, length = 0, callback = dialog_show_callback)]
    show: (),
    #[webapi(method, length = 0, callback = dialog_show_modal_callback)]
    show_modal: (),
    #[webapi(method, length = 1, callback = dialog_close_callback)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLMetaElement", enumerable)]
struct HtmlMetaElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = meta_content_getter_function,
        setter = meta_content_setter_function
    )]
    content: (),
    #[webapi(
        accessor_property = "httpEquiv",
        getter = meta_http_equiv_getter_function,
        setter = meta_http_equiv_setter_function
    )]
    http_equiv: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLMetaElement", enumerable)]
struct HtmlMetaElementMediaPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_media_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::MetaMedia
    )]
    media: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLAnchorElement", enumerable)]
struct HtmlAnchorElementTargetPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = anchor_target_getter_function,
        setter = anchor_target_setter_function
    )]
    target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLAreaElement", enumerable)]
struct HtmlAreaElementTargetPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = area_target_getter_function,
        setter = area_target_setter_function
    )]
    target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLAreaElement", enumerable)]
struct HtmlAreaElementReferrerPolicyPrototypeDeclaration {
    #[webapi(
        accessor_property = "referrerPolicy",
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaReferrerPolicy
    )]
    referrer_policy: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLAreaElement", enumerable)]
struct HtmlAreaElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_alt_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaAlt
    )]
    alt: (),
    #[webapi(
        accessor_property,
        getter = html_coords_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaCoords
    )]
    coords: (),
    #[webapi(
        accessor_property,
        getter = html_download_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaDownload
    )]
    download: (),
    #[webapi(
        accessor_property,
        getter = html_hreflang_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaHreflang
    )]
    hreflang: (),
    #[webapi(
        accessor_property,
        getter = html_shape_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaShape
    )]
    shape: (),
    #[webapi(
        accessor_property,
        getter = html_ping_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::AreaPing
    )]
    ping: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaType
    )]
    r#type: (),
    #[webapi(
        accessor_property = "noHref",
        getter = html_no_href_getter_function,
        setter = area_no_href_setter_function
    )]
    no_href: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLBaseElement", enumerable)]
struct HtmlBaseElementTargetPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = base_target_getter_function,
        setter = base_target_setter_function
    )]
    target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLLinkElement", enumerable)]
struct HtmlLinkElementTargetPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = link_target_getter_function,
        setter = link_target_setter_function
    )]
    target: (),
}

fn install_html_rel_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: ElementReflectionInterface,
) {
    let getter = v8::FunctionTemplate::builder(html_rel_getter_function)
        .length(0)
        .build(scope);
    getter.set_class_name(v8str(scope, "get rel"));
    let setter_data = interface
        .to_v8_template_value(scope)
        .expect("Element reflection interface must convert to V8 template data");
    let setter = v8::FunctionTemplate::builder(html_rel_setter_function)
        .data(setter_data)
        .length(1)
        .build(scope);
    setter.set_class_name(v8str(scope, "set rel"));
    prototype.set_accessor_property(
        v8str(scope, "rel").into(),
        Some(getter),
        Some(setter),
        v8::PropertyAttribute::NONE,
    );

    let getter = v8::FunctionTemplate::builder(html_rel_list_getter_function)
        .length(0)
        .build(scope);
    getter.set_class_name(v8str(scope, "get relList"));
    let setter_data = interface
        .to_v8_template_value(scope)
        .expect("Element reflection interface must convert to V8 template data");
    let setter = v8::FunctionTemplate::builder(html_rel_list_setter_function)
        .data(setter_data)
        .length(1)
        .build(scope);
    setter.set_class_name(v8str(scope, "set relList"));
    prototype.set_accessor_property(
        v8str(scope, "relList").into(),
        Some(getter),
        Some(setter),
        v8::PropertyAttribute::NONE,
    );
}

fn install_html_name_template_binding<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: ElementReflectionInterface,
) {
    let getter = v8::FunctionTemplate::builder(html_name_getter_function)
        .length(0)
        .build(scope);
    getter.set_class_name(v8str(scope, "get name"));
    let setter_data = interface
        .to_v8_template_value(scope)
        .expect("Element reflection interface must convert to V8 template data");
    let setter = v8::FunctionTemplate::builder(html_name_setter_function)
        .data(setter_data)
        .length(1)
        .build(scope);
    setter.set_class_name(v8str(scope, "set name"));
    prototype.set_accessor_property(
        v8str(scope, "name").into(),
        Some(getter),
        Some(setter),
        v8::PropertyAttribute::NONE,
    );
}

const HTML_NAME_REFLECTION_INTERFACES: &[ElementReflectionInterface] = &[
    ElementReflectionInterface::HtmlAnchorElement,
    ElementReflectionInterface::HtmlButtonElement,
    ElementReflectionInterface::HtmlDetailsElement,
    ElementReflectionInterface::HtmlEmbedElement,
    ElementReflectionInterface::HtmlFieldSetElement,
    ElementReflectionInterface::HtmlFrameElement,
    ElementReflectionInterface::HtmlIFrameElement,
    ElementReflectionInterface::HtmlImageElement,
    ElementReflectionInterface::HtmlInputElement,
    ElementReflectionInterface::HtmlMapElement,
    ElementReflectionInterface::HtmlMetaElement,
    ElementReflectionInterface::HtmlObjectElement,
    ElementReflectionInterface::HtmlOutputElement,
    ElementReflectionInterface::HtmlParamElement,
    ElementReflectionInterface::HtmlSelectElement,
    ElementReflectionInterface::HtmlTextAreaElement,
];

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Object", enumerable)]
struct HtmlFormOwnerPrototypeDeclaration {
    #[webapi(accessor_property, getter = form_associated_form_getter_function)]
    form: (),
}

const HTML_FORM_OWNER_REFLECTION_INTERFACES: &[ElementReflectionInterface] = &[
    ElementReflectionInterface::HtmlButtonElement,
    ElementReflectionInterface::HtmlFieldSetElement,
    ElementReflectionInterface::HtmlInputElement,
    ElementReflectionInterface::HtmlObjectElement,
    ElementReflectionInterface::HtmlOutputElement,
    ElementReflectionInterface::HtmlSelectElement,
    ElementReflectionInterface::HtmlTextAreaElement,
];

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "LabelableElement", enumerable)]
struct LabelableElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = control_labels_getter_function)]
    labels: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLFieldSetElement", enumerable)]
struct HtmlFieldSetElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = fieldset_disabled_getter_function,
        setter = fieldset_disabled_setter_function
    )]
    disabled: (),
    #[webapi(accessor_property = "type", getter = fieldset_type_getter_function)]
    type_: (),
    #[webapi(accessor_property, getter = fieldset_elements_getter_function)]
    elements: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLDataListElement", enumerable)]
struct HtmlDataListElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = datalist_options_getter_function)]
    options: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLLegendElement", enumerable)]
struct HtmlLegendElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = legend_form_getter_function)]
    form: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLButtonElement", enumerable)]
struct HtmlButtonElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = button_disabled_getter_function,
        setter = button_disabled_setter_function
    )]
    disabled: (),
    #[webapi(
        accessor_property,
        getter = button_form_action_getter_function,
        setter = button_form_action_setter_function
    )]
    form_action: (),
    #[webapi(
        accessor_property,
        getter = button_form_enctype_getter_function,
        setter = button_form_enctype_setter_function
    )]
    form_enctype: (),
    #[webapi(
        accessor_property,
        getter = button_form_method_getter_function,
        setter = button_form_method_setter_function
    )]
    form_method: (),
    #[webapi(
        accessor_property,
        getter = button_form_no_validate_getter_function,
        setter = button_form_no_validate_setter_function
    )]
    form_no_validate: (),
    #[webapi(
        accessor_property,
        getter = button_form_target_getter_function,
        setter = button_form_target_setter_function
    )]
    form_target: (),
    #[webapi(
        accessor_property = "type",
        getter = button_type_getter_function,
        setter = button_type_setter_function
    )]
    type_: (),
    #[webapi(
        accessor_property,
        getter = button_command_for_element_getter_function,
        setter = button_command_for_element_setter_function
    )]
    command_for_element: (),
    #[webapi(
        accessor_property,
        getter = button_popover_target_element_getter_function,
        setter = button_popover_target_element_setter_function
    )]
    popover_target_element: (),
    #[webapi(
        accessor_property,
        getter = button_popover_target_action_getter_function,
        setter = button_popover_target_action_setter_function
    )]
    popover_target_action: (),
    #[webapi(
        accessor_property,
        getter = button_interest_for_element_getter_function,
        setter = button_interest_for_element_setter_function
    )]
    interest_for_element: (),
    #[webapi(
        accessor_property,
        getter = button_value_getter_function,
        setter = button_value_setter_function
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLInputElement", enumerable)]
struct HtmlInputElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = input_accept_getter_function,
        setter = input_accept_setter_function
    )]
    accept: (),
    #[webapi(
        accessor_property,
        getter = input_alt_getter_function,
        setter = input_alt_setter_function
    )]
    alt: (),
    #[webapi(
        accessor_property,
        getter = input_default_checked_getter_function,
        setter = input_default_checked_setter_function
    )]
    default_checked: (),
    #[webapi(
        accessor_property,
        getter = input_default_value_getter_function,
        setter = input_default_value_setter_function
    )]
    default_value: (),
    #[webapi(
        accessor_property,
        getter = input_disabled_getter_function,
        setter = input_disabled_setter_function
    )]
    disabled: (),
    #[webapi(
        accessor_property,
        getter = input_dir_name_getter_function,
        setter = input_dir_name_setter_function
    )]
    dir_name: (),
    #[webapi(
        accessor_property,
        getter = input_files_getter_function,
        setter = input_files_setter_function
    )]
    files: (),
    #[webapi(
        accessor_property,
        getter = input_form_action_getter_function,
        setter = input_form_action_setter_function
    )]
    form_action: (),
    #[webapi(
        accessor_property,
        getter = input_form_enctype_getter_function,
        setter = input_form_enctype_setter_function
    )]
    form_enctype: (),
    #[webapi(
        accessor_property,
        getter = input_form_method_getter_function,
        setter = input_form_method_setter_function
    )]
    form_method: (),
    #[webapi(
        accessor_property,
        getter = input_form_no_validate_getter_function,
        setter = input_form_no_validate_setter_function
    )]
    form_no_validate: (),
    #[webapi(
        accessor_property,
        getter = input_form_target_getter_function,
        setter = input_form_target_setter_function
    )]
    form_target: (),
    #[webapi(
        accessor_property,
        getter = input_height_getter_function,
        setter = input_height_setter_function
    )]
    height: (),
    #[webapi(accessor_property, getter = input_list_getter_function)]
    list: (),
    #[webapi(
        accessor_property,
        getter = input_max_length_getter_function,
        setter = input_max_length_setter_function
    )]
    max_length: (),
    #[webapi(
        accessor_property,
        getter = input_max_getter_function,
        setter = input_max_setter_function
    )]
    max: (),
    #[webapi(
        accessor_property,
        getter = input_min_length_getter_function,
        setter = input_min_length_setter_function
    )]
    min_length: (),
    #[webapi(
        accessor_property,
        getter = input_min_getter_function,
        setter = input_min_setter_function
    )]
    min: (),
    #[webapi(
        accessor_property,
        getter = input_multiple_getter_function,
        setter = input_multiple_setter_function
    )]
    multiple: (),
    #[webapi(
        accessor_property,
        getter = input_pattern_getter_function,
        setter = input_pattern_setter_function
    )]
    pattern: (),
    #[webapi(
        accessor_property,
        getter = input_placeholder_getter_function,
        setter = input_placeholder_setter_function
    )]
    placeholder: (),
    #[webapi(
        accessor_property,
        getter = input_read_only_getter_function,
        setter = input_read_only_setter_function
    )]
    read_only: (),
    #[webapi(
        accessor_property,
        getter = input_required_getter_function,
        setter = input_required_setter_function
    )]
    required: (),
    #[webapi(
        accessor_property,
        getter = input_size_getter_function,
        setter = input_size_setter_function
    )]
    size: (),
    #[webapi(
        accessor_property,
        getter = input_src_getter_function,
        setter = input_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property = "useMap",
        getter = html_use_map_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::InputUseMap
    )]
    use_map: (),
    #[webapi(
        accessor_property,
        getter = input_step_getter_function,
        setter = input_step_setter_function
    )]
    step: (),
    #[webapi(
        accessor_property = "type",
        getter = input_type_getter_function,
        setter = input_type_setter_function
    )]
    type_: (),
    #[webapi(
        accessor_property,
        getter = input_value_as_date_getter_function,
        setter = input_value_as_date_setter_function
    )]
    value_as_date: (),
    #[webapi(
        accessor_property,
        getter = input_value_as_number_getter_function,
        setter = input_value_as_number_setter_function
    )]
    value_as_number: (),
    #[webapi(
        accessor_property,
        getter = input_value_getter_function,
        setter = input_value_setter_function
    )]
    value: (),
    #[webapi(
        accessor_property,
        getter = input_width_getter_function,
        setter = input_width_setter_function
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = input_checked_getter_function,
        setter = input_checked_setter_function
    )]
    checked: (),
    #[webapi(
        accessor_property,
        getter = input_indeterminate_getter_function,
        setter = input_indeterminate_setter_function
    )]
    indeterminate: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOutputElement", enumerable)]
struct HtmlOutputElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = output_default_value_getter_function,
        setter = output_default_value_setter_function
    )]
    default_value: (),
    #[webapi(
        accessor_property,
        getter = output_value_getter_function,
        setter = output_value_setter_function
    )]
    value: (),
    #[webapi(accessor_property = "type", getter = output_type_getter_function)]
    type_: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLMeterElement", enumerable)]
struct HtmlMeterElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = meter_value_getter_function,
        setter = meter_value_setter_function
    )]
    value: (),
    #[webapi(
        accessor_property,
        getter = meter_min_getter_function,
        setter = meter_min_setter_function
    )]
    min: (),
    #[webapi(
        accessor_property,
        getter = meter_max_getter_function,
        setter = meter_max_setter_function
    )]
    max: (),
    #[webapi(
        accessor_property,
        getter = meter_low_getter_function,
        setter = meter_low_setter_function
    )]
    low: (),
    #[webapi(
        accessor_property,
        getter = meter_high_getter_function,
        setter = meter_high_setter_function
    )]
    high: (),
    #[webapi(
        accessor_property,
        getter = meter_optimum_getter_function,
        setter = meter_optimum_setter_function
    )]
    optimum: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLProgressElement", enumerable)]
struct HtmlProgressElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = progress_value_getter_function,
        setter = progress_value_setter_function
    )]
    value: (),
    #[webapi(
        accessor_property,
        getter = progress_max_getter_function,
        setter = progress_max_setter_function
    )]
    max: (),
    #[webapi(accessor_property, getter = progress_position_getter_function)]
    position: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTextAreaElement", enumerable)]
struct HtmlTextAreaElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = textarea_disabled_getter_function,
        setter = textarea_disabled_setter_function
    )]
    disabled: (),
    #[webapi(
        accessor_property,
        getter = textarea_dir_name_getter_function,
        setter = textarea_dir_name_setter_function
    )]
    dir_name: (),
    #[webapi(
        accessor_property,
        getter = textarea_max_length_getter_function,
        setter = textarea_max_length_setter_function
    )]
    max_length: (),
    #[webapi(
        accessor_property,
        getter = textarea_min_length_getter_function,
        setter = textarea_min_length_setter_function
    )]
    min_length: (),
    #[webapi(
        accessor_property,
        getter = textarea_required_getter_function,
        setter = textarea_required_setter_function
    )]
    required: (),
    #[webapi(accessor_property, getter = textarea_text_length_getter_function)]
    text_length: (),
    #[webapi(accessor_property = "type", getter = textarea_type_getter_function)]
    type_: (),
    #[webapi(
        accessor_property,
        getter = textarea_cols_getter_function,
        setter = textarea_cols_setter_function
    )]
    cols: (),
    #[webapi(
        accessor_property,
        getter = textarea_rows_getter_function,
        setter = textarea_rows_setter_function
    )]
    rows: (),
    #[webapi(
        accessor_property,
        getter = textarea_wrap_getter_function,
        setter = textarea_wrap_setter_function
    )]
    wrap: (),
    #[webapi(
        accessor_property,
        getter = textarea_placeholder_getter_function,
        setter = textarea_placeholder_setter_function
    )]
    placeholder: (),
    #[webapi(
        accessor_property,
        getter = textarea_read_only_getter_function,
        setter = textarea_read_only_setter_function
    )]
    read_only: (),
    #[webapi(
        accessor_property,
        getter = textarea_default_value_getter_function,
        setter = textarea_default_value_setter_function
    )]
    default_value: (),
    #[webapi(
        accessor_property,
        getter = textarea_value_getter_function,
        setter = textarea_value_setter_function
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOptionElement", enumerable)]
struct HtmlOptionElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = option_value_getter_function,
        setter = option_value_setter_function
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOptionElement", enumerable)]
struct HtmlOptionElementStatePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = option_default_selected_getter_function,
        setter = option_default_selected_setter_function
    )]
    default_selected: (),
    #[webapi(
        accessor_property,
        getter = option_disabled_getter_function,
        setter = option_disabled_setter_function
    )]
    disabled: (),
    #[webapi(accessor_property, getter = option_form_getter_function)]
    form: (),
    #[webapi(accessor_property, getter = option_index_getter_function)]
    index: (),
    #[webapi(
        accessor_property,
        getter = option_selected_getter_function,
        setter = option_selected_setter_function
    )]
    selected: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOptGroupElement", enumerable)]
struct HtmlOptGroupElementDisabledPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = optgroup_disabled_getter_function,
        setter = optgroup_disabled_setter_function
    )]
    disabled: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOptGroupElement", enumerable)]
struct HtmlOptGroupElementLabelPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_label_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::OptgroupLabel
    )]
    label: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOptionElement", enumerable)]
struct HtmlOptionElementLabelPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = option_label_getter_function,
        setter = option_label_setter_function
    )]
    label: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTrackElement", enumerable)]
struct HtmlTrackElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "default",
        getter = track_default_getter_function,
        setter = track_default_setter_function
    )]
    default_: (),
    #[webapi(
        accessor_property,
        getter = track_kind_getter_function,
        setter = track_kind_setter_function
    )]
    kind: (),
    #[webapi(
        accessor_property,
        getter = track_src_getter_function,
        setter = track_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = track_srclang_getter_function,
        setter = track_srclang_setter_function
    )]
    srclang: (),
    #[webapi(
        accessor_property,
        getter = html_label_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TrackLabel
    )]
    label: (),
    #[webapi(accessor_property = "readyState", getter = track_ready_state_getter_function)]
    ready_state: (),
    #[webapi(accessor_property, getter = track_text_track_getter_function)]
    track: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLDataElement", enumerable)]
struct HtmlDataElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_value_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::DataValue
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLParamElement", enumerable)]
struct HtmlParamElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_value_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ParamValue
    )]
    value: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ParamType
    )]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = html_value_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ParamValueType
    )]
    value_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLObjectElement", enumerable)]
struct HtmlObjectElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = object_data_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::ObjectData
    )]
    data: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ObjectType
    )]
    r#type: (),
    #[webapi(
        accessor_property = "useMap",
        getter = html_use_map_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ObjectUseMap
    )]
    use_map: (),
    #[webapi(
        accessor_property,
        getter = object_archive_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ObjectArchive
    )]
    archive: (),
    #[webapi(
        accessor_property,
        getter = object_code_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ObjectCode
    )]
    code: (),
    #[webapi(
        accessor_property,
        getter = object_code_base_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::ObjectCodeBase
    )]
    code_base: (),
    #[webapi(
        accessor_property,
        getter = object_code_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ObjectCodeType
    )]
    code_type: (),
    #[webapi(
        accessor_property = "declare",
        getter = object_declare_getter_function,
        setter = object_declare_setter_function
    )]
    declare_attr: (),
    #[webapi(
        accessor_property,
        getter = object_standby_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ObjectStandby
    )]
    standby: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ObjectWidth
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = html_height_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ObjectHeight
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = html_border_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::ObjectBorder
    )]
    border: (),
    #[webapi(
        accessor_property,
        getter = html_hspace_getter_function,
        setter = unsigned_long_reflection_setter_function,
        setter_data = UnsignedLongReflection::ObjectHspace
    )]
    hspace: (),
    #[webapi(
        accessor_property,
        getter = html_vspace_getter_function,
        setter = unsigned_long_reflection_setter_function,
        setter_data = UnsignedLongReflection::ObjectVspace
    )]
    vspace: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLHtmlElement", enumerable)]
struct HtmlHtmlElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_version_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::HtmlVersion
    )]
    version: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLMediaElement", enumerable)]
struct HtmlMediaElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "crossOrigin",
        getter = media_cross_origin_getter_function,
        setter = media_cross_origin_setter_function
    )]
    cross_origin: (),
    #[webapi(
        accessor_property,
        getter = media_loading_getter_function,
        setter = media_loading_setter_function
    )]
    loading: (),
    #[webapi(
        accessor_property,
        getter = media_preload_getter_function,
        setter = media_preload_setter_function
    )]
    preload: (),
    #[webapi(accessor_property, getter = media_paused_getter_function)]
    paused: (),
    #[webapi(
        accessor_property,
        getter = media_src_getter_function,
        setter = media_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = media_volume_getter_function,
        setter = media_volume_setter_function
    )]
    volume: (),
    #[webapi(
        accessor_property,
        getter = media_muted_getter_function,
        setter = media_muted_setter_function
    )]
    muted: (),
    #[webapi(
        accessor_property = "defaultMuted",
        getter = media_default_muted_getter_function,
        setter = media_default_muted_setter_function
    )]
    default_muted: (),
    #[webapi(
        accessor_property = "playbackRate",
        getter = media_playback_rate_getter_function,
        setter = media_playback_rate_setter_function
    )]
    playback_rate: (),
    #[webapi(
        accessor_property = "currentTime",
        getter = media_current_time_getter_function,
        setter = media_current_time_setter_function
    )]
    current_time: (),
    #[webapi(accessor_property, getter = media_duration_getter_function)]
    duration: (),
    #[webapi(accessor_property, getter = media_ended_getter_function)]
    ended: (),
    #[webapi(accessor_property, getter = media_seeking_getter_function)]
    seeking: (),
    #[webapi(accessor_property = "readyState", getter = media_ready_state_getter_function)]
    ready_state: (),
    #[webapi(accessor_property = "networkState", getter = media_network_state_getter_function)]
    network_state: (),
    #[webapi(accessor_property = "textTracks", getter = media_text_tracks_getter_function)]
    text_tracks: (),
    #[webapi(
        accessor_property,
        getter = media_autoplay_getter_function,
        setter = media_autoplay_setter_function
    )]
    autoplay: (),
    #[webapi(
        accessor_property,
        getter = media_controls_getter_function,
        setter = media_controls_setter_function
    )]
    controls: (),
    #[webapi(
        accessor_property = "loop",
        getter = media_loop_getter_function,
        setter = media_loop_setter_function
    )]
    loop_: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLVideoElement", enumerable)]
struct HtmlVideoElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = media_poster_getter_function,
        setter = media_poster_setter_function
    )]
    poster: (),
    #[webapi(
        accessor_property,
        getter = media_width_getter_function,
        setter = media_width_setter_function
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = media_height_getter_function,
        setter = media_height_setter_function
    )]
    height: (),
    #[webapi(
        accessor_property = "playsInline",
        getter = media_plays_inline_getter_function,
        setter = media_plays_inline_setter_function
    )]
    plays_inline: (),
    #[webapi(accessor_property = "videoWidth", getter = media_video_width_getter_function)]
    video_width: (),
    #[webapi(accessor_property = "videoHeight", getter = media_video_height_getter_function)]
    video_height: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLQuoteElement", enumerable)]
struct HtmlQuoteElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_cite_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::QuoteCite
    )]
    cite: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLModElement", enumerable)]
struct HtmlModElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_cite_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::ModCite
    )]
    cite: (),
    #[webapi(
        accessor_property,
        getter = html_date_time_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ModDateTime
    )]
    date_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTimeElement", enumerable)]
struct HtmlTimeElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_date_time_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::HtmlTimeDateTime
    )]
    date_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLPreElement", enumerable)]
struct HtmlPreElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = pre_width_getter_function,
        setter = pre_width_setter_function
    )]
    width: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLBRElement", enumerable)]
struct HtmlBrElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::BrClear
    )]
    clear: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLAnchorElement", enumerable)]
struct HtmlAnchorElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "referrerPolicy",
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AnchorReferrerPolicy
    )]
    referrer_policy: (),
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::AnchorRev
    )]
    rev: (),
    #[webapi(
        accessor_property,
        getter = html_coords_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AnchorCoords
    )]
    coords: (),
    #[webapi(
        accessor_property,
        getter = html_charset_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AnchorCharset
    )]
    charset: (),
    #[webapi(
        accessor_property,
        getter = html_download_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AnchorDownload
    )]
    download: (),
    #[webapi(
        accessor_property,
        getter = html_ping_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::AnchorPing
    )]
    ping: (),
    #[webapi(
        accessor_property,
        getter = html_hreflang_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AnchorHreflang
    )]
    hreflang: (),
    #[webapi(
        accessor_property,
        getter = html_shape_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AnchorShape
    )]
    shape: (),
    #[webapi(
        accessor_property,
        getter = anchor_type_getter_function,
        setter = anchor_type_setter_function
    )]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = anchor_text_getter_function,
        setter = anchor_text_setter_function
    )]
    text: (),
    #[webapi(method = "toString", length = 0, callback = anchor_to_string_callback)]
    to_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTitleElement", enumerable)]
struct HtmlTitleElementTextPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = title_text_getter_function,
        setter = title_text_setter_function
    )]
    text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOptionElement", enumerable)]
struct HtmlOptionElementTextPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = option_text_getter_function,
        setter = option_text_setter_function
    )]
    text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLLabelElement", enumerable)]
struct HtmlLabelElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "htmlFor",
        getter = label_html_for_getter_function,
        setter = label_html_for_setter_function
    )]
    html_for: (),
    #[webapi(accessor_property, getter = label_control_getter_function)]
    control: (),
    #[webapi(accessor_property, getter = label_form_getter_function)]
    form: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLFormElement", enumerable)]
struct HtmlFormElementPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = form_action_getter_function,
        setter = form_action_setter_function
    )]
    action: (),
    #[webapi(
        accessor_property,
        getter = form_accept_charset_getter_function,
        setter = form_accept_charset_setter_function
    )]
    accept_charset: (),
    #[webapi(
        accessor_property,
        getter = form_autocomplete_getter_function,
        setter = form_autocomplete_setter_function
    )]
    autocomplete: (),
    #[webapi(
        accessor_property,
        getter = form_enctype_getter_function,
        setter = form_enctype_setter_function
    )]
    enctype: (),
    #[webapi(
        accessor_property,
        getter = form_encoding_getter_function,
        setter = form_encoding_setter_function
    )]
    encoding: (),
    #[webapi(accessor_property, getter = form_elements_getter_function)]
    elements: (),
    #[webapi(accessor_property, getter = form_length_getter_function)]
    length: (),
    #[webapi(
        accessor_property,
        getter = form_method_getter_function,
        setter = form_method_setter_function
    )]
    method: (),
    #[webapi(
        accessor_property,
        getter = form_name_getter_function,
        setter = form_name_setter_function
    )]
    name: (),
    #[webapi(
        accessor_property,
        getter = form_no_validate_getter_function,
        setter = form_no_validate_setter_function
    )]
    no_validate: (),
    #[webapi(
        accessor_property,
        getter = form_target_getter_function,
        setter = form_target_setter_function
    )]
    target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLMediaElement", enumerable)]
struct HtmlMediaElementPrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = media_play_callback)]
    play: (),
    #[webapi(method, length = 0, callback = media_pause_callback)]
    pause: (),
    #[webapi(method, length = 0, callback = media_load_callback)]
    load: (),
    #[webapi(method, length = 1, callback = media_can_play_type_callback)]
    can_play_type: (),
    #[webapi(method, length = 1, callback = media_add_text_track_callback)]
    add_text_track: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLImageElement", enumerable)]
struct HtmlImageElementPrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = image_decode_callback)]
    decode: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLInputElement", enumerable)]
struct HtmlInputElementPrototypeMethodsDeclaration {
    #[webapi(
        accessor_property,
        getter = input_autocomplete_getter_function,
        setter = input_autocomplete_setter_function
    )]
    autocomplete: (),
    #[webapi(method, length = 0, callback = input_show_picker_callback)]
    show_picker: (),
    #[webapi(method, length = 0, callback = input_step_up_callback)]
    step_up: (),
    #[webapi(method, length = 0, callback = input_step_down_callback)]
    step_down: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextControl", enumerable)]
struct TextControlPrototypeMethodsDeclaration {
    #[webapi(method, length = 2, callback = text_control_set_selection_range_callback)]
    set_selection_range: (),
    #[webapi(method, length = 1, callback = text_control_set_range_text_callback)]
    set_range_text: (),
    #[webapi(method, length = 0, callback = text_control_select_callback)]
    select: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextControlSelection", enumerable)]
struct TextControlSelectionPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = text_control_selection_start_getter_function,
        setter = text_control_selection_start_setter_function
    )]
    selection_start: (),
    #[webapi(
        accessor_property,
        getter = text_control_selection_end_getter_function,
        setter = text_control_selection_end_setter_function
    )]
    selection_end: (),
    #[webapi(
        accessor_property,
        getter = text_control_selection_direction_getter_function,
        setter = text_control_selection_direction_setter_function
    )]
    selection_direction: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTextAreaElement", enumerable)]
struct HtmlTextAreaElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = textarea_autocomplete_getter_function,
        setter = textarea_autocomplete_setter_function
    )]
    autocomplete: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLSelectElement", enumerable)]
struct HtmlSelectElementPrototypeMethodsDeclaration {
    #[webapi(
        accessor_property,
        getter = select_autocomplete_getter_function,
        setter = select_autocomplete_setter_function
    )]
    autocomplete: (),
    #[webapi(
        accessor_property,
        getter = select_length_getter_function,
        setter = select_length_setter_function
    )]
    length: (),
    #[webapi(accessor_property, getter = select_options_getter_function)]
    options: (),
    #[webapi(accessor_property, getter = select_selected_options_getter_function)]
    selected_options: (),
    #[webapi(
        accessor_property,
        getter = select_selected_index_getter_function,
        setter = select_selected_index_setter_function
    )]
    selected_index: (),
    #[webapi(
        accessor_property,
        getter = select_value_getter_function,
        setter = select_value_setter_function
    )]
    value: (),
    #[webapi(
        accessor_property,
        getter = select_disabled_getter_function,
        setter = select_disabled_setter_function
    )]
    disabled: (),
    #[webapi(
        accessor_property,
        getter = select_multiple_getter_function,
        setter = select_multiple_setter_function
    )]
    multiple: (),
    #[webapi(
        accessor_property,
        getter = select_required_getter_function,
        setter = select_required_setter_function
    )]
    required: (),
    #[webapi(
        accessor_property,
        getter = select_size_getter_function,
        setter = select_size_setter_function
    )]
    size: (),
    #[webapi(method, length = 1, callback = select_add_callback)]
    add: (),
    #[webapi(method, length = 1, callback = select_item_callback)]
    item: (),
    #[webapi(method, length = 1, callback = select_named_item_callback)]
    named_item: (),
    #[webapi(method, length = 0, callback = select_remove_callback)]
    remove: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FormControlValidation", enumerable)]
struct FormControlValidationPrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = control_check_validity_callback)]
    check_validity: (),
    #[webapi(method, length = 0, callback = control_report_validity_callback)]
    report_validity: (),
    #[webapi(method, length = 1, callback = control_set_custom_validity_callback)]
    set_custom_validity: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FormControlValidationState", enumerable)]
struct FormControlValidationPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = control_validity_getter_function)]
    validity: (),
    #[webapi(accessor_property, getter = control_validation_message_getter_function)]
    validation_message: (),
    #[webapi(accessor_property, getter = control_will_validate_getter_function)]
    will_validate: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableSectionElement", enumerable)]
struct HtmlTableSectionElementPrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = table_section_insert_row_callback)]
    insert_row: (),
    #[webapi(method, length = 1, callback = table_section_delete_row_callback)]
    delete_row: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableSectionElement", enumerable)]
struct HtmlTableSectionElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = table_section_rows_getter_function)]
    rows: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableSectionElement", enumerable)]
struct HtmlTableSectionElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = table_ch_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableSectionCh
    )]
    ch: (),
    #[webapi(
        accessor_property,
        getter = table_ch_off_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableSectionChOff
    )]
    ch_off: (),
    #[webapi(
        accessor_property,
        getter = table_v_align_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableSectionVAlign
    )]
    v_align: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableRowElement", enumerable)]
struct HtmlTableRowElementPrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = table_row_insert_cell_callback)]
    insert_cell: (),
    #[webapi(method, length = 1, callback = table_row_delete_cell_callback)]
    delete_cell: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableRowElement", enumerable)]
struct HtmlTableRowElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = table_row_index_getter_function)]
    row_index: (),
    #[webapi(accessor_property, getter = table_section_row_index_getter_function)]
    section_row_index: (),
    #[webapi(accessor_property, getter = table_row_cells_getter_function)]
    cells: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableRowElement", enumerable)]
struct HtmlTableRowElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property = "bgColor",
        getter = html_bg_color_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::TableRowBgColor
    )]
    bg_color: (),
    #[webapi(
        accessor_property,
        getter = table_ch_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableRowCh
    )]
    ch: (),
    #[webapi(
        accessor_property,
        getter = table_ch_off_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableRowChOff
    )]
    ch_off: (),
    #[webapi(
        accessor_property,
        getter = table_v_align_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableRowVAlign
    )]
    v_align: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableColElement", enumerable)]
struct HtmlTableColElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = table_col_span_getter_function,
        setter = table_col_span_setter_function
    )]
    span: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableColWidth
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = table_ch_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableColCh
    )]
    ch: (),
    #[webapi(
        accessor_property,
        getter = table_ch_off_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableColChOff
    )]
    ch_off: (),
    #[webapi(
        accessor_property,
        getter = table_v_align_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableColVAlign
    )]
    v_align: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableCellElement", enumerable)]
struct HtmlTableCellElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property = "bgColor",
        getter = html_bg_color_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::TableCellBgColor
    )]
    bg_color: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellWidth
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = html_height_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellHeight
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = table_cell_headers_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellHeaders
    )]
    headers: (),
    #[webapi(
        accessor_property,
        getter = table_cell_abbr_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellAbbr
    )]
    abbr: (),
    #[webapi(
        accessor_property,
        getter = table_cell_axis_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellAxis
    )]
    axis: (),
    #[webapi(
        accessor_property,
        getter = table_cell_scope_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellScope
    )]
    scope: (),
    #[webapi(
        accessor_property = "noWrap",
        getter = table_cell_no_wrap_getter_function,
        setter = table_cell_no_wrap_setter_function
    )]
    no_wrap: (),
    #[webapi(
        accessor_property,
        getter = table_ch_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellCh
    )]
    ch: (),
    #[webapi(
        accessor_property,
        getter = table_ch_off_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellChOff
    )]
    ch_off: (),
    #[webapi(
        accessor_property,
        getter = table_v_align_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::TableCellVAlign
    )]
    v_align: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTableCellElement", enumerable)]
struct HtmlTableCellElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = table_cell_col_span_getter_function,
        setter = table_cell_col_span_setter_function
    )]
    col_span: (),
    #[webapi(
        accessor_property,
        getter = table_cell_row_span_getter_function,
        setter = table_cell_row_span_setter_function
    )]
    row_span: (),
    #[webapi(accessor_property = "cellIndex", getter = table_cell_index_getter_function)]
    cell_index: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ShadowRoot")]
struct ShadowRootPrototypeReflectionDeclaration {
    #[webapi(accessor_property, enumerable, getter = shadow_root_host_getter_function)]
    host: (),
    #[webapi(accessor_property, enumerable, getter = shadow_root_mode_getter_function)]
    mode: (),
    #[webapi(
        accessor_property = "delegatesFocus",
        enumerable,
        getter = shadow_root_delegates_focus_getter_function
    )]
    delegates_focus: (),
    #[webapi(
        accessor_property = "slotAssignment",
        enumerable,
        getter = shadow_root_slot_assignment_getter_function
    )]
    slot_assignment: (),
    #[webapi(accessor_property, enumerable, getter = shadow_root_clonable_getter_function)]
    clonable: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = shadow_root_serializable_getter_function
    )]
    serializable: (),
    #[webapi(
        accessor_property = "referenceTarget",
        enumerable,
        getter = shadow_root_reference_target_getter_function,
        setter = shadow_root_reference_target_setter_function
    )]
    reference_target: (),
    #[webapi(
        accessor_property = "activeElement",
        enumerable,
        getter = shadow_root_active_element_getter_function
    )]
    active_element: (),
    #[webapi(
        accessor_property = "innerHTML",
        enumerable,
        getter = node_inner_html_getter_function,
        setter = node_inner_html_setter_function
    )]
    inner_html: (),
    #[webapi(
        accessor_property = "customElementRegistry",
        enumerable,
        getter = element_custom_element_registry_getter_function
    )]
    custom_element_registry: (),
    #[webapi(accessor_property = "styleSheets", enumerable, getter = shadow_root_style_sheets_getter_function)]
    style_sheets: (),
    #[webapi(
        accessor_property = "adoptedStyleSheets",
        enumerable,
        getter = shadow_root_adopted_style_sheets_getter_function,
        setter = shadow_root_adopted_style_sheets_setter_function
    )]
    adopted_style_sheets: (),
    #[webapi(method = "getHTML", callback = node_get_html_callback)]
    get_html: (),
    #[webapi(method = "setHTMLUnsafe", length = 1, callback = node_set_html_unsafe_callback)]
    set_html_unsafe: (),
    #[webapi(
        method = "elementFromPoint",
        length = 2,
        callback = node_shadow_root_element_from_point_callback
    )]
    element_from_point: (),
    #[webapi(
        method = "elementsFromPoint",
        length = 2,
        callback = node_shadow_root_elements_from_point_callback
    )]
    elements_from_point: (),
    #[webapi(method = "getSelection", callback = shadow_root_get_selection_callback)]
    get_selection: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Text")]
struct TextPrototypeReflectionDeclaration {
    #[webapi(accessor_property = "assignedSlot", enumerable, getter = slot_assigned_slot_getter_function)]
    assigned_slot: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLSlotElement")]
struct HtmlSlotElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = slot_name_getter_function,
        setter = slot_name_setter_function
    )]
    name: (),
    #[webapi(method = "assignedNodes", callback = slot_assigned_nodes_callback)]
    assigned_nodes: (),
    #[webapi(method = "assignedElements", callback = slot_assigned_elements_callback)]
    assigned_elements: (),
    #[webapi(method, callback = slot_assign_callback)]
    assign: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLTemplateElement")]
struct HtmlTemplateElementPrototypeDeclaration {
    #[webapi(accessor_property, enumerable, getter = template_content_getter_function)]
    content: (),

    #[webapi(
        accessor_property = "shadowRootMode",
        enumerable,
        getter = template_shadow_root_mode_getter_function,
        setter = template_shadow_root_mode_setter_function
    )]
    shadow_root_mode: (),
    #[webapi(
        accessor_property = "shadowRootDelegatesFocus",
        enumerable,
        getter = template_shadow_root_delegates_focus_getter_function,
        setter = template_shadow_root_delegates_focus_setter_function
    )]
    shadow_root_delegates_focus: (),
    #[webapi(
        accessor_property = "shadowRootClonable",
        enumerable,
        getter = template_shadow_root_clonable_getter_function,
        setter = template_shadow_root_clonable_setter_function
    )]
    shadow_root_clonable: (),
    #[webapi(
        accessor_property = "shadowRootSerializable",
        enumerable,
        getter = template_shadow_root_serializable_getter_function,
        setter = template_shadow_root_serializable_setter_function
    )]
    shadow_root_serializable: (),
    #[webapi(
        accessor_property = "shadowRootCustomElementRegistry",
        enumerable,
        getter = template_shadow_root_custom_element_registry_getter_function,
        setter = template_shadow_root_custom_element_registry_setter_function
    )]
    shadow_root_custom_element_registry: (),
    #[webapi(
        accessor_property = "shadowRootSlotAssignment",
        enumerable,
        getter = template_shadow_root_slot_assignment_getter_function,
        setter = template_shadow_root_slot_assignment_setter_function
    )]
    shadow_root_slot_assignment: (),
    #[webapi(
        accessor_property = "shadowRootAdoptedStyleSheets",
        enumerable,
        getter = template_shadow_root_adopted_style_sheets_getter_function,
        setter = template_shadow_root_adopted_style_sheets_setter_function
    )]
    shadow_root_adopted_style_sheets: (),
}

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
    if did_set {
        event_handlers::invalidate_event_handler_content_attribute(
            scope,
            runtime_ptr,
            handle,
            name,
        );
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
    if did_set && namespace.is_none() {
        event_handlers::invalidate_event_handler_content_attribute(
            scope,
            runtime_ptr,
            handle,
            local_name,
        );
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
    if did_remove {
        event_handlers::invalidate_event_handler_content_attribute(
            scope,
            runtime_ptr,
            handle,
            name,
        );
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
    if did_remove && namespace.is_none() {
        event_handlers::invalidate_event_handler_content_attribute(
            scope,
            runtime_ptr,
            handle,
            local_name,
        );
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

fn element_id_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if element_setter_receiver(scope, args.this(), "id").is_none() {
        return;
    }
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "id",
        args.get(0),
        "Element",
        "id",
    );
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
    let Some((_runtime_ptr, _handle)) = element_getter_receiver(scope, args.this(), "attributes")
    else {
        rv.set_undefined();
        return;
    };
    let wrapper = super::document::live_named_node_map_wrapper(scope, args.this());
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
        "Element" => install!(
            ElementAriaStringReflectionDeclaration,
            ElementAriaElementReflectionDeclaration,
            ElementPrototypeReflectionDeclaration,
            ElementPrototypeQueryAndAttributeMethodsDeclaration,
            ExtendedElementPrototypeMethodsDeclaration,
            ElementGeometryPrototypeDeclaration,
            ElementStylePrototypeDeclaration,
        ),
        "Document" => install!(DocumentCustomElementRegistryPrototypeDeclaration),
        "HTMLElement" => install!(
            ElementStylePrototypeDeclaration,
            HtmlOrForeignElementPrototypeDeclaration,
            HtmlElementStandardPrototypeDeclaration,
            HtmlElementActionPrototypeDeclaration,
            HtmlElementPopoverPrototypeDeclaration,
            HtmlElementGeometryPrototypeDeclaration,
        ),
        "SVGElement" | "MathMLElement" => install!(
            ElementStylePrototypeDeclaration,
            HtmlOrForeignElementPrototypeDeclaration,
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
        "HTMLBodyElement" => install!(
            HtmlBodyOrFrameSetEventHandlersPrototypeDeclaration,
            HtmlBodyElementLegacyPrototypeDeclaration,
        ),
        "HTMLFrameSetElement" => {
            install!(HtmlBodyOrFrameSetEventHandlersPrototypeDeclaration)
        }
        "HTMLHRElement" => install!(HtmlHrElementLegacyPrototypeDeclaration),
        "HTMLFontElement" => install!(HtmlFontElementLegacyPrototypeDeclaration),
        "HTMLMarqueeElement" => install!(HtmlMarqueeElementLegacyPrototypeDeclaration),
        "HTMLScriptElement" => install!(HtmlScriptElementPrototypeDeclaration),
        "SVGScriptElement" => install!(SvgScriptElementPrototypeDeclaration),
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
            install!(HtmlFormElementPrototypeAccessorsDeclaration);
            install_html_rel_template_bindings(
                scope,
                prototype,
                ElementReflectionInterface::HtmlFormElement,
            );
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

fn aria_element_reference_slot(attribute: &str) -> String {
    format!("__moliAriaElementReference:{attribute}")
}

fn aria_element_reference_is_singular(attribute: &str) -> bool {
    attribute == "aria-activedescendant"
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
    let slot = aria_element_reference_slot(&attribute);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
        return;
    }
    if aria_element_reference_is_singular(&attribute) {
        rv.set_null();
    } else {
        rv.set(v8::Array::new(scope, 0).into());
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
    let slot = aria_element_reference_slot(&attribute);
    set_private_value(scope, args.this(), &slot, args.get(0));
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.set_attribute(scope, runtime_ptr, handle, &attribute, "");
    rv.set_undefined();
}
