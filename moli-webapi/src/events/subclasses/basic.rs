use super::super::ui::UiEventInit;
use super::*;
use crate::interfaces as web_api_interfaces;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FocusEventInit", snapshot)]
pub(super) struct FocusEventInit<'s> {
    #[webidl(inherit)]
    ui: UiEventInit<'s>,
    #[webidl(nullable, interface = web_api_interfaces::EventTarget)]
    related_target: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CompositionEventInit", snapshot)]
pub(super) struct CompositionEventInit<'s> {
    #[webidl(inherit)]
    ui: UiEventInit<'s>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    data: webidl::DomString16,
}

impl<'s> FocusEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.ui.event_flags()
    }
    pub(super) fn initialize(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("FocusEventInit state");
    }
}

pub fn construct_native_focus_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    related_target: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
    view: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let related_target = nullable_native_event_interface(
        scope,
        related_target,
        "FocusEventInit",
        "relatedTarget",
        webidl::InterfaceOptions {
            name: "EventTarget",
            brand_check: web_api_interfaces::EventTarget::is_instance,
        },
    )?;
    let flags = EventInit {
        bubbles,
        cancelable: false,
        composed: true,
    };
    let init = FocusEventInit {
        ui: UiEventInit::from_native(flags, 0, view),
        related_target,
    };
    construct_native_subclass(
        scope,
        EventSubclassKind::FocusEvent,
        event_type,
        SubclassInit::Basic(BasicEventInit::Focus(init)),
    )
}

impl<'s> CompositionEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.ui.event_flags()
    }
    pub(super) fn initialize(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("CompositionEventInit state");
    }
}

// Legacy createEvent("TextEvent") uses an empty, internal dictionary.
pub fn initialize_text_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let value = init
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    match webidl::convert::<webidl::Dictionary<CompositionEventInit>>(
        scope,
        value,
        webidl::Context::argument("TextEvent", 2),
    ) {
        Ok(init) => {
            init.0.initialize(scope, event);
            true
        }
        Err(error) => {
            webidl::throw_error(scope, &error);
            false
        }
    }
}

pub(super) enum BasicEventInit<'s> {
    Ui(UiEventInit<'s>),
    Focus(FocusEventInit<'s>),
    Text(CompositionEventInit<'s>),
    Composition(CompositionEventInit<'s>),
}

impl<'s> BasicEventInit<'s> {
    pub(super) fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        kind: EventSubclassKind,
    ) -> Option<Self> {
        use crate::events::init::parse_event_dictionary;
        Some(match kind {
            EventSubclassKind::UiEvent => Self::Ui(parse_event_dictionary(scope, args, "UIEvent")?),
            EventSubclassKind::FocusEvent => {
                Self::Focus(parse_event_dictionary(scope, args, "FocusEvent")?)
            }
            EventSubclassKind::TextEvent => {
                Self::Text(parse_event_dictionary(scope, args, "TextEvent")?)
            }
            EventSubclassKind::CompositionEvent => {
                Self::Composition(parse_event_dictionary(scope, args, "CompositionEvent")?)
            }
            _ => unreachable!("only basic event kinds use BasicEventInit"),
        })
    }
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        match self {
            Self::Ui(init) => init.event_flags(),
            Self::Focus(init) => init.event_flags(),
            Self::Text(init) | Self::Composition(init) => init.event_flags(),
        }
    }
    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        match self {
            Self::Ui(init) => init.initialize(scope, event),
            Self::Focus(init) => init.initialize(scope, event),
            Self::Text(init) | Self::Composition(init) => init.initialize(scope, event),
        }
    }
}
