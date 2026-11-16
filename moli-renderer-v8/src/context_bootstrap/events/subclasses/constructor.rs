use super::*;

// Every kind has a complete derived dictionary. A new kind must select its
// dictionary explicitly; there is no legacy EventInit-only fallback.
enum SubclassInit<'s> {
    Basic(basic::BasicEventInit<'s>),
    Pointing(pointer::PointerEventInitPayload<'s>),
    Interaction(input::InteractionEventInit<'s>),
    Keyboard(keyboard::KeyboardEventInit<'s>),
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
            | EventSubclassKind::CompositionEvent => {
                Self::Basic(basic::BasicEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::MouseEvent
            | EventSubclassKind::WheelEvent
            | EventSubclassKind::PointerEvent
            | EventSubclassKind::DragEvent => {
                Self::Pointing(pointer::PointerEventInitPayload::parse(scope, args, kind)?)
            }
            EventSubclassKind::InputEvent
            | EventSubclassKind::TouchEvent
            | EventSubclassKind::CapturedMouseEvent => {
                Self::Interaction(input::InteractionEventInit::parse(scope, args, kind)?)
            }
            EventSubclassKind::KeyboardEvent => {
                Self::Keyboard(keyboard::parse_keyboard_event_init(scope, args)?)
            }
            EventSubclassKind::NavigationCurrentEntryChangeEvent => Self::CurrentEntry(
                navigation_init::parse_current_entry_change_event_init(scope, args)?,
            ),
            EventSubclassKind::NavigateEvent => {
                Self::Navigate(navigation_init::parse_navigate_event_init(scope, args)?)
            }
            _ => unreachable!("value event templates are constructed in moli-webapi"),
        })
    }

    fn event_flags(&self) -> (bool, bool, bool) {
        match self {
            Self::Basic(init) => init.event_flags(),
            Self::Pointing(init) => init.event_flags(),
            Self::Interaction(init) => init.event_flags(),
            Self::Keyboard(init) => init.event_flags(),
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
            Self::Interaction(init) => return init.initialize(scope, event),
            Self::Keyboard(init) => keyboard::initialize_keyboard_event(scope, event, init),
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
    if let Some(wrapper) = moli_webapi::events::subclasses::construct_subclass(
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

pub(in crate::context_bootstrap) fn build_event_subclass_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: EventSubclassKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    if let Some(template) = moli_webapi::events::subclasses::build_template(scope, kind) {
        return template;
    }
    let data: v8::Local<'s, v8::Value> = v8::Integer::new(scope, kind as i32).into();
    v8::FunctionTemplate::builder(event_subclass_constructor_callback)
        .data(data)
        .length(kind.required_arguments())
        .build(scope)
}
