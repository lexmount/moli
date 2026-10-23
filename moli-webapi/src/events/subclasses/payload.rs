use super::super::init::{EventInit, parse_event_dictionary};
use super::*;
use crate::interfaces as web_api_interfaces;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PromiseRejectionEventInit", snapshot)]
pub(super) struct PromiseRejectionInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(with = rejection_promise_member)]
    promise: v8::Local<'s, v8::Promise>,
    #[webidl(converter = "raw", snapshot_default = v8::undefined(scope).into())]
    reason: Option<v8::Local<'s, v8::Value>>,
}

fn rejection_promise_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Promise>, webidl::WebIdlError> {
    let context = webidl::Context::member("PromiseRejectionEventInit", name);
    let value = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
        .ok_or_else(|| webidl::WebIdlError::missing_required(context))?;
    // Chromium preserves native Promise identity, including cross-realm values,
    // and resolves other inputs with an intrinsic capability.
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        Ok(promise)
    } else {
        webidl::convert(
            scope,
            value,
            webidl::Context::member("PromiseRejectionEventInit", name),
        )
    }
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

pub fn construct_native_promise_rejection_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    promise: v8::Local<'s, v8::Promise>,
    reason: Option<v8::Local<'s, v8::Value>>,
) -> Option<v8::Local<'s, v8::Object>> {
    // Keep the dispatch owners' existing behavior when the exposed
    // PromiseRejectionEvent constructor is unavailable.
    let global = scope.get_current_context().global(scope);
    let name = moli_v8_util::v8str(scope, "PromiseRejectionEvent");
    let constructor = global.get(scope, name.into())?;
    if !constructor.is_function() {
        return None;
    }

    let init = PromiseRejectionInit {
        base: EventInit {
            cancelable: event_type == "unhandledrejection",
            ..EventInit::default()
        },
        promise,
        reason: Some(reason.unwrap_or_else(|| v8::null(scope).into())),
    };
    construct_native_subclass(
        scope,
        EventSubclassKind::PromiseRejectionEvent,
        event_type,
        SubclassInit::Payload(PayloadEventInit::Promise(init)),
    )
}

pub fn construct_native_submit_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    submitter: Option<v8::Local<'s, v8::Value>>,
    bubbles: bool,
    cancelable: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let submitter = nullable_native_event_interface(
        scope,
        submitter,
        "SubmitEventInit",
        "submitter",
        webidl::InterfaceOptions {
            name: "HTMLElement",
            brand_check: web_api_interfaces::HTMLElement::is_instance,
        },
    )?;
    let init = SubmitInit {
        base: EventInit {
            bubbles,
            cancelable,
            composed: false,
        },
        submitter,
        agent_invoked: false,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::SubmitEvent,
        "submit",
        SubclassInit::Payload(PayloadEventInit::Submit(init)),
    )
}

pub fn construct_native_form_data_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    form_data: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let form_data = native_event_interface(
        scope,
        form_data.into(),
        "FormDataEventInit",
        "formData",
        webidl::InterfaceOptions {
            name: "FormData",
            brand_check: web_api_interfaces::FormData::is_instance,
        },
    )?;
    let init = FormDataInit {
        base: EventInit {
            bubbles: true,
            ..EventInit::default()
        },
        form_data,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::FormDataEvent,
        "formdata",
        SubclassInit::Payload(PayloadEventInit::FormData(init)),
    )
}

pub fn construct_native_track_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    track: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let track = native_event_interface(
        scope,
        track.into(),
        "TrackEventInit",
        "track",
        webidl::InterfaceOptions {
            name: "TextTrack",
            brand_check: web_api_interfaces::TextTrack::is_instance,
        },
    )?;
    let init = TrackInit {
        base: EventInit::default(),
        track: Some(track),
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::TrackEvent,
        event_type,
        SubclassInit::Payload(PayloadEventInit::Track(init)),
    )
}
