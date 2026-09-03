use super::{
    event_document::{document_create_event_callback, document_has_focus_callback},
    event_legacy::{
        composition_event_init_callback, custom_event_init_callback, event_init_event_callback,
        keyboard_event_get_modifier_state_callback, keyboard_event_init_callback,
        mouse_event_init_callback, storage_event_init_callback, text_event_init_callback,
        ui_event_init_callback,
    },
    events::{
        before_unload_event_return_value_getter_function,
        before_unload_event_return_value_setter_function, close_event_code_getter_function,
        close_event_reason_getter_function, close_event_was_clean_getter_function,
        command_event_command_getter_function, command_event_source_getter_function,
        event_bubbles_getter_function, event_cancel_bubble_getter_function,
        event_cancel_bubble_setter_function, event_cancelable_getter_function,
        event_composed_getter_function, event_composed_path_callback,
        event_current_target_getter_function, event_default_prevented_getter_function,
        event_event_phase_getter_function, event_prevent_default_callback,
        event_return_value_getter_function, event_return_value_setter_function,
        event_src_element_getter_function, event_stop_immediate_propagation_callback,
        event_stop_propagation_callback, event_target_getter_function,
        event_time_stamp_getter_function, event_type_getter_function,
        focus_event_related_target_getter_function, form_data_event_form_data_getter_function,
        mouse_event_related_target_getter_function, pointer_event_get_predicted_events_callback,
        submit_event_submitter_getter_function, toggle_event_source_getter_function,
        track_event_track_getter_function, ui_event_pseudo_target_getter_function,
    },
    selection_surface::document_get_selection_callback,
    specs::{ConstructorKind, ConstructorSpec},
};
use crate::{native_bridge::document, window_host};
use moli_webapi_declare::{EVENT_TARGET_INTERFACE_BRAND_SLOT, WebApiFunctionTemplate};

pub(in crate::context_bootstrap) fn object_is_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if crate::util::get_private_value(scope, object, EVENT_TARGET_INTERFACE_BRAND_SLOT)
        .is_some_and(|value| value.is_true())
    {
        return true;
    }
    super::exposed_interfaces::object_is_intrinsic_interface_instance(scope, "EventTarget", object)
        || crate::native_bridge::object_is_native_event_target_wrapper_or_detached(scope, object)
}

pub(in crate::context_bootstrap) fn mark_event_target_interface_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) {
    crate::util::set_private_value(
        scope,
        prototype,
        EVENT_TARGET_INTERFACE_BRAND_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Event", enumerable)]
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
#[webapi(name = "BeforeUnloadEvent", enumerable)]
struct BeforeUnloadEventTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "returnValue",
        getter = before_unload_event_return_value_getter_function,
        setter = before_unload_event_return_value_setter_function
    )]
    return_value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CloseEvent", enumerable)]
struct CloseEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "wasClean", getter = close_event_was_clean_getter_function)]
    was_clean: (),

    #[webapi(accessor_property, getter = close_event_code_getter_function)]
    code: (),

    #[webapi(accessor_property, getter = close_event_reason_getter_function)]
    reason: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TrackEvent", enumerable)]
struct TrackEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = track_event_track_getter_function)]
    track: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "SubmitEvent", enumerable)]
struct SubmitEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = submit_event_submitter_getter_function)]
    submitter: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FormDataEvent", enumerable)]
struct FormDataEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "formData", getter = form_data_event_form_data_getter_function)]
    form_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CommandEvent", enumerable)]
struct CommandEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = command_event_source_getter_function)]
    source: (),

    #[webapi(accessor_property, getter = command_event_command_getter_function)]
    command: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ToggleEvent", enumerable)]
struct ToggleEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = toggle_event_source_getter_function)]
    source: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "UIEvent", enumerable)]
struct UiEventTemplateMethodsDeclaration {
    #[webapi(method = "initUIEvent", length = 0, callback = ui_event_init_callback)]
    init_ui_event: (),

    #[webapi(
        accessor_property = "pseudoTarget",
        getter = ui_event_pseudo_target_getter_function
    )]
    pseudo_target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FocusEvent", enumerable)]
struct FocusEventTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "relatedTarget",
        getter = focus_event_related_target_getter_function
    )]
    related_target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "TextEvent", enumerable)]
struct TextEventTemplateMethodsDeclaration {
    #[webapi(method = "initTextEvent", length = 1, callback = text_event_init_callback)]
    init_text_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CompositionEvent", enumerable)]
struct CompositionEventTemplateMethodsDeclaration {
    #[webapi(
        method = "initCompositionEvent",
        length = 1,
        callback = composition_event_init_callback
    )]
    init_composition_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CustomEvent", enumerable)]
struct CustomEventTemplateMethodsDeclaration {
    #[webapi(method = "initCustomEvent", length = 1, callback = custom_event_init_callback)]
    init_custom_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "StorageEvent", enumerable)]
struct StorageEventTemplateMethodsDeclaration {
    #[webapi(
        method = "initStorageEvent",
        length = 1,
        callback = storage_event_init_callback
    )]
    init_storage_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "KeyboardEvent", enumerable)]
struct KeyboardEventTemplateMethodsDeclaration {
    #[webapi(method = "initKeyboardEvent", length = 7, callback = keyboard_event_init_callback)]
    init_keyboard_event: (),

    #[webapi(
        method = "getModifierState",
        length = 0,
        callback = keyboard_event_get_modifier_state_callback
    )]
    get_modifier_state: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "MouseEvent", enumerable)]
struct MouseEventTemplateMethodsDeclaration {
    #[webapi(
        accessor_property = "relatedTarget",
        getter = mouse_event_related_target_getter_function
    )]
    related_target: (),

    #[webapi(accessor_property = "offsetX", getter = window_host::mouse_event_offset_x_getter)]
    offset_x: (),

    #[webapi(accessor_property = "offsetY", getter = window_host::mouse_event_offset_y_getter)]
    offset_y: (),

    #[webapi(method = "initMouseEvent", length = 15, callback = mouse_event_init_callback)]
    init_mouse_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "WheelEvent", enumerable)]
struct WheelEventTemplateConstantsDeclaration {
    #[webapi(constant = "DOM_DELTA_PIXEL", value = 0u32)]
    dom_delta_pixel: (),

    #[webapi(constant = "DOM_DELTA_LINE", value = 1u32)]
    dom_delta_line: (),

    #[webapi(constant = "DOM_DELTA_PAGE", value = 2u32)]
    dom_delta_page: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "PointerEvent", enumerable)]
struct PointerEventTemplateMethodsDeclaration {
    #[webapi(
        method = "getPredictedEvents",
        length = 0,
        callback = pointer_event_get_predicted_events_callback
    )]
    get_predicted_events: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "EventTarget", enumerable)]
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
#[webapi(name = "Document", enumerable)]
struct DocumentEventTemplateMethodsDeclaration {
    #[webapi(method = "createEvent", length = 1, callback = document_create_event_callback)]
    create_event: (),

    #[webapi(method = "hasFocus", length = 0, callback = document_has_focus_callback)]
    has_focus: (),

    #[webapi(method = "getSelection", length = 0, callback = document_get_selection_callback)]
    get_selection: (),
}

fn install_event_base_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    EventBaseTemplateMethodsDeclaration::initialize_template(scope, template);
    EventBaseTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
}

pub(super) fn install_event_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    spec: ConstructorSpec,
) {
    if spec.kind == ConstructorKind::Event {
        install_event_base_bindings(scope, template);
    }

    match spec.name {
        "BeforeUnloadEvent" => {
            let proto = template.prototype_template(scope);
            BeforeUnloadEventTemplateAccessorsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "UIEvent" => {
            let proto = template.prototype_template(scope);
            UiEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "FocusEvent" => {
            let proto = template.prototype_template(scope);
            FocusEventTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "TextEvent" => {
            let proto = template.prototype_template(scope);
            TextEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "CompositionEvent" => {
            let proto = template.prototype_template(scope);
            CompositionEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "CustomEvent" => {
            let proto = template.prototype_template(scope);
            CustomEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "StorageEvent" => {
            let proto = template.prototype_template(scope);
            StorageEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "KeyboardEvent" => {
            let proto = template.prototype_template(scope);
            KeyboardEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "MouseEvent" => {
            let proto = template.prototype_template(scope);
            MouseEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "WheelEvent" => {
            let proto = template.prototype_template(scope);
            WheelEventTemplateConstantsDeclaration::initialize_template(scope, template);
            WheelEventTemplateConstantsDeclaration::initialize_prototype_template(scope, proto);
        }
        "PointerEvent" => {
            let proto = template.prototype_template(scope);
            PointerEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "CloseEvent" => {
            let proto = template.prototype_template(scope);
            CloseEventTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "TrackEvent" => {
            let proto = template.prototype_template(scope);
            TrackEventTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "SubmitEvent" => {
            let proto = template.prototype_template(scope);
            SubmitEventTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "FormDataEvent" => {
            let proto = template.prototype_template(scope);
            FormDataEventTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "CommandEvent" => {
            let proto = template.prototype_template(scope);
            CommandEventTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "ToggleEvent" => {
            let proto = template.prototype_template(scope);
            ToggleEventTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "EventTarget" => {
            let prototype = template.prototype_template(scope);
            EventTargetTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "Document" => {
            let proto = template.prototype_template(scope);
            document::install_document_prototype_methods(scope, proto);
            DocumentEventTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        _ => {}
    }
}
