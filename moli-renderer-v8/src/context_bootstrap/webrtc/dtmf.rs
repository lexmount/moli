//! DTMF identity and events for local, unnegotiated RTP senders. There is no
//! negotiated telephone-event codec or connected RTP transport yet, so sending
//! tones is unavailable. WebIDL conversion still precedes InvalidStateError.

use crate::{
    context_bootstrap::{exposed_interfaces, media_queries},
    native_bridge::throw_dom_exception,
    util::{get_private_value, set_private_value},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const TONE_BUFFER: &str = "__moliRtcDtmfToneBuffer";
const HANDLER: &str = "__moliRtcOnToneChange";
const LISTENERS: &str = "__moliRtcDtmfListeners";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCDTMFSender)]
struct Slots {
    #[webapi(slot = TONE_BUFFER, init = "")]
    tone_buffer: (),
    #[webapi(slot = HANDLER, init = "null")]
    ontonechange: (),
    #[webapi(slot = super::SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS)]
    event_target_slot: (),
    #[webapi(slot = super::SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCDTMFSender, enumerable, receiver)]
struct Prototype {
    #[webapi(accessor_property, getter = tone_buffer)]
    tone_buffer: (),
    #[webapi(accessor_property = "canInsertDTMF", getter = can_insert_dtmf)]
    can_insert_dtmf: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler)]
    ontonechange: (),
    #[webapi(method = "insertDTMF", length = 1, callback = insert_dtmf)]
    insert_dtmf: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCDTMFSender.insertDTMF")]
struct InsertArgs {
    #[webidl(required, name = "tones", converter = "raw")]
    _tones: webidl::DomString16,
    #[webidl(name = "duration", default = 100)]
    _duration: u32,
    #[webidl(name = "interToneGap", default = 70)]
    _inter_tone_gap: u32,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    Prototype::initialize_prototype_template(scope, prototype);
}

pub(super) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sender: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = sender
        .get_creation_context(scope)
        .expect("RTP sender realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let object =
        exposed_interfaces::build_intrinsic_interface_instance(scope, "RTCDTMFSender").ok()?;
    let prototype =
        exposed_interfaces::ensure_intrinsic_interface_prototype(scope, "RTCDTMFSender").ok()?;
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    Slots::new()
        .initialize(scope, object)
        .expect("DTMF sender slots");
    Some(object)
}

fn tone_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let dtmf = super::rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, dtmf, TONE_BUFFER).expect("DTMF tone buffer"));
}

fn can_insert_dtmf<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Sender.transport is null and getParameters().codecs is empty until
    // negotiation is implemented. Neither permits sending telephone events.
    rv.set_bool(false);
}

fn insert_dtmf<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_) = webidl::parse_args::<InsertArgs>(scope, &args) else {
        return;
    };
    // The capability check precedes tone validation and any buffer mutation,
    // including when tones is empty. No playout task or synthetic event starts.
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "DTMF sending is unavailable.",
    );
}

fn handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let dtmf = super::rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, dtmf, HANDLER).expect("DTMF event handler"));
}

fn set_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let dtmf = super::rtp_transceivers::target(scope, args.this());
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, dtmf, HANDLER, value);
    media_queries::simple_object_event_set_ordered_handler(
        scope,
        dtmf,
        LISTENERS,
        "tonechange",
        HANDLER,
        active,
    );
}
