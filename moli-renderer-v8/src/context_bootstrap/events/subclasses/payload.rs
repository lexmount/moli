use super::super::{EventInit, parse_event_dictionary};
use super::*;
use crate::{web_api_interfaces, webidl};
use webidl::WebIdlDictionarySerialize;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PromiseRejectionEventInit", snapshot)]
pub(super) struct PromiseRejectionInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    // The shared Promise converter retains actual Promises across realms and
    // resolves other values; a then getter failure rejects that Promise.
    #[webidl(required, converter = "raw")]
    promise: v8::Local<'s, v8::Promise>,
    #[webidl(converter = "raw", snapshot_default = v8::undefined(scope).into())]
    reason: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FormDataEventInit", snapshot)]
pub(super) struct FormDataInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::FormData, snapshot_slot = FORM_DATA_EVENT_FORM_DATA_SLOT)]
    form_data: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SubmitEventInit", snapshot)]
pub(super) struct SubmitInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(default = false, snapshot_slot = SUBMIT_EVENT_AGENT_INVOKED_SLOT)]
    agent_invoked: bool,
    #[webidl(nullable, interface = web_api_interfaces::HTMLElement, snapshot_slot = SUBMIT_EVENT_SUBMITTER_SLOT)]
    submitter: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TrackEventInit", snapshot)]
pub(super) struct TrackInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    // TextTrack is the currently implemented member of the track union. The
    // runtime does not expose AudioTrack or VideoTrack native objects yet.
    #[webidl(nullable, interface = web_api_interfaces::TextTrack, snapshot_slot = TRACK_EVENT_TRACK_SLOT)]
    track: Option<v8::Local<'s, v8::Object>>,
}

pub(super) enum PayloadEventInit<'s> {
    Promise(PromiseRejectionInit<'s>),
    FormData(FormDataInit<'s>),
    Submit(SubmitInit<'s>),
    Track(TrackInit<'s>),
}

impl<'s> PayloadEventInit<'s> {
    pub(super) fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        kind: EventSubclassKind,
    ) -> Option<Self> {
        Some(match kind {
            EventSubclassKind::PromiseRejectionEvent => Self::Promise(parse_event_dictionary(
                scope,
                args,
                "PromiseRejectionEvent",
            )?),
            EventSubclassKind::FormDataEvent => {
                Self::FormData(parse_event_dictionary(scope, args, "FormDataEvent")?)
            }
            EventSubclassKind::SubmitEvent => {
                Self::Submit(parse_event_dictionary(scope, args, "SubmitEvent")?)
            }
            EventSubclassKind::TrackEvent => {
                Self::Track(parse_event_dictionary(scope, args, "TrackEvent")?)
            }
            _ => unreachable!("payload event kind"),
        })
    }

    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        let base = match self {
            Self::Promise(init) => &init.base,
            Self::FormData(init) => &init.base,
            Self::Submit(init) => &init.base,
            Self::Track(init) => &init.base,
        };
        (base.bubbles, base.cancelable, base.composed)
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        match self {
            Self::Promise(init) => init.initialize_snapshot(scope, event),
            Self::FormData(init) => init.initialize_snapshot(scope, event),
            Self::Submit(init) => init.initialize_snapshot(scope, event),
            Self::Track(init) => init.initialize_snapshot(scope, event),
        }
        .expect("payload event state");
    }
}
