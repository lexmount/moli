use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Object", enumerable)]
pub(super) struct HtmlFormOwnerPrototypeDeclaration {
    #[webapi(accessor_property, getter = form_associated_form_getter_function)]
    form: (),
}

pub(super) const HTML_FORM_OWNER_REFLECTION_INTERFACES: &[ElementReflectionInterface] = &[
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
pub(super) struct LabelableElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = control_labels_getter_function)]
    labels: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLFieldSetElement, enumerable)]
pub(super) struct HtmlFieldSetElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLDataListElement, enumerable)]
pub(super) struct HtmlDataListElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = datalist_options_getter_function)]
    options: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLLegendElement, enumerable)]
pub(super) struct HtmlLegendElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = legend_form_getter_function)]
    form: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLButtonElement, enumerable)]
pub(super) struct HtmlButtonElementValuePrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLInputElement, enumerable)]
pub(super) struct HtmlInputElementValuePrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLOutputElement, enumerable)]
pub(super) struct HtmlOutputElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property = "htmlFor",
        getter = output_html_for_getter_function,
        setter = output_html_for_setter_function
    )]
    html_for: (),
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
#[webapi(interface = web_api_interfaces::SVGAElement, enumerable)]
struct SvgAElementRelListPrototypeDeclaration {
    #[webapi(
        accessor_property = "relList",
        getter = html_rel_list_getter_function,
        setter = svg_rel_list_setter_function
    )]
    rel_list: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLMeterElement, enumerable)]
pub(super) struct HtmlMeterElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLProgressElement, enumerable)]
pub(super) struct HtmlProgressElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLTextAreaElement, enumerable)]
pub(super) struct HtmlTextAreaElementValuePrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLOptionElement, enumerable)]
pub(super) struct HtmlOptionElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = option_value_getter_function,
        setter = option_value_setter_function
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLOptionElement, enumerable)]
pub(super) struct HtmlOptionElementStatePrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLOptGroupElement, enumerable)]
pub(super) struct HtmlOptGroupElementDisabledPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = optgroup_disabled_getter_function,
        setter = optgroup_disabled_setter_function
    )]
    disabled: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLOptGroupElement, enumerable)]
pub(super) struct HtmlOptGroupElementLabelPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_label_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::OptgroupLabel
    )]
    label: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLOptionElement, enumerable)]
pub(super) struct HtmlOptionElementLabelPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = option_label_getter_function,
        setter = option_label_setter_function
    )]
    label: (),
}
