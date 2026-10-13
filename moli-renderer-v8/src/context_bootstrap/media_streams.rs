//! MediaStream track sets and track lifecycle, independent of capture devices.
//! Capture producers, constraints, and media consumers still need a backend.

use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, initialize_web_api_constructor_receiver,
};

use super::{exposed_interfaces, media_queries};
use crate::{
    util::{get_private_object, get_private_value, set_private_value, throw_type_error, v8str},
    web_api_interfaces, webidl,
};

const ID: &str = "__moliMediaIdentity";
const TRACKS: &str = "__moliMediaStreamTracks";
const SOURCE: &str = "__moliMediaTrackSource";
const KIND: &str = "__moliMediaSourceKind";
const LABEL: &str = "__moliMediaSourceLabel";
const MUTED: &str = "__moliMediaSourceMuted";
const ENABLED: &str = "__moliMediaTrackEnabled";
const READY_STATE: &str = "__moliMediaTrackReadyState";
pub(super) const LISTENERS: &str = "__moliMediaFrontendListeners";
const HANDLER_PREFIX: &str = "__moliMediaHandlerOn";
const HANDLER_SLOTS: &[&str] = &[
    "__moliMediaHandlerOnaddtrack",
    "__moliMediaHandlerOnremovetrack",
    "__moliMediaHandlerOnmute",
    "__moliMediaHandlerOnunmute",
    "__moliMediaHandlerOnended",
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaStream)]
struct StreamSlots<'s> {
    #[webapi(slot = ID)]
    id: v8::Local<'s, v8::String>,
    #[webapi(slot = TRACKS)]
    tracks: v8::Local<'s, v8::Array>,
    #[webapi(slot = "__moliMediaHandlerOnaddtrack", init = "null")]
    onaddtrack: (),
    #[webapi(slot = "__moliMediaHandlerOnremovetrack", init = "null")]
    onremovetrack: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaStreamTrack)]
struct TrackSlots<'s> {
    #[webapi(slot = ID)]
    id: v8::Local<'s, v8::String>,
    #[webapi(slot = SOURCE)]
    source: v8::Local<'s, v8::Object>,
    #[webapi(slot = ENABLED)]
    enabled: bool,
    #[webapi(slot = READY_STATE)]
    ready_state: v8::Local<'s, v8::Value>,
    #[webapi(slot = "__moliMediaHandlerOnmute", init = "null")]
    onmute: (),
    #[webapi(slot = "__moliMediaHandlerOnunmute", init = "null")]
    onunmute: (),
    #[webapi(slot = "__moliMediaHandlerOnended", init = "null")]
    onended: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaStream, enumerable, receiver)]
struct StreamPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, ID))]
    id: (),
    #[webapi(accessor_property, getter = active_getter)]
    active: (),
    #[webapi(accessor_property, getter = slot_getter, setter = handler_setter, data = v8str(scope, "__moliMediaHandlerOnaddtrack"))]
    onaddtrack: (),
    #[webapi(accessor_property, getter = slot_getter, setter = handler_setter, data = v8str(scope, "__moliMediaHandlerOnremovetrack"))]
    onremovetrack: (),
    #[webapi(method, length = 0, callback = tracks_getter, data = v8str(scope, "audio"))]
    get_audio_tracks: (),
    #[webapi(method, length = 0, callback = tracks_getter, data = v8str(scope, "video"))]
    get_video_tracks: (),
    #[webapi(method, length = 0, callback = tracks_getter, data = v8str(scope, ""))]
    get_tracks: (),
    #[webapi(method, length = 1, callback = track_by_id)]
    get_track_by_id: (),
    #[webapi(method, length = 1, callback = add_track)]
    add_track: (),
    #[webapi(method, length = 1, callback = remove_track)]
    remove_track: (),
    #[webapi(method, length = 0, callback = clone_stream)]
    clone: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaStreamTrack, enumerable, receiver)]
struct TrackPrototype {
    #[webapi(accessor_property, getter = source_slot_getter, data = v8str(scope, KIND))]
    kind: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, ID))]
    id: (),
    #[webapi(accessor_property, getter = source_slot_getter, data = v8str(scope, LABEL))]
    label: (),
    #[webapi(accessor_property, getter = slot_getter, setter = enabled_setter, data = v8str(scope, ENABLED))]
    enabled: (),
    #[webapi(accessor_property, getter = source_slot_getter, data = v8str(scope, MUTED))]
    muted: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, READY_STATE))]
    ready_state: (),
    #[webapi(accessor_property, getter = slot_getter, setter = handler_setter, data = v8str(scope, "__moliMediaHandlerOnmute"))]
    onmute: (),
    #[webapi(accessor_property, getter = slot_getter, setter = handler_setter, data = v8str(scope, "__moliMediaHandlerOnunmute"))]
    onunmute: (),
    #[webapi(accessor_property, getter = slot_getter, setter = handler_setter, data = v8str(scope, "__moliMediaHandlerOnended"))]
    onended: (),
    #[webapi(method, length = 0, callback = clone_track_callback)]
    clone: (),
    #[webapi(method, length = 0, callback = stop_track)]
    stop: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "MediaStream" => StreamPrototype::initialize_prototype_template(scope, prototype),
        "MediaStreamTrack" => TrackPrototype::initialize_prototype_template(scope, prototype),
        _ => (),
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaStream")]
struct ConstructorArgs<'s> {
    #[webidl(with = constructor_tracks)]
    tracks: Vec<v8::Local<'s, v8::Object>>,
}

fn constructor_tracks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Vec<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    if args.length() <= index {
        return Ok(Vec::new());
    }
    let value = args.get(index);
    // This constructor's overload distinguishes native MediaStream identity
    // before consulting @@iterator; sequence conversion stays in shared WebIDL.
    if let Ok(stream) = v8::Local::<v8::Object>::try_from(value)
        && web_api_interfaces::MediaStream::is_instance(scope, stream)
    {
        let stream = target(scope, stream);
        return Ok(track_set(scope, stream));
    }
    let options = webidl::InterfaceOptions {
        name: web_api_interfaces::MediaStreamTrack::NAME,
        brand_check: web_api_interfaces::MediaStreamTrack::is_instance,
    };
    let tracks = webidl::convert_with_options::<webidl::Sequence<webidl::InterfaceObject>>(
        scope,
        value,
        webidl::Context::argument("MediaStream", (index + 1) as usize),
        &options,
    )?;
    Ok(tracks.0.into_iter().map(|track| track.0).collect())
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaStream track")]
struct TrackArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::MediaStreamTrack)]
    track: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaStream.getTrackById")]
struct IdArgs {
    #[webidl(required, converter = "raw")]
    id: webidl::DomString16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaStreamTrack.enabled")]
struct EnabledArgs {
    #[webidl(required)]
    value: bool,
}

pub(super) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "MediaStream requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "MediaStream") {
        return;
    }
    if initialize_stream(scope, args.this(), parsed.tracks).is_some() {
        rv.set(args.this().into());
    }
}

fn identifier<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::String>> {
    let mut bytes = [0_u8; 16];
    if let Err(error) = moli_crypto::fill_secure_random(&mut bytes) {
        super::throw_error_exception(scope, &format!("Media identity failed: {error}"));
        return None;
    }
    let id = uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string();
    crate::util::v8_string(scope, &id)
}

pub(super) fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver).expect("validated media receiver")
}

pub(super) fn initialize_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    media_queries::mark_simple_event_target_slot(scope, object, LISTENERS);
    media_queries::install_simple_event_target_ordered_handlers(scope, object);
}

fn initialize_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    tracks: Vec<v8::Local<'s, v8::Object>>,
) -> Option<()> {
    let id = identifier(scope)?;
    let mut unique: Vec<v8::Local<v8::Value>> = Vec::new();
    let mut identities = Vec::new();
    for track in tracks {
        let identity = target(scope, track);
        if !identities
            .iter()
            .any(|other: &v8::Local<v8::Object>| other.strict_equals(identity.into()))
        {
            identities.push(identity);
            unique.push(track.into());
        }
    }
    StreamSlots::new(id, v8::Array::new_with_elements(scope, &unique))
        .initialize(scope, object)
        .expect("MediaStream slots");
    initialize_event_target(scope, object);
    Some(())
}

fn track_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let array = v8::Local::<v8::Array>::try_from(
        get_private_value(scope, stream, TRACKS).expect("MediaStream tracks"),
    )
    .expect("private track set array");
    (0..array.length())
        .map(|index| {
            v8::Local::<v8::Object>::try_from(
                array
                    .get_index(scope, index)
                    .expect("native track set entry"),
            )
            .expect("native track object")
        })
        .collect()
}

pub(super) fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, object, &slot).expect("media slot"));
}

pub(super) fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    let slot = HANDLER_SLOTS
        .iter()
        .copied()
        .find(|candidate| *candidate == slot)
        .expect("native media handler slot");
    let event = slot
        .strip_prefix(HANDLER_PREFIX)
        .expect("media handler name");
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, object, slot, value);
    media_queries::simple_object_event_set_ordered_handler(
        scope, object, LISTENERS, event, slot, active,
    );
}

fn source_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::Value> {
    let track = target(scope, track);
    let source = get_private_object(scope, track, SOURCE).expect("native media source");
    get_private_value(scope, source, slot).expect("native media source slot")
}

fn source_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(source_value(scope, args.this(), &slot));
}

fn enabled_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<EnabledArgs>(scope, &args) else {
        return;
    };
    let object = target(scope, args.this());
    set_private_value(
        scope,
        object,
        ENABLED,
        v8::Boolean::new(scope, parsed.value).into(),
    );
}

fn active_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = target(scope, args.this());
    let active = track_set(scope, stream).iter().any(|track| {
        let track = target(scope, *track);
        get_private_value(scope, track, READY_STATE)
            .expect("native track state")
            .strict_equals(v8str(scope, "live").into())
    });
    rv.set(v8::Boolean::new(scope, active).into());
}

fn tracks_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = target(scope, args.this());
    let context = stream
        .get_creation_context(scope)
        .expect("MediaStream realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let kind = args.data().to_rust_string_lossy(scope);
    let tracks: Vec<v8::Local<v8::Value>> = track_set(scope, stream)
        .into_iter()
        .filter(|track| {
            kind.is_empty() || source_value(scope, *track, KIND).strict_equals(args.data())
        })
        .map(Into::into)
        .collect();
    rv.set(v8::Array::new_with_elements(scope, &tracks).into());
}

fn track_by_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdArgs>(scope, &args) else {
        return;
    };
    let stream = target(scope, args.this());
    let id = v8::String::new_from_two_byte(scope, &parsed.id.0, v8::NewStringType::Normal)
        .expect("track id query");
    let found = track_set(scope, stream).into_iter().find(|track| {
        let object = target(scope, *track);
        get_private_value(scope, object, ID)
            .expect("native track id")
            .strict_equals(id.into())
    });
    rv.set(
        found
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn replace_tracks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    tracks: &[v8::Local<'s, v8::Object>],
) {
    let values: Vec<v8::Local<v8::Value>> = tracks.iter().copied().map(Into::into).collect();
    let array = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, stream, TRACKS, array.into());
}

fn add_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TrackArgs>(scope, &args) else {
        return;
    };
    let stream = target(scope, args.this());
    let identity = target(scope, parsed.track);
    let mut tracks = track_set(scope, stream);
    if tracks
        .iter()
        .any(|track| target(scope, *track).strict_equals(identity.into()))
    {
        return;
    }
    tracks.push(parsed.track);
    // Script mutations do not fire the UA's addtrack/removetrack notifications.
    replace_tracks(scope, stream, &tracks);
}

fn remove_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TrackArgs>(scope, &args) else {
        return;
    };
    let stream = target(scope, args.this());
    let identity = target(scope, parsed.track);
    let tracks: Vec<_> = track_set(scope, stream)
        .into_iter()
        .filter(|track| !target(scope, *track).strict_equals(identity.into()))
        .collect();
    replace_tracks(scope, stream, &tracks);
}

fn clone_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let stream = target(scope, args.this());
    let context = stream
        .get_creation_context(scope)
        .expect("MediaStream realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let mut clones = Vec::new();
    for track in track_set(scope, stream) {
        let Some(clone) = clone_track(scope, track) else {
            return;
        };
        clones.push(clone);
    }
    let clone = exposed_interfaces::build_intrinsic_interface_instance(scope, "MediaStream")
        .expect("MediaStream instance");
    let prototype = exposed_interfaces::ensure_intrinsic_interface_prototype(scope, "MediaStream")
        .expect("MediaStream prototype");
    if clone.set_prototype(scope, prototype.into()) != Some(true) {
        return;
    }
    if initialize_stream(scope, clone, clones).is_some() {
        rv.set(clone.into());
    }
}

fn new_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    enabled: bool,
    state: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = identifier(scope)?;
    let object = exposed_interfaces::build_intrinsic_interface_instance(scope, "MediaStreamTrack")
        .expect("MediaStreamTrack instance");
    let prototype =
        exposed_interfaces::ensure_intrinsic_interface_prototype(scope, "MediaStreamTrack")
            .expect("MediaStreamTrack prototype");
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    TrackSlots::new(id, source, enabled, state)
        .initialize(scope, object)
        .expect("MediaStreamTrack slots");
    initialize_event_target(scope, object);
    Some(object)
}

fn clone_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let track = target(scope, track);
    let source = get_private_object(scope, track, SOURCE).expect("native media source");
    let enabled = get_private_value(scope, track, ENABLED)
        .expect("track enabled")
        .boolean_value(scope);
    let state = get_private_value(scope, track, READY_STATE).expect("track ready state");
    new_track(scope, source, enabled, state)
}

fn clone_track_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let track = target(scope, args.this());
    let context = track
        .get_creation_context(scope)
        .expect("MediaStreamTrack realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(clone) = clone_track(scope, track) {
        rv.set(clone.into());
    }
}

fn stop_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let track = target(scope, args.this());
    set_private_value(scope, track, READY_STATE, v8str(scope, "ended").into());
    // stop() ends only this track, synchronously, without an ended event.
}

/// A native inert source for lifecycle/collection tests; it is never exposed as
/// a production capture API and does not claim to produce audio/video samples.
#[cfg(test)]
pub(crate) fn inert_track_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &'static str,
    label: &[u16],
) -> v8::Local<'s, v8::Object> {
    let source = v8::Object::new(scope);
    set_private_value(scope, source, KIND, v8str(scope, kind).into());
    let label = v8::String::new_from_two_byte(scope, label, v8::NewStringType::Normal).unwrap();
    set_private_value(scope, source, LABEL, label.into());
    set_private_value(scope, source, MUTED, v8::Boolean::new(scope, false).into());
    new_track(scope, source, true, v8str(scope, "live").into()).unwrap()
}
