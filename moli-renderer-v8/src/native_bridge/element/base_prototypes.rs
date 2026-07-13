use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLSelectElement)]
struct HtmlSelectElementIndexedPropertiesDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

pub(crate) fn install_html_select_element_prototype_bindings(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<'_, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    HtmlSelectElementIndexedPropertiesDeclaration::initialize_prototype_template(scope, proto);
}

pub(in crate::native_bridge) const BODY_LEGACY_PROTOTYPE_ACCESSORS: &[&str] = &[
    "onload",
    "onmessageerror",
    "text",
    "link",
    "vLink",
    "aLink",
    "background",
];

pub(crate) fn computed_style_property_for_handle(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> String {
    styles::ComputedStyleRead::new(runtime, handle).property(property)
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Element)]
pub(super) struct ElementPrototypeReflectionDeclaration {
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
        receiver = web_api_interfaces::Element::is_instance,
        callback = node_set_pointer_capture_callback
    )]
    set_pointer_capture: (),
    #[webapi(
        method = "releasePointerCapture",
        length = 1,
        enumerable,
        receiver = web_api_interfaces::Element::is_instance,
        callback = node_release_pointer_capture_callback
    )]
    release_pointer_capture: (),
    #[webapi(
        method = "hasPointerCapture",
        length = 1,
        enumerable,
        receiver = web_api_interfaces::Element::is_instance,
        callback = node_has_pointer_capture_callback
    )]
    has_pointer_capture: (),
    #[webapi(
        method = "requestPointerLock",
        length = 0,
        enumerable,
        callback = super::super::pointer_lock::element_request_pointer_lock_callback
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
#[webapi(interface = web_api_interfaces::Element)]
pub(super) struct ElementPrototypeQueryAndAttributeMethodsDeclaration {
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
#[webapi(interface = web_api_interfaces::Element, enumerable)]
pub(super) struct ExtendedElementPrototypeMethodsDeclaration {
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
#[webapi(interface = web_api_interfaces::Element)]
pub(super) struct ElementGeometryPrototypeDeclaration {
    #[webapi(
        accessor_property = "currentCSSZoom",
        enumerable,
        getter = node_current_css_zoom_getter_function
    )]
    current_css_zoom: (),
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
#[webapi(interface = web_api_interfaces::HTMLElement)]
pub(super) struct HtmlElementGeometryPrototypeDeclaration {
    #[webapi(
        accessor_property = "offsetWidth",
        enumerable,
        receiver = web_api_interfaces::HTMLElement::is_instance,
        getter = node_offset_width_getter_function
    )]
    offset_width: (),
    #[webapi(
        accessor_property = "offsetHeight",
        enumerable,
        receiver = web_api_interfaces::HTMLElement::is_instance,
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
#[webapi(interface = web_api_interfaces::Element, enumerable)]
pub(super) struct ElementAriaStringReflectionDeclaration {
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
#[webapi(interface = web_api_interfaces::Element, enumerable)]
pub(super) struct ElementAriaElementReflectionDeclaration {
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
#[webapi(interface = web_api_interfaces::Document, receiver)]
pub(super) struct DocumentCustomElementRegistryPrototypeDeclaration {
    #[webapi(
        accessor_property = "customElementRegistry",
        enumerable,
        getter = element_custom_element_registry_getter_function
    )]
    custom_element_registry: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Element)]
pub(super) struct ElementStylePrototypeDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_style_getter_function,
        setter = node_style_setter_function
    )]
    style: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLElement, receiver)]
pub(super) struct HtmlElementStandardPrototypeDeclaration {
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
        getter = node_autocorrect_getter_function,
        setter = node_autocorrect_setter_function
    )]
    autocorrect: (),
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
        accessor_property,
        enumerable,
        getter = node_inert_getter_function,
        setter = node_inert_setter_function
    )]
    inert: (),
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
        accessor_property = "writingSuggestions",
        enumerable,
        getter = node_writing_suggestions_getter_function,
        setter = node_writing_suggestions_setter_function
    )]
    writing_suggestions: (),
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
#[webapi(interface = web_api_interfaces::HTMLElement, receiver)]
pub(super) struct HtmlElementActionPrototypeDeclaration {
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
pub(super) struct HtmlOrForeignElementPrototypeDeclaration {
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
        accessor_property = "focusGroup",
        enumerable,
        getter = node_focus_group_getter_function,
        setter = node_focus_group_setter_function
    )]
    focus_group: (),
    #[webapi(
        accessor_property = "focusGroupStart",
        enumerable,
        getter = node_focus_group_start_getter_function,
        setter = node_focus_group_start_setter_function
    )]
    focus_group_start: (),
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
#[webapi(interface = web_api_interfaces::HTMLElement)]
pub(super) struct HtmlElementPopoverPrototypeDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = node_popover_getter_function,
        setter = node_popover_setter_function
    )]
    popover: (),
}

pub(super) fn install_html_align_template_binding<'s>(
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

pub(super) const HTML_ALIGN_REFLECTION_INTERFACES: &[ElementReflectionInterface] = &[
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
pub(super) struct HtmlCompactPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_compact_getter_function,
        setter = html_compact_setter_function
    )]
    compact: (),
}

pub(super) const HTML_COMPACT_REFLECTION_INTERFACES: &[&str] = &[
    "HTMLDirectoryElement",
    "HTMLDListElement",
    "HTMLMenuElement",
    "HTMLOListElement",
    "HTMLUListElement",
];

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLLIElement, enumerable)]
pub(super) struct HtmlLiElementValuePrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLOListElement, enumerable)]
pub(super) struct HtmlOListElementPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLUListElement, enumerable)]
pub(super) struct HtmlUListElementPrototypeDeclaration {
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
pub(super) struct HtmlBodyOrFrameSetEventHandlersPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLBodyElement, enumerable, receiver)]
pub(super) struct HtmlBodyElementLegacyPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLFrameSetElement, enumerable)]
pub(super) struct HtmlFrameSetElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::FrameSetCols
    )]
    cols: (),
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::FrameSetRows
    )]
    rows: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLHRElement, enumerable)]
pub(super) struct HtmlHrElementLegacyPrototypeDeclaration {
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
#[webapi(interface = web_api_interfaces::HTMLFontElement, enumerable)]
pub(super) struct HtmlFontElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::FontFace
    )]
    face: (),
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
#[webapi(interface = web_api_interfaces::HTMLMarqueeElement, enumerable)]
pub(super) struct HtmlMarqueeElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property = "trueSpeed",
        getter = html_true_speed_getter_function,
        setter = html_true_speed_setter_function
    )]
    true_speed: (),
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
#[webapi(interface = web_api_interfaces::HTMLTableElement)]
pub(super) struct HtmlTableElementPrototypeDeclaration {
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
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::TableFrame
    )]
    frame: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::TableRules
    )]
    rules: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::TableSummary
    )]
    summary: (),
    #[webapi(
        accessor_property = "cellPadding",
        enumerable,
        getter = null_to_empty_dom_string_reflection_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        data = NullToEmptyDomStringReflection::TableCellPadding
    )]
    cell_padding: (),
    #[webapi(
        accessor_property = "cellSpacing",
        enumerable,
        getter = null_to_empty_dom_string_reflection_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        data = NullToEmptyDomStringReflection::TableCellSpacing
    )]
    cell_spacing: (),
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
