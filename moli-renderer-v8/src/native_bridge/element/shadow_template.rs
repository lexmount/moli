use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ShadowRoot)]
pub(super) struct ShadowRootPrototypeReflectionDeclaration {
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
#[webapi(interface = web_api_interfaces::Text)]
pub(super) struct TextPrototypeReflectionDeclaration {
    #[webapi(accessor_property = "assignedSlot", enumerable, getter = slot_assigned_slot_getter_function)]
    assigned_slot: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLSlotElement)]
pub(super) struct HtmlSlotElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLTemplateElement)]
pub(super) struct HtmlTemplateElementPrototypeDeclaration {
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
