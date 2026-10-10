//! Local RTP object relationships, offer snapshots and transport association.
//! Remote negotiation, encoding and reception still require a WebRTC backend.

use super::rtp_parameters::{Codec, Encoding, capabilities};
use crate::{
    context_bootstrap::{exposed_interfaces, media_queries, media_streams},
    native_bridge::throw_dom_exception,
    page_task_queue::RendererPageWebRtcTaskKind,
    util::{
        context_host_ptr_from_global_bridge, get_private_object, get_private_value,
        set_private_value, throw_range_error, throw_type_error, v8_string, v8str,
    },
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const TRANSCEIVERS: &str = "__moliRtcTransceivers";
const OWNER: &str = "__moliRtpOwner";
const SENDER: &str = "__moliRtpSender";
const RECEIVER: &str = "__moliRtpReceiver";
const TRACK: &str = "__moliRtpTrack";
const DIRECTION: &str = "__moliRtpDirection";
const CURRENT_DIRECTION: &str = "__moliRtpCurrentDirection";
const MID: &str = "__moliRtpMid";
const JSEP_MID: &str = "__moliRtpJsepMid";
const NEXT_MID: &str = "__moliRtcNextMid";
const PLANNED_TRANSPORT: &str = "__moliRtpPlannedTransport";
const OFFER_SNAPSHOT: &str = "__moliRtcOfferSnapshot";
const DATA_MID: &str = "__moliRtcDataMid";
const DATA_TRANSPORT: &str = "__moliRtcDataTransport";
const LEGACY_TRANSPORT: &str = "__moliRtcLegacyOfferTransport";
const STOPPING: &str = "__moliRtpStopping";
const STOPPED: &str = "__moliRtpStopped";
const CODECS: &str = "__moliRtpPreferredCodecs";
const NEGOTIATION_TASK: &str = "__moliRtcNegotiationTask";
const NEGOTIATION_NEEDED: &str = "__moliRtcNegotiationNeeded";
pub(super) const NEGOTIATION_HANDLER: &str = "__moliRtcOnNegotiationNeeded";

#[derive(Clone, Copy, PartialEq, Eq, webidl::WebIdlEnum)]
#[webidl(name = "RTCRtpTransceiverDirection")]
enum Direction {
    #[webidl(token = "sendrecv")]
    SendRecv,
    #[webidl(token = "sendonly")]
    SendOnly,
    #[webidl(token = "recvonly")]
    RecvOnly,
    #[webidl(token = "inactive")]
    Inactive,
    #[webidl(token = "stopped")]
    Stopped,
}
impl Direction {
    fn token(self) -> &'static str {
        match self {
            Self::SendRecv => "sendrecv",
            Self::SendOnly => "sendonly",
            Self::RecvOnly => "recvonly",
            Self::Inactive => "inactive",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCRtpTransceiverInit")]
struct Init<'s> {
    #[webidl(converter = "enum", default = Direction::SendRecv)]
    direction: Direction,
    #[webidl(sequence, converter = "dictionary", default = Vec::new())]
    send_encodings: Vec<Encoding>,
    #[webidl(sequence, interface = web_api_interfaces::MediaStream, default = Vec::new())]
    streams: Vec<v8::Local<'s, v8::Object>>,
}
enum TrackOrKind<'s> {
    Track(v8::Local<'s, v8::Object>),
    Kind(String),
}
fn track_or_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<TrackOrKind<'s>, webidl::WebIdlError> {
    let value = args.get(index);
    if let Ok(track) = v8::Local::<v8::Object>::try_from(value)
        && web_api_interfaces::MediaStreamTrack::is_instance(scope, track)
    {
        return Ok(TrackOrKind::Track(track));
    }
    webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::argument("RTCPeerConnection.addTransceiver", (index + 1) as usize),
    )
    .map(|kind| TrackOrKind::Kind(kind.0))
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.addTransceiver")]
struct AddArgs<'s> {
    #[webidl(required, with = track_or_kind)]
    track_or_kind: TrackOrKind<'s>,
    #[webidl(dictionary)]
    init: Init<'s>,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCRtpTransceiver.direction")]
struct DirectionArgs {
    #[webidl(required)]
    value: String,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCRtpTransceiver.setCodecPreferences")]
struct CodecArgs {
    #[webidl(required, sequence, converter = "dictionary")]
    codecs: Vec<Codec>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCRtpTransceiver)]
struct TransceiverSlots<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = SENDER)]
    sender: v8::Local<'s, v8::Object>,
    #[webapi(slot = RECEIVER)]
    receiver: v8::Local<'s, v8::Object>,
    #[webapi(slot = DIRECTION)]
    direction: v8::Local<'s, v8::String>,
    #[webapi(slot = CURRENT_DIRECTION, init = "null")]
    current_direction: (),
    #[webapi(slot = MID, init = "null")]
    mid: (),
    #[webapi(slot = JSEP_MID, init = "null")]
    jsep_mid: (),
    #[webapi(slot = PLANNED_TRANSPORT, init = "null")]
    planned_transport: (),
    #[webapi(slot = STOPPING, init = false)]
    stopping: (),
    #[webapi(slot = STOPPED, init = false)]
    stopped: (),
    #[webapi(slot = CODECS, init = string("[]"))]
    codecs: (),
}
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCRtpReceiver)]
struct ReceiverSlots<'s> {
    #[webapi(slot = TRACK)]
    track: v8::Local<'s, v8::Object>,
    #[webapi(slot = super::transports::RTP_TRANSPORT, init = "null")]
    transport: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct OfferAssociation<'s> {
    #[webapi(slot = TRANSCEIVERS)]
    transceiver: v8::Local<'s, v8::Value>,
    #[webapi(slot = MID)]
    mid: v8::Local<'s, v8::String>,
    #[webapi(slot = PLANNED_TRANSPORT)]
    transport: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCPeerConnection, enumerable, receiver)]
struct PeerPrototype {
    #[webapi(accessor_property, getter = slot_getter, setter = negotiation_handler_setter, data = v8str(scope, NEGOTIATION_HANDLER))]
    onnegotiationneeded: (),
    #[webapi(method, length = 1, callback = add_transceiver)]
    add_transceiver: (),
    #[webapi(method, length = 0, callback = collect, data = v8str(scope, TRANSCEIVERS))]
    get_transceivers: (),
    #[webapi(method, length = 0, callback = collect, data = v8str(scope, SENDER))]
    get_senders: (),
    #[webapi(method, length = 0, callback = collect, data = v8str(scope, RECEIVER))]
    get_receivers: (),
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCRtpTransceiver, enumerable, receiver)]
struct TransceiverPrototype {
    // Chromium retains this non-standard compatibility attribute. It reflects
    // stopping, while [[Stopped]] waits for the completed negotiation/close.
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, STOPPING))]
    stopped: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, MID))]
    mid: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, SENDER))]
    sender: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, RECEIVER))]
    receiver: (),
    #[webapi(accessor_property, getter = direction_getter, setter = direction_setter)]
    direction: (),
    #[webapi(accessor_property, getter = current_direction_getter)]
    current_direction: (),
    #[webapi(method, length = 0, callback = stop)]
    stop: (),
    #[webapi(method, length = 1, callback = set_codec_preferences)]
    set_codec_preferences: (),
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCRtpReceiver, enumerable, receiver)]
struct ReceiverPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, TRACK))]
    track: (),
    #[webapi(accessor_property, getter = transport_getter)]
    transport: (),
    #[webapi(method, returns_promise, length = 0, callback = super::stats::rtp_get_stats)]
    get_stats: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
) {
    match name {
        "RTCPeerConnection" => PeerPrototype::initialize_prototype_template(scope, prototype),
        "RTCRtpTransceiver" => {
            TransceiverPrototype::initialize_prototype_template(scope, prototype)
        }
        "RTCRtpSender" => super::rtp_sender::install(scope, prototype),
        "RTCRtpReceiver" => ReceiverPrototype::initialize_prototype_template(scope, prototype),
        _ => (),
    }
}
pub(super) fn initialize_pc<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    let list = v8::Array::new(scope, 0);
    set_private_value(scope, pc, TRANSCEIVERS, list.into());
    set_bool(scope, pc, NEGOTIATION_TASK, false);
    set_bool(scope, pc, NEGOTIATION_NEEDED, false);
    set_private_value(
        scope,
        pc,
        NEXT_MID,
        v8::Integer::new_from_unsigned(scope, 0).into(),
    );
    set_private_value(scope, pc, OFFER_SNAPSHOT, v8::Array::new(scope, 0).into());
    for slot in [DATA_MID, DATA_TRANSPORT, LEGACY_TRANSPORT] {
        set_private_value(scope, pc, slot, v8::null(scope).into());
    }
}
pub(super) fn has_transceivers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> bool {
    list(scope, pc).length() != 0
}
pub(super) fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, object).expect("validated RTP receiver")
}
fn flag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    get_private_value(scope, object, slot)
        .expect("RTP state slot")
        .boolean_value(scope)
}
fn set_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: bool,
) {
    set_private_value(scope, object, slot, v8::Boolean::new(scope, value).into());
}
pub(super) fn closed<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, pc, super::RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT)
        .expect("PC state")
        .strict_equals(v8str(scope, "closed").into())
}
pub(super) fn stopping<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transceiver: v8::Local<'s, v8::Object>,
) -> bool {
    flag(scope, transceiver, STOPPING)
}
pub(super) fn sending<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transceiver: v8::Local<'s, v8::Object>,
) -> bool {
    let direction =
        get_private_value(scope, transceiver, CURRENT_DIRECTION).expect("current RTP direction");
    direction.strict_equals(v8str(scope, "sendrecv").into())
        || direction.strict_equals(v8str(scope, "sendonly").into())
}
fn invalid_state(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "The RTP transceiver or connection is stopped.",
    );
}
fn list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    v8::Local::<v8::Array>::try_from(
        get_private_value(scope, pc, TRANSCEIVERS).expect("transceiver list"),
    )
    .expect("native RTP array")
}
fn entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let list = list(scope, pc);
    (0..list.length())
        .map(|i| {
            v8::Local::<v8::Object>::try_from(list.get_index(scope, i).expect("RTP list entry"))
                .expect("native transceiver")
        })
        .collect()
}

pub(super) fn stats_selector_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    track: v8::Local<'s, v8::Object>,
) -> usize {
    let track = target(scope, track);
    let mut count = 0;
    for transceiver in entries(scope, pc) {
        // Stats associations outlive stop()/close(), just like the monitored
        // endpoints. Closing a connection does not remove its track selectors.
        for slot in [SENDER, RECEIVER] {
            let object = get_private_object(scope, transceiver, slot).expect("RTP endpoint");
            if let Some(candidate) = get_private_object(scope, object, TRACK)
                && target(scope, candidate).strict_equals(track.into())
            {
                count += 1;
            }
        }
    }
    count
}
fn instance<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> Option<v8::Local<'s, v8::Object>> {
    let object = exposed_interfaces::build_intrinsic_interface_instance(scope, name).ok()?;
    let prototype = exposed_interfaces::ensure_intrinsic_interface_prototype(scope, name).ok()?;
    (object.set_prototype(scope, prototype.into()) == Some(true)).then_some(object)
}

fn validate_encodings(
    scope: &mut v8::PinScope<'_, '_>,
    kind: &str,
    encodings: &mut Vec<Encoding>,
) -> bool {
    let rids: Vec<_> = encodings
        .iter()
        .filter_map(|e| e.coding.rid.as_ref())
        .collect();
    if (!rids.is_empty() && rids.len() != encodings.len())
        || rids.iter().any(|rid| {
            rid.is_empty() || rid.len() > 16 || !rid.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        || rids
            .iter()
            .enumerate()
            .any(|(i, rid)| rids[..i].contains(rid))
    {
        throw_type_error(
            scope,
            "Encodings require unique, consistently present alphanumeric RIDs of at most 16 characters.",
        );
        return false;
    }
    let supported = capabilities(kind);
    for encoding in encodings.iter_mut() {
        if encoding
            .codec
            .as_ref()
            .is_some_and(|codec| !supported.iter().any(|candidate| codec.matches(candidate)))
        {
            throw_dom_exception(
                scope,
                "OperationError",
                0,
                "The requested encoding codec is unsupported.",
            );
            return false;
        }
        if encoding.codec.is_some() {
            throw_dom_exception(
                scope,
                "OperationError",
                0,
                "Per-encoding codec selection requires a media backend.",
            );
            return false;
        }
        if kind == "audio" {
            encoding.max_framerate = None;
            encoding.scale_resolution_down_by = None;
        }
        if encoding.max_framerate.as_ref().is_some_and(|v| v.0 <= 0.0)
            || encoding
                .scale_resolution_down_by
                .as_ref()
                .is_some_and(|v| v.0 < 1.0)
        {
            throw_range_error(scope, "Invalid video encoding rate or scale.");
            return false;
        }
    }
    // The local frontend retains at most three simulcast encodings. No RTP
    // encoder is started. Validate every input before applying this limit.
    let any_scale = encodings
        .iter()
        .any(|e| e.scale_resolution_down_by.is_some());
    encodings.truncate(if kind == "audio" { 1 } else { 3 });
    let count = encodings.len();
    for (index, encoding) in encodings.iter_mut().enumerate() {
        if count == 1 {
            encoding.coding.rid = None;
        }
        if kind == "video" && encoding.scale_resolution_down_by.is_none() {
            encoding.scale_resolution_down_by = Some(webidl::Double(if any_scale {
                1.0
            } else {
                2_f64.powi((count - index - 1) as i32)
            }));
        }
    }
    true
}

fn add_transceiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(mut parsed) = webidl::parse_args::<AddArgs>(scope, &args) else {
        return;
    };
    let (kind, track) = match parsed.track_or_kind {
        TrackOrKind::Track(track) => (media_streams::track_kind(scope, track), Some(track)),
        TrackOrKind::Kind(kind) => (kind, None),
    };
    if !matches!(kind.as_str(), "audio" | "video") || parsed.init.direction == Direction::Stopped {
        throw_type_error(scope, "Invalid transceiver kind or direction.");
        return;
    }
    let pc = target(scope, args.this());
    if closed(scope, pc) {
        invalid_state(scope);
        return;
    }
    if !validate_encodings(scope, &kind, &mut parsed.init.send_encodings) {
        return;
    }
    let Some(receiver_track) = media_streams::new_remote_track(scope, &kind) else {
        return;
    };
    let Some(sender) = instance(scope, "RTCRtpSender") else {
        return;
    };
    let Some(receiver) = instance(scope, "RTCRtpReceiver") else {
        return;
    };
    let Some(transceiver) = instance(scope, "RTCRtpTransceiver") else {
        return;
    };
    let encodings = if parsed.init.send_encodings.is_empty() {
        if kind == "video" {
            serde_json::json!([{"active":true,"scaleResolutionDownBy":1.0}])
        } else {
            serde_json::json!([{"active":true}])
        }
    } else {
        serde_json::Value::Array(
            parsed
                .init
                .send_encodings
                .iter()
                .map(Encoding::snapshot)
                .collect(),
        )
    };
    // Senders retain associated IDs, rather than keeping author stream objects
    // alive. Native identity is consulted without invoking public id getters.
    let mut streams: Vec<v8::Local<v8::Value>> = Vec::new();
    for stream in parsed.init.streams {
        let id = media_streams::stream_id(scope, stream);
        if !streams
            .iter()
            .any(|existing| existing.strict_equals(id.into()))
        {
            streams.push(id.into());
        }
    }
    ReceiverSlots::new(receiver_track)
        .initialize(scope, receiver)
        .expect("RTP receiver slots");
    TransceiverSlots::new(
        pc,
        sender,
        receiver,
        v8str(scope, parsed.init.direction.token()),
    )
    .initialize(scope, transceiver)
    .expect("RTP transceiver slots");
    if super::rtp_sender::initialize(
        scope,
        sender,
        super::rtp_sender::Init {
            pc,
            transceiver,
            kind: &kind,
            track,
            encodings,
            streams: &streams,
        },
    )
    .is_none()
    {
        return;
    }
    let array = list(scope, pc);
    if array.set_index(scope, array.length(), transceiver.into()) != Some(true) {
        return;
    }
    update_negotiation_needed(scope, pc);
    rv.set(transceiver.into());
}
fn collect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let pc = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    let result: Vec<_> = entries(scope, pc)
        .into_iter()
        .filter_map(|transceiver| {
            if slot == TRANSCEIVERS {
                Some(transceiver.into())
            } else if flag(scope, transceiver, STOPPED) {
                None
            } else {
                get_private_value(scope, transceiver, &slot)
            }
        })
        .collect();
    rv.set(v8::Array::new_with_elements(scope, &result).into());
}
fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, object, &slot).expect("RTP slot"));
}
fn transport_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = target(scope, args.this());
    rv.set(
        get_private_value(scope, receiver, super::transports::RTP_TRANSPORT)
            .expect("receiver transport"),
    );
}
fn direction_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    rv.set(if flag(scope, object, STOPPING) {
        v8str(scope, "stopped").into()
    } else {
        get_private_value(scope, object, DIRECTION).expect("RTP direction")
    });
}
fn current_direction_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    rv.set(if flag(scope, object, STOPPED) {
        v8str(scope, "stopped").into()
    } else {
        get_private_value(scope, object, CURRENT_DIRECTION).expect("current RTP direction")
    });
}
fn direction_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DirectionArgs>(scope, &args) else {
        return;
    };
    // WebIDL enum attributes ignore unknown strings after DOMString
    // conversion. Operations and dictionaries use the strict enum converter.
    let Some(direction) = <Direction as webidl::WebIdlEnum>::parse_token(&parsed.value) else {
        return;
    };
    let object = target(scope, args.this());
    if flag(scope, object, STOPPING) {
        invalid_state(scope);
        return;
    }
    if direction == Direction::Stopped {
        throw_type_error(scope, "Use stop() to stop a transceiver.");
        return;
    }
    let next = v8str(scope, direction.token());
    if get_private_value(scope, object, DIRECTION)
        .expect("RTP direction")
        .strict_equals(next.into())
    {
        return;
    }
    set_private_value(scope, object, DIRECTION, next.into());
    let pc = get_private_object(scope, object, OWNER).expect("RTP owner");
    update_negotiation_needed(scope, pc);
}
fn stop_receiving<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) {
    if flag(scope, object, STOPPING) {
        return;
    }
    set_bool(scope, object, STOPPING, true);
    set_private_value(scope, object, DIRECTION, v8str(scope, "inactive").into());
    let receiver = get_private_object(scope, object, RECEIVER).expect("RTP receiver");
    let track = get_private_object(scope, receiver, TRACK).expect("receiver track");
    queue_task(scope, track, RendererPageWebRtcTaskKind::TrackEnded);
}
fn stop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let pc = get_private_object(scope, object, OWNER).expect("RTP owner");
    if closed(scope, pc) {
        invalid_state(scope);
        return;
    }
    stop_receiving(scope, object);
    update_negotiation_needed(scope, pc);
}
pub(super) fn close<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    for object in entries(scope, pc) {
        stop_receiving(scope, object);
        set_bool(scope, object, STOPPED, true);
    }
}
fn set_codec_preferences<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CodecArgs>(scope, &args) else {
        return;
    };
    let object = target(scope, args.this());
    let receiver = get_private_object(scope, object, RECEIVER).expect("RTP receiver");
    let track = get_private_object(scope, receiver, TRACK).expect("receiver track");
    let supported = capabilities(&media_streams::track_kind(scope, track));
    let mut codecs: Vec<Codec> = Vec::new();
    for codec in parsed.codecs {
        if !supported.iter().any(|candidate| codec.matches(candidate)) {
            throw_dom_exception(
                scope,
                "InvalidModificationError",
                13,
                "The requested codec is unsupported.",
            );
            return;
        }
        if !codecs.iter().any(|candidate| codec.matches(candidate)) {
            codecs.push(codec);
        }
    }
    if !codecs.is_empty() && !codecs.iter().any(Codec::is_media) {
        throw_dom_exception(
            scope,
            "InvalidModificationError",
            13,
            "At least one media codec is required.",
        );
        return;
    }
    set_private_value(
        scope,
        object,
        CODECS,
        v8_string(
            scope,
            &serde_json::to_string(&codecs).expect("codec snapshot"),
        )
        .expect("codec snapshot string")
        .into(),
    );
}
fn queue_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: RendererPageWebRtcTaskKind,
) -> bool {
    context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host| unsafe { &mut *host }.queue_webrtc_task(scope, object, kind))
}
pub(super) fn update_negotiation_needed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) {
    if !closed(scope, pc) && !flag(scope, pc, NEGOTIATION_TASK) {
        let queued = queue_task(scope, pc, RendererPageWebRtcTaskKind::NegotiationNeeded);
        set_bool(scope, pc, NEGOTIATION_TASK, queued);
    }
}
fn negotiation_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let pc = target(scope, args.this());
    let active = args.get(0).is_object();
    let handler = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, pc, NEGOTIATION_HANDLER, handler);
    media_queries::simple_object_event_set_ordered_handler(
        scope,
        pc,
        super::RTC_PEER_CONNECTION_LISTENERS_SLOT,
        "negotiationneeded",
        NEGOTIATION_HANDLER,
        active,
    );
}
pub(crate) fn apply_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: RendererPageWebRtcTaskKind,
) -> bool {
    let (event_type, listeners) = match kind {
        RendererPageWebRtcTaskKind::DtlsStateChange => {
            return super::transports::state_change_task(scope, object);
        }
        RendererPageWebRtcTaskKind::StartDataChannelClose
        | RendererPageWebRtcTaskKind::DataChannelClosed => {
            return super::data_channel::apply_close_task(scope, object, kind);
        }
        RendererPageWebRtcTaskKind::GetStats => return super::stats::apply(scope, object),
        RendererPageWebRtcTaskKind::CreateOffer
        | RendererPageWebRtcTaskKind::SetLocalDescription
        | RendererPageWebRtcTaskKind::ReplaceTrack
        | RendererPageWebRtcTaskKind::CompleteReplaceTrack => {
            return super::operations::apply(scope, object, kind);
        }
        RendererPageWebRtcTaskKind::ClearRtpParameters
        | RendererPageWebRtcTaskKind::SetRtpParameters => {
            return super::rtp_sender::apply_parameters(scope, object, kind);
        }
        RendererPageWebRtcTaskKind::TrackEnded => {
            if !media_streams::end_track(scope, object) {
                return false;
            }
            ("ended", media_streams::LISTENERS)
        }
        RendererPageWebRtcTaskKind::NegotiationNeeded => {
            set_bool(scope, object, NEGOTIATION_TASK, false);
            if !super::operations::is_empty(scope, object) {
                super::operations::defer_negotiation(scope, object);
                return false;
            }
            if closed(scope, object)
                || flag(scope, object, NEGOTIATION_NEEDED)
                || !get_private_value(
                    scope,
                    object,
                    super::RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT,
                )
                .expect("PC state")
                .strict_equals(v8str(scope, "stable").into())
                || (!flag(
                    scope,
                    object,
                    super::RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT,
                ) && !entries(scope, object)
                    .iter()
                    .any(|entry| !flag(scope, *entry, STOPPING)))
            {
                return false;
            }
            set_bool(scope, object, NEGOTIATION_NEEDED, true);
            (
                "negotiationneeded",
                super::RTC_PEER_CONNECTION_LISTENERS_SLOT,
            )
        }
    };
    super::signaling::dispatch_event(scope, object, listeners, event_type)
}

fn allocate_mid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::String>> {
    let next = get_private_value(scope, pc, NEXT_MID)?.uint32_value(scope)?;
    let Some(after) = next.checked_add(1) else {
        throw_dom_exception(
            scope,
            "OperationError",
            0,
            "No unused media identifiers remain.",
        );
        return None;
    };
    set_private_value(
        scope,
        pc,
        NEXT_MID,
        v8::Integer::new_from_unsigned(scope, after).into(),
    );
    v8_string(scope, &next.to_string())
}

fn offer_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    v8::Local::try_from(get_private_value(scope, pc, OFFER_SNAPSHOT).expect("offer snapshot"))
        .expect("native offer association array")
}

/// Reserve MIDs and transport credentials at offer creation. Observable MIDs
/// and endpoint transports change only when this exact snapshot is applied.
pub(super) fn offer_sdp<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    audio: bool,
    video: bool,
    data: bool,
) -> Option<String> {
    let mut snapshot = Vec::new();
    // Preserve the existing offerToReceive compatibility surface. It has no
    // native transceivers to associate, but still needs valid ICE credentials.
    if !has_transceivers(scope, pc) && (audio || video) {
        let existing = get_private_object(scope, pc, LEGACY_TRANSPORT);
        let transport = super::transports::plan(scope, pc, existing)?;
        set_private_value(scope, pc, LEGACY_TRANSPORT, transport.into());
        let next = get_private_value(scope, pc, NEXT_MID)?
            .uint32_value(scope)?
            .max(3);
        set_private_value(
            scope,
            pc,
            NEXT_MID,
            v8::Integer::new_from_unsigned(scope, next).into(),
        );
        snapshot.push(
            OfferAssociation::new(v8::null(scope).into(), v8str(scope, ""), transport)
                .bind(scope)
                .ok()?
                .into(),
        );
        if data {
            set_private_value(scope, pc, DATA_MID, v8str(scope, "2").into());
            set_private_value(scope, pc, DATA_TRANSPORT, transport.into());
        }
        let mut sdp = super::build_signaling_only_offer(audio, video, data);
        let at = sdp.find("m=").unwrap_or(sdp.len());
        sdp.insert_str(at, &super::transports::sdp_attributes(scope, transport));
        set_private_value(
            scope,
            pc,
            OFFER_SNAPSHOT,
            v8::Array::new_with_elements(scope, &snapshot).into(),
        );
        return Some(sdp);
    }
    let mut sections = Vec::new();
    for object in entries(scope, pc) {
        let stopped = flag(scope, object, STOPPING);
        if stopped && get_private_value(scope, object, MID)?.is_null() {
            continue;
        }
        let mid =
            match v8::Local::<v8::String>::try_from(get_private_value(scope, object, JSEP_MID)?) {
                Ok(mid) => mid,
                Err(_) => {
                    let mid = allocate_mid(scope, pc)?;
                    set_private_value(scope, object, JSEP_MID, mid.into());
                    mid
                }
            };
        let existing = get_private_object(scope, object, PLANNED_TRANSPORT);
        let transport = super::transports::plan(scope, pc, existing)?;
        set_private_value(scope, object, PLANNED_TRANSPORT, transport.into());
        snapshot.push(
            OfferAssociation::new(object.into(), mid, transport)
                .bind(scope)
                .ok()?
                .into(),
        );
        let receiver = get_private_object(scope, object, RECEIVER).expect("RTP receiver");
        let track = get_private_object(scope, receiver, TRACK).expect("receiver track");
        let kind = media_streams::track_kind(scope, track);
        let direction = if stopped {
            "inactive".into()
        } else {
            get_private_value(scope, object, DIRECTION)
                .expect("direction")
                .to_rust_string_lossy(scope)
        };
        let codecs = get_private_value(scope, object, CODECS)
            .expect("codec snapshot")
            .to_rust_string_lossy(scope);
        let codecs: Vec<Codec> = serde_json::from_str(&codecs).expect("native codec snapshot");
        let sender = get_private_object(scope, object, SENDER).expect("RTP sender");
        let (streams, track) = super::rtp_sender::sdp_identity(scope, sender);
        sections.push(super::rtp_offer::Section {
            mid: mid.to_rust_string_lossy(scope),
            rejected: stopped,
            transport_attributes: super::transports::sdp_attributes(scope, transport),
            kind: kind.clone(),
            direction,
            codecs: if codecs.is_empty() {
                capabilities(&kind)
            } else {
                codecs
            },
            streams,
            track,
        });
    }
    let data_section = if data {
        let mid = match v8::Local::<v8::String>::try_from(get_private_value(scope, pc, DATA_MID)?) {
            Ok(mid) => mid,
            Err(_) => {
                let mid = allocate_mid(scope, pc)?;
                set_private_value(scope, pc, DATA_MID, mid.into());
                mid
            }
        };
        let existing = get_private_object(scope, pc, DATA_TRANSPORT);
        let transport = super::transports::plan(scope, pc, existing)?;
        set_private_value(scope, pc, DATA_TRANSPORT, transport.into());
        snapshot.push(
            OfferAssociation::new(v8::null(scope).into(), mid, transport)
                .bind(scope)
                .ok()?
                .into(),
        );
        Some((
            mid.to_rust_string_lossy(scope),
            super::transports::sdp_attributes(scope, transport),
        ))
    } else {
        None
    };
    set_private_value(
        scope,
        pc,
        OFFER_SNAPSHOT,
        v8::Array::new_with_elements(scope, &snapshot).into(),
    );
    Some(super::rtp_offer::build(
        &sections,
        data_section
            .as_ref()
            .map(|(mid, attributes)| (mid.as_str(), attributes.as_str())),
    ))
}

pub(super) fn apply_local_offer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let snapshot = offer_snapshot(scope, pc);
    let replacements = v8::Map::new(scope);
    // Stage allocations before mutating endpoint associations. A rollback may
    // have closed a previously generated offer's transport in the meantime.
    let mut associations = Vec::new();
    for index in 0..snapshot.length() {
        let association =
            v8::Local::<v8::Object>::try_from(snapshot.get_index(scope, index)?).ok()?;
        let planned = get_private_object(scope, association, PLANNED_TRANSPORT)?;
        let transport = if let Ok(transport) =
            v8::Local::<v8::Object>::try_from(replacements.get(scope, planned.into())?)
        {
            transport
        } else {
            let transport = super::transports::from_offer(scope, pc, planned)?;
            replacements.set(scope, planned.into(), transport.into())?;
            transport
        };
        associations.push((association, transport));
    }
    for (association, transport) in associations {
        super::transports::applied(scope, transport);
        super::transports::remember_bundle(scope, pc, transport);
        let mid = get_private_value(scope, association, MID)?;
        if let Some(transceiver) = get_private_object(scope, association, TRANSCEIVERS) {
            set_private_value(scope, transceiver, JSEP_MID, mid);
            set_private_value(scope, transceiver, MID, mid);
            set_private_value(scope, transceiver, PLANNED_TRANSPORT, transport.into());
            if !flag(scope, transceiver, STOPPED) {
                for endpoint in [SENDER, RECEIVER] {
                    let endpoint = get_private_object(scope, transceiver, endpoint)?;
                    set_private_value(
                        scope,
                        endpoint,
                        super::transports::RTP_TRANSPORT,
                        transport.into(),
                    );
                }
            }
        } else if mid.strict_equals(v8str(scope, "").into()) {
            set_private_value(scope, pc, LEGACY_TRANSPORT, transport.into());
        } else {
            set_private_value(scope, pc, DATA_MID, mid);
            set_private_value(scope, pc, DATA_TRANSPORT, transport.into());
        }
    }
    Some(())
}

pub(super) fn rollback_local_offer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) {
    // The frontend has no remote/answer path yet, so every association is new
    // since the last stable state. Full negotiation must retain stable snapshots.
    for transceiver in entries(scope, pc) {
        if let Some(transport) = get_private_object(scope, transceiver, PLANNED_TRANSPORT) {
            super::transports::rollback(scope, transport);
        }
        for slot in [MID, JSEP_MID, PLANNED_TRANSPORT] {
            set_private_value(scope, transceiver, slot, v8::null(scope).into());
        }
        for endpoint in [SENDER, RECEIVER] {
            let endpoint = get_private_object(scope, transceiver, endpoint).expect("RTP endpoint");
            set_private_value(
                scope,
                endpoint,
                super::transports::RTP_TRANSPORT,
                v8::null(scope).into(),
            );
        }
    }
    for slot in [DATA_TRANSPORT, LEGACY_TRANSPORT] {
        if let Some(transport) = get_private_object(scope, pc, slot) {
            super::transports::rollback(scope, transport);
        }
        set_private_value(scope, pc, slot, v8::null(scope).into());
    }
    set_private_value(scope, pc, DATA_MID, v8::null(scope).into());
    super::transports::clear_bundle(scope, pc);
}
