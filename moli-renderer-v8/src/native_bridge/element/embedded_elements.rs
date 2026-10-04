use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLTrackElement, enumerable, receiver)]
pub(super) struct HtmlTrackElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLDataElement, enumerable)]
pub(super) struct HtmlDataElementValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_value_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::DataValue
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLParamElement, enumerable)]
pub(super) struct HtmlParamElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLObjectElement, enumerable)]
pub(super) struct HtmlObjectElementPrototypeDeclaration {
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
        getter = frame_owner_content_document_getter_function,
        receiver = web_api_interfaces::HTMLObjectElement::is_instance
    )]
    content_document: (),
    #[webapi(
        accessor_property,
        getter = frame_owner_content_window_getter_function,
        receiver = web_api_interfaces::HTMLObjectElement::is_instance
    )]
    content_window: (),
    #[webapi(
        method = "getSVGDocument",
        length = 0,
        callback = super::resource_elements::frame_owner_get_svg_document_function,
        receiver = web_api_interfaces::HTMLObjectElement::is_instance
    )]
    get_svg_document: (),
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
