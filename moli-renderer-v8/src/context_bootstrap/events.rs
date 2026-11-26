use super::*;
use crate::util::{get_private_value, v8_string, v8str};
pub(crate) use moli_webapi::events::subclasses::construct_original_page_transition_event;
use moli_webapi_declare::WebApiObject;

mod base;
use moli_webapi::events::device;
mod init;
use moli_webapi::events::kind;
pub(crate) use moli_webapi::events::subclasses::message::construct_original_message_event;
pub(crate) use moli_webapi::events::subclasses::storage::construct_original_storage_event_utf16;
mod methods;
use moli_webapi::events::modifiers;
mod subclasses;
pub(crate) use moli_webapi::events::subclasses::submit::{
    mark_agent_submit_event, take_submit_event_response,
};
pub(in crate::context_bootstrap) use moli_webapi::events::ui::{
    composition_event_init_callback, ui_event_init_callback,
};
use moli_webapi::events::value;
pub(crate) use subclasses::clear_input_event_target_ranges;
pub(crate) use subclasses::initialize_native_security_policy_violation_event;
pub(in crate::context_bootstrap) use subclasses::input_event_get_target_ranges_callback;
mod wrappers;

pub(in crate::context_bootstrap) use moli_webapi::events::{audio, speech};

pub(in crate::context_bootstrap) use device::{
    device_motion_event_constructor, device_orientation_event_constructor,
    install_device_event_template_bindings,
};

pub(in crate::context_bootstrap) use value::{
    ValueEventKind, build_value_event_template, install_value_event_template_bindings,
};

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "PointerEvent", enumerable)]
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
    let event = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        moli_webapi::events::construct_native_event(&mut scope, event_type, Default::default())
    }?;
    mark_event_trusted(scope, event);
    Some(event)
}

pub(in crate::context_bootstrap) fn new_uninitialized_text_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "TextEvent")?;
    let event = v8::Object::new(scope);
    if event.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    let wrapper = event;
    let event = new_event_state(scope);
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
    web_api_interfaces::initialize(scope, event, "TextEvent").ok()?;
    initialize_event_wrapper(scope, wrapper, event)?;
    Some(wrapper)
}

#[cfg(feature = "devtools")]
pub(crate) fn construct_original_text_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    text: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    // TextEvent has no public constructor. Use its native legacy backing,
    // without calling author-replaceable constructors or initTextEvent.
    let event = new_uninitialized_text_event(scope)?;
    let backing = event_backing(scope, event);
    initialize_event_object(scope, backing, "textInput", true, true);
    define_event_property(
        scope,
        backing,
        "composed",
        v8::Boolean::new(scope, true).into(),
    );
    let view = scope.get_current_context().global(scope);
    moli_webapi::events::ui::initialize_legacy_ui_event(scope, backing, Some(view), 0);
    let data = v8_string(scope, text)?;
    define_event_property(scope, backing, "data", data.into());
    base::set_event_initialized(scope, backing, true);
    mark_event_trusted(scope, event);
    Some(event)
}

pub(crate) use base::{
    EVENT_DISPATCHING_SLOT, EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT,
    EVENT_STOP_PROPAGATION_SLOT, clear_event_composed_path, define_event_property, event_backing,
    event_initialized, event_internal_bool_flag, event_is_dispatching, initialize_event_object,
    mark_event_trusted, set_event_composed_path, set_event_internal_flag, set_event_trusted,
};

pub(crate) fn event_is_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    event_subclass_kind(scope, event) == Some(EventSubclassKind::ErrorEvent)
}

/// Read the converted native payload without invoking author-shadowed attributes.
pub(crate) fn error_event_handler_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<[v8::Local<'s, v8::Value>; 5]> {
    if !event_is_error_event(scope, event) {
        return None;
    }
    let backing = event_backing(scope, event);
    Some([
        backing.get(scope, v8str(scope, "message").into())?,
        backing.get(scope, v8str(scope, "filename").into())?,
        backing.get(scope, v8str(scope, "lineno").into())?,
        backing.get(scope, v8str(scope, "colno").into())?,
        backing.get(scope, v8str(scope, "error").into())?,
    ])
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
    wrappers::event_attribute_in_wrapper(scope, event, "relatedTarget")
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
    event_bubbles_getter_function, event_cancelable_getter_function,
    event_composed_getter_function, event_constructor_callback,
    event_current_target_getter_function, event_default_prevented_getter_function,
    event_event_phase_getter_function, event_src_element_getter_function,
    event_target_getter_function, event_type_getter_function, set_event_initialized,
};
pub(in crate::context_bootstrap) use init::{EventInit, parse_event_dictionary};
pub(super) use kind::EventSubclassKind;
pub(crate) use methods::set_event_default_prevented;
pub(super) use methods::{
    event_cancel_bubble_getter_function, event_cancel_bubble_setter_function,
    event_composed_path_callback, event_prevent_default_callback,
    event_return_value_getter_function, event_return_value_setter_function,
    event_stop_immediate_propagation_callback, event_stop_propagation_callback,
    event_time_stamp_getter_function,
};
pub(in crate::context_bootstrap) use moli_webapi::events::subclasses::new_uninitialized_before_unload_event;
use moli_webapi::events::subclasses::{
    EVENT_SUBCLASS_KIND_SLOT, FORM_DATA_EVENT_FORM_DATA_SLOT, SUBMIT_EVENT_SUBMITTER_SLOT,
    event_subclass_kind,
};
pub(super) use subclasses::{
    build_event_subclass_template, pointer_event_get_predicted_events_callback,
};
pub(in crate::context_bootstrap) use subclasses::{
    navigation_precommit_controller_for_event, run_navigate_event_precommit_handlers,
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

pub(crate) fn submit_event_submitter_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    event_private_value(scope, event, SUBMIT_EVENT_SUBMITTER_SLOT)
}

pub(super) fn submit_event_submitter_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value =
        submit_event_submitter_value(scope, args.this()).unwrap_or_else(|| v8::null(scope).into());
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    if let Some(value) = super::platform_object_worlds::in_realm(scope, value, context) {
        rv.set(value);
    }
}

pub(super) fn form_data_event_form_data_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = crate::context_bootstrap::event_private_value(
        scope,
        args.this(),
        FORM_DATA_EVENT_FORM_DATA_SLOT,
    )
    .unwrap_or_else(|| v8::undefined(scope).into());
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    if let Some(value) = super::platform_object_worlds::in_realm(scope, value, context) {
        rv.set(value);
    }
}

pub(crate) use wrappers::{
    event_attribute, event_bool_attribute, event_callback_values, event_private_value,
    initialize_event_wrapper, new_event_state, new_event_wrapper, node_event_callback_values,
    set_event_private_value,
};

#[cfg(any(feature = "devtools", test))]
pub(crate) use subclasses::construct_native_touch_event;
pub(in crate::context_bootstrap) use wrappers::{
    event_platform_attribute_getter, event_value_attribute_getter,
};

pub(in crate::context_bootstrap) use modifiers::event_get_modifier_state_callback;
pub(in crate::context_bootstrap) use subclasses::keyboard_event_init_callback;

pub(crate) use subclasses::construct_original_error_event;

mod ui_properties;
pub(in crate::context_bootstrap) use ui_properties::ui_event_which_getter_function;

pub(crate) use self::base::event_trusted;

pub(crate) fn construct_original_before_unload_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let event = new_uninitialized_before_unload_event(scope)?;
    let backing = event_backing(scope, event);
    base::initialize_event_object(scope, backing, "beforeunload", false, true);
    mark_event_trusted(scope, event);
    Some(event)
}
