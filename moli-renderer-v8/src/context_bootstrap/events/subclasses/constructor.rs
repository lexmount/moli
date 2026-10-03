use super::super::{message, ui};
use super::*;
use crate::web_api_interfaces;

fn event_subclass_kind_from_callback_data(
    data: v8::Local<'_, v8::Value>,
) -> Option<EventSubclassKind> {
    let value = i32::try_from(v8::Local::<v8::Integer>::try_from(data).ok()?.value()).ok()?;
    EventSubclassKind::from_i32(value)
}

// A converted dictionary retains exactly its payload type. Parsing, base
// flags and initialization share this dispatch instead of independent Options.
enum EventSubclassInit<'s> {
    Ui(ui::UiEventInit<'s>),
    Composition(ui::CompositionEventInit<'s>),
    Clipboard(data::ClipboardEventInitMembers<'s>),
    ClipboardChange(data::ClipboardChangeEventInitMembers<'s>),
    Keyboard(keyboard::KeyboardEventInit<'s>),
    Mouse(pointer::MouseEventInit<'s>),
    Wheel(pointer::WheelEventInit<'s>),
    Pointer(pointer::PointerEventInit<'s>),
    Drag(pointer::DragEventInit<'s>),
    Message(message::MessageEventInit<'s>),
    Storage(data::StorageEventInitMembers<'s>),
    Error(error::ErrorEventInit<'s>),
    SecurityPolicy(security_policy::SecurityPolicyViolationEventInit),
    Toggle(interaction::ToggleEventInit<'s>),
    Command(interaction::CommandEventInit<'s>),
    CurrentEntryChange(navigation_init::NavigationCurrentEntryChangeEventInitMembers<'s>),
    Navigate(navigation_init::NavigateEventInitMembers<'s>),
    Legacy,
}

impl<'s> EventSubclassInit<'s> {
    fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        kind: EventSubclassKind,
    ) -> Option<Self> {
        Some(match kind {
            EventSubclassKind::UiEvent => Self::Ui(ui::parse_ui_event_init(scope, args)?),
            EventSubclassKind::CompositionEvent => {
                Self::Composition(ui::parse_composition_event_init(scope, args)?)
            }
            EventSubclassKind::ClipboardEvent => {
                Self::Clipboard(data::parse_clipboard_event_init(scope, args)?)
            }
            EventSubclassKind::ClipboardChangeEvent => {
                Self::ClipboardChange(data::parse_clipboard_change_event_init(scope, args)?)
            }
            EventSubclassKind::KeyboardEvent => {
                Self::Keyboard(keyboard::parse_keyboard_event_init(scope, args)?)
            }
            EventSubclassKind::MouseEvent => {
                Self::Mouse(ui::parse_ui_dictionary(scope, args, "MouseEvent")?)
            }
            EventSubclassKind::WheelEvent => {
                Self::Wheel(ui::parse_ui_dictionary(scope, args, "WheelEvent")?)
            }
            EventSubclassKind::PointerEvent => {
                Self::Pointer(ui::parse_ui_dictionary(scope, args, "PointerEvent")?)
            }
            EventSubclassKind::DragEvent => {
                Self::Drag(ui::parse_ui_dictionary(scope, args, "DragEvent")?)
            }
            EventSubclassKind::MessageEvent => {
                Self::Message(message::parse_message_event_init(scope, args)?)
            }
            EventSubclassKind::StorageEvent => {
                Self::Storage(data::parse_storage_event_init(scope, args)?)
            }
            EventSubclassKind::ErrorEvent => {
                Self::Error(error::parse_error_event_init(scope, args)?)
            }
            EventSubclassKind::SecurityPolicyViolationEvent => Self::SecurityPolicy(
                security_policy::parse_security_policy_violation_event_init(scope, args)?,
            ),
            EventSubclassKind::ToggleEvent => Self::Toggle(
                interaction::parse_interaction_event_init(scope, args, "ToggleEvent")?,
            ),
            EventSubclassKind::CommandEvent => Self::Command(
                interaction::parse_interaction_event_init(scope, args, "CommandEvent")?,
            ),
            EventSubclassKind::NavigationCurrentEntryChangeEvent => Self::CurrentEntryChange(
                navigation_init::parse_current_entry_change_event_init(scope, args)?,
            ),
            EventSubclassKind::NavigateEvent => {
                Self::Navigate(navigation_init::parse_navigate_event_init(scope, args)?)
            }
            _ => Self::Legacy,
        })
    }

    fn event_flags(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
    ) -> (bool, bool, bool) {
        match self {
            Self::Ui(init) => init.event_flags(),
            Self::Composition(init) => init.event_flags(),
            Self::Clipboard(init) => init.event_flags(),
            Self::ClipboardChange(init) => init.event_flags(),
            Self::Keyboard(init) => init.event_flags(),
            Self::Mouse(init) => init.event_flags(),
            Self::Wheel(init) => init.event_flags(),
            Self::Pointer(init) => init.event_flags(),
            Self::Drag(init) => init.event_flags(),
            Self::Message(init) => init.event_flags(),
            Self::Storage(init) => init.event_flags(),
            Self::Error(init) => init.event_flags(),
            Self::SecurityPolicy(init) => init.event_flags(),
            Self::Toggle(init) => init.event_flags(),
            Self::Command(init) => init.event_flags(),
            Self::CurrentEntryChange(init) => init.event_flags(),
            Self::Navigate(init) => init.event_flags(),
            Self::Legacy => read_event_init(scope, args),
        }
    }

    fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
        kind: EventSubclassKind,
        legacy_init: Option<v8::Local<'s, v8::Object>>,
    ) -> bool {
        match self {
            Self::Ui(init) => init.initialize(scope, event),
            Self::Composition(init) => ui::initialize_composition_event(scope, event, init),
            Self::Clipboard(init) => data::initialize_clipboard_event(scope, event, init),
            Self::ClipboardChange(init) => {
                return data::initialize_clipboard_change_event(scope, event, init);
            }
            Self::Keyboard(init) => keyboard::initialize_keyboard_event(scope, event, init),
            Self::Mouse(init) => init.initialize(scope, event),
            Self::Wheel(init) => init.initialize(scope, event),
            Self::Pointer(init) => init.initialize(scope, event),
            Self::Drag(init) => init.initialize(scope, event),
            Self::Message(init) => message::initialize_message_event(scope, event, init),
            Self::Storage(init) => data::initialize_storage_event(scope, event, init),
            Self::Error(init) => init.initialize(scope, event),
            Self::SecurityPolicy(init) => init.initialize(scope, event),
            Self::Toggle(init) => init.initialize(scope, event),
            Self::Command(init) => init.initialize(scope, event),
            Self::CurrentEntryChange(init) => {
                data::initialize_navigation_current_entry_change_event(scope, event, init)
            }
            Self::Navigate(init) => data::initialize_navigate_event(scope, event, init),
            Self::Legacy => return initialize_legacy_event(scope, event, kind, legacy_init),
        }
        true
    }
}

fn event_subclass_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(kind) = event_subclass_kind_from_callback_data(args.data()) else {
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

    if matches!(
        kind,
        EventSubclassKind::NavigateEvent | EventSubclassKind::NavigationCurrentEntryChangeEvent
    ) && args.length() < 2
    {
        throw_type_error(
            scope,
            &format!(
                "Failed to construct '{}': 2 arguments required.",
                kind.constructor_name()
            ),
        );
        return;
    }

    let Some(event_type) = event_type_argument(scope, &args, kind.constructor_name()) else {
        return;
    };
    let init = {
        let init_arg = args.get(1);
        if init_arg.is_null_or_undefined() || !init_arg.is_object() {
            None
        } else {
            init_arg.to_object(scope)
        }
    };
    let Some(parsed_init) = EventSubclassInit::parse(scope, &args, kind) else {
        return;
    };
    let (bubbles, cancelable, composed) = parsed_init.event_flags(scope, &args);
    let event = new_event_state(scope);
    initialize_event_object_with_type(scope, event, event_type, bubbles, cancelable);
    define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, composed).into(),
    );
    if !parsed_init.initialize(scope, event, kind, init) {
        return;
    }
    let wrapper = args.this();
    web_api_interfaces::initialize(scope, event, kind.constructor_name())
        .expect("event primary interface should initialize");
    set_private_value(
        scope,
        event,
        EVENT_SUBCLASS_KIND_SLOT,
        v8::Integer::new(scope, kind as i32).into(),
    );
    if initialize_event_wrapper(scope, wrapper, event).is_some() {
        rv.set(wrapper.into());
    }
}

fn initialize_legacy_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    kind: EventSubclassKind,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    match kind {
        EventSubclassKind::FocusEvent => {
            if !basic::initialize_focus_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::TextEvent => {
            if !basic::initialize_text_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::CustomEvent => basic::initialize_custom_event(scope, event, init),
        EventSubclassKind::CapturedMouseEvent => {
            if !data::initialize_captured_mouse_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::InputEvent => data::initialize_input_event(scope, event, init),
        EventSubclassKind::TouchEvent => {
            crate::context_bootstrap::touch_runtime::initialize_touch_event(scope, event, init);
        }
        EventSubclassKind::PromiseRejectionEvent => {
            if !data::initialize_promise_rejection_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::CloseEvent => data::initialize_close_event(scope, event, init),
        EventSubclassKind::SubmitEvent => {
            if !data::initialize_submit_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::FormDataEvent => {
            if !data::initialize_form_data_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::InterestEvent => data::initialize_interest_event(scope, event, init),
        EventSubclassKind::PopStateEvent => {
            if !data::initialize_pop_state_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::HashChangeEvent => {
            if !data::initialize_hash_change_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::PageTransitionEvent => {
            data::initialize_page_transition_event(scope, event, init);
        }
        EventSubclassKind::TrackEvent => data::initialize_track_event(scope, event, init),
        EventSubclassKind::FontFaceSetLoadEvent => {
            if !crate::context_bootstrap::css_fontface_runtime::initialize_font_face_set_load_event(
                scope, event, init,
            ) {
                return false;
            }
        }
        EventSubclassKind::ClipboardChangeEvent
        | EventSubclassKind::ClipboardEvent
        | EventSubclassKind::CommandEvent
        | EventSubclassKind::CompositionEvent
        | EventSubclassKind::ErrorEvent
        | EventSubclassKind::KeyboardEvent
        | EventSubclassKind::MouseEvent
        | EventSubclassKind::WheelEvent
        | EventSubclassKind::PointerEvent
        | EventSubclassKind::DragEvent
        | EventSubclassKind::MessageEvent
        | EventSubclassKind::NavigateEvent
        | EventSubclassKind::NavigationCurrentEntryChangeEvent
        | EventSubclassKind::SecurityPolicyViolationEvent
        | EventSubclassKind::StorageEvent
        | EventSubclassKind::ToggleEvent
        | EventSubclassKind::UiEvent => {
            unreachable!("typed event initialization must retain its payload")
        }
    }
    true
}

pub(in crate::context_bootstrap) fn build_event_subclass_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: EventSubclassKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let data: v8::Local<'s, v8::Value> = v8::Integer::new(scope, kind as i32).into();
    let length = if matches!(
        kind,
        EventSubclassKind::NavigateEvent | EventSubclassKind::NavigationCurrentEntryChangeEvent
    ) {
        2
    } else {
        1
    };
    v8::FunctionTemplate::builder(event_subclass_constructor_callback)
        .data(data)
        .length(length)
        .build(scope)
}
