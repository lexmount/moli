use super::*;
use super::{message::message_event_init_callback, storage::storage_event_init_callback};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiFunctionTemplateDeclaration};

pub struct EventTemplateDeclaration {
    pub attributes: &'static [&'static str],
    pub install: for<'s, 'p> fn(&mut v8::PinScope<'s, 'p, ()>, v8::Local<'s, v8::FunctionTemplate>),
}

impl EventTemplateDeclaration {
    pub fn new<D: WebApiFunctionTemplateDeclaration>() -> Self {
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

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CloseEvent, enumerable)]
struct CloseEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "wasClean", getter = close_event_was_clean_getter_function)]
    was_clean: (),

    #[webapi(accessor_property, getter = close_event_code_getter_function)]
    code: (),

    #[webapi(accessor_property, getter = close_event_reason_getter_function)]
    reason: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TrackEvent, enumerable)]
struct TrackEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = track_event_track_getter_function)]
    track: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CommandEvent, enumerable)]
struct CommandEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = nullable_event_slot_getter, data = moli_v8_util::v8str(scope, COMMAND_EVENT_SOURCE_SLOT))]
    source: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "command"))]
    command: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ToggleEvent, enumerable)]
struct ToggleEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "oldState", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "oldState"))]
    old_state: (),

    #[webapi(accessor_property = "newState", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "newState"))]
    new_state: (),

    #[webapi(accessor_property, getter = nullable_event_slot_getter, data = moli_v8_util::v8str(scope, TOGGLE_EVENT_SOURCE_SLOT))]
    source: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::BeforeUnloadEvent, enumerable)]
struct BeforeUnloadEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = before_unload_event_return_value_getter, setter = before_unload_event_return_value_setter)]
    return_value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HashChangeEvent, enumerable)]
struct HashChangeEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "oldURL", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "oldURL"))]
    old_url: (),
    #[webapi(accessor_property = "newURL", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "newURL"))]
    new_url: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CustomEvent, enumerable)]
struct CustomEventTemplateMethodsDeclaration {
    #[webapi(method = "initCustomEvent", length = 1, callback = custom_event_init_callback)]
    init_custom_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SecurityPolicyViolationEvent, enumerable)]
struct SecurityPolicyViolationEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "documentURI", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "documentURI"))]
    document_uri: (),

    #[webapi(accessor_property = "referrer", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "referrer"))]
    referrer: (),

    #[webapi(accessor_property = "blockedURI", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "blockedURI"))]
    blocked_uri: (),

    #[webapi(accessor_property = "effectiveDirective", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "effectiveDirective"))]
    effective_directive: (),

    #[webapi(accessor_property = "violatedDirective", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "violatedDirective"))]
    violated_directive: (),

    #[webapi(accessor_property = "originalPolicy", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "originalPolicy"))]
    original_policy: (),

    #[webapi(accessor_property = "sourceFile", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "sourceFile"))]
    source_file: (),

    #[webapi(accessor_property = "sample", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "sample"))]
    sample: (),

    #[webapi(accessor_property = "disposition", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "disposition"))]
    disposition: (),

    #[webapi(accessor_property = "statusCode", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "statusCode"))]
    status_code: (),

    #[webapi(accessor_property = "lineNumber", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "lineNumber"))]
    line_number: (),

    #[webapi(accessor_property = "columnNumber", getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "columnNumber"))]
    column_number: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ErrorEvent, enumerable)]
struct ErrorEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "message"))]
    message: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "filename"))]
    filename: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "lineno"))]
    lineno: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "colno"))]
    colno: (),

    #[webapi(accessor_property, getter = event_value_attribute_getter, data = moli_v8_util::v8str(scope, "error"))]
    error: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ClipboardEvent, enumerable)]
struct ClipboardEventTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "clipboardData",
        getter = clipboard_event_clipboard_data_getter_function
    )]
    clipboard_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ClipboardChangeEvent, enumerable)]
struct ClipboardChangeEventTemplateAccessorsDeclaration {
    #[webapi(accessor_property, getter = clipboard_change_event_types_getter_function)]
    types: (),

    #[webapi(
        accessor_property = "changeId",
        getter = clipboard_change_event_change_id_getter_function
    )]
    change_id: (),
}

pub fn template_declaration(interface: &str) -> Option<EventTemplateDeclaration> {
    Some(match interface {
        "PageRevealEvent" => EventTemplateDeclaration::new::<PageRevealEventTemplateDeclaration>(),
        "PageSwapEvent" => EventTemplateDeclaration::new::<PageSwapEventTemplateDeclaration>(),
        "MessageEvent" => EventTemplateDeclaration::new::<MessageEventTemplateMethodsDeclaration>(),
        "StorageEvent" => EventTemplateDeclaration::new::<StorageEventTemplateMethodsDeclaration>(),
        "ClipboardEvent" => {
            EventTemplateDeclaration::new::<ClipboardEventTemplateAccessorsDeclaration>()
        }
        "ClipboardChangeEvent" => {
            EventTemplateDeclaration::new::<ClipboardChangeEventTemplateAccessorsDeclaration>()
        }
        "ErrorEvent" => EventTemplateDeclaration::new::<ErrorEventTemplateAccessorsDeclaration>(),
        "SecurityPolicyViolationEvent" => EventTemplateDeclaration::new::<
            SecurityPolicyViolationEventTemplateAccessorsDeclaration,
        >(),
        "BeforeUnloadEvent" => {
            EventTemplateDeclaration::new::<BeforeUnloadEventTemplateAccessorsDeclaration>()
        }
        "CustomEvent" => EventTemplateDeclaration::new::<CustomEventTemplateMethodsDeclaration>(),
        "CommandEvent" => {
            EventTemplateDeclaration::new::<CommandEventTemplateAccessorsDeclaration>()
        }
        "ToggleEvent" => EventTemplateDeclaration::new::<ToggleEventTemplateAccessorsDeclaration>(),
        "CloseEvent" => EventTemplateDeclaration::new::<CloseEventTemplateAccessorsDeclaration>(),
        "TrackEvent" => EventTemplateDeclaration::new::<TrackEventTemplateAccessorsDeclaration>(),
        "HashChangeEvent" => {
            EventTemplateDeclaration::new::<HashChangeEventTemplateAccessorsDeclaration>()
        }
        _ => return None,
    })
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MessageEvent, enumerable)]
struct MessageEventTemplateMethodsDeclaration {
    #[webapi(method = "initMessageEvent", length = 1, callback = message_event_init_callback)]
    init_message_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::StorageEvent, enumerable)]
struct StorageEventTemplateMethodsDeclaration {
    #[webapi(
        method = "initStorageEvent",
        length = 1,
        callback = storage_event_init_callback
    )]
    init_storage_event: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PageRevealEvent, enumerable)]
struct PageRevealEventTemplateDeclaration {
    #[webapi(accessor_property = "viewTransition", getter = page_reveal_event_view_transition_getter)]
    view_transition: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PageSwapEvent, enumerable)]
struct PageSwapEventTemplateDeclaration {
    #[webapi(
        accessor_property,
        getter = nullable_event_slot_getter,
        data = moli_v8_util::v8str(scope, PAGE_SWAP_EVENT_ACTIVATION_SLOT)
    )]
    activation: (),

    #[webapi(
        accessor_property = "viewTransition",
        getter = nullable_event_slot_getter,
        data = moli_v8_util::v8str(scope, PAGE_SWAP_EVENT_VIEW_TRANSITION_SLOT)
    )]
    view_transition: (),
}
