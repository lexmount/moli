//! Synthetic WebRTC event payloads, independent of an RTP or DTMF backend.

use crate::{
    context_bootstrap::events::{
        EventInit, event_private_value, initialize_event_object_with_type,
        initialize_event_wrapper, new_event_state,
    },
    util::{throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
};

const TONE: &str = "__moliRtcDtmfTone";
const RECEIVER: &str = "__moliRtcTrackReceiver";
const TRACK: &str = "__moliRtcTrackTrack";
const STREAMS: &str = "__moliRtcTrackStreams";
const TRANSCEIVER: &str = "__moliRtcTrackTransceiver";

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCDTMFToneChangeEventInit")]
struct ToneInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(converter = "raw", default = webidl::DomString16(Vec::new()))]
    tone: webidl::DomString16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCDTMFToneChangeEvent")]
struct ToneArgs {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(dictionary)]
    init: ToneInit,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCTrackEventInit")]
struct TrackInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::RTCRtpReceiver)]
    receiver: v8::Local<'s, v8::Object>,
    #[webidl(sequence, interface = web_api_interfaces::MediaStream, default = Vec::new())]
    streams: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(required, interface = web_api_interfaces::MediaStreamTrack)]
    track: v8::Local<'s, v8::Object>,
    #[webidl(required, interface = web_api_interfaces::RTCRtpTransceiver)]
    transceiver: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCTrackEvent")]
struct TrackArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(required, dictionary)]
    init: TrackInit<'s>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCDTMFToneChangeEvent)]
struct ToneSlots<'s> {
    #[webapi(data_property, enumerable)]
    composed: bool,
    #[webapi(slot = TONE)]
    tone: v8::Local<'s, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCTrackEvent)]
struct TrackSlots<'s> {
    #[webapi(data_property, enumerable)]
    composed: bool,
    #[webapi(slot = RECEIVER)]
    receiver: v8::Local<'s, v8::Object>,
    #[webapi(slot = TRACK)]
    track: v8::Local<'s, v8::Object>,
    #[webapi(slot = STREAMS)]
    streams: v8::Local<'s, v8::Array>,
    #[webapi(slot = TRANSCEIVER)]
    transceiver: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCDTMFToneChangeEvent, enumerable, receiver)]
struct TonePrototype {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, TONE))]
    tone: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCTrackEvent, enumerable, receiver)]
struct TrackPrototype {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, RECEIVER))]
    receiver: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, TRACK))]
    track: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, STREAMS))]
    streams: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, TRANSCEIVER))]
    transceiver: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "RTCDTMFToneChangeEvent" => TonePrototype::initialize_prototype_template(scope, prototype),
        "RTCTrackEvent" => TrackPrototype::initialize_prototype_template(scope, prototype),
        _ => (),
    }
}

pub(in crate::context_bootstrap) fn rtc_dtmf_tone_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCDTMFToneChangeEvent requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ToneArgs>(scope, &args) else {
        return;
    };
    let Some(state) = event_state(
        scope,
        &args,
        "RTCDTMFToneChangeEvent",
        &parsed.event_type,
        &parsed.init.base,
    ) else {
        return;
    };
    let Some(tone) =
        v8::String::new_from_two_byte(scope, &parsed.init.tone.0, v8::NewStringType::Normal)
    else {
        return;
    };
    if ToneSlots::new(parsed.init.base.composed, tone)
        .initialize(scope, state)
        .is_ok()
        && initialize_event_wrapper(scope, args.this(), state).is_some()
    {
        rv.set(args.this().into());
    }
}

pub(in crate::context_bootstrap) fn rtc_track_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCTrackEvent requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<TrackArgs>(scope, &args) else {
        return;
    };
    let Some(state) = event_state(
        scope,
        &args,
        "RTCTrackEvent",
        &parsed.event_type,
        &parsed.init.base,
    ) else {
        return;
    };
    // Snapshot the sequence in the event's realm, retaining supplied platform
    // object identity and duplicates. Never consult author Array/Object helpers.
    let elements: Vec<_> = parsed.init.streams.into_iter().map(Into::into).collect();
    let streams = v8::Array::new_with_elements(scope, &elements);
    if streams.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return;
    }
    if TrackSlots::new(
        parsed.init.base.composed,
        parsed.init.receiver,
        parsed.init.track,
        streams,
        parsed.init.transceiver,
    )
    .initialize(scope, state)
    .is_ok()
        && initialize_event_wrapper(scope, args.this(), state).is_some()
    {
        rv.set(args.this().into());
    }
}

fn event_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    interface: &str,
    event_type: &webidl::DomString16,
    init: &EventInit,
) -> Option<v8::Local<'s, v8::Object>> {
    // The public constructor uses the shared deferred-prototype entry. Complete
    // all WebIDL conversion before observing newTarget.prototype.
    if !initialize_web_api_constructor_receiver(scope, args.this(), interface) {
        return None;
    }
    let event_type =
        v8::String::new_from_two_byte(scope, &event_type.0, v8::NewStringType::Normal)?;
    let state = new_event_state(scope);
    initialize_event_object_with_type(scope, state, event_type, init.bubbles, init.cancelable);
    Some(state)
}

fn payload_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = event_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}
