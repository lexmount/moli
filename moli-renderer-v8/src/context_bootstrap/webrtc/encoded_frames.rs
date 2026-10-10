//! Encoded frame value semantics. Only a media backend can produce an initial
//! frame; copy construction does not pretend to encode or negotiate media.

use crate::{
    blob::array_buffer_from_bytes,
    util::{
        get_private_value, new_null_prototype_object, set_private_value, throw_type_error, v8str,
    },
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod metadata;
use metadata::{AudioMetadata, VideoMetadata};

const DATA: &str = "__moliRtcEncodedFrameData";
const METADATA: &str = "__moliRtcEncodedFrameMetadata";
const TYPE: &str = "__moliRtcEncodedFrameType";

#[derive(Clone, Copy)]
pub(crate) enum FrameKind {
    Audio,
    Video,
}

impl FrameKind {
    fn name(self) -> &'static str {
        match self {
            Self::Audio => "RTCEncodedAudioFrame",
            Self::Video => "RTCEncodedVideoFrame",
        }
    }

    fn members(self) -> impl Iterator<Item = &'static str> {
        metadata::BASE_MEMBERS
            .iter()
            .chain(match self {
                Self::Audio => metadata::AUDIO_MEMBERS,
                Self::Video => metadata::VIDEO_MEMBERS,
            })
            .copied()
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCEncodedAudioFrameOptions")]
struct AudioOptions {
    #[webidl(dictionary)]
    metadata: Option<AudioMetadata>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCEncodedVideoFrameOptions")]
struct VideoOptions {
    #[webidl(dictionary)]
    metadata: Option<VideoMetadata>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCEncodedAudioFrame")]
struct AudioArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::RTCEncodedAudioFrame)]
    original_frame: v8::Local<'s, v8::Object>,
    #[webidl(dictionary)]
    options: AudioOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCEncodedVideoFrame")]
struct VideoArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::RTCEncodedVideoFrame)]
    original_frame: v8::Local<'s, v8::Object>,
    #[webidl(dictionary)]
    options: VideoOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCEncodedFrame.data")]
struct DataArgs<'s> {
    #[webidl(required, converter = "raw")]
    value: v8::Local<'s, v8::ArrayBuffer>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCEncodedAudioFrame, require_prototype)]
struct AudioObject<'s> {
    #[webapi(slot = DATA)]
    data: v8::Local<'s, v8::ArrayBuffer>,
    #[webapi(slot = METADATA)]
    metadata: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCEncodedVideoFrame, require_prototype)]
struct VideoObject<'s> {
    #[webapi(slot = DATA)]
    data: v8::Local<'s, v8::ArrayBuffer>,
    #[webapi(slot = METADATA)]
    metadata: v8::Local<'s, v8::Object>,
    #[webapi(slot = TYPE)]
    kind: v8::Local<'s, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCEncodedAudioFrame, enumerable, receiver)]
struct AudioPrototype {
    #[webapi(accessor_property, getter = data, setter = set_data)]
    data: (),
    #[webapi(method, length = 0, callback = get_audio_metadata)]
    get_metadata: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCEncodedVideoFrame, enumerable, receiver)]
struct VideoPrototype {
    #[webapi(accessor_property = "type", getter = frame_type)]
    kind: (),
    #[webapi(accessor_property, getter = data, setter = set_data)]
    data: (),
    #[webapi(method, length = 0, callback = get_video_metadata)]
    get_metadata: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    name: &str,
) {
    match name {
        "RTCEncodedAudioFrame" => AudioPrototype::initialize_prototype_template(scope, prototype),
        "RTCEncodedVideoFrame" => VideoPrototype::initialize_prototype_template(scope, prototype),
        _ => (),
    }
}

pub(in crate::context_bootstrap) fn audio_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCEncodedAudioFrame requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<AudioArgs>(scope, &args) else {
        return;
    };
    let overrides = parsed.options.metadata.map(|value| value.object(scope));
    if copy_frame(
        scope,
        args.this(),
        parsed.original_frame,
        overrides,
        FrameKind::Audio,
    )
    .is_some()
    {
        rv.set(args.this().into());
    }
}

pub(in crate::context_bootstrap) fn video_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCEncodedVideoFrame requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<VideoArgs>(scope, &args) else {
        return;
    };
    let overrides = parsed.options.metadata.map(|value| value.object(scope));
    if copy_frame(
        scope,
        args.this(),
        parsed.original_frame,
        overrides,
        FrameKind::Video,
    )
    .is_some()
    {
        rv.set(args.this().into());
    }
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, object).expect("validated encoded frame")
}

fn frame_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::ArrayBuffer> {
    let object = target(scope, object);
    v8::Local::try_from(get_private_value(scope, object, DATA).expect("native encoded frame data"))
        .expect("encoded frame data is an ArrayBuffer")
}

fn frame_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let object = target(scope, object);
    v8::Local::try_from(get_private_value(scope, object, METADATA).expect("native frame metadata"))
        .expect("encoded frame metadata is a private dictionary")
}

fn frame_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    rv.set(get_private_value(scope, object, TYPE).expect("encoded video frame type"));
}

fn data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(frame_data(scope, args.this()).into());
}

fn set_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataArgs>(scope, &args) else {
        return;
    };
    let object = target(scope, args.this());
    set_private_value(scope, object, DATA, parsed.value.into());
}

fn get_audio_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = frame_metadata(scope, args.this());
    rv.set(copy_metadata(scope, source, None, FrameKind::Audio, false).into());
}

fn get_video_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = frame_metadata(scope, args.this());
    rv.set(copy_metadata(scope, source, None, FrameKind::Video, false).into());
}

fn copy_frame<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    original: v8::Local<'s, v8::Object>,
    overrides: Option<v8::Local<'s, v8::Object>>,
    kind: FrameKind,
) -> Option<()> {
    // Read internal data after all argument conversions: a metadata getter may
    // have replaced or detached the original data. No public frame getters run.
    let original = target(scope, original);
    let buffer = frame_data(scope, original);
    if buffer.was_detached() {
        throw_type_error(scope, "Cannot copy detached encoded frame data.");
        return None;
    }
    let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())?;
    let mut bytes = vec![0; buffer.byte_length()];
    assert_eq!(view.copy_contents(&mut bytes), bytes.len());
    let data = array_buffer_from_bytes(scope, bytes)?;
    let source = frame_metadata(scope, original);
    let metadata = copy_metadata(scope, source, overrides, kind, true);
    let frame_type = get_private_value(scope, original, TYPE);
    initialize(scope, receiver, data, metadata, frame_type, kind);
    Some(())
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::ArrayBuffer>,
    metadata: v8::Local<'s, v8::Object>,
    frame_type: Option<v8::Local<'s, v8::Value>>,
    kind: FrameKind,
) {
    match kind {
        FrameKind::Audio => AudioObject::new(data, metadata)
            .initialize(scope, object)
            .expect("native audio frame slots"),
        FrameKind::Video => {
            let frame_type =
                v8::Local::<v8::String>::try_from(frame_type.expect("video frame type"))
                    .expect("video frame type is a string");
            VideoObject::new(data, metadata, frame_type)
                .initialize(scope, object)
                .expect("native video frame slots");
        }
    }
}

fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::ArrayBuffer>,
    metadata: v8::Local<'s, v8::Object>,
    frame_type: Option<v8::Local<'s, v8::Value>>,
    kind: FrameKind,
) -> Option<v8::Local<'s, v8::Object>> {
    if !super::super::exposed_interfaces::is_realm_interface_exposed(scope, kind.name()) {
        return None;
    }
    match kind {
        FrameKind::Audio => AudioObject::new(data, metadata).bind(scope).ok(),
        FrameKind::Video => {
            let frame_type = v8::Local::<v8::String>::try_from(frame_type?).ok()?;
            VideoObject::new(data, metadata, frame_type)
                .bind(scope)
                .ok()
        }
    }
}

/// Both sources are private native dictionaries. Only sequences need a deep
/// copy; scalars and strings are immutable. Own-property reads also prevent an
/// absent field from being supplied by a polluted Object.prototype.
fn copy_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    overrides: Option<v8::Local<'s, v8::Object>>,
    kind: FrameKind,
    private: bool,
) -> v8::Local<'s, v8::Object> {
    let object = if private {
        new_null_prototype_object(scope)
    } else {
        v8::Object::new(scope)
    };
    for name in kind.members() {
        let key = v8str(scope, name);
        let from = overrides
            .filter(|value| value.has_own_property(scope, key.into()) == Some(true))
            .unwrap_or(source);
        if from.has_own_property(scope, key.into()) != Some(true) {
            continue;
        }
        let value = from
            .get(scope, key.into())
            .expect("private metadata member");
        let value = if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
            let values = (0..array.length())
                .map(|index| {
                    array
                        .get_index(scope, index)
                        .expect("private metadata sequence")
                })
                .collect::<Vec<_>>();
            v8::Array::new_with_elements(scope, &values).into()
        } else {
            value
        };
        assert_eq!(
            object.create_data_property(scope, key.into(), value),
            Some(true)
        );
    }
    object
}

/// Use the enclosing serializer for data, retaining graph aliases and transfer
/// identity when a frame and its ArrayBuffer occur in the same object graph.
pub(crate) fn write_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    serializer: &dyn v8::ValueSerializerHelper,
    kind: FrameKind,
) -> Option<bool> {
    let object = target(scope, object);
    if matches!(kind, FrameKind::Video) {
        let value = get_private_value(scope, object, TYPE)?;
        serializer.write_value(scope.get_current_context(), value)?;
    }
    let metadata = frame_metadata(scope, object);
    serializer.write_value(scope.get_current_context(), metadata.into())?;
    let data = frame_data(scope, object);
    serializer.write_value(scope.get_current_context(), data.into())
}

pub(crate) fn read_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deserializer: &dyn v8::ValueDeserializerHelper,
    kind: FrameKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = scope.get_current_context();
    let frame_type = match kind {
        FrameKind::Audio => None,
        FrameKind::Video => Some(deserializer.read_value(context)?),
    };
    let metadata = v8::Local::<v8::Object>::try_from(deserializer.read_value(context)?).ok()?;
    let data = v8::Local::<v8::ArrayBuffer>::try_from(deserializer.read_value(context)?).ok()?;
    let metadata = copy_metadata(scope, metadata, None, kind, true);
    build(scope, data, metadata, frame_type, kind)
}

/// Test-native seeds exercise the value algorithms without exposing a fake
/// author-accessible encoder, transport, or initial-frame constructor.
#[cfg(test)]
pub(crate) fn native_frame_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: FrameKind,
    data: &[u8],
    metadata: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    super::super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, kind.name())
        .expect("test frame intrinsic constructor");
    let metadata = copy_metadata(scope, metadata, None, kind, true);
    let data = array_buffer_from_bytes(scope, data.to_vec()).expect("native seed data");
    let frame_type = Some(v8str(scope, "key").into());
    build(scope, data, metadata, frame_type, kind).expect("native seed frame")
}
