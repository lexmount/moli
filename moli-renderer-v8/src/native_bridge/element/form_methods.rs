use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLFormElement, enumerable)]
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
#[webapi(interface = web_api_interfaces::HTMLFormElement, enumerable, receiver)]
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
#[webapi(interface = web_api_interfaces::HTMLMediaElement, enumerable)]
pub(super) struct HtmlMediaElementPrototypeMethodsDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLImageElement, enumerable)]
pub(super) struct HtmlImageElementPrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = image_decode_callback)]
    decode: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLInputElement, enumerable)]
pub(super) struct HtmlInputElementPrototypeMethodsDeclaration {
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
pub(super) struct TextControlPrototypeMethodsDeclaration {
    #[webapi(method, length = 2, callback = text_control_set_selection_range_callback)]
    set_selection_range: (),
    #[webapi(method, length = 1, callback = text_control_set_range_text_callback)]
    set_range_text: (),
    #[webapi(method, length = 0, callback = text_control_select_callback)]
    select: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextControlSelection", enumerable)]
pub(super) struct TextControlSelectionPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLTextAreaElement, enumerable)]
pub(super) struct HtmlTextAreaElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = textarea_autocomplete_getter_function,
        setter = textarea_autocomplete_setter_function
    )]
    autocomplete: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLSelectElement, enumerable)]
pub(super) struct HtmlSelectElementPrototypeMethodsDeclaration {
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
    #[webapi(method, length = 0, callback = select_show_picker_callback)]
    show_picker: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FormControlValidation", enumerable)]
pub(super) struct FormControlValidationPrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = control_check_validity_callback)]
    check_validity: (),
    #[webapi(method, length = 0, callback = control_report_validity_callback)]
    report_validity: (),
    #[webapi(method, length = 1, callback = control_set_custom_validity_callback)]
    set_custom_validity: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FormControlValidationState", enumerable)]
pub(super) struct FormControlValidationPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = control_validity_getter_function)]
    validity: (),
    #[webapi(accessor_property, getter = control_validation_message_getter_function)]
    validation_message: (),
    #[webapi(accessor_property, getter = control_will_validate_getter_function)]
    will_validate: (),
}

pub(super) fn install_html_form_element_prototype_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    HtmlFormElementTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
    HtmlFormElementPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
    install_html_rel_template_bindings(
        scope,
        prototype,
        ElementReflectionInterface::HtmlFormElement,
    );
}
