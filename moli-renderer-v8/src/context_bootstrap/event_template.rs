use super::{
    event_document::{document_create_event_callback, document_has_focus_callback},
    event_legacy::{
        event_init_event_callback, mouse_event_init_callback, text_event_init_callback,
    },
    events::{
        composition_event_init_callback, event_bubbles_getter_function,
        event_cancel_bubble_getter_function, event_cancel_bubble_setter_function,
        event_cancelable_getter_function, event_composed_getter_function,
        event_composed_path_callback, event_current_target_getter_function,
        event_default_prevented_getter_function, event_event_phase_getter_function,
        event_get_modifier_state_callback, event_platform_attribute_getter,
        event_prevent_default_callback, event_return_value_getter_function,
        event_return_value_setter_function, event_src_element_getter_function,
        event_stop_immediate_propagation_callback, event_stop_propagation_callback,
        event_target_getter_function, event_time_stamp_getter_function, event_type_getter_function,
        event_value_attribute_getter, focus_event_related_target_getter_function,
        form_data_event_form_data_getter_function, input_event_get_target_ranges_callback,
        keyboard_event_init_callback, mouse_event_related_target_getter_function,
        pointer_event_get_predicted_events_callback, submit_event_submitter_getter_function,
        ui_event_init_callback, ui_event_pseudo_target_getter_function,
        ui_event_which_getter_function,
    },
    selection_surface::document_get_selection_callback,
    specs::ConstructorSpec,
};
use crate::web_api_interfaces;
use crate::{native_bridge::document, window_host};
use moli_webapi::events::subclasses::EventTemplateDeclaration;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Event, enumerable)]
struct EventBaseTemplateMethodsDeclaration {
    #[webapi(accessor_property = "type", getter = event_type_getter_function)]
    event_type: (),

    #[webapi(accessor_property = "target", getter = event_target_getter_function)]
    target: (),

    #[webapi(
        accessor_property = "currentTarget",
        getter = event_current_target_getter_function
    )]
    current_target: (),

    #[webapi(
        accessor_property = "eventPhase",
        getter = event_event_phase_getter_function
    )]
    event_phase: (),

    #[webapi(accessor_property = "bubbles", getter = event_bubbles_getter_function)]
    bubbles: (),

    #[webapi(
        accessor_property = "cancelable",
        getter = event_cancelable_getter_function
    )]
    cancelable: (),

    #[webapi(
        accessor_property = "defaultPrevented",
        getter = event_default_prevented_getter_function
    )]
    default_prevented: (),

    #[webapi(accessor_property = "composed", getter = event_composed_getter_function)]
    composed: (),

    #[webapi(
        accessor_property = "srcElement",
        getter = event_src_element_getter_function
    )]
    src_element: (),

    #[webapi(constant = "NONE", value = 0u32)]
    none: (),

    #[webapi(constant = "CAPTURING_PHASE", value = 1u32)]
    capturing_phase: (),

    #[webapi(constant = "AT_TARGET", value = 2u32)]
    at_target: (),

    #[webapi(constant = "BUBBLING_PHASE", value = 3u32)]
    bubbling_phase: (),

    #[webapi(
        accessor_property = "cancelBubble",
        getter = event_cancel_bubble_getter_function,
        setter = event_cancel_bubble_setter_function
    )]
    cancel_bubble: (),

    #[webapi(
        accessor_property = "returnValue",
        getter = event_return_value_getter_function,
        setter = event_return_value_setter_function
    )]
    return_value: (),

    #[webapi(accessor_property = "timeStamp", getter = event_time_stamp_getter_function)]
    time_stamp: (),

    #[webapi(method = "preventDefault", length = 0, callback = event_prevent_default_callback)]
    prevent_default: (),

    #[webapi(method = "stopPropagation", length = 0, callback = event_stop_propagation_callback)]
    stop_propagation: (),

    #[webapi(
        method = "stopImmediatePropagation",
        length = 0,
        callback = event_stop_immediate_propagation_callback
    )]
    stop_immediate_propagation: (),

    #[webapi(method = "composedPath", length = 0, callback = event_composed_path_callback)]
    composed_path: (),

    #[webapi(method = "initEvent", length = 1, callback = event_init_event_callback)]
    init_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SubmitEvent, enumerable)]
struct SubmitEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = submit_event_submitter_getter_function)]
    submitter: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TouchEvent, enumerable)]
struct TouchEventTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = event_get_modifier_state_callback)]
    get_modifier_state: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FormDataEvent, enumerable)]
struct FormDataEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "formData", getter = form_data_event_form_data_getter_function)]
    form_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DragEvent, enumerable)]
struct DragEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "dataTransfer", getter = event_platform_attribute_getter, data = crate::util::v8str(scope, "dataTransfer"))]
    data_transfer: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::UIEvent, enumerable)]
struct UiEventTemplateMethodsDeclaration {
    #[webapi(accessor_property, getter = event_platform_attribute_getter, data = crate::util::v8str(scope, "sourceCapabilities"))]
    source_capabilities: (),

    #[webapi(accessor_property = "view", getter = event_platform_attribute_getter, data = crate::util::v8str(scope, "view"))]
    view: (),
    #[webapi(accessor_property = "detail", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "detail"))]
    detail: (),
    #[webapi(accessor_property = "which", getter = ui_event_which_getter_function, data = crate::util::v8str(scope, "which"))]
    which: (),

    #[webapi(method = "initUIEvent", length = 1, callback = ui_event_init_callback)]
    init_ui_event: (),

    #[webapi(
        accessor_property = "pseudoTarget",
        getter = ui_event_pseudo_target_getter_function
    )]
    pseudo_target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FocusEvent, enumerable)]
struct FocusEventTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "relatedTarget",
        getter = focus_event_related_target_getter_function
    )]
    related_target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextEvent, enumerable)]
struct TextEventTemplateMethodsDeclaration {
    #[webapi(accessor_property = "data", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "data"))]
    data: (),

    #[webapi(method = "initTextEvent", length = 1, callback = text_event_init_callback)]
    init_text_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CompositionEvent, enumerable)]
struct CompositionEventTemplateMethodsDeclaration {
    #[webapi(accessor_property = "data", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "data"))]
    data: (),

    #[webapi(
        method = "initCompositionEvent",
        length = 1,
        callback = composition_event_init_callback
    )]
    init_composition_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::InputEvent, enumerable)]
struct InputEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "dataTransfer"))]
    data_transfer: (),

    #[webapi(method, length = 0, callback = input_event_get_target_ranges_callback)]
    get_target_ranges: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "data"))]
    data: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "isComposing"))]
    is_composing: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "inputType"))]
    input_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::KeyboardEvent, enumerable)]
struct KeyboardEventTemplateMethodsDeclaration {
    #[webapi(constant = "DOM_KEY_LOCATION_STANDARD", value = 0u32)]
    dom_key_location_standard: (),

    #[webapi(constant = "DOM_KEY_LOCATION_LEFT", value = 1u32)]
    dom_key_location_left: (),

    #[webapi(constant = "DOM_KEY_LOCATION_RIGHT", value = 2u32)]
    dom_key_location_right: (),

    #[webapi(constant = "DOM_KEY_LOCATION_NUMPAD", value = 3u32)]
    dom_key_location_numpad: (),

    #[webapi(accessor_property = "key", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "key"))]
    key: (),

    #[webapi(accessor_property = "code", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "code"))]
    code: (),

    #[webapi(accessor_property = "location", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "location"))]
    location: (),

    #[webapi(accessor_property = "ctrlKey", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "ctrlKey"))]
    ctrl_key: (),

    #[webapi(accessor_property = "shiftKey", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "shiftKey"))]
    shift_key: (),

    #[webapi(accessor_property = "altKey", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "altKey"))]
    alt_key: (),

    #[webapi(accessor_property = "metaKey", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "metaKey"))]
    meta_key: (),

    #[webapi(accessor_property = "repeat", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "repeat"))]
    repeat: (),

    #[webapi(accessor_property = "isComposing", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "isComposing"))]
    is_composing: (),

    #[webapi(accessor_property = "charCode", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "charCode"))]
    char_code: (),

    #[webapi(accessor_property = "keyCode", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "keyCode"))]
    key_code: (),

    #[webapi(method = "initKeyboardEvent", length = 1, callback = keyboard_event_init_callback)]
    init_keyboard_event: (),

    #[webapi(
        method = "getModifierState",
        length = 1,
        callback = event_get_modifier_state_callback
    )]
    get_modifier_state: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MouseEvent, enumerable)]
struct MouseEventTemplateMethodsDeclaration {
    #[webapi(accessor_property = "screenX", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "screenX"))]
    screen_x: (),
    #[webapi(accessor_property = "screenY", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "screenY"))]
    screen_y: (),
    #[webapi(accessor_property = "clientX", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "clientX"))]
    client_x: (),
    #[webapi(accessor_property = "clientY", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "clientY"))]
    client_y: (),
    #[webapi(accessor_property = "button", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "button"))]
    button: (),
    #[webapi(accessor_property = "buttons", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "buttons"))]
    buttons: (),
    #[webapi(accessor_property = "ctrlKey", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "ctrlKey"))]
    ctrl_key: (),
    #[webapi(accessor_property = "altKey", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "altKey"))]
    alt_key: (),
    #[webapi(accessor_property = "shiftKey", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "shiftKey"))]
    shift_key: (),
    #[webapi(accessor_property = "metaKey", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "metaKey"))]
    meta_key: (),
    #[webapi(accessor_property = "movementX", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "movementX"))]
    movement_x: (),
    #[webapi(accessor_property = "movementY", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "movementY"))]
    movement_y: (),
    #[webapi(accessor_property = "pageX", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "pageX"))]
    page_x: (),
    #[webapi(accessor_property = "pageY", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "pageY"))]
    page_y: (),
    #[webapi(accessor_property = "x", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "x"))]
    x: (),
    #[webapi(accessor_property = "y", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "y"))]
    y: (),

    #[webapi(method = "getModifierState", length = 1, callback = event_get_modifier_state_callback)]
    get_modifier_state: (),

    #[webapi(
        accessor_property = "relatedTarget",
        getter = mouse_event_related_target_getter_function
    )]
    related_target: (),

    #[webapi(accessor_property = "offsetX", getter = window_host::mouse_event_offset_x_getter)]
    offset_x: (),

    #[webapi(accessor_property = "offsetY", getter = window_host::mouse_event_offset_y_getter)]
    offset_y: (),

    #[webapi(method = "initMouseEvent", length = 1, callback = mouse_event_init_callback)]
    init_mouse_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WheelEvent, enumerable)]
struct WheelEventTemplateConstantsDeclaration {
    #[webapi(accessor_property = "deltaMode", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "deltaMode"))]
    delta_mode: (),
    #[webapi(accessor_property = "deltaX", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "deltaX"))]
    delta_x: (),
    #[webapi(accessor_property = "deltaY", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "deltaY"))]
    delta_y: (),
    #[webapi(accessor_property = "deltaZ", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "deltaZ"))]
    delta_z: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "wheelDeltaX"))]
    wheel_delta_x: (),
    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "wheelDeltaY"))]
    wheel_delta_y: (),
    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "wheelDelta"))]
    wheel_delta: (),

    #[webapi(constant = "DOM_DELTA_PIXEL", value = 0u32)]
    dom_delta_pixel: (),

    #[webapi(constant = "DOM_DELTA_LINE", value = 1u32)]
    dom_delta_line: (),

    #[webapi(constant = "DOM_DELTA_PAGE", value = 2u32)]
    dom_delta_page: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PointerEvent, enumerable)]
struct PointerEventTemplateMethodsDeclaration {
    #[webapi(accessor_property = "pointerId", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "pointerId"))]
    pointer_id: (),
    #[webapi(accessor_property = "width", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "width"))]
    width: (),
    #[webapi(accessor_property = "height", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "height"))]
    height: (),
    #[webapi(accessor_property = "pressure", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "pressure"))]
    pressure: (),
    #[webapi(accessor_property = "tangentialPressure", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "tangentialPressure"))]
    tangential_pressure: (),
    #[webapi(accessor_property = "tiltX", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "tiltX"))]
    tilt_x: (),
    #[webapi(accessor_property = "tiltY", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "tiltY"))]
    tilt_y: (),
    #[webapi(accessor_property = "twist", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "twist"))]
    twist: (),
    #[webapi(accessor_property = "altitudeAngle", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "altitudeAngle"))]
    altitude_angle: (),
    #[webapi(accessor_property = "azimuthAngle", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "azimuthAngle"))]
    azimuth_angle: (),
    #[webapi(accessor_property = "pointerType", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "pointerType"))]
    pointer_type: (),
    #[webapi(accessor_property = "isPrimary", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "isPrimary"))]
    is_primary: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "persistentDeviceId"))]
    persistent_device_id: (),

    #[webapi(
        method = "getPredictedEvents",
        length = 0,
        callback = pointer_event_get_predicted_events_callback
    )]
    get_predicted_events: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::EventTarget, enumerable)]
struct EventTargetTemplateMethodsDeclaration {
    #[webapi(
        method = "addEventListener",
        length = 2,
        callback = window_host::event_target_add_event_listener_callback
    )]
    add_event_listener: (),

    #[webapi(
        method = "removeEventListener",
        length = 2,
        callback = window_host::event_target_remove_event_listener_callback
    )]
    remove_event_listener: (),

    #[webapi(
        method = "dispatchEvent",
        length = 1,
        callback = window_host::event_target_dispatch_event_callback
    )]
    dispatch_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Document, enumerable)]
struct DocumentEventTemplateMethodsDeclaration {
    #[webapi(method = "createEvent", length = 1, callback = document_create_event_callback)]
    create_event: (),

    #[webapi(method = "hasFocus", length = 0, callback = document_has_focus_callback)]
    has_focus: (),

    #[webapi(method = "getSelection", length = 0, callback = document_get_selection_callback)]
    get_selection: (),
}

// Installation and wrapper placement share the same native declarations. Public
// prototype mutations cannot change which fields are exposed as own properties.
fn event_template_declaration(interface: &str) -> Option<EventTemplateDeclaration> {
    if let Some(declaration) = moli_webapi::events::subclasses::template_declaration(interface) {
        return Some(declaration);
    }
    Some(match interface {
        "Event" => EventTemplateDeclaration::new::<EventBaseTemplateMethodsDeclaration>(),
        "DragEvent" => EventTemplateDeclaration::new::<DragEventTemplateAccessorsDeclaration>(),
        "UIEvent" => EventTemplateDeclaration::new::<UiEventTemplateMethodsDeclaration>(),
        "FocusEvent" => EventTemplateDeclaration::new::<FocusEventTemplateAccessorsDeclaration>(),
        "TextEvent" => EventTemplateDeclaration::new::<TextEventTemplateMethodsDeclaration>(),
        "CompositionEvent" => {
            EventTemplateDeclaration::new::<CompositionEventTemplateMethodsDeclaration>()
        }
        "InputEvent" => EventTemplateDeclaration::new::<InputEventTemplateAccessorsDeclaration>(),
        "KeyboardEvent" => {
            EventTemplateDeclaration::new::<KeyboardEventTemplateMethodsDeclaration>()
        }
        "MouseEvent" => EventTemplateDeclaration::new::<MouseEventTemplateMethodsDeclaration>(),
        "WheelEvent" => EventTemplateDeclaration::new::<WheelEventTemplateConstantsDeclaration>(),
        "PointerEvent" => EventTemplateDeclaration::new::<PointerEventTemplateMethodsDeclaration>(),
        "TouchEvent" => EventTemplateDeclaration::new::<TouchEventTemplateMethodsDeclaration>(),
        "SubmitEvent" => EventTemplateDeclaration::new::<SubmitEventTemplateAccessorsDeclaration>(),
        "FormDataEvent" => {
            EventTemplateDeclaration::new::<FormDataEventTemplateAccessorsDeclaration>()
        }
        "EventTarget" => EventTemplateDeclaration::new::<EventTargetTemplateMethodsDeclaration>(),
        "Document" => EventTemplateDeclaration::new::<DocumentEventTemplateMethodsDeclaration>(),
        _ => return None,
    })
}

pub(super) fn event_has_prototype_attribute(interface: &str, property: &str) -> bool {
    let mut interface = Some(interface);
    while let Some(name) = interface {
        if name == "TouchEvent"
            && super::touch_runtime::TOUCH_EVENT_PROTOTYPE_ATTRIBUTES.contains(&property)
        {
            return true;
        }
        if event_template_declaration(name)
            .is_some_and(|declaration| declaration.attributes.contains(&property))
        {
            return true;
        }
        interface =
            web_api_interfaces::descriptor(name).and_then(|descriptor| descriptor.parent_name());
    }
    false
}

pub(super) fn install_event_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    spec: ConstructorSpec,
) {
    let interface = spec.interface.name();
    if interface == "SubmitEvent" {
        moli_webapi::events::subclasses::submit::install_template_bindings(scope, template);
    }
    super::events::install_value_event_template_bindings(scope, template, interface);
    super::events::install_device_event_template_bindings(scope, template, interface);
    if interface == "Document" {
        let prototype = template.prototype_template(scope);
        document::install_document_prototype_methods(scope, prototype);
    }
    super::events::speech::install(scope, template, interface);
    if let Some(declaration) = event_template_declaration(interface) {
        (declaration.install)(scope, template);
    }
}
