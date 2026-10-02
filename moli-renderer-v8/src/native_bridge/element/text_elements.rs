use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLQuoteElement, enumerable)]
pub(super) struct HtmlQuoteElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_cite_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::QuoteCite
    )]
    cite: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLModElement, enumerable)]
pub(super) struct HtmlModElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLTimeElement, enumerable)]
pub(super) struct HtmlTimeElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_date_time_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::HtmlTimeDateTime
    )]
    date_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLPreElement, enumerable)]
pub(super) struct HtmlPreElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = pre_width_getter_function,
        setter = pre_width_setter_function
    )]
    width: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLBRElement, enumerable)]
pub(super) struct HtmlBrElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::BrClear
    )]
    clear: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLAnchorElement, enumerable)]
pub(super) struct HtmlAnchorElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLTitleElement, enumerable)]
pub(super) struct HtmlTitleElementTextPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = title_text_getter_function,
        setter = title_text_setter_function
    )]
    text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLOptionElement, enumerable)]
pub(super) struct HtmlOptionElementTextPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = option_text_getter_function,
        setter = option_text_setter_function
    )]
    text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLLabelElement, enumerable, receiver)]
pub(super) struct HtmlLabelElementPrototypeDeclaration {
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
