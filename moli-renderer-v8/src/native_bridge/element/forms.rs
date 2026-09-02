use crate::dom::{
    forms::InputType,
    native::{Element, Node},
};
use crate::{
    construct_form_data_entries_for_form, custom_elements,
    document_runtime::DomHandle,
    form_data_entries_multipart_body_with_prefix, form_data_entries_to_string_pairs,
    form_data_object_from_entries,
    util::{throw_type_error, v8_string},
    webidl,
};

use super::super::{
    CollectionKind, JsContextHost, LiveCollectionDescriptor, LiveCollectionQueryKind, collections,
    node::{
        append_child_in_reaction_scope, append_child_to_current_reaction_queue,
        current_or_live_delegate_node_arg_handle, insert_before_in_reaction_scope,
        insert_before_to_current_reaction_queue, node_is_element, node_or_foreign_arg_handle,
        node_or_foreign_arg_handle_allow_detached, node_runtime_and_handle_from_args,
        node_runtime_and_handle_from_args_or_detached, node_runtime_and_handle_from_object,
        node_runtime_and_handle_from_object_or_detached, remove_child_in_reaction_scope,
        remove_child_to_current_reaction_queue, set_text_content_in_reaction_scope,
        set_wrapped_node_or_null,
    },
    throw_dom_exception, webidl_long_from_number,
};
use super::{
    attribute_property_getter_from_object_or_detached,
    boolean_attribute_property_getter_from_object_or_detached, close_dialog_element,
    construct_simple_event, construct_submit_event, dispatch_public_event, element_attribute,
    element_has_attribute, html_element_getter_receiver, html_element_setter_receiver,
    navigate_form_target_browsing_context, parse_non_negative_dimension, property_usv_string_value,
    queue_deferred_named_iframe_target_navigation_from_document,
    queue_deferred_named_iframe_target_request, reflected_element_attribute_handle,
    resolve_url_like_attribute, set_attribute_property_on_object_or_detached,
    set_reflected_attribute, set_reflected_boolean_attribute, update_focus,
};
use std::str::FromStr;

mod autocomplete;
mod form_element;
mod input;
mod labels;
mod owner;
mod select;
mod simple_controls;
mod submission;
mod text_control;
mod validation;

pub(in crate::native_bridge::element) fn form_dom_string_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
    treat_null_as_empty_string: bool,
) -> Option<String> {
    let options = webidl::StringOptions {
        treat_null_as_empty_string,
    };
    match webidl::convert_with_options::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member(owner, property),
        &options,
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(in crate::native_bridge::element) fn set_form_dom_string_attribute_property_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) {
    let Some(value) = form_dom_string_property_value(scope, value, owner, property, false) else {
        return;
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value);
}

pub(in crate::native_bridge::element) fn textarea_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_getter_receiver(scope, receiver, "HTMLTextAreaElement", member, "textarea")
}

pub(in crate::native_bridge::element) fn textarea_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_setter_receiver(scope, receiver, "HTMLTextAreaElement", member, "textarea")
}

pub(crate) use self::autocomplete::autocomplete_field_name;
pub(in crate::native_bridge) use self::autocomplete::{
    input_autocomplete_getter_function, input_autocomplete_setter_function,
    select_autocomplete_getter_function, select_autocomplete_setter_function,
    textarea_autocomplete_getter_function, textarea_autocomplete_setter_function,
};

pub(crate) use self::form_element::{
    autofill_related_form_control_elements, form_control_elements, form_data_control_elements,
};
pub(in crate::native_bridge) use self::form_element::{
    fieldset_elements_getter_function, form_accept_charset_getter_function,
    form_accept_charset_setter_function, form_action_getter_function, form_action_setter_function,
    form_autocomplete_getter_function, form_autocomplete_setter_function,
    form_elements_getter_function, form_encoding_getter_function, form_encoding_setter_function,
    form_enctype_getter_function, form_enctype_setter_function, form_indexed_definer,
    form_indexed_deleter, form_indexed_descriptor, form_indexed_enumerator, form_indexed_getter,
    form_indexed_query, form_indexed_setter, form_length_getter_function,
    form_method_getter_function, form_method_setter_function, form_name_getter_function,
    form_name_setter_function, form_named_definer, form_named_deleter, form_named_descriptor,
    form_named_getter, form_named_query, form_no_validate_getter_function,
    form_no_validate_setter_function, form_target_getter_function, form_target_setter_function,
};
pub(crate) use self::input::cache_input_files_from_selected_files;
pub(in crate::native_bridge) use self::input::{
    input_accept_getter_function, input_accept_setter_function, input_alt_getter_function,
    input_alt_setter_function, input_checked_getter_function, input_checked_setter_function,
    input_default_checked_getter_function, input_default_checked_setter_function,
    input_default_value_getter_function, input_default_value_setter_function,
    input_dir_name_getter_function, input_dir_name_setter_function, input_disabled_getter_function,
    input_disabled_setter_function, input_files_getter_function, input_files_setter_function,
    input_form_action_getter_function, input_form_action_setter_function,
    input_form_enctype_getter_function, input_form_enctype_setter_function,
    input_form_method_getter_function, input_form_method_setter_function,
    input_form_no_validate_getter_function, input_form_no_validate_setter_function,
    input_form_target_getter_function, input_form_target_setter_function,
    input_height_getter_function, input_height_setter_function,
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
    textarea_cols_getter_function, textarea_cols_setter_function,
    textarea_default_value_getter_function, textarea_default_value_setter_function,
    textarea_dir_name_getter_function, textarea_dir_name_setter_function,
    textarea_disabled_getter_function, textarea_disabled_setter_function,
    textarea_max_length_getter_function, textarea_max_length_setter_function,
    textarea_min_length_getter_function, textarea_min_length_setter_function,
    textarea_placeholder_getter_function, textarea_placeholder_setter_function,
    textarea_read_only_getter_function, textarea_read_only_setter_function,
    textarea_required_getter_function, textarea_required_setter_function,
    textarea_rows_getter_function, textarea_rows_setter_function,
    textarea_text_length_getter_function, textarea_type_getter_function,
    textarea_wrap_getter_function, textarea_wrap_setter_function,
};
pub(in crate::native_bridge) use self::labels::{
    control_label_handles, control_labels_getter_function, label_activation_control_handle,
    label_control_getter_function, label_control_handle, label_form_getter_function,
    label_html_for_getter_function, label_html_for_setter_function,
    label_receives_programmatic_focus,
};
pub(in crate::native_bridge) use self::owner::form_associated_form_getter_function;
pub(crate) use self::owner::{
    form_associated_form_owner, form_control_is_effectively_disabled, is_valid_submit_button,
};
pub(in crate::native_bridge) use self::select::{
    option_default_selected_getter_function, option_default_selected_setter_function,
    option_disabled_getter_function, option_disabled_setter_function, option_form_getter_function,
    option_index_getter_function, option_label_getter_function, option_label_setter_function,
    option_selected_getter_function, option_selected_setter_function, option_text_getter_function,
    option_text_setter_function, option_value_getter_function, option_value_setter_function,
    resize_select_options, select_add_callback, select_add_insertion_point,
    select_disabled_getter_function, select_disabled_setter_function, select_indexed_definer,
    select_indexed_deleter, select_indexed_descriptor, select_indexed_enumerator,
    select_indexed_getter, select_indexed_query, select_indexed_setter, select_item_callback,
    select_length_getter_function, select_length_setter_function, select_multiple_getter_function,
    select_multiple_setter_function, select_named_item_callback, select_options_getter_function,
    select_options_resize_target, select_remove_callback, select_required_getter_function,
    select_required_setter_function, select_selected_index_getter_function,
    select_selected_index_setter_function, select_selected_options_getter_function,
    select_size_getter_function, select_size_setter_function, select_value_getter_function,
    select_value_setter_function, set_select_indexed_option,
};
pub(in crate::native_bridge::element) use self::simple_controls::normalized_button_command;
pub(in crate::native_bridge) use self::simple_controls::{
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
    button_value_setter_function, datalist_options_getter_function,
    fieldset_disabled_getter_function, fieldset_disabled_setter_function,
    fieldset_type_getter_function, legend_form_getter_function, meter_high_getter_function,
    meter_high_setter_function, meter_low_getter_function, meter_low_setter_function,
    meter_max_getter_function, meter_max_setter_function, meter_min_getter_function,
    meter_min_setter_function, meter_optimum_getter_function, meter_optimum_setter_function,
    meter_value_getter_function, meter_value_setter_function, output_default_value_getter_function,
    output_default_value_setter_function, output_type_getter_function,
    output_value_getter_function, output_value_setter_function, progress_max_getter_function,
    progress_max_setter_function, progress_position_getter_function,
    progress_value_getter_function, progress_value_setter_function,
};
pub(in crate::native_bridge) use self::submission::{
    FormAssociatedResetCallbackTiming, form_request_submit_callback, form_reset_callback,
    form_submit_callback, reset_form_default_action, submit_form_with_submit_event,
};
pub(crate) use self::submission::{
    align_event_constructor_function_realm_with_constructor,
    align_event_constructor_function_realm_with_target,
};
pub(crate) use self::text_control::{
    dispatch_text_control_event, is_text_control,
    queue_text_control_document_selection_change_event, replace_text_control_selection,
    text_control_set_selection_range_internal,
    text_control_set_selection_range_with_direction_internal, text_control_value,
};
pub(in crate::native_bridge) use self::text_control::{
    text_control_select_callback, text_control_selection_direction_getter_function,
    text_control_selection_direction_setter_function, text_control_selection_end_getter_function,
    text_control_selection_end_setter_function, text_control_selection_start_getter_function,
    text_control_selection_start_setter_function, text_control_set_range_text_callback,
    text_control_set_selection_range_callback, textarea_value_getter_function,
    textarea_value_setter_function,
};
pub(in crate::native_bridge) use self::validation::{
    control_check_validity_callback, control_matches_validity_pseudo,
    control_report_validity_callback, control_set_custom_validity_callback,
    control_validation_message_getter_function, control_validity_getter_function,
    control_validity_pseudo_state, control_will_validate_getter_function, dispatch_invalid_event,
    form_check_validity_callback, form_report_validity_callback, form_validate_for_submission,
};

#[derive(strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
enum FormMethod {
    Get,
    Post,
    Dialog,
}

fn normalized_form_method(value: &str) -> &'static str {
    FormMethod::from_str(value.trim())
        .map(Into::into)
        .unwrap_or("get")
}

#[derive(strum::EnumString, strum::IntoStaticStr)]
#[strum(ascii_case_insensitive)]
enum FormEnctype {
    #[strum(serialize = "application/x-www-form-urlencoded")]
    UrlEncoded,
    #[strum(serialize = "multipart/form-data")]
    MultipartFormData,
    #[strum(serialize = "text/plain")]
    TextPlain,
}

fn normalized_form_enctype(value: &str) -> &'static str {
    FormEnctype::from_str(value.trim())
        .map(Into::into)
        .unwrap_or("application/x-www-form-urlencoded")
}

pub(super) fn node_direct_text_content(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let dom = runtime.dom_host().dom();
    dom.node(handle).map(|node| node.direct_text_content(dom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_form_method_uses_html_form_tokens() {
        assert_eq!(normalized_form_method("get"), "get");
        assert_eq!(normalized_form_method("POST"), "post");
        assert_eq!(normalized_form_method(" dialog "), "dialog");
        assert_eq!(normalized_form_method("put"), "get");
    }

    #[test]
    fn normalized_form_enctype_uses_html_form_tokens() {
        assert_eq!(
            normalized_form_enctype("application/x-www-form-urlencoded"),
            "application/x-www-form-urlencoded"
        );
        assert_eq!(
            normalized_form_enctype("MULTIPART/FORM-DATA"),
            "multipart/form-data"
        );
        assert_eq!(normalized_form_enctype(" text/plain "), "text/plain");
        assert_eq!(
            normalized_form_enctype("application/json"),
            "application/x-www-form-urlencoded"
        );
    }
}
