//! Native sender state before transport negotiation. Parameter transactions and
//! track changes are real local state; no RTP encoder or transport is started.

use super::{
    operations,
    rtp_parameters::{SendParameters, capabilities},
    rtp_transceivers,
};
use crate::{
    context_bootstrap::media_streams,
    native_bridge::throw_dom_exception,
    page_task_queue::RendererPageWebRtcTaskKind,
    util::{
        get_private_object, get_private_value, set_private_value, throw_range_error,
        throw_type_error, v8_string,
    },
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const OWNER: &str = "__moliRtpOwner";
const TRANSCEIVER: &str = "__moliRtpSenderTransceiver";
const KIND: &str = "__moliRtpSenderKind";
const TRACK: &str = "__moliRtpTrack";
const TRACK_ID: &str = "__moliRtpSdpTrackId";
const ENCODINGS: &str = "__moliRtpEncodings";
const STREAMS: &str = "__moliRtpAssociatedStreamIds";
const LAST_PARAMETERS: &str = "__moliRtpLastReturnedParameters";
const DTMF: &str = "__moliRtpDtmf";
const CNAME: &str = "__moliRtcRtcpCname";
const TASK_SENDER: &str = "__moliRtpTaskSender";
const TASK_PARAMETERS: &str = "__moliRtpTaskParameters";
const TASK_RESOLVER: &str = "__moliRtpTaskResolver";
const TASK_TRACK: &str = "__moliRtpTaskTrack";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCRtpSender)]
struct SenderSlots<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = TRANSCEIVER)]
    transceiver: v8::Local<'s, v8::Object>,
    #[webapi(slot = KIND)]
    kind: v8::Local<'s, v8::String>,
    #[webapi(slot = TRACK)]
    track: v8::Local<'s, v8::Value>,
    #[webapi(slot = TRACK_ID)]
    track_id: v8::Local<'s, v8::String>,
    #[webapi(slot = ENCODINGS)]
    encodings: v8::Local<'s, v8::String>,
    #[webapi(slot = STREAMS)]
    streams: v8::Local<'s, v8::Array>,
    #[webapi(slot = LAST_PARAMETERS, init = "null")]
    last_parameters: (),
    #[webapi(slot = DTMF)]
    dtmf: v8::Local<'s, v8::Value>,
}
#[derive(WebApiObject)]
#[webapi(plain)]
struct ParameterTask<'s> {
    #[webapi(slot = TASK_SENDER)]
    sender: v8::Local<'s, v8::Object>,
    #[webapi(slot = TASK_PARAMETERS)]
    parameters: v8::Local<'s, v8::String>,
    #[webapi(slot = TASK_RESOLVER)]
    resolver: v8::Local<'s, v8::Value>,
}
#[derive(WebApiObject)]
#[webapi(plain)]
struct Replacement<'s> {
    #[webapi(slot = TASK_SENDER)]
    sender: v8::Local<'s, v8::Object>,
    #[webapi(slot = TASK_TRACK)]
    track: v8::Local<'s, v8::Value>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCSetParameterOptions")]
struct SetParameterOptions {}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCRtpSender.setParameters")]
struct SetParametersArgs {
    #[webidl(required, dictionary)]
    parameters: SendParameters,
    #[webidl(dictionary)]
    _options: SetParameterOptions,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCRtpSender.replaceTrack")]
struct ReplaceTrackArgs<'s> {
    #[webidl(required, nullable, interface = web_api_interfaces::MediaStreamTrack)]
    track: Option<v8::Local<'s, v8::Object>>,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCRtpSender.setStreams")]
struct SetStreamsArgs<'s> {
    #[webidl(variadic, interface = web_api_interfaces::MediaStream)]
    streams: Vec<v8::Local<'s, v8::Object>>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCRtpSender, enumerable, receiver)]
struct SenderPrototype {
    #[webapi(accessor_property, getter = track)]
    track: (),
    #[webapi(accessor_property, getter = transport)]
    transport: (),
    #[webapi(accessor_property, getter = dtmf)]
    dtmf: (),
    #[webapi(method, length = 0, callback = get_parameters)]
    get_parameters: (),
    #[webapi(method, returns_promise, length = 1, callback = set_parameters)]
    set_parameters: (),
    #[webapi(method, returns_promise, length = 1, callback = replace_track)]
    replace_track: (),
    #[webapi(method, length = 0, callback = set_streams)]
    set_streams: (),
}
pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    SenderPrototype::initialize_prototype_template(scope, prototype);
}
pub(super) fn initialize_pc<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let cname = media_streams::identifier(scope)?;
    set_private_value(scope, pc, CNAME, cname.into());
    Some(())
}
pub(super) struct Init<'s, 'a> {
    pub pc: v8::Local<'s, v8::Object>,
    pub transceiver: v8::Local<'s, v8::Object>,
    pub kind: &'a str,
    pub track: Option<v8::Local<'s, v8::Object>>,
    pub encodings: serde_json::Value,
    pub streams: &'a [v8::Local<'s, v8::Value>],
}
pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sender: v8::Local<'s, v8::Object>,
    init: Init<'s, '_>,
) -> Option<()> {
    let Init {
        pc,
        transceiver,
        kind,
        track,
        encodings,
        streams,
    } = init;
    let track_id = if let Some(track) = track {
        media_streams::track_id(scope, track)
    } else {
        media_streams::identifier(scope)?
    };
    let dtmf = if kind == "audio" {
        super::dtmf::build(scope, sender)?.into()
    } else {
        v8::null(scope).into()
    };
    SenderSlots::new(
        pc,
        transceiver,
        v8_string(scope, kind)?,
        track
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
        track_id,
        v8_string(scope, &encodings.to_string())?,
        v8::Array::new_with_elements(scope, streams),
        dtmf,
    )
    .initialize(scope, sender)
    .expect("RTP sender slots");
    Some(())
}
fn owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sender: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_object(scope, sender, OWNER).expect("sender owner")
}
fn transceiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sender: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_object(scope, sender, TRANSCEIVER).expect("sender transceiver")
}
fn json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> serde_json::Value {
    let snapshot = get_private_value(scope, object, slot)
        .expect("RTP snapshot")
        .to_rust_string_lossy(scope);
    serde_json::from_str(&snapshot).expect("native RTP JSON")
}
fn invalid_modification(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidModificationError",
        13,
        "Read-only RTP parameters were modified.",
    );
}
fn track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let sender = rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, sender, TRACK).expect("sender track"));
}
fn transport<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_null();
}

fn dtmf<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let sender = rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, sender, DTMF).expect("sender DTMF identity"));
}

fn get_parameters<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let sender = rtp_transceivers::target(scope, args.this());
    let pc = owner(scope, sender);
    let last = get_private_value(scope, sender, LAST_PARAMETERS).expect("last returned parameters");
    let snapshot = if let Ok(snapshot) = v8::Local::<v8::String>::try_from(last) {
        snapshot
    } else {
        let Some(id) = media_streams::identifier(scope) else {
            return;
        };
        let cname = get_private_value(scope, pc, CNAME)
            .expect("connection CNAME")
            .to_rust_string_lossy(scope);
        let value = serde_json::json!({
            "transactionId": id.to_rust_string_lossy(scope),
            "codecs": [], "headerExtensions": [],
            "rtcp": {"cname": cname, "reducedSize": false},
            "encodings": json(scope, sender, ENCODINGS),
        });
        let Some(snapshot) = v8_string(scope, &value.to_string()) else {
            return;
        };
        set_private_value(scope, sender, LAST_PARAMETERS, snapshot.into());
        let task = ParameterTask::new(sender, snapshot, v8::null(scope).into())
            .bind(scope)
            .expect("parameter cache task");
        operations::queue(
            scope,
            pc,
            task,
            RendererPageWebRtcTaskKind::ClearRtpParameters,
        );
        snapshot
    };
    // WebIDL returns a fresh dictionary, never the native transaction snapshot.
    // V8's intrinsic JSON parser ignores replaced global constructors/functions.
    if let Some(value) = v8::json::parse(scope, snapshot) {
        rv.set(value);
    }
}

fn set_parameters<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(mut parsed) = webidl::parse_args::<SetParametersArgs>(scope, &args) else {
        return;
    };
    let sender = rtp_transceivers::target(scope, args.this());
    let transceiver = transceiver(scope, sender);
    if rtp_transceivers::stopping(scope, transceiver)
        || get_private_value(scope, sender, LAST_PARAMETERS)
            .expect("parameter cache")
            .is_null()
    {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The sender is stopped or the parameter transaction expired.",
        );
        return;
    }
    let previous = json(scope, sender, LAST_PARAMETERS);
    let supplied = parsed.parameters.snapshot();
    let previous_encodings = previous["encodings"].as_array().expect("encoding list");
    if ["transactionId", "codecs", "headerExtensions", "rtcp"]
        .iter()
        .any(|key| previous[*key] != supplied[*key])
        || previous_encodings.len() != parsed.parameters.encodings.len()
        || previous_encodings
            .iter()
            .zip(&parsed.parameters.encodings)
            .any(|(old, new)| old.get("rid") != new.snapshot().get("rid"))
    {
        invalid_modification(scope);
        return;
    }
    let kind = get_private_value(scope, sender, KIND)
        .expect("sender kind")
        .to_rust_string_lossy(scope);
    let supported = capabilities(&kind);
    for encoding in &mut parsed.parameters.encodings {
        if let Some(codec) = &encoding.codec {
            if !supported.iter().any(|candidate| codec.matches(candidate)) {
                invalid_modification(scope);
                return;
            }
            throw_dom_exception(
                scope,
                "OperationError",
                0,
                "Per-encoding codec selection requires a media backend.",
            );
            return;
        }
        if kind == "audio" {
            encoding.max_framerate = None;
            encoding.scale_resolution_down_by = None;
        } else {
            let scale = encoding
                .scale_resolution_down_by
                .get_or_insert(webidl::Double(1.0));
            if scale.0 < 1.0 || encoding.max_framerate.as_ref().is_some_and(|v| v.0 < 0.0) {
                throw_range_error(scope, "Invalid video encoding rate or scale.");
                return;
            }
        }
    }
    let encodings: Vec<_> = parsed
        .parameters
        .encodings
        .iter()
        .map(super::rtp_parameters::Encoding::snapshot)
        .collect();
    let Some(snapshot) = v8_string(
        scope,
        &serde_json::to_string(&encodings).expect("encoding snapshot"),
    ) else {
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let task = ParameterTask::new(sender, snapshot, resolver.into())
        .bind(scope)
        .expect("setParameters task");
    let pc = owner(scope, sender);
    operations::queue(
        scope,
        pc,
        task,
        RendererPageWebRtcTaskKind::SetRtpParameters,
    );
    rv.set(resolver.get_promise(scope).into());
}

pub(super) fn apply_parameters<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
    kind: RendererPageWebRtcTaskKind,
) -> bool {
    let sender = get_private_object(scope, task, TASK_SENDER).expect("parameter task sender");
    let snapshot =
        get_private_value(scope, task, TASK_PARAMETERS).expect("parameter task snapshot");
    match kind {
        RendererPageWebRtcTaskKind::ClearRtpParameters => {
            if !get_private_value(scope, sender, LAST_PARAMETERS)
                .expect("parameter cache")
                .strict_equals(snapshot)
            {
                return false;
            }
            set_private_value(scope, sender, LAST_PARAMETERS, v8::null(scope).into());
        }
        RendererPageWebRtcTaskKind::SetRtpParameters => {
            set_private_value(scope, sender, ENCODINGS, snapshot);
            set_private_value(scope, sender, LAST_PARAMETERS, v8::null(scope).into());
            let object =
                get_private_object(scope, task, TASK_RESOLVER).expect("parameter resolver");
            let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) };
            resolver.resolve(scope, v8::undefined(scope).into());
        }
        _ => unreachable!("sender parameter task"),
    }
    true
}

fn replace_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ReplaceTrackArgs>(scope, &args) else {
        return;
    };
    let sender = rtp_transceivers::target(scope, args.this());
    let kind = get_private_value(scope, sender, KIND)
        .expect("sender kind")
        .to_rust_string_lossy(scope);
    if parsed
        .track
        .is_some_and(|track| media_streams::track_kind(scope, track) != kind)
    {
        throw_type_error(scope, "The replacement track has a different kind.");
        return;
    }
    let payload = Replacement::new(
        sender,
        parsed
            .track
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    )
    .bind(scope)
    .expect("replacement snapshot");
    let pc = owner(scope, sender);
    if let Some(promise) =
        operations::enqueue(scope, pc, operations::Kind::ReplaceTrack, payload.into())
    {
        rv.set(promise.into());
    }
}
pub(super) fn start_replace_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> bool {
    let payload = v8::Local::<v8::Object>::try_from(operations::payload(scope, request))
        .expect("replacement payload");
    let sender = get_private_object(scope, payload, TASK_SENDER).expect("replacement sender");
    let transceiver = transceiver(scope, sender);
    let error = if rtp_transceivers::stopping(scope, transceiver) {
        Some(("InvalidStateError", 11, "The transceiver is stopped."))
    } else if rtp_transceivers::sending(scope, transceiver) {
        Some((
            "InvalidModificationError",
            13,
            "Track replacement while sending requires a media backend.",
        ))
    } else {
        None
    };
    if let Some((name, code, message)) = error {
        v8::tc_scope!(let caught, scope);
        throw_dom_exception(caught, name, code, message);
        let reason = caught.exception().expect("replacement error");
        caught.reset();
        operations::reject(caught, request, reason);
        return false;
    }
    true
}
pub(super) fn apply_replace_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    pc: v8::Local<'s, v8::Object>,
) -> bool {
    let payload = v8::Local::<v8::Object>::try_from(operations::payload(scope, request))
        .expect("replacement payload");
    let sender = get_private_object(scope, payload, TASK_SENDER).expect("replacement sender");
    let track = get_private_value(scope, payload, TASK_TRACK).expect("replacement track");
    set_private_value(scope, sender, TRACK, track);
    operations::queue(
        scope,
        pc,
        request,
        RendererPageWebRtcTaskKind::CompleteReplaceTrack,
    )
}
fn set_streams<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SetStreamsArgs>(scope, &args) else {
        return;
    };
    let sender = rtp_transceivers::target(scope, args.this());
    let pc = owner(scope, sender);
    if rtp_transceivers::closed(scope, pc) {
        throw_dom_exception(scope, "InvalidStateError", 11, "The connection is closed.");
        return;
    }
    let mut ids: Vec<v8::Local<v8::Value>> = Vec::new();
    for stream in parsed.streams {
        let id = media_streams::stream_id(scope, stream);
        if !ids.iter().any(|other| other.strict_equals(id.into())) {
            ids.push(id.into());
        }
    }
    let streams = v8::Array::new_with_elements(scope, &ids);
    set_private_value(scope, sender, STREAMS, streams.into());
    rtp_transceivers::update_negotiation_needed(scope, pc);
}

pub(super) fn sdp_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sender: v8::Local<'s, v8::Object>,
) -> (Vec<String>, String) {
    let ids = v8::Local::<v8::Array>::try_from(
        get_private_value(scope, sender, STREAMS).expect("associated stream IDs"),
    )
    .expect("stream ID array");
    let streams = (0..ids.length())
        .map(|i| {
            ids.get_index(scope, i)
                .expect("native stream ID")
                .to_rust_string_lossy(scope)
        })
        .collect();
    let track = get_private_value(scope, sender, TRACK_ID)
        .expect("SDP track ID")
        .to_rust_string_lossy(scope);
    (streams, track)
}
