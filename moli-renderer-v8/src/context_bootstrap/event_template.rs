use super::{
    event_document::{document_create_event_callback, document_has_focus_callback},
    event_legacy::{
        custom_event_init_callback, event_init_event_callback, mouse_event_init_callback,
        storage_event_init_callback, text_event_init_callback,
    },
    events::{
        before_unload_event_return_value_getter_function,
        before_unload_event_return_value_setter_function,
        clipboard_change_event_change_id_getter_function,
        clipboard_change_event_types_getter_function,
        clipboard_event_clipboard_data_getter_function, close_event_code_getter_function,
        close_event_reason_getter_function, close_event_was_clean_getter_function,
        command_event_command_getter_function, command_event_source_getter_function,
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
        form_data_event_form_data_getter_function, keyboard_event_init_callback,
        message_event_init_callback, mouse_event_related_target_getter_function,
        pointer_event_get_predicted_events_callback, submit_event_submitter_getter_function,
        toggle_event_source_getter_function, track_event_track_getter_function,
        ui_event_init_callback, ui_event_pseudo_target_getter_function,
        ui_event_which_getter_function,
    },
    selection_surface::document_get_selection_callback,
    specs::ConstructorSpec,
};
use crate::web_api_interfaces;
use crate::{native_bridge::document, window_host};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiFunctionTemplateDeclaration};

pub(in crate::context_bootstrap) fn object_is_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::EventTarget::is_instance(scope, object)
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Event, enumerable, receiver)]
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
#[webapi(interface = web_api_interfaces::BeforeUnloadEvent, enumerable, receiver)]
struct BeforeUnloadEventTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "returnValue",
        getter = before_unload_event_return_value_getter_function,
        setter = before_unload_event_return_value_setter_function
    )]
    return_value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CloseEvent, enumerable, receiver)]
struct CloseEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "wasClean", getter = close_event_was_clean_getter_function)]
    was_clean: (),

    #[webapi(accessor_property, getter = close_event_code_getter_function)]
    code: (),

    #[webapi(accessor_property, getter = close_event_reason_getter_function)]
    reason: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ClipboardEvent, enumerable, receiver)]
struct ClipboardEventTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "clipboardData",
        getter = clipboard_event_clipboard_data_getter_function
    )]
    clipboard_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ClipboardChangeEvent, enumerable, receiver)]
struct ClipboardChangeEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = clipboard_change_event_types_getter_function)]
    types: (),

    #[webapi(
        accessor_property = "changeId",
        getter = clipboard_change_event_change_id_getter_function
    )]
    change_id: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TrackEvent, enumerable, receiver)]
struct TrackEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = track_event_track_getter_function)]
    track: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SubmitEvent, enumerable, receiver)]
struct SubmitEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = submit_event_submitter_getter_function)]
    submitter: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FormDataEvent, enumerable, receiver)]
struct FormDataEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "formData", getter = form_data_event_form_data_getter_function)]
    form_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CommandEvent, enumerable, receiver)]
struct CommandEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = command_event_source_getter_function)]
    source: (),

    #[webapi(accessor_property, getter = command_event_command_getter_function)]
    command: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ToggleEvent, enumerable, receiver)]
struct ToggleEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = toggle_event_source_getter_function)]
    source: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::UIEvent, enumerable, receiver)]
struct UiEventTemplateMethodsDeclaration {
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
#[webapi(interface = web_api_interfaces::FocusEvent, enumerable, receiver)]
struct FocusEventTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "relatedTarget",
        getter = focus_event_related_target_getter_function
    )]
    related_target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextEvent, enumerable, receiver)]
struct TextEventTemplateMethodsDeclaration {
    #[webapi(accessor_property = "data", getter = event_value_attribute_getter, data = crate::util::v8str(scope, "data"))]
    data: (),

    #[webapi(method = "initTextEvent", length = 1, callback = text_event_init_callback)]
    init_text_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CompositionEvent, enumerable, receiver)]
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
#[webapi(interface = web_api_interfaces::CustomEvent, enumerable, receiver)]
struct CustomEventTemplateMethodsDeclaration {
    #[webapi(method = "initCustomEvent", length = 1, callback = custom_event_init_callback)]
    init_custom_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::InputEvent, enumerable, receiver)]
struct InputEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "data"))]
    data: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "isComposing"))]
    is_composing: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = crate::util::v8str(scope, "inputType"))]
    input_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::StorageEvent, enumerable, receiver)]
struct StorageEventTemplateMethodsDeclaration {
    #[webapi(
        method = "initStorageEvent",
        length = 1,
        callback = storage_event_init_callback
    )]
    init_storage_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MessageEvent, enumerable, receiver)]
struct MessageEventTemplateMethodsDeclaration {
    #[webapi(method = "initMessageEvent", length = 1, callback = message_event_init_callback)]
    init_message_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::KeyboardEvent, enumerable, receiver)]
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
#[webapi(interface = web_api_interfaces::MouseEvent, enumerable, receiver)]
struct MouseEventTemplateMethodsDeclaration {
    #[webapi(
        method = "getModifierState",
        length = 1,
        callback = event_get_modifier_state_callback
    )]
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
    #[webapi(constant = "DOM_DELTA_PIXEL", value = 0u32)]
    dom_delta_pixel: (),

    #[webapi(constant = "DOM_DELTA_LINE", value = 1u32)]
    dom_delta_line: (),

    #[webapi(constant = "DOM_DELTA_PAGE", value = 2u32)]
    dom_delta_page: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PointerEvent, enumerable, receiver)]
struct PointerEventTemplateMethodsDeclaration {
    #[webapi(
        method = "getPredictedEvents",
        length = 0,
        callback = pointer_event_get_predicted_events_callback
    )]
    get_predicted_events: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::EventTarget, enumerable, receiver)]
struct EventTargetTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = crate::observable::event_target_when)]
    when: (),

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
#[webapi(interface = web_api_interfaces::Document, enumerable, receiver)]
struct DocumentEventTemplateMethodsDeclaration {
    #[webapi(method = "createEvent", length = 1, callback = document_create_event_callback)]
    create_event: (),

    #[webapi(method = "hasFocus", length = 0, callback = document_has_focus_callback)]
    has_focus: (),

    #[webapi(method = "getSelection", length = 0, callback = document_get_selection_callback)]
    get_selection: (),
}

struct EventTemplateDeclaration {
    attributes: &'static [&'static str],
    install: for<'s, 'p> fn(&mut v8::PinScope<'s, 'p, ()>, v8::Local<'s, v8::FunctionTemplate>),
}

impl EventTemplateDeclaration {
    fn new<D: WebApiFunctionTemplateDeclaration>() -> Self {
        Self {
            attributes: D::PROTOTYPE_ATTRIBUTE_NAMES,
            install: install_declaration::<D>,
        }
    }
}

fn install_declaration<'s, D: WebApiFunctionTemplateDeclaration>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    D::initialize_template(scope, template);
    let prototype = template.prototype_template(scope);
    D::initialize_prototype_template(scope, prototype);
}

// Installation and wrapper placement share the same native declarations. Public
// prototype mutations cannot change which fields are exposed as own properties.
fn event_template_declaration(interface: &str) -> Option<EventTemplateDeclaration> {
    Some(match interface {
        "Event" => EventTemplateDeclaration::new::<EventBaseTemplateMethodsDeclaration>(),
        "BeforeUnloadEvent" => {
            EventTemplateDeclaration::new::<BeforeUnloadEventTemplateAccessorsDeclaration>()
        }
        "UIEvent" => EventTemplateDeclaration::new::<UiEventTemplateMethodsDeclaration>(),
        "FocusEvent" => EventTemplateDeclaration::new::<FocusEventTemplateAccessorsDeclaration>(),
        "TextEvent" => EventTemplateDeclaration::new::<TextEventTemplateMethodsDeclaration>(),
        "CompositionEvent" => {
            EventTemplateDeclaration::new::<CompositionEventTemplateMethodsDeclaration>()
        }
        "CustomEvent" => EventTemplateDeclaration::new::<CustomEventTemplateMethodsDeclaration>(),
        "InputEvent" => EventTemplateDeclaration::new::<InputEventTemplateAccessorsDeclaration>(),
        "StorageEvent" => EventTemplateDeclaration::new::<StorageEventTemplateMethodsDeclaration>(),
        "MessageEvent" => EventTemplateDeclaration::new::<MessageEventTemplateMethodsDeclaration>(),
        "KeyboardEvent" => {
            EventTemplateDeclaration::new::<KeyboardEventTemplateMethodsDeclaration>()
        }
        "MouseEvent" => EventTemplateDeclaration::new::<MouseEventTemplateMethodsDeclaration>(),
        "WheelEvent" => EventTemplateDeclaration::new::<WheelEventTemplateConstantsDeclaration>(),
        "PointerEvent" => EventTemplateDeclaration::new::<PointerEventTemplateMethodsDeclaration>(),
        "CloseEvent" => EventTemplateDeclaration::new::<CloseEventTemplateAccessorsDeclaration>(),
        "ClipboardEvent" => {
            EventTemplateDeclaration::new::<ClipboardEventTemplateAccessorsDeclaration>()
        }
        "ClipboardChangeEvent" => {
            EventTemplateDeclaration::new::<ClipboardChangeEventTemplateAccessorsDeclaration>()
        }
        "TrackEvent" => EventTemplateDeclaration::new::<TrackEventTemplateAccessorsDeclaration>(),
        "SubmitEvent" => EventTemplateDeclaration::new::<SubmitEventTemplateAccessorsDeclaration>(),
        "FormDataEvent" => {
            EventTemplateDeclaration::new::<FormDataEventTemplateAccessorsDeclaration>()
        }
        "CommandEvent" => {
            EventTemplateDeclaration::new::<CommandEventTemplateAccessorsDeclaration>()
        }
        "ToggleEvent" => EventTemplateDeclaration::new::<ToggleEventTemplateAccessorsDeclaration>(),
        "EventTarget" => EventTemplateDeclaration::new::<EventTargetTemplateMethodsDeclaration>(),
        "Document" => EventTemplateDeclaration::new::<DocumentEventTemplateMethodsDeclaration>(),
        _ => return None,
    })
}

pub(super) fn event_has_prototype_attribute(interface: &str, property: &str) -> bool {
    let mut interface = Some(interface);
    while let Some(name) = interface {
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
    super::events::install_value_event_template_bindings(scope, template, interface);
    super::events::install_device_event_template_bindings(scope, template, interface);
    if interface == "Document" {
        let prototype = template.prototype_template(scope);
        document::install_document_prototype_methods(scope, prototype);
    }
    if let Some(declaration) = event_template_declaration(interface) {
        (declaration.install)(scope, template);
    }
}
