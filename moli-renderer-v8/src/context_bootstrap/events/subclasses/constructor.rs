use super::super::message;
use super::*;
use crate::web_api_interfaces;

fn event_subclass_kind_from_callback_data(
    data: v8::Local<'_, v8::Value>,
) -> Option<EventSubclassKind> {
    let value = i32::try_from(v8::Local::<v8::Integer>::try_from(data).ok()?.value()).ok()?;
    EventSubclassKind::from_i32(value)
}

enum EventSubclassInit<'s> {
    Custom(custom::CustomEventInit<'s>),
    Keyboard(keyboard::KeyboardEventInit<'s>),
    Storage(data::StorageEventInitMembers<'s>),
    SecurityPolicy(security_policy::SecurityPolicyViolationEventInit),
    Message(message::MessageEventInit<'s>),
    Error(error::ErrorEventInit<'s>),
    Legacy {
        bubbles: bool,
        cancelable: bool,
        composed: bool,
    },
}

impl<'s> EventSubclassInit<'s> {
    fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        kind: EventSubclassKind,
    ) -> Option<Self> {
        Some(match kind {
            EventSubclassKind::CustomEvent => Self::Custom(super::super::parse_event_dictionary(
                scope,
                args,
                "CustomEvent",
            )?),
            EventSubclassKind::KeyboardEvent => {
                Self::Keyboard(keyboard::parse_keyboard_event_init(scope, args)?)
            }
            EventSubclassKind::StorageEvent => {
                Self::Storage(data::parse_storage_event_init(scope, args)?)
            }
            EventSubclassKind::SecurityPolicyViolationEvent => Self::SecurityPolicy(
                security_policy::parse_security_policy_violation_event_init(scope, args)?,
            ),
            EventSubclassKind::MessageEvent => {
                Self::Message(message::parse_message_event_init(scope, args)?)
            }
            EventSubclassKind::ErrorEvent => {
                Self::Error(error::parse_error_event_init(scope, args)?)
            }
            _ => {
                let (bubbles, cancelable, composed) = read_event_init(scope, args);
                Self::Legacy {
                    bubbles,
                    cancelable,
                    composed,
                }
            }
        })
    }

    fn event_flags(&self) -> (bool, bool, bool) {
        match self {
            Self::Custom(init) => init.event_flags(),
            Self::Keyboard(init) => init.event_flags(),
            Self::Storage(init) => init.event_flags(),
            Self::SecurityPolicy(init) => init.event_flags(),
            Self::Message(init) => init.event_flags(),
            Self::Error(init) => init.event_flags(),
            Self::Legacy {
                bubbles,
                cancelable,
                composed,
            } => (*bubbles, *cancelable, *composed),
        }
    }

    fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
        kind: EventSubclassKind,
        init: Option<v8::Local<'s, v8::Object>>,
    ) -> bool {
        match self {
            Self::Custom(parsed) => parsed.initialize(scope, event),
            Self::Keyboard(parsed) => keyboard::initialize_keyboard_event(scope, event, parsed),
            Self::Storage(parsed) => data::initialize_storage_event(scope, event, parsed),
            Self::SecurityPolicy(parsed) => parsed.initialize(scope, event),
            Self::Message(parsed) => message::initialize_message_event(scope, event, parsed),
            Self::Error(parsed) => parsed.initialize(scope, event),
            Self::Legacy { .. } => return initialize_legacy_event(scope, event, kind, init),
        }
        true
    }
}

fn initialize_legacy_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    kind: EventSubclassKind,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    match kind {
        EventSubclassKind::UiEvent => {
            if !basic::initialize_ui_event(scope, event, init) {
                return false;
            }
        }
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
        EventSubclassKind::CompositionEvent => {
            if !basic::initialize_composition_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::MouseEvent => {
            if !pointer::initialize_mouse_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::DragEvent => {
            if !data::initialize_drag_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::ClipboardEvent => {
            data::initialize_clipboard_event(scope, event, init);
        }
        EventSubclassKind::CapturedMouseEvent => {
            if !data::initialize_captured_mouse_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::InputEvent => data::initialize_input_event(scope, event, init),
        EventSubclassKind::WheelEvent => {
            if !pointer::initialize_wheel_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::PointerEvent => {
            if !pointer::initialize_pointer_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::TouchEvent => {
            crate::context_bootstrap::touch_runtime::initialize_touch_event(scope, event, init);
        }
        EventSubclassKind::PromiseRejectionEvent => {
            if !data::initialize_promise_rejection_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::NavigationCurrentEntryChangeEvent => {
            if !data::initialize_navigation_current_entry_change_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::NavigateEvent => {
            if !data::initialize_navigate_event(scope, event, init) {
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
        EventSubclassKind::CommandEvent => data::initialize_command_event(scope, event, init),
        EventSubclassKind::ToggleEvent => {
            if !data::initialize_toggle_event(scope, event, init) {
                return false;
            }
        }
        EventSubclassKind::InterestEvent => data::initialize_interest_event(scope, event, init),
        EventSubclassKind::PopStateEvent => data::initialize_pop_state_event(scope, event, init),
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
        EventSubclassKind::CustomEvent
        | EventSubclassKind::KeyboardEvent
        | EventSubclassKind::StorageEvent
        | EventSubclassKind::SecurityPolicyViolationEvent
        | EventSubclassKind::MessageEvent
        | EventSubclassKind::ErrorEvent => unreachable!("event has typed initialization"),
    }

    true
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

    let wrapper = args.this();
    let event = new_event_state(scope);
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
    let Some(parsed) = EventSubclassInit::parse(scope, &args, kind) else {
        return;
    };
    let (bubbles, cancelable, composed) = parsed.event_flags();

    initialize_event_object_with_type(scope, event, event_type, bubbles, cancelable);
    define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, composed).into(),
    );

    if !parsed.initialize(scope, event, kind, init) {
        return;
    }

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

pub(in crate::context_bootstrap) fn build_event_subclass_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: EventSubclassKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let data: v8::Local<'s, v8::Value> = v8::Integer::new(scope, kind as i32).into();
    let length = if matches!(kind, EventSubclassKind::NavigateEvent) {
        2
    } else {
        1
    };
    v8::FunctionTemplate::builder(event_subclass_constructor_callback)
        .data(data)
        .length(length)
        .build(scope)
}
