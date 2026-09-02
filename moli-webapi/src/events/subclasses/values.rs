use super::super::init::{EventInit, parse_event_dictionary};
use super::*;
use crate::interfaces as web_api_interfaces;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ClipboardEventInit", snapshot)]
pub(super) struct ClipboardInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, interface = web_api_interfaces::DataTransfer, snapshot_slot = CLIPBOARD_EVENT_CLIPBOARD_DATA_SLOT)]
    clipboard_data: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CloseEventInit", snapshot)]
pub(super) struct CloseInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(default = 0, snapshot_slot = CLOSE_EVENT_CODE_SLOT)]
    code: u16,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw", snapshot_slot = CLOSE_EVENT_REASON_SLOT)]
    reason: webidl::DomString16,
    #[webidl(default = false, snapshot_slot = CLOSE_EVENT_WAS_CLEAN_SLOT)]
    was_clean: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CommandEventInit", snapshot)]
pub(super) struct CommandInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw", snapshot_order = 1, snapshot_slot = COMMAND_EVENT_COMMAND_SLOT)]
    command: webidl::DomString16,
    #[webidl(nullable, interface = web_api_interfaces::Element, snapshot_order = 0, snapshot_slot = COMMAND_EVENT_SOURCE_SLOT)]
    source: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ToggleEventInit", snapshot)]
pub(super) struct ToggleInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw", snapshot_order = 1)]
    #[webidl(snapshot_attributes = v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE | v8::PropertyAttribute::DONT_ENUM)]
    new_state: webidl::DomString16,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw", snapshot_order = 0)]
    #[webidl(snapshot_attributes = v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE | v8::PropertyAttribute::DONT_ENUM)]
    old_state: webidl::DomString16,
    #[webidl(nullable, interface = web_api_interfaces::Element, snapshot_order = 2, snapshot_slot = TOGGLE_EVENT_SOURCE_SLOT)]
    source: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "InterestEventInit", snapshot)]
pub(super) struct InterestInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, interface = web_api_interfaces::Element)]
    source: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PopStateEventInit", snapshot)]
pub(super) struct PopStateInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "hasUAVisualTransition", default = false, snapshot_order = 1)]
    has_ua_visual_transition: bool,
    #[webidl(converter = "raw", snapshot_default = v8::null(scope).into(), snapshot_order = 0)]
    state: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "HashChangeEventInit", snapshot)]
pub(super) struct HashChangeInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "newURL", default = webidl::DomString16(Vec::new()), converter = "raw")]
    new_url: webidl::DomString16,
    #[webidl(name = "oldURL", default = webidl::DomString16(Vec::new()), converter = "raw")]
    old_url: webidl::DomString16,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PageTransitionEventInit", snapshot)]
pub(super) struct PageTransitionInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(default = false)]
    persisted: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PageRevealEventInit", snapshot)]
pub(super) struct PageRevealInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, interface = web_api_interfaces::ViewTransition, snapshot_slot = PAGE_REVEAL_EVENT_VIEW_TRANSITION_SLOT)]
    view_transition: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PageSwapEventInit", snapshot)]
pub(super) struct PageSwapInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, interface = web_api_interfaces::NavigationActivation, snapshot_slot = PAGE_SWAP_EVENT_ACTIVATION_SLOT)]
    activation: Option<v8::Local<'s, v8::Object>>,
    #[webidl(nullable, interface = web_api_interfaces::ViewTransition, snapshot_slot = PAGE_SWAP_EVENT_VIEW_TRANSITION_SLOT)]
    view_transition: Option<v8::Local<'s, v8::Object>>,
}

pub(super) enum ValueEventInit<'s> {
    Clipboard(ClipboardInit<'s>),
    Close(CloseInit),
    Command(CommandInit<'s>),
    Toggle(ToggleInit<'s>),
    Interest(InterestInit<'s>),
    PopState(PopStateInit<'s>),
    HashChange(HashChangeInit),
    PageTransition(PageTransitionInit),
    PageReveal(PageRevealInit<'s>),
    PageSwap(PageSwapInit<'s>),
}

impl<'s> ValueEventInit<'s> {
    pub(super) fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        kind: EventSubclassKind,
    ) -> Option<Self> {
        Some(match kind {
            EventSubclassKind::ClipboardEvent => {
                Self::Clipboard(parse_event_dictionary(scope, args, "ClipboardEvent")?)
            }
            EventSubclassKind::CloseEvent => {
                Self::Close(parse_event_dictionary(scope, args, "CloseEvent")?)
            }
            EventSubclassKind::CommandEvent => {
                Self::Command(parse_event_dictionary(scope, args, "CommandEvent")?)
            }
            EventSubclassKind::ToggleEvent => {
                Self::Toggle(parse_event_dictionary(scope, args, "ToggleEvent")?)
            }
            EventSubclassKind::InterestEvent => {
                Self::Interest(parse_event_dictionary(scope, args, "InterestEvent")?)
            }
            EventSubclassKind::PopStateEvent => {
                Self::PopState(parse_event_dictionary(scope, args, "PopStateEvent")?)
            }
            EventSubclassKind::HashChangeEvent => {
                Self::HashChange(parse_event_dictionary(scope, args, "HashChangeEvent")?)
            }
            EventSubclassKind::PageRevealEvent => {
                Self::PageReveal(parse_event_dictionary(scope, args, "PageRevealEvent")?)
            }
            EventSubclassKind::PageSwapEvent => {
                Self::PageSwap(parse_event_dictionary(scope, args, "PageSwapEvent")?)
            }
            EventSubclassKind::PageTransitionEvent => {
                Self::PageTransition(parse_event_dictionary(scope, args, "PageTransitionEvent")?)
            }
            _ => unreachable!("only value event kinds use ValueEventInit"),
        })
    }

    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        let base = match self {
            Self::Clipboard(init) => &init.base,
            Self::Close(init) => &init.base,
            Self::Command(init) => &init.base,
            Self::Toggle(init) => &init.base,
            Self::Interest(init) => &init.base,
            Self::PopState(init) => &init.base,
            Self::HashChange(init) => &init.base,
            Self::PageTransition(init) => &init.base,
            Self::PageReveal(init) => &init.base,
            Self::PageSwap(init) => &init.base,
        };
        (base.bubbles, base.cancelable, base.composed)
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        match self {
            Self::Clipboard(init) => init.initialize_snapshot(scope, event),
            Self::Close(init) => init.initialize_snapshot(scope, event),
            Self::Command(init) => init.initialize_snapshot(scope, event),
            Self::Toggle(init) => init.initialize_snapshot(scope, event),
            Self::Interest(init) => init.initialize_snapshot(scope, event),
            Self::PopState(init) => init.initialize_snapshot(scope, event),
            Self::HashChange(init) => init.initialize_snapshot(scope, event),
            Self::PageTransition(init) => init.initialize_snapshot(scope, event),
            Self::PageReveal(init) => init.initialize_snapshot(scope, event),
            Self::PageSwap(init) => init.initialize_snapshot(scope, event),
        }
        .expect("value event dictionary state");
    }
}

fn native_source_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    prefix: &'static str,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    nullable_native_event_interface(
        scope,
        Some(value),
        prefix,
        "source",
        webidl::InterfaceOptions {
            name: "Element",
            brand_check: web_api_interfaces::Element::is_instance,
        },
    )
}

pub fn construct_native_command_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    command: &str,
    source: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = CommandInit {
        base: EventInit {
            cancelable: true,
            ..EventInit::default()
        },
        command: webidl::DomString16(command.encode_utf16().collect()),
        source: native_source_element(scope, source, "CommandEventInit")?,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::CommandEvent,
        "command",
        SubclassInit::Values(ValueEventInit::Command(init)),
    )
}

pub fn construct_native_toggle_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    old_state: &str,
    new_state: &str,
    cancelable: bool,
    source: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = ToggleInit {
        base: EventInit {
            cancelable,
            ..EventInit::default()
        },
        old_state: webidl::DomString16(old_state.encode_utf16().collect()),
        new_state: webidl::DomString16(new_state.encode_utf16().collect()),
        source: native_source_element(scope, source, "ToggleEventInit")?,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::ToggleEvent,
        event_type,
        SubclassInit::Values(ValueEventInit::Toggle(init)),
    )
}

pub fn construct_native_interest_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    source: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = InterestInit {
        base: EventInit::default(),
        source: native_source_element(scope, source, "InterestEventInit")?,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::InterestEvent,
        event_type,
        SubclassInit::Values(ValueEventInit::Interest(init)),
    )
}

pub fn construct_original_page_transition_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    persisted: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = ValueEventInit::PageTransition(PageTransitionInit {
        base: EventInit {
            bubbles: true,
            cancelable: true,
            ..EventInit::default()
        },
        persisted,
    });
    construct_trusted_subclass(
        scope,
        EventSubclassKind::PageTransitionEvent,
        event_type,
        SubclassInit::Values(init),
    )
}

pub fn construct_native_popstate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = ValueEventInit::PopState(PopStateInit {
        base: EventInit::default(),
        state: (!state.is_undefined()).then_some(state),
        has_ua_visual_transition: false,
    });
    construct_trusted_subclass(
        scope,
        EventSubclassKind::PopStateEvent,
        "popstate",
        SubclassInit::Values(init),
    )
}

pub fn construct_native_hash_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    old_url: &str,
    new_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = ValueEventInit::HashChange(HashChangeInit {
        base: EventInit::default(),
        old_url: webidl::DomString16(old_url.encode_utf16().collect()),
        new_url: webidl::DomString16(new_url.encode_utf16().collect()),
    });
    construct_trusted_subclass(
        scope,
        EventSubclassKind::HashChangeEvent,
        "hashchange",
        SubclassInit::Values(init),
    )
}
