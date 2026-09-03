use super::*;
use crate::util::{get_private_value, utf16_units, v8_string, v8_string_from_utf16_units, v8str};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject};

mod base;
mod init;
mod kind;
mod methods;
mod subclasses;

const CLOSE_EVENT_WAS_CLEAN_SLOT: &str = "__moliCloseEventWasClean";
const CLOSE_EVENT_CODE_SLOT: &str = "__moliCloseEventCode";
const CLOSE_EVENT_REASON_SLOT: &str = "__moliCloseEventReason";
const SUBMIT_EVENT_SUBMITTER_SLOT: &str = "__moliSubmitEventSubmitter";
const FORM_DATA_EVENT_FORM_DATA_SLOT: &str = "__moliFormDataEventFormData";
const TRACK_EVENT_TRACK_SLOT: &str = "__moliTrackEventTrack";
const COMMAND_EVENT_SOURCE_SLOT: &str = "__moliCommandEventSource";
const COMMAND_EVENT_COMMAND_SLOT: &str = "__moliCommandEventCommand";
const TOGGLE_EVENT_SOURCE_SLOT: &str = "__moliToggleEventSource";
const EVENT_SUBCLASS_KIND_SLOT: &str = "__moliEventSubclassKind";
const BEFORE_UNLOAD_EVENT_RETURN_VALUE_SLOT: &str = "__moliBeforeUnloadEventReturnValue";
#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct PageTransitionEventInitDeclaration {
    #[webapi(data_property, enumerable)]
    bubbles: bool,
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    persisted: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", enumerable)]
struct StorageEventStateDeclaration<'scope> {
    #[webapi(data_property)]
    key: v8::Local<'scope, v8::Value>,
    #[webapi(data_property = "oldValue")]
    old_value: v8::Local<'scope, v8::Value>,
    #[webapi(data_property = "newValue")]
    new_value: v8::Local<'scope, v8::Value>,
    #[webapi(data_property)]
    url: v8::Local<'scope, v8::Value>,
    #[webapi(data_property = "storageArea")]
    storage_area: v8::Local<'scope, v8::Value>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "PointerEvent", enumerable)]
struct SecurePointerEventPrototypeRuntimeDeclaration {
    #[webapi(
        method = "getCoalescedEvents",
        length = 0,
        callback = subclasses::pointer_event_get_coalesced_events_callback
    )]
    get_coalesced_events: (),
}

pub(crate) fn construct_original_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let event_ctor =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "Event").ok()?;
    let event_type = v8_string(scope, event_type)?;
    let event = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = try_catch.init();
        event_ctor.new_instance(&scope, &[event_type.into()])
    }?;
    mark_event_trusted(scope, event);
    Some(event)
}

fn new_before_unload_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    cancelable: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "BeforeUnloadEvent")?;
    let event = v8::Object::new(scope);
    if event.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    base::initialize_event_object(scope, event, event_type, false, cancelable);
    set_private_value(
        scope,
        event,
        BEFORE_UNLOAD_EVENT_RETURN_VALUE_SLOT,
        v8str(scope, "").into(),
    );
    Some(event)
}

pub(crate) fn construct_original_before_unload_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let event = new_before_unload_event(scope, "beforeunload", true)?;
    mark_event_trusted(scope, event);
    Some(event)
}

pub(in crate::context_bootstrap) fn new_uninitialized_before_unload_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let event = new_before_unload_event(scope, "", false)?;
    base::set_event_initialized(scope, event, false);
    Some(event)
}

fn before_unload_event_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::String>> {
    get_private_value(scope, event, BEFORE_UNLOAD_EVENT_RETURN_VALUE_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
}

pub(crate) fn event_is_before_unload_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    before_unload_event_return_value(scope, event).is_some()
}

pub(crate) fn set_before_unload_event_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::String>,
) {
    set_private_value(
        scope,
        event,
        BEFORE_UNLOAD_EVENT_RETURN_VALUE_SLOT,
        value.into(),
    );
}

pub(crate) fn before_unload_event_return_value_is_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    before_unload_event_return_value(scope, event).is_some_and(|value| value.length() == 0)
}

pub(super) fn before_unload_event_return_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = before_unload_event_return_value(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value.into());
}

pub(super) fn before_unload_event_return_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !event_is_before_unload_event(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    set_before_unload_event_return_value(scope, args.this(), value);
}

pub(in crate::context_bootstrap) fn new_uninitialized_text_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "TextEvent")?;
    let event = v8::Object::new(scope);
    if event.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    base::initialize_event_object(scope, event, "", false, false);
    if !subclasses::initialize_text_event(scope, event, None) {
        return None;
    }
    set_private_value(
        scope,
        event,
        EVENT_SUBCLASS_KIND_SLOT,
        v8::Integer::new(scope, EventSubclassKind::TextEvent as i32).into(),
    );
    base::set_event_initialized(scope, event, false);
    Some(event)
}

pub(crate) fn construct_original_page_transition_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    persisted: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let event_ctor = super::exposed_interfaces::ensure_intrinsic_interface_constructor(
        scope,
        "PageTransitionEvent",
    )
    .ok()?;
    let event_type = v8_string(scope, event_type)?;
    let init = PageTransitionEventInitDeclaration::new(true, true, persisted)
        .bind(scope)
        .expect("PageTransitionEvent init declaration should bind");
    let event = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = try_catch.init();
        event_ctor.new_instance(&scope, &[event_type.into(), init.into()])
    }?;
    mark_event_trusted(scope, event);
    Some(event)
}

pub(crate) fn construct_original_storage_event_utf16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    key: Option<&[u16]>,
    old_value: Option<&[u16]>,
    new_value: Option<&[u16]>,
    url: &str,
    storage_area: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let event_ctor =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "StorageEvent")
            .ok()?;
    let event_type = v8_string(scope, event_type)?;
    let init = ObjectLiteralDeclaration::bind(scope);
    define_storage_event_init_properties_utf16(
        scope,
        &init,
        key,
        old_value,
        new_value,
        url,
        storage_area,
    );
    let init = init.into_object();
    let event = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = try_catch.init();
        event_ctor.new_instance(&scope, &[event_type.into(), init.into()])
    }?;
    define_storage_event_properties_utf16(
        scope,
        event,
        key,
        old_value,
        new_value,
        url,
        storage_area,
    );
    Some(event)
}

pub(in crate::context_bootstrap) fn define_storage_event_properties<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    key: Option<&str>,
    old_value: Option<&str>,
    new_value: Option<&str>,
    url: &str,
    storage_area: Option<v8::Local<'s, v8::Value>>,
) {
    let key = key.map(utf16_units);
    let old_value = old_value.map(utf16_units);
    let new_value = new_value.map(utf16_units);
    define_storage_event_properties_utf16(
        scope,
        event,
        key.as_deref(),
        old_value.as_deref(),
        new_value.as_deref(),
        url,
        storage_area,
    );
}

pub(in crate::context_bootstrap) fn define_storage_event_properties_utf16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    key: Option<&[u16]>,
    old_value: Option<&[u16]>,
    new_value: Option<&[u16]>,
    url: &str,
    storage_area: Option<v8::Local<'s, v8::Value>>,
) {
    let key_value = storage_event_nullable_string_value_utf16(scope, key);
    let old_value = storage_event_nullable_string_value_utf16(scope, old_value);
    let new_value = storage_event_nullable_string_value_utf16(scope, new_value);
    let url_value = v8_string(scope, url).expect("storage event url").into();
    let storage_area = storage_area.unwrap_or_else(|| v8::null(scope).into());
    let _ =
        StorageEventStateDeclaration::new(key_value, old_value, new_value, url_value, storage_area)
            .initialize(scope, event);
}

fn define_storage_event_init_properties_utf16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: &ObjectLiteralDeclaration<'s>,
    key: Option<&[u16]>,
    old_value: Option<&[u16]>,
    new_value: Option<&[u16]>,
    url: &str,
    storage_area: Option<v8::Local<'s, v8::Value>>,
) {
    let key_value = storage_event_nullable_string_value_utf16(scope, key);
    init.set_string_property(scope, "key", key_value);
    let old_value = storage_event_nullable_string_value_utf16(scope, old_value);
    init.set_string_property(scope, "oldValue", old_value);
    let new_value = storage_event_nullable_string_value_utf16(scope, new_value);
    init.set_string_property(scope, "newValue", new_value);
    let url_value = v8_string(scope, url).expect("storage event url").into();
    init.set_string_property(scope, "url", url_value);
    let storage_area = storage_area.unwrap_or_else(|| v8::null(scope).into());
    init.set_string_property(scope, "storageArea", storage_area);
}

fn storage_event_nullable_string_value_utf16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<&[u16]>,
) -> v8::Local<'s, v8::Value> {
    value
        .map(|value| {
            v8_string_from_utf16_units(scope, value)
                .expect("storage event string")
                .into()
        })
        .unwrap_or_else(|| v8::null(scope).into())
}

pub(crate) use base::{
    EVENT_DISPATCHING_SLOT, EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT,
    EVENT_STOP_PROPAGATION_SLOT, clear_event_composed_path, event_initialized,
    event_internal_bool_flag, event_is_dispatching, initialize_event_object, mark_event_trusted,
    set_event_composed_path, set_event_internal_flag, set_event_trusted,
};
fn event_subclass_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<EventSubclassKind> {
    get_private_value(scope, event, EVENT_SUBCLASS_KIND_SLOT)
        .and_then(|value| v8::Local::<v8::Integer>::try_from(value).ok())
        .and_then(|value| i32::try_from(value.value()).ok())
        .and_then(EventSubclassKind::from_i32)
}

pub(crate) fn event_is_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    event_subclass_kind(scope, event) == Some(EventSubclassKind::ErrorEvent)
}

pub(crate) fn set_event_source_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    match event_subclass_kind(scope, event) {
        Some(EventSubclassKind::CommandEvent) => {
            set_private_value(scope, event, COMMAND_EVENT_SOURCE_SLOT, value);
        }
        Some(EventSubclassKind::ToggleEvent) => {
            set_private_value(scope, event, TOGGLE_EVENT_SOURCE_SLOT, value);
        }
        _ => {
            let _ = event.set(scope, v8str(scope, "source").into(), value);
        }
    }
}

pub(crate) fn event_is_mouse_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(
        event_subclass_kind(scope, event),
        Some(
            EventSubclassKind::MouseEvent
                | EventSubclassKind::DragEvent
                | EventSubclassKind::WheelEvent
                | EventSubclassKind::PointerEvent
        )
    )
}

fn event_is_ui_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(
        event_subclass_kind(scope, event),
        Some(
            EventSubclassKind::UiEvent
                | EventSubclassKind::FocusEvent
                | EventSubclassKind::TextEvent
                | EventSubclassKind::CompositionEvent
                | EventSubclassKind::MouseEvent
                | EventSubclassKind::DragEvent
                | EventSubclassKind::KeyboardEvent
                | EventSubclassKind::InputEvent
                | EventSubclassKind::WheelEvent
                | EventSubclassKind::PointerEvent
                | EventSubclassKind::TouchEvent
        )
    )
}

pub(super) fn ui_event_pseudo_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !event_is_ui_event(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    // Moli does not currently synthesize UI events whose origin is a CSS
    // pseudo-element, so every supported UIEvent has the default null value.
    rv.set(v8::null(scope).into());
}

fn event_related_target_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    event
        .get_own_property_descriptor(scope, v8str(scope, "relatedTarget").into())
        .and_then(|descriptor| v8::Local::<v8::Object>::try_from(descriptor).ok())
        .and_then(|descriptor| descriptor.get(scope, v8str(scope, "value").into()))
        .unwrap_or_else(|| v8::null(scope).into())
}

pub(super) fn focus_event_related_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_subclass_kind(scope, args.this()) != Some(EventSubclassKind::FocusEvent) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(event_related_target_value(scope, args.this()));
}

pub(super) fn mouse_event_related_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !event_is_mouse_event(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(event_related_target_value(scope, args.this()));
}

pub(super) use base::{
    clear_event_dispatch_fields, event_bubbles_getter_function, event_cancelable_getter_function,
    event_composed_getter_function, event_constructor_callback,
    event_current_target_getter_function, event_default_prevented_getter_function,
    event_event_phase_getter_function, event_src_element_getter_function,
    event_target_getter_function, event_type_getter_function, set_event_dispatch_fields,
    set_event_initialized,
};
pub(super) use kind::EventSubclassKind;
pub(crate) use methods::{
    EventHandlerType, apply_before_unload_event_handler_return_value,
    apply_event_handler_return_value,
};
pub(super) use methods::{
    event_cancel_bubble_getter_function, event_cancel_bubble_setter_function,
    event_composed_path_callback, event_prevent_default_callback,
    event_return_value_getter_function, event_return_value_setter_function,
    event_stop_immediate_propagation_callback, event_stop_propagation_callback,
    event_time_stamp_getter_function,
};
pub(in crate::context_bootstrap) use subclasses::run_navigate_event_precommit_handlers;
pub(super) use subclasses::{
    build_event_subclass_template, pointer_event_get_predicted_events_callback,
};

pub(in crate::context_bootstrap) fn finalize_pointer_event_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> anyhow::Result<()> {
    let global = scope.get_current_context().global(scope);
    let secure_context_available = get_private_value(
        scope,
        global,
        super::runtime_state::WINDOW_SECURE_CONTEXT_AVAILABLE_SLOT,
    )
    .is_some_and(|value| value.boolean_value(scope));
    if !secure_context_available {
        return Ok(());
    }
    SecurePointerEventPrototypeRuntimeDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow::anyhow!("failed to initialize PointerEvent.prototype: {error}"))
}

pub(super) fn close_event_was_clean_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), CLOSE_EVENT_WAS_CLEAN_SLOT)
        .unwrap_or_else(|| v8::Boolean::new(scope, false).into());
    rv.set(value);
}

pub(super) fn close_event_code_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), CLOSE_EVENT_CODE_SLOT)
        .unwrap_or_else(|| v8::Number::new(scope, 0.0).into());
    rv.set(value);
}

pub(super) fn close_event_reason_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), CLOSE_EVENT_REASON_SLOT)
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

pub(super) fn submit_event_submitter_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), SUBMIT_EVENT_SUBMITTER_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(super) fn command_event_source_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_subclass_kind(scope, args.this()) != Some(EventSubclassKind::CommandEvent) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = get_private_value(scope, args.this(), COMMAND_EVENT_SOURCE_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(super) fn command_event_command_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_subclass_kind(scope, args.this()) != Some(EventSubclassKind::CommandEvent) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = get_private_value(scope, args.this(), COMMAND_EVENT_COMMAND_SLOT)
        .unwrap_or_else(|| v8str(scope, "").into());
    rv.set(value);
}

pub(super) fn toggle_event_source_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_subclass_kind(scope, args.this()) != Some(EventSubclassKind::ToggleEvent) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let value = get_private_value(scope, args.this(), TOGGLE_EVENT_SOURCE_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(super) fn form_data_event_form_data_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), FORM_DATA_EVENT_FORM_DATA_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

pub(super) fn track_event_track_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), TRACK_EVENT_TRACK_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}
