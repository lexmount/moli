//! Value event constructors, dictionaries and intrinsic prototype bindings.
//! DOM target projection and dispatch remain embedder responsibilities.
use super::init::{EventInit, event_type_argument, parse_event_dictionary};
use super::kind::EventSubclassKind;
use super::state::*;
use super::{initialize_event_object, initialize_event_object_with_type, initialize_event_wrapper};
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{set_private_value, throw_type_error, v8str};
use moli_webidl as webidl;

mod accessors;
mod basic;
pub use basic::{construct_native_focus_event, initialize_text_event};
mod captured_mouse;
mod clipboard_change;
mod custom;
mod input;
mod keyboard;
mod navigation;
mod pointer;
mod touch;
pub use input::{InputEventInit, construct_native_input_event};
pub use keyboard::{construct_native_keyboard_event, keyboard_event_init_callback};
pub use navigation::{
    NAVIGATE_EVENT_SYNTHETIC_SLOT, construct_native_current_entry_change_event,
    construct_native_navigate_event,
};
pub use payload::{
    construct_native_form_data_event, construct_native_promise_rejection_event,
    construct_native_submit_event, construct_native_track_event,
};
pub use pointer::{
    InputEventCoordinates, MouseEventInit, PointerEventProperties, construct_native_drag_event,
    construct_native_mouse_event, construct_native_pointer_click, construct_native_pointer_event,
    construct_native_wheel_event, pointer_event_get_coalesced_events_callback,
    pointer_event_get_predicted_events_callback,
};
pub use values::{
    construct_native_clipboard_event, construct_native_command_event,
    construct_native_hash_change_event, construct_native_interest_event,
    construct_native_popstate_event, construct_native_toggle_event,
};
pub mod error;
pub mod font;
pub mod message;
mod payload;
pub mod security_policy;
pub mod storage;
pub mod submit;
mod templates;
mod values;

use accessors::*;
pub use accessors::{event_subclass_kind, new_uninitialized_before_unload_event};
use custom::custom_event_init_callback;
pub use templates::{EventTemplateDeclaration, template_declaration};

const CLOSE_EVENT_WAS_CLEAN_SLOT: &str = "__moliCloseEventWasClean";
const CLOSE_EVENT_CODE_SLOT: &str = "__moliCloseEventCode";
const CLOSE_EVENT_REASON_SLOT: &str = "__moliCloseEventReason";
pub const SUBMIT_EVENT_SUBMITTER_SLOT: &str = "__moliSubmitEventSubmitter";
const SUBMIT_EVENT_AGENT_INVOKED_SLOT: &str = "__moliSubmitEventAgentInvoked";
const SUBMIT_EVENT_RESPONSE_SLOT: &str = "__moliSubmitEventResponse";
pub const FORM_DATA_EVENT_FORM_DATA_SLOT: &str = "__moliFormDataEventFormData";
const PAGE_REVEAL_EVENT_VIEW_TRANSITION_SLOT: &str = "__moliPageRevealEventViewTransition";
const PAGE_SWAP_EVENT_ACTIVATION_SLOT: &str = "__moliPageSwapEventActivation";
const PAGE_SWAP_EVENT_VIEW_TRANSITION_SLOT: &str = "__moliPageSwapEventViewTransition";
pub const COMMAND_EVENT_SOURCE_SLOT: &str = "__moliCommandEventSource";
pub const COMMAND_EVENT_COMMAND_SLOT: &str = "__moliCommandEventCommand";
pub const TOGGLE_EVENT_SOURCE_SLOT: &str = "__moliToggleEventSource";
const TRACK_EVENT_TRACK_SLOT: &str = "__moliTrackEventTrack";
const CLIPBOARD_EVENT_CLIPBOARD_DATA_SLOT: &str = "__moliClipboardEventClipboardData";
const CLIPBOARD_CHANGE_EVENT_TYPES_SLOT: &str = "__moliClipboardChangeEventTypes";
const CLIPBOARD_CHANGE_EVENT_CHANGE_ID_SLOT: &str = "__moliClipboardChangeEventChangeId";
pub const EVENT_SUBCLASS_KIND_SLOT: &str = "__moliEventSubclassKind";
pub const BEFORE_UNLOAD_EVENT_RETURN_VALUE_SLOT: &str = "__moliBeforeUnloadEventReturnValue";
pub const FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT: &str = "__moliFontFaceSetLoadEventFontfaces";

enum SubclassInit<'s> {
    Pointing(pointer::PointerEventInitPayload<'s>),
    Keyboard(keyboard::KeyboardEventInit<'s>),
    Touch(touch::TouchEventInit<'s>),
    Input(input::InputEventValues<'s>),
    Basic(basic::BasicEventInit<'s>),
    CurrentEntry(navigation::NavigationCurrentEntryChangeEventInitMembers<'s>),
    Navigate(navigation::NavigateEventInitMembers<'s>, bool),
    Captured(captured_mouse::CapturedMouseEventInit),
    Custom(custom::CustomEventInit<'s>),
    Message(message::MessageEventInit<'s>),
    Storage(storage::StorageEventInitMembers<'s>),
    Values(values::ValueEventInit<'s>),
    Payload(payload::PayloadEventInit<'s>),
    ClipboardChange(clipboard_change::ClipboardChangeEventInitMembers<'s>),
    Font(font::FontFaceSetLoadEventInit<'s>),
    Security(security_policy::SecurityPolicyViolationEventInit),
    Error(error::ErrorEventInit<'s>),
}
impl<'s> SubclassInit<'s> {
    fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        kind: EventSubclassKind,
    ) -> Option<Self> {
        Some(match kind {
            EventSubclassKind::MouseEvent
            | EventSubclassKind::WheelEvent
            | EventSubclassKind::PointerEvent
            | EventSubclassKind::DragEvent => {
                Self::Pointing(pointer::PointerEventInitPayload::parse(scope, args, kind)?)
            }
            EventSubclassKind::KeyboardEvent => {
                Self::Keyboard(keyboard::parse_keyboard_event_init(scope, args)?)
            }
            EventSubclassKind::TouchEvent => {
                Self::Touch(parse_event_dictionary(scope, args, "TouchEvent")?)
            }
            EventSubclassKind::UiEvent
            | EventSubclassKind::FocusEvent
            | EventSubclassKind::TextEvent
            | EventSubclassKind::CompositionEvent => {
                Self::Basic(basic::BasicEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::NavigationCurrentEntryChangeEvent => Self::CurrentEntry(
                navigation::parse_current_entry_change_event_init(scope, args)?,
            ),
            EventSubclassKind::NavigateEvent => {
                Self::Navigate(navigation::parse_navigate_event_init(scope, args)?, true)
            }
            EventSubclassKind::CapturedMouseEvent => {
                Self::Captured(captured_mouse::CapturedMouseEventInit::parse(scope, args)?)
            }
            EventSubclassKind::MessageEvent => {
                Self::Message(message::parse_message_event_init(scope, args)?)
            }
            EventSubclassKind::StorageEvent => {
                Self::Storage(storage::parse_storage_event_init(scope, args)?)
            }
            EventSubclassKind::CustomEvent => {
                Self::Custom(parse_event_dictionary(scope, args, "CustomEvent")?)
            }
            EventSubclassKind::ClipboardEvent
            | EventSubclassKind::CloseEvent
            | EventSubclassKind::CommandEvent
            | EventSubclassKind::ToggleEvent
            | EventSubclassKind::InterestEvent
            | EventSubclassKind::PopStateEvent
            | EventSubclassKind::HashChangeEvent
            | EventSubclassKind::PageTransitionEvent
            | EventSubclassKind::PageRevealEvent
            | EventSubclassKind::PageSwapEvent => {
                Self::Values(values::ValueEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::PromiseRejectionEvent
            | EventSubclassKind::FormDataEvent
            | EventSubclassKind::SubmitEvent
            | EventSubclassKind::TrackEvent => {
                Self::Payload(payload::PayloadEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::ClipboardChangeEvent => Self::ClipboardChange(
                clipboard_change::parse_clipboard_change_event_init(scope, args)?,
            ),
            EventSubclassKind::FontFaceSetLoadEvent => {
                Self::Font(parse_event_dictionary(scope, args, "FontFaceSetLoadEvent")?)
            }
            EventSubclassKind::SecurityPolicyViolationEvent => Self::Security(
                security_policy::parse_security_policy_violation_event_init(scope, args)?,
            ),
            EventSubclassKind::ErrorEvent => {
                Self::Error(error::parse_error_event_init(scope, args)?)
            }
            _ => unreachable!("host-dependent event constructor"),
        })
    }
    fn event_flags(&self) -> (bool, bool, bool) {
        match self {
            Self::Pointing(init) => init.event_flags(),
            Self::Keyboard(init) => init.event_flags(),
            Self::Touch(init) => init.event_flags(),
            Self::Input(init) => init.event_flags(),
            Self::Basic(init) => init.event_flags(),
            Self::CurrentEntry(init) => init.event_flags(),
            Self::Navigate(init, _) => init.event_flags(),
            Self::Captured(init) => init.event_flags(),
            Self::Custom(init) => init.event_flags(),
            Self::Message(init) => init.event_flags(),
            Self::Storage(init) => init.event_flags(),
            Self::Values(init) => init.event_flags(),
            Self::Payload(init) => init.event_flags(),
            Self::ClipboardChange(init) => init.event_flags(),
            Self::Font(init) => init.event_flags(),
            Self::Security(init) => init.event_flags(),
            Self::Error(init) => init.event_flags(),
        }
    }
    fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) -> bool {
        match self {
            Self::Pointing(init) => init.initialize(scope, event),
            Self::Keyboard(init) => keyboard::initialize_keyboard_event(scope, event, init),
            Self::Touch(init) => init.initialize(scope, event),
            Self::Input(init) => return init.initialize(scope, event),
            Self::Basic(init) => init.initialize(scope, event),
            Self::CurrentEntry(init) => init.initialize(scope, event),
            Self::Navigate(init, synthetic) => init.initialize(scope, event, synthetic),
            Self::Captured(init) => return init.initialize(scope, event),
            Self::Custom(init) => init.initialize(scope, event),
            Self::Message(init) => message::initialize_message_event(scope, event, init),
            Self::Storage(init) => storage::initialize_storage_event(scope, event, init),
            Self::Values(init) => init.initialize(scope, event),
            Self::Payload(init) => init.initialize(scope, event),
            Self::ClipboardChange(init) => {
                return clipboard_change::initialize_clipboard_change_event(scope, event, init);
            }
            Self::Font(init) => init.initialize(scope, event),
            Self::Security(init) => init.initialize(scope, event),
            Self::Error(init) => init.initialize(scope, event),
        }
        true
    }
}

/// Constructs one event after the complete WebIDL dictionary has been converted.
/// The initializer is a direct Rust call; it introduces no embedder callback.
pub fn construct_subclass<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    kind: EventSubclassKind,
    event_type: v8::Local<'s, v8::String>,
    flags: (bool, bool, bool),
    initialize: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>) -> bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let (bubbles, cancelable, composed) = flags;
    let event = new_event_state(scope);
    initialize_event_object_with_type(scope, event, event_type, bubbles, cancelable);
    define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, composed).into(),
    );
    if !initialize(scope, event) {
        return None;
    }
    web_api_interfaces::initialize(scope, event, kind.constructor_name())
        .expect("event primary interface should initialize");
    set_private_value(
        scope,
        event,
        EVENT_SUBCLASS_KIND_SLOT,
        v8::Integer::new(scope, kind as i32).into(),
    );
    initialize_event_wrapper(scope, wrapper, event)?;
    Some(wrapper)
}

fn construct_trusted_subclass<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: EventSubclassKind,
    event_type: &str,
    init: SubclassInit<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let event = construct_native_subclass(scope, kind, event_type, init)?;
    mark_event_trusted(scope, event);
    Some(event)
}

fn construct_native_subclass<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: EventSubclassKind,
    event_type: &str,
    init: SubclassInit<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let event_type = moli_v8_util::v8_string(scope, event_type)?;
    let wrapper = moli_v8_util::new_intrinsic_interface_instance(scope, kind.constructor_name())?;
    construct_subclass(
        scope,
        wrapper,
        kind,
        event_type,
        init.event_flags(),
        |scope, event| init.initialize(scope, event),
    )
}

fn native_event_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    prefix: &'static str,
    member: &'static str,
    options: webidl::InterfaceOptions,
) -> Option<v8::Local<'s, v8::Object>> {
    match webidl::convert_with_options::<webidl::InterfaceObject<'s>>(
        scope,
        value,
        webidl::Context::member(prefix, member),
        &options,
    ) {
        Ok(object) => Some(object.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn nullable_native_event_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
    prefix: &'static str,
    member: &'static str,
    options: webidl::InterfaceOptions,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    match value.filter(|value| !value.is_null_or_undefined()) {
        Some(value) => native_event_interface(scope, value, prefix, member, options).map(Some),
        None => Some(None),
    }
}

fn constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(kind) = EventSubclassKind::from_value(args.data()) else {
        throw_type_error(scope, "Invalid event subclass constructor.");
        return;
    };
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            &format!(
                "Failed to construct '{}': Please use the 'new' operator.",
                kind.constructor_name()
            ),
        );
        return;
    }
    if kind.required_arguments() == 2 && args.length() < 2 {
        throw_type_error(
            scope,
            &format!(
                "Failed to construct '{}': 2 arguments required, but only {} present.",
                kind.constructor_name(),
                args.length()
            ),
        );
        return;
    }
    let wrapper = args.this();
    let Some(event_type) = event_type_argument(scope, &args, kind.constructor_name()) else {
        return;
    };
    let Some(init) = SubclassInit::parse(scope, &args, kind) else {
        return;
    };
    if let Some(wrapper) = construct_subclass(
        scope,
        wrapper,
        kind,
        event_type,
        init.event_flags(),
        |scope, event| init.initialize(scope, event),
    ) {
        rv.set(wrapper.into());
    }
}

fn template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: EventSubclassKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let data: v8::Local<'s, v8::Value> = v8::Integer::new(scope, kind as i32).into();
    v8::FunctionTemplate::builder(constructor_callback)
        .data(data)
        .length(kind.required_arguments())
        .build(scope)
}

pub fn build_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: EventSubclassKind,
) -> Option<v8::Local<'s, v8::FunctionTemplate>> {
    match kind {
        EventSubclassKind::MouseEvent
        | EventSubclassKind::WheelEvent
        | EventSubclassKind::PointerEvent
        | EventSubclassKind::DragEvent
        | EventSubclassKind::KeyboardEvent
        | EventSubclassKind::TouchEvent
        | EventSubclassKind::UiEvent
        | EventSubclassKind::FocusEvent
        | EventSubclassKind::TextEvent
        | EventSubclassKind::CompositionEvent
        | EventSubclassKind::CapturedMouseEvent
        | EventSubclassKind::NavigateEvent
        | EventSubclassKind::NavigationCurrentEntryChangeEvent
        | EventSubclassKind::MessageEvent
        | EventSubclassKind::StorageEvent
        | EventSubclassKind::CustomEvent
        | EventSubclassKind::ClipboardEvent
        | EventSubclassKind::CloseEvent
        | EventSubclassKind::CommandEvent
        | EventSubclassKind::ToggleEvent
        | EventSubclassKind::InterestEvent
        | EventSubclassKind::PopStateEvent
        | EventSubclassKind::HashChangeEvent
        | EventSubclassKind::PageTransitionEvent
        | EventSubclassKind::PageRevealEvent
        | EventSubclassKind::PageSwapEvent
        | EventSubclassKind::PromiseRejectionEvent
        | EventSubclassKind::FormDataEvent
        | EventSubclassKind::SubmitEvent
        | EventSubclassKind::TrackEvent
        | EventSubclassKind::ClipboardChangeEvent
        | EventSubclassKind::FontFaceSetLoadEvent
        | EventSubclassKind::SecurityPolicyViolationEvent
        | EventSubclassKind::ErrorEvent => Some(template(scope, kind)),
        _ => None,
    }
}

pub use values::construct_original_page_transition_event;
