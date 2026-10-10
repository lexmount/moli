use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

pub(crate) mod certificate;
mod configuration;
pub(crate) mod encoded_frames;
mod events;
mod ice_candidate;
mod ice_candidate_parser;
mod ice_error_event;
mod operations;
mod payload_events;
mod rtp_capabilities;
mod rtp_offer;
mod rtp_parameters;
mod rtp_sender;
pub(crate) mod rtp_transceivers;
mod session_description;
pub(in crate::context_bootstrap) use encoded_frames::{
    audio_constructor as rtc_encoded_audio_frame_constructor,
    video_constructor as rtc_encoded_video_frame_constructor,
};
pub(in crate::context_bootstrap) use events::{
    rtc_data_channel_event_constructor_callback, rtc_error_event_constructor_callback,
    rtc_peer_connection_ice_event_constructor_callback,
};
pub(in crate::context_bootstrap) use ice_candidate::rtc_ice_candidate_constructor_callback;
pub(in crate::context_bootstrap) use ice_error_event::rtc_peer_connection_ice_error_event_constructor;
pub(in crate::context_bootstrap) use payload_events::{
    rtc_dtmf_tone_event_constructor, rtc_track_event_constructor,
};
pub(in crate::context_bootstrap) use session_description::rtc_session_description_constructor_callback;

const RTC_PEER_CONNECTION_CONFIGURATION_SLOT: &str = "__moliRtcPeerConnectionConfiguration";
const RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT: &str = "__moliRtcPeerConnectionSignalingState";
const RTC_PEER_CONNECTION_ICE_GATHERING_STATE_SLOT: &str =
    "__moliRtcPeerConnectionIceGatheringState";
const RTC_PEER_CONNECTION_ICE_CONNECTION_STATE_SLOT: &str =
    "__moliRtcPeerConnectionIceConnectionState";
const RTC_PEER_CONNECTION_CONNECTION_STATE_SLOT: &str = "__moliRtcPeerConnectionConnectionState";
const RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT: &str = "__moliRtcPeerConnectionLocalDescription";
const RTC_PEER_CONNECTION_CURRENT_LOCAL_DESCRIPTION_SLOT: &str =
    "__moliRtcPeerConnectionCurrentLocalDescription";
const RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT: &str =
    "__moliRtcPeerConnectionPendingLocalDescription";
const RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT: &str = "__moliRtcPeerConnectionHasDataChannel";
const RTC_PEER_CONNECTION_LISTENERS_SLOT: &str = "__moliRtcPeerConnectionListeners";

const RTC_DATA_CHANNEL_LABEL_SLOT: &str = "__moliRtcDataChannelLabel";
const RTC_DATA_CHANNEL_ORDERED_SLOT: &str = "__moliRtcDataChannelOrdered";
const RTC_DATA_CHANNEL_MAX_PACKET_LIFETIME_SLOT: &str = "__moliRtcDataChannelMaxPacketLifetime";
const RTC_DATA_CHANNEL_MAX_RETRANSMITS_SLOT: &str = "__moliRtcDataChannelMaxRetransmits";
const RTC_DATA_CHANNEL_PROTOCOL_SLOT: &str = "__moliRtcDataChannelProtocol";
const RTC_DATA_CHANNEL_NEGOTIATED_SLOT: &str = "__moliRtcDataChannelNegotiated";
const RTC_DATA_CHANNEL_ID_SLOT: &str = "__moliRtcDataChannelId";
const RTC_DATA_CHANNEL_READY_STATE_SLOT: &str = "__moliRtcDataChannelReadyState";
const RTC_DATA_CHANNEL_BUFFERED_AMOUNT_SLOT: &str = "__moliRtcDataChannelBufferedAmount";
const RTC_DATA_CHANNEL_BINARY_TYPE_SLOT: &str = "__moliRtcDataChannelBinaryType";
const RTC_DATA_CHANNEL_LISTENERS_SLOT: &str = "__moliRtcDataChannelListeners";

const RTC_PEER_CONNECTION_STATE_SLOTS: &[&str] = &[
    RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT,
    RTC_PEER_CONNECTION_ICE_GATHERING_STATE_SLOT,
    RTC_PEER_CONNECTION_ICE_CONNECTION_STATE_SLOT,
    RTC_PEER_CONNECTION_CONNECTION_STATE_SLOT,
];

const RTC_PEER_CONNECTION_DESCRIPTION_SLOTS: &[&str] = &[
    RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT,
    RTC_PEER_CONNECTION_CURRENT_LOCAL_DESCRIPTION_SLOT,
    RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT,
];

const RTC_DATA_CHANNEL_VALUE_SLOTS: &[&str] = &[
    RTC_DATA_CHANNEL_LABEL_SLOT,
    RTC_DATA_CHANNEL_ORDERED_SLOT,
    RTC_DATA_CHANNEL_MAX_PACKET_LIFETIME_SLOT,
    RTC_DATA_CHANNEL_MAX_RETRANSMITS_SLOT,
    RTC_DATA_CHANNEL_PROTOCOL_SLOT,
    RTC_DATA_CHANNEL_NEGOTIATED_SLOT,
    RTC_DATA_CHANNEL_ID_SLOT,
    RTC_DATA_CHANNEL_READY_STATE_SLOT,
    RTC_DATA_CHANNEL_BUFFERED_AMOUNT_SLOT,
    RTC_DATA_CHANNEL_BINARY_TYPE_SLOT,
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCPeerConnection)]
struct RtcPeerConnectionObjectDeclaration<'scope> {
    #[webapi(slot = RTC_PEER_CONNECTION_CONFIGURATION_SLOT)]
    configuration: v8::Local<'scope, v8::Object>,
    #[webapi(slot = RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT)]
    signaling_state: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_PEER_CONNECTION_ICE_GATHERING_STATE_SLOT)]
    ice_gathering_state: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_PEER_CONNECTION_ICE_CONNECTION_STATE_SLOT)]
    ice_connection_state: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_PEER_CONNECTION_CONNECTION_STATE_SLOT)]
    connection_state: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT, init = "null")]
    local_description: (),
    #[webapi(slot = RTC_PEER_CONNECTION_CURRENT_LOCAL_DESCRIPTION_SLOT, init = "null")]
    current_local_description: (),
    #[webapi(slot = RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT, init = "null")]
    pending_local_description: (),
    #[webapi(slot = RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT, init = false)]
    has_data_channel: (),
    #[webapi(slot = rtp_transceivers::NEGOTIATION_HANDLER, init = "null")]
    onnegotiationneeded: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = RTC_PEER_CONNECTION_LISTENERS_SLOT)]
    event_target_slot: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCDataChannel)]
struct RtcDataChannelObjectDeclaration<'scope> {
    #[webapi(slot = RTC_DATA_CHANNEL_LABEL_SLOT)]
    label: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_DATA_CHANNEL_ORDERED_SLOT, init = true)]
    ordered: (),
    #[webapi(slot = RTC_DATA_CHANNEL_MAX_PACKET_LIFETIME_SLOT, init = "null")]
    max_packet_lifetime: (),
    #[webapi(slot = RTC_DATA_CHANNEL_MAX_RETRANSMITS_SLOT, init = "null")]
    max_retransmits: (),
    #[webapi(slot = RTC_DATA_CHANNEL_PROTOCOL_SLOT, init = "")]
    protocol: (),
    #[webapi(slot = RTC_DATA_CHANNEL_NEGOTIATED_SLOT, init = false)]
    negotiated: (),
    #[webapi(slot = RTC_DATA_CHANNEL_ID_SLOT, init = "null")]
    id: (),
    #[webapi(slot = RTC_DATA_CHANNEL_READY_STATE_SLOT, init = string("connecting"))]
    ready_state: (),
    #[webapi(slot = RTC_DATA_CHANNEL_BUFFERED_AMOUNT_SLOT, init = 0)]
    buffered_amount: (),
    #[webapi(slot = RTC_DATA_CHANNEL_BINARY_TYPE_SLOT, init = string("arraybuffer"))]
    binary_type: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = RTC_DATA_CHANNEL_LISTENERS_SLOT)]
    event_target_slot: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct RtcSessionDescriptionInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    sdp: v8::Local<'scope, v8::String>,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCPeerConnection, enumerable, receiver)]
struct RtcPeerConnectionPrototypeDeclaration {
    #[webapi(accessor_property, getter = rtc_peer_connection_state_getter, data = callback_data_index_value(scope, 0))]
    signaling_state: (),
    #[webapi(accessor_property = "iceGatheringState", getter = rtc_peer_connection_state_getter, data = callback_data_index_value(scope, 1))]
    ice_gathering_state: (),
    #[webapi(accessor_property = "iceConnectionState", getter = rtc_peer_connection_state_getter, data = callback_data_index_value(scope, 2))]
    ice_connection_state: (),
    #[webapi(accessor_property = "connectionState", getter = rtc_peer_connection_state_getter, data = callback_data_index_value(scope, 3))]
    connection_state: (),

    #[webapi(accessor_property = "localDescription", getter = rtc_peer_connection_description_getter, data = callback_data_index_value(scope, 0))]
    local_description: (),
    #[webapi(accessor_property = "currentLocalDescription", getter = rtc_peer_connection_description_getter, data = callback_data_index_value(scope, 1))]
    current_local_description: (),
    #[webapi(accessor_property = "pendingLocalDescription", getter = rtc_peer_connection_description_getter, data = callback_data_index_value(scope, 2))]
    pending_local_description: (),

    #[webapi(method = "createDataChannel", length = 1, callback = rtc_peer_connection_create_data_channel_callback)]
    create_data_channel: (),
    #[webapi(method = "createOffer", returns_promise, length = 0, callback = rtc_peer_connection_create_offer_callback)]
    create_offer: (),
    #[webapi(method = "setLocalDescription", returns_promise, length = 0, callback = rtc_peer_connection_set_local_description_callback)]
    set_local_description: (),
    #[webapi(method, length = 0, callback = rtc_peer_connection_close_callback)]
    close: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCDataChannel, enumerable)]
struct RtcDataChannelPrototypeDeclaration {
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 0))]
    label: (),
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 1))]
    ordered: (),
    #[webapi(accessor_property = "maxPacketLifeTime", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 2))]
    max_packet_lifetime: (),
    #[webapi(accessor_property = "maxRetransmits", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 3))]
    max_retransmits: (),
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 4))]
    protocol: (),
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 5))]
    negotiated: (),
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 6))]
    id: (),
    #[webapi(accessor_property = "readyState", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 7))]
    ready_state: (),
    #[webapi(accessor_property = "bufferedAmount", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 8))]
    buffered_amount: (),
    #[webapi(accessor_property = "binaryType", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 9))]
    binary_type: (),
    #[webapi(method, length = 0, callback = rtc_data_channel_close_callback)]
    close: (),
    #[webapi(method, length = 1, callback = rtc_data_channel_send_callback)]
    send: (),
}

pub(in crate::context_bootstrap) fn install_webrtc_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "RTCEncodedAudioFrame" | "RTCEncodedVideoFrame" => {
            encoded_frames::install(scope, prototype, interface_name);
        }
        "RTCPeerConnectionIceErrorEvent" => ice_error_event::install(scope, prototype),
        "RTCPeerConnectionIceEvent" | "RTCDataChannelEvent" | "RTCErrorEvent" => {
            events::install_event_template_bindings(scope, template, interface_name)
        }
        "RTCDTMFToneChangeEvent" | "RTCTrackEvent" => {
            payload_events::install(scope, prototype, interface_name)
        }
        "RTCIceCandidate" => {
            ice_candidate::install_ice_candidate_template_bindings(scope, template)
        }
        "RTCSessionDescription" => {
            session_description::install_session_description_template_bindings(scope, template)
        }
        "RTCPeerConnection" => {
            RtcPeerConnectionPrototypeDeclaration::initialize_prototype_template(scope, prototype);
            configuration::install(scope, prototype);
            certificate::install_static(scope, template);
            rtp_transceivers::install(scope, prototype, interface_name);
        }
        "RTCCertificate" => certificate::install(scope, prototype),
        "RTCRtpSender" | "RTCRtpReceiver" => {
            rtp_capabilities::install(scope, template, interface_name);
            rtp_transceivers::install(scope, prototype, interface_name);
        }
        "RTCRtpTransceiver" => rtp_transceivers::install(scope, prototype, interface_name),
        "RTCDataChannel" => {
            RtcDataChannelPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn rtc_peer_connection_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'RTCPeerConnection': Please use the 'new' operator.",
        );
        return;
    }
    let Some(configuration) = configuration::constructor_configuration(scope, &args) else {
        return;
    };
    let declaration = RtcPeerConnectionObjectDeclaration {
        configuration,
        signaling_state: v8str(scope, "stable"),
        ice_gathering_state: v8str(scope, "new"),
        ice_connection_state: v8str(scope, "new"),
        connection_state: v8str(scope, "new"),
        local_description: (),
        current_local_description: (),
        pending_local_description: (),
        has_data_channel: (),
        onnegotiationneeded: (),
        event_target_slot: (),
        ordered_handlers: (),
    };
    if declaration.initialize(scope, args.this()).is_err() {
        rv.set_undefined();
        return;
    }
    rtp_transceivers::initialize_pc(scope, args.this());
    operations::initialize(scope, args.this());
    if rtp_sender::initialize_pc(scope, args.this()).is_none() {
        return;
    }
    rv.set(args.this().into());
}

fn rtc_peer_connection_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        RTC_PEER_CONNECTION_STATE_SLOTS,
        "RTCPeerConnection state slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let target = rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, target, slot).unwrap_or_else(|| v8::undefined(scope).into()));
}

fn rtc_peer_connection_description_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        RTC_PEER_CONNECTION_DESCRIPTION_SLOTS,
        "RTCPeerConnection description slots",
    ) else {
        rv.set_null();
        return;
    };
    let target = rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, target, slot).unwrap_or_else(|| v8::null(scope).into()));
}

fn rtc_peer_connection_create_data_channel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'createDataChannel' on 'RTCPeerConnection': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(label) = args.get(0).to_string(scope) else {
        return;
    };
    let target = rtp_transceivers::target(scope, args.this());
    if rtp_transceivers::closed(scope, target) {
        crate::native_bridge::throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The connection is closed.",
        );
        return;
    }
    let Some(channel) = build_rtc_data_channel(scope, label) else {
        rv.set_undefined();
        return;
    };
    set_private_value(
        scope,
        target,
        RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    rv.set(channel.into());
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCOfferOptions")]
struct OfferOptions {
    #[webidl(default = false)]
    ice_restart: bool,
    offer_to_receive_audio: Option<bool>,
    offer_to_receive_video: Option<bool>,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.createOffer")]
struct OfferArgs {
    #[webidl(dictionary)]
    options: OfferOptions,
}
#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RTCSdpType")]
enum SdpType {
    #[webidl(token = "offer")]
    Offer,
    #[webidl(token = "pranswer")]
    Pranswer,
    #[webidl(token = "answer")]
    Answer,
    #[webidl(token = "rollback")]
    Rollback,
}
impl SdpType {
    fn token(self) -> &'static str {
        match self {
            Self::Offer => "offer",
            Self::Pranswer => "pranswer",
            Self::Answer => "answer",
            Self::Rollback => "rollback",
        }
    }
}
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCLocalSessionDescriptionInit")]
struct LocalDescription {
    #[webidl(converter = "raw", default = webidl::DomString16(Vec::new()))]
    sdp: webidl::DomString16,
    #[webidl(converter = "enum")]
    r#type: Option<SdpType>,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.setLocalDescription")]
struct LocalDescriptionArgs {
    #[webidl(dictionary)]
    description: LocalDescription,
}

fn rtc_peer_connection_create_offer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<OfferArgs>(scope, &args) else {
        return;
    };
    let target = rtp_transceivers::target(scope, args.this());
    // ICE restarts remain a transport-backend limitation. These legacy receive
    // options continue to describe the local signaling-only offer.
    let _ = parsed.options.ice_restart;
    let options = i32::from(parsed.options.offer_to_receive_audio.unwrap_or(false))
        | (i32::from(parsed.options.offer_to_receive_video.unwrap_or(false)) << 1);
    let payload = v8::Integer::new(scope, options);
    if let Some(promise) =
        operations::enqueue(scope, target, operations::Kind::CreateOffer, payload.into())
    {
        rv.set(promise.into());
    }
}

fn offer_sdp<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    audio: bool,
    video: bool,
) -> String {
    let data = get_private_value(scope, target, RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT)
        .expect("data channel flag")
        .boolean_value(scope);
    let sections = rtp_transceivers::offer_sections(scope, target);
    let mut sdp = if rtp_transceivers::has_transceivers(scope, target) {
        rtp_offer::build(&sections, data)
    } else {
        build_signaling_only_offer(audio, video, data)
    };
    let config = configuration::configuration(scope, target);
    let mut fingerprints = String::new();
    certificate::connection_fingerprints(scope, config, &mut fingerprints);
    let at = sdp.find("m=").unwrap_or(sdp.len());
    sdp.insert_str(at, &fingerprints);
    sdp
}
fn complete_create_offer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    pc: v8::Local<'s, v8::Object>,
) -> bool {
    let options = operations::payload(scope, request)
        .int32_value(scope)
        .expect("native offer options");
    let sdp = offer_sdp(scope, pc, options & 1 != 0, options & 2 != 0);
    let Some(sdp) = v8_string(scope, &sdp) else {
        return false;
    };
    let offer = RtcSessionDescriptionInitDeclaration::new(v8str(scope, "offer"), sdp)
        .bind(scope)
        .expect("offer dictionary");
    operations::resolve(scope, request, offer.into());
    true
}
fn rtc_peer_connection_set_local_description_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<LocalDescriptionArgs>(scope, &args) else {
        return;
    };
    let pc = rtp_transceivers::target(scope, args.this());
    let Some(sdp) =
        v8::String::new_from_two_byte(scope, &parsed.description.sdp.0, v8::NewStringType::Normal)
    else {
        return;
    };
    let kind = parsed
        .description
        .r#type
        .map(|kind| v8str(scope, kind.token()))
        .unwrap_or_else(|| v8str(scope, ""));
    // Copy the dictionary now, before entering the operations chain. Author
    // getters and subsequent mutations never run inside a networking task.
    let snapshot = RtcSessionDescriptionInitDeclaration::new(kind, sdp)
        .bind(scope)
        .expect("local description snapshot");
    if let Some(promise) = operations::enqueue(
        scope,
        pc,
        operations::Kind::SetLocalDescription,
        snapshot.into(),
    ) {
        rv.set(promise.into());
    }
}
fn complete_set_local_description<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    pc: v8::Local<'s, v8::Object>,
) -> bool {
    let snapshot = v8::Local::<v8::Object>::try_from(operations::payload(scope, request))
        .expect("description snapshot");
    let kind = snapshot
        .get(scope, v8str(scope, "type").into())
        .expect("copied type")
        .to_rust_string_lossy(scope);
    let mut sdp = v8::Local::<v8::String>::try_from(
        snapshot
            .get(scope, v8str(scope, "sdp").into())
            .expect("copied SDP"),
    )
    .expect("SDP string");
    let kind = if kind.is_empty() {
        "offer"
    } else {
        kind.as_str()
    };
    let state = get_private_value(scope, pc, RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT)
        .expect("signaling state")
        .to_rust_string_lossy(scope);
    if !(kind == "offer" && matches!(state.as_str(), "stable" | "have-local-offer")
        || kind == "rollback" && state == "have-local-offer")
    {
        v8::tc_scope!(let caught, scope);
        crate::native_bridge::throw_dom_exception(
            caught,
            "InvalidStateError",
            11,
            "The description is incompatible with the signaling state.",
        );
        let reason = caught.exception().expect("description error");
        caught.reset();
        operations::reject(caught, request, reason);
        return true;
    }
    if kind == "rollback" {
        set_private_value(
            scope,
            pc,
            RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT,
            v8::null(scope).into(),
        );
        let current = get_private_value(
            scope,
            pc,
            RTC_PEER_CONNECTION_CURRENT_LOCAL_DESCRIPTION_SLOT,
        )
        .expect("current local description");
        set_private_value(
            scope,
            pc,
            RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT,
            current,
        );
        set_string_slot(
            scope,
            pc,
            RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT,
            "stable",
        );
    } else {
        if sdp.length() == 0 {
            let text = offer_sdp(scope, pc, false, false);
            let Some(value) = v8_string(scope, &text) else {
                return false;
            };
            sdp = value;
        }
        let Some(kind) = v8_string(scope, kind) else {
            return false;
        };
        let Some(description) = session_description::from_parts(scope, kind, sdp) else {
            return false;
        };
        set_private_value(
            scope,
            pc,
            RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT,
            description.into(),
        );
        set_private_value(
            scope,
            pc,
            RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT,
            description.into(),
        );
        set_string_slot(
            scope,
            pc,
            RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT,
            "have-local-offer",
        );
    }
    // No ICE/DTLS transport or fabricated candidates are produced.
    operations::resolve(scope, request, v8::undefined(scope).into());
    true
}

fn rtc_peer_connection_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("RTCPeerConnection receiver");
    rtp_transceivers::close(scope, target);
    operations::close(scope, target);
    for (slot, state) in [
        (RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT, "closed"),
        (RTC_PEER_CONNECTION_ICE_CONNECTION_STATE_SLOT, "closed"),
        (RTC_PEER_CONNECTION_CONNECTION_STATE_SLOT, "closed"),
    ] {
        set_string_slot(scope, target, slot, state);
    }
    rv.set_undefined();
}

fn rtc_data_channel_value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_data_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        RTC_DATA_CHANNEL_VALUE_SLOTS,
        "RTCDataChannel value slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn rtc_data_channel_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_data_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    set_string_slot(
        scope,
        args.this(),
        RTC_DATA_CHANNEL_READY_STATE_SLOT,
        "closed",
    );
    rv.set_undefined();
}

fn rtc_data_channel_send_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_data_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set_undefined();
}

fn build_rtc_data_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    label: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "RTCDataChannel")?;
    let channel = v8::Object::new(scope);
    if channel.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    RtcDataChannelObjectDeclaration::new(label)
        .initialize(scope, channel)
        .ok()?;
    Some(channel)
}

fn set_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: &'static str,
) {
    set_private_value(scope, object, slot, v8str(scope, value).into());
}

fn rtc_data_channel_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::RTCDataChannel::is_instance(scope, receiver)
}

fn build_signaling_only_offer(audio: bool, video: bool, data: bool) -> String {
    let mut mids = Vec::new();
    if audio {
        mids.push("0");
    }
    if video {
        mids.push("1");
    }
    if data {
        mids.push("2");
    }
    let mut sdp = format!(
        "v=0\r\no=- 0 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE {}\r\na=extmap-allow-mixed\r\na=msid-semantic: WMS\r\n",
        mids.join(" ")
    );
    if audio {
        sdp.push_str(RTC_AUDIO_OFFER_SECTION);
    }
    if video {
        sdp.push_str(RTC_VIDEO_OFFER_SECTION);
    }
    if data {
        sdp.push_str(RTC_DATA_OFFER_SECTION);
    }
    sdp
}

const RTC_AUDIO_OFFER_SECTION: &str = concat!(
    "m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126\r\n",
    "c=IN IP4 0.0.0.0\r\na=mid:0\r\na=recvonly\r\na=rtcp-mux\r\na=rtcp-rsize\r\n",
    "a=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=1\r\n",
    "a=rtpmap:63 red/48000/2\r\na=fmtp:63 111/111\r\n",
    "a=rtpmap:9 G722/8000\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\n",
    "a=rtpmap:13 CN/8000\r\na=rtpmap:110 telephone-event/48000\r\n",
    "a=rtpmap:126 telephone-event/8000\r\n"
);

const RTC_VIDEO_OFFER_SECTION: &str = concat!(
    "m=video 9 UDP/TLS/RTP/SAVPF 96 97 98 99 100 101 35 36 37 38 103 104 107 108 109 114 115 116 117 118 39 40 41 42 43 44 45 46 47 48 119 120 121 49\r\n",
    "c=IN IP4 0.0.0.0\r\na=mid:1\r\na=recvonly\r\na=rtcp-mux\r\na=rtcp-rsize\r\n",
    "a=rtpmap:96 VP8/90000\r\na=rtpmap:97 rtx/90000\r\na=fmtp:97 apt=96\r\n",
    "a=rtpmap:98 VP9/90000\r\na=fmtp:98 profile-id=0\r\n",
    "a=rtpmap:99 rtx/90000\r\na=fmtp:99 apt=98\r\n",
    "a=rtpmap:100 VP9/90000\r\na=fmtp:100 profile-id=2\r\n",
    "a=rtpmap:101 rtx/90000\r\na=fmtp:101 apt=100\r\n",
    "a=rtpmap:35 VP9/90000\r\na=fmtp:35 profile-id=1\r\n",
    "a=rtpmap:36 rtx/90000\r\na=fmtp:36 apt=35\r\n",
    "a=rtpmap:37 VP9/90000\r\na=fmtp:37 profile-id=3\r\n",
    "a=rtpmap:38 rtx/90000\r\na=fmtp:38 apt=37\r\n",
    "a=rtpmap:103 H264/90000\r\na=fmtp:103 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f\r\n",
    "a=rtpmap:104 rtx/90000\r\na=fmtp:104 apt=103\r\n",
    "a=rtpmap:107 H264/90000\r\na=fmtp:107 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f\r\n",
    "a=rtpmap:108 rtx/90000\r\na=fmtp:108 apt=107\r\n",
    "a=rtpmap:109 H264/90000\r\na=fmtp:109 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n",
    "a=rtpmap:114 rtx/90000\r\na=fmtp:114 apt=109\r\n",
    "a=rtpmap:115 H264/90000\r\na=fmtp:115 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f\r\n",
    "a=rtpmap:116 rtx/90000\r\na=fmtp:116 apt=115\r\n",
    "a=rtpmap:117 H264/90000\r\na=fmtp:117 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d001f\r\n",
    "a=rtpmap:118 rtx/90000\r\na=fmtp:118 apt=117\r\n",
    "a=rtpmap:39 H264/90000\r\na=fmtp:39 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=4d001f\r\n",
    "a=rtpmap:40 rtx/90000\r\na=fmtp:40 apt=39\r\n",
    "a=rtpmap:41 H264/90000\r\na=fmtp:41 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=f4001f\r\n",
    "a=rtpmap:42 rtx/90000\r\na=fmtp:42 apt=41\r\n",
    "a=rtpmap:43 H264/90000\r\na=fmtp:43 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=f4001f\r\n",
    "a=rtpmap:44 rtx/90000\r\na=fmtp:44 apt=43\r\n",
    "a=rtpmap:45 AV1/90000\r\na=fmtp:45 level-idx=5;profile=0;tier=0\r\n",
    "a=rtpmap:46 rtx/90000\r\na=fmtp:46 apt=45\r\n",
    "a=rtpmap:47 AV1/90000\r\na=fmtp:47 level-idx=5;profile=1;tier=0\r\n",
    "a=rtpmap:48 rtx/90000\r\na=fmtp:48 apt=47\r\n",
    "a=rtpmap:119 red/90000\r\na=rtpmap:120 rtx/90000\r\na=fmtp:120 apt=119\r\n",
    "a=rtpmap:121 ulpfec/90000\r\na=rtpmap:49 flexfec-03/90000\r\n",
    "a=fmtp:49 repair-window=10000000\r\n"
);

const RTC_DATA_OFFER_SECTION: &str = concat!(
    "m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n",
    "c=IN IP4 0.0.0.0\r\na=mid:2\r\na=sctp-port:5000\r\na=max-message-size:262144\r\n"
);
