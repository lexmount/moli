use super::super::{EventInit, parse_event_dictionary};
use super::*;
use crate::{web_api_interfaces, webidl};
use webidl::WebIdlDictionarySerialize;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ClipboardEventInit", snapshot)]
pub(super) struct ClipboardInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, interface = web_api_interfaces::DataTransfer)]
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
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw", snapshot_order = 1)]
    command: webidl::DomString16,
    #[webidl(nullable, interface = web_api_interfaces::Element, snapshot_order = 0)]
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
    #[webidl(nullable, interface = web_api_interfaces::Element, snapshot_order = 2)]
    #[webidl(snapshot_attributes = v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE | v8::PropertyAttribute::DONT_ENUM)]
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

pub(super) enum ValueEventInit<'s> {
    Clipboard(ClipboardInit<'s>),
    Close(CloseInit),
    Command(CommandInit<'s>),
    Toggle(ToggleInit<'s>),
    Interest(InterestInit<'s>),
    PopState(PopStateInit<'s>),
    HashChange(HashChangeInit),
    PageTransition(PageTransitionInit),
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
        }
        .expect("value event dictionary state");
    }
}
