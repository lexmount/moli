use super::super::message;
use super::*;
use crate::context_bootstrap::css_fontface_runtime::FontFaceSetLoadEventInit;
use crate::web_api_interfaces;

fn event_subclass_kind_from_callback_data(
    data: v8::Local<'_, v8::Value>,
) -> Option<EventSubclassKind> {
    let value = i32::try_from(v8::Local::<v8::Integer>::try_from(data).ok()?.value()).ok()?;
    EventSubclassKind::from_i32(value)
}

fn required_arguments(kind: EventSubclassKind) -> i32 {
    match kind {
        EventSubclassKind::NavigateEvent
        | EventSubclassKind::NavigationCurrentEntryChangeEvent
        | EventSubclassKind::PromiseRejectionEvent
        | EventSubclassKind::FormDataEvent => 2,
        _ => 1,
    }
}

// Every kind has a complete derived dictionary. A new kind must select its
// dictionary explicitly; there is no legacy EventInit-only fallback.
enum SubclassInit<'s> {
    Basic(basic::BasicEventInit<'s>),
    Pointing(pointer::PointerEventInitPayload<'s>),
    Values(values::ValueEventInit<'s>),
    Interaction(input::InteractionEventInit<'s>),
    Payload(payload::PayloadEventInit<'s>),
    Font(FontFaceSetLoadEventInit<'s>),
    Storage(data::StorageEventInitMembers<'s>),
    Keyboard(keyboard::KeyboardEventInit<'s>),
    Security(security_policy::SecurityPolicyViolationEventInit),
    Message(message::MessageEventInit<'s>),
    Error(error::ErrorEventInit<'s>),
    CurrentEntry(navigation_init::NavigationCurrentEntryChangeEventInitMembers<'s>),
    Navigate(navigation_init::NavigateEventInitMembers<'s>),
}

impl<'s> SubclassInit<'s> {
    fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        kind: EventSubclassKind,
    ) -> Option<Self> {
        Some(match kind {
            EventSubclassKind::UiEvent
            | EventSubclassKind::FocusEvent
            | EventSubclassKind::TextEvent
            | EventSubclassKind::CompositionEvent
            | EventSubclassKind::CustomEvent => {
                Self::Basic(basic::BasicEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::MouseEvent
            | EventSubclassKind::WheelEvent
            | EventSubclassKind::PointerEvent
            | EventSubclassKind::DragEvent => {
                Self::Pointing(pointer::PointerEventInitPayload::parse(scope, args, kind)?)
            }
            EventSubclassKind::ClipboardEvent
            | EventSubclassKind::CloseEvent
            | EventSubclassKind::CommandEvent
            | EventSubclassKind::ToggleEvent
            | EventSubclassKind::InterestEvent
            | EventSubclassKind::PopStateEvent
            | EventSubclassKind::HashChangeEvent
            | EventSubclassKind::PageTransitionEvent => {
                Self::Values(values::ValueEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::InputEvent
            | EventSubclassKind::TouchEvent
            | EventSubclassKind::CapturedMouseEvent => {
                Self::Interaction(input::InteractionEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::PromiseRejectionEvent
            | EventSubclassKind::FormDataEvent
            | EventSubclassKind::SubmitEvent
            | EventSubclassKind::TrackEvent => {
                Self::Payload(payload::PayloadEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::FontFaceSetLoadEvent => Self::Font(
                super::super::parse_event_dictionary(scope, args, "FontFaceSetLoadEvent")?,
            ),
            EventSubclassKind::StorageEvent => {
                Self::Storage(data::parse_storage_event_init(scope, args)?)
            }
            EventSubclassKind::KeyboardEvent => {
                Self::Keyboard(keyboard::parse_keyboard_event_init(scope, args)?)
            }
            EventSubclassKind::SecurityPolicyViolationEvent => Self::Security(
                security_policy::parse_security_policy_violation_event_init(scope, args)?,
            ),
            EventSubclassKind::MessageEvent => {
                Self::Message(message::parse_message_event_init(scope, args)?)
            }
            EventSubclassKind::ErrorEvent => {
                Self::Error(error::parse_error_event_init(scope, args)?)
            }
            EventSubclassKind::NavigationCurrentEntryChangeEvent => Self::CurrentEntry(
                navigation_init::parse_current_entry_change_event_init(scope, args)?,
            ),
            EventSubclassKind::NavigateEvent => {
                Self::Navigate(navigation_init::parse_navigate_event_init(scope, args)?)
            }
        })
    }

    fn event_flags(&self) -> (bool, bool, bool) {
        match self {
            Self::Basic(init) => init.event_flags(),
            Self::Pointing(init) => init.event_flags(),
            Self::Values(init) => init.event_flags(),
            Self::Interaction(init) => init.event_flags(),
            Self::Payload(init) => init.event_flags(),
            Self::Font(init) => init.event_flags(),
            Self::Storage(init) => init.event_flags(),
            Self::Keyboard(init) => init.event_flags(),
            Self::Security(init) => init.event_flags(),
            Self::Message(init) => init.event_flags(),
            Self::Error(init) => init.event_flags(),
            Self::CurrentEntry(init) => init.event_flags(),
            Self::Navigate(init) => init.event_flags(),
        }
    }

    fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) -> bool {
        match self {
            Self::Basic(init) => init.initialize(scope, event),
            Self::Pointing(init) => init.initialize(scope, event),
            Self::Values(init) => init.initialize(scope, event),
            Self::Interaction(init) => return init.initialize(scope, event),
            Self::Payload(init) => init.initialize(scope, event),
            Self::Font(init) => init.initialize(scope, event),
            Self::Storage(init) => data::initialize_storage_event(scope, event, init),
            Self::Keyboard(init) => keyboard::initialize_keyboard_event(scope, event, init),
            Self::Security(init) => init.initialize(scope, event),
            Self::Message(init) => message::initialize_message_event(scope, event, init),
            Self::Error(init) => init.initialize(scope, event),
            Self::CurrentEntry(init) => {
                data::initialize_navigation_current_entry_change_event(scope, event, init)
            }
            Self::Navigate(init) => data::initialize_navigate_event(scope, event, init),
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
    if required_arguments(kind) == 2 && args.length() < 2 {
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
    let (bubbles, cancelable, composed) = init.event_flags();
    let event = new_event_state(scope);
    initialize_event_object_with_type(scope, event, event_type, bubbles, cancelable);
    define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, composed).into(),
    );
    if !init.initialize(scope, event) {
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
    v8::FunctionTemplate::builder(event_subclass_constructor_callback)
        .data(data)
        .length(required_arguments(kind))
        .build(scope)
}
