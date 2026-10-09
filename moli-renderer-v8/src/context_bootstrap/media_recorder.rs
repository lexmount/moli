//! MediaRecorder construction and inactive state, without an encoding backend.
//! A valid configuration is separate from support: no nonempty MIME type or
//! recording session is supported, and no encoded data or recording events are
//! fabricated. Receiver checks and input conversion use shared WebIDL bindings.

use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiObject, WebApiValue, initialize_web_api_constructor_receiver,
};

use super::{media_streams, throw_dom_exception_value};
use crate::{
    util::{get_private_value, throw_type_error, v8str},
    web_api_interfaces, webidl,
};

const STREAM: &str = "__moliRecorderStream";
const MIME_TYPE: &str = "__moliRecorderMimeType";
const STATE: &str = "__moliRecorderState";
const VIDEO_RATE: &str = "__moliRecorderVideoBitsPerSecond";
const AUDIO_RATE: &str = "__moliRecorderAudioBitsPerSecond";
const AUDIO_MODE: &str = "__moliRecorderAudioBitrateMode";
const KEY_FRAME_COUNT: &str = "__moliRecorderVideoKeyFrameIntervalCount";
const KEY_FRAME_DURATION: &str = "__moliRecorderVideoKeyFrameIntervalDuration";

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "BitrateMode")]
enum AudioBitrateMode {
    #[default]
    #[webidl(token = "variable")]
    Variable,
    #[webidl(token = "constant")]
    Constant,
}

impl<'s> WebApiValue<'s> for AudioBitrateMode {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::Variable => "variable",
            Self::Constant => "constant",
        }
        .to_v8_value(scope)
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaRecorderOptions")]
struct RecorderOptions {
    #[webidl(converter = "enum", default = AudioBitrateMode::Variable)]
    audio_bitrate_mode: AudioBitrateMode,
    audio_bits_per_second: Option<u32>,
    bits_per_second: Option<u32>,
    #[webidl(converter = "raw", default = webidl::DomString16(Vec::new()))]
    mime_type: webidl::DomString16,
    video_bits_per_second: Option<u32>,
    video_key_frame_interval_count: Option<u32>,
    #[webidl(converter = "double")]
    video_key_frame_interval_duration: Option<f64>,
}

impl Default for RecorderOptions {
    fn default() -> Self {
        Self {
            audio_bitrate_mode: AudioBitrateMode::Variable,
            audio_bits_per_second: None,
            bits_per_second: None,
            mime_type: webidl::DomString16(Vec::new()),
            video_bits_per_second: None,
            video_key_frame_interval_count: None,
            video_key_frame_interval_duration: None,
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaRecorder")]
struct ConstructorArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::MediaStream)]
    stream: v8::Local<'s, v8::Object>,
    #[webidl(dictionary, default = RecorderOptions::default())]
    options: RecorderOptions,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaRecorder)]
struct RecorderSlots<'s> {
    #[webapi(slot = STREAM)]
    stream: v8::Local<'s, v8::Object>,
    #[webapi(slot = MIME_TYPE, value = "")]
    mime_type: (),
    #[webapi(slot = STATE, value = "inactive")]
    state: (),
    #[webapi(slot = VIDEO_RATE)]
    video_bits_per_second: u32,
    #[webapi(slot = AUDIO_RATE)]
    audio_bits_per_second: u32,
    // The frontend falls back to variable mode when constant mode is unavailable.
    #[webapi(slot = AUDIO_MODE, value = "variable")]
    audio_bitrate_mode: (),
    #[webapi(slot = "__moliRecorderRequestedAudioBitrateMode")]
    requested_audio_bitrate_mode: AudioBitrateMode,
    #[webapi(slot = KEY_FRAME_COUNT)]
    video_key_frame_interval_count: Option<u32>,
    #[webapi(slot = KEY_FRAME_DURATION)]
    video_key_frame_interval_duration: Option<f64>,
    #[webapi(slot = "__moliMediaHandlerOnstart", init = "null")]
    onstart: (),
    #[webapi(slot = "__moliMediaHandlerOnstop", init = "null")]
    onstop: (),
    #[webapi(slot = "__moliMediaHandlerOndataavailable", init = "null")]
    ondataavailable: (),
    #[webapi(slot = "__moliMediaHandlerOnpause", init = "null")]
    onpause: (),
    #[webapi(slot = "__moliMediaHandlerOnresume", init = "null")]
    onresume: (),
    #[webapi(slot = "__moliMediaHandlerOnerror", init = "null")]
    onerror: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaRecorder, enumerable, receiver)]
struct RecorderPrototype {
    #[webapi(accessor_property, getter = media_streams::slot_getter, data = v8str(scope, STREAM))]
    stream: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, data = v8str(scope, MIME_TYPE))]
    mime_type: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, data = v8str(scope, STATE))]
    state: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, data = v8str(scope, VIDEO_RATE))]
    video_bits_per_second: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, data = v8str(scope, AUDIO_RATE))]
    audio_bits_per_second: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, data = v8str(scope, AUDIO_MODE))]
    audio_bitrate_mode: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, setter = media_streams::handler_setter, data = v8str(scope, "__moliMediaHandlerOnstart"))]
    onstart: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, setter = media_streams::handler_setter, data = v8str(scope, "__moliMediaHandlerOnstop"))]
    onstop: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, setter = media_streams::handler_setter, data = v8str(scope, "__moliMediaHandlerOndataavailable"))]
    ondataavailable: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, setter = media_streams::handler_setter, data = v8str(scope, "__moliMediaHandlerOnpause"))]
    onpause: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, setter = media_streams::handler_setter, data = v8str(scope, "__moliMediaHandlerOnresume"))]
    onresume: (),
    #[webapi(accessor_property, getter = media_streams::slot_getter, setter = media_streams::handler_setter, data = v8str(scope, "__moliMediaHandlerOnerror"))]
    onerror: (),
    #[webapi(method, length = 0, callback = start)]
    start: (),
    #[webapi(method, length = 0, callback = stop)]
    stop: (),
    #[webapi(method, length = 0, callback = require_recording)]
    pause: (),
    #[webapi(method, length = 0, callback = require_recording)]
    resume: (),
    #[webapi(method, length = 0, callback = require_recording)]
    request_data: (),
    #[webapi(static_method, length = 1, callback = is_type_supported)]
    is_type_supported: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name == "MediaRecorder" {
        RecorderPrototype::initialize_template(scope, template);
        RecorderPrototype::initialize_prototype_template(scope, template.prototype_template(scope));
    }
}

pub(super) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "MediaRecorder requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "MediaRecorder") {
        return;
    }
    if !parsed.options.mime_type.0.is_empty() {
        throw_dom_exception_value(
            scope,
            "No recording MIME type is supported.",
            "NotSupportedError",
        );
        return;
    }
    let (audio, video) = if let Some(total) = parsed.options.bits_per_second {
        // Bitrate allocation is UA-defined. A total target overrides per-kind
        // targets; use a deterministic 5/95 split without unsigned overflow.
        let audio = total / 20;
        (audio, total - audio)
    } else {
        (
            parsed.options.audio_bits_per_second.unwrap_or(128_000),
            parsed.options.video_bits_per_second.unwrap_or(2_500_000),
        )
    };
    RecorderSlots::new(
        parsed.stream,
        video,
        audio,
        parsed.options.audio_bitrate_mode,
        parsed.options.video_key_frame_interval_count,
        parsed.options.video_key_frame_interval_duration,
    )
    .initialize(scope, args.this())
    .expect("MediaRecorder slots");
    media_streams::initialize_event_target(scope, args.this());
    rv.set(args.this().into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaRecorder.isTypeSupported")]
struct SupportArgs {
    #[webidl(required, converter = "raw")]
    mime_type: webidl::DomString16,
}

fn is_type_supported<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SupportArgs>(scope, &args) else {
        return;
    };
    // An empty type lets the UA choose later; it does not promise an encoder.
    rv.set(v8::Boolean::new(scope, parsed.mime_type.0.is_empty()).into());
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaRecorder.start")]
struct StartArgs {
    #[webidl(name = "timeslice")]
    _timeslice: Option<u32>,
}

fn is_inactive<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, object, STATE)
        .expect("MediaRecorder state")
        .strict_equals(v8str(scope, "inactive").into())
}

fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(_parsed) = webidl::parse_args::<StartArgs>(scope, &args) else {
        return;
    };
    let object = media_streams::target(scope, args.this());
    if !is_inactive(scope, object) {
        throw_dom_exception_value(
            scope,
            "MediaRecorder is already recording.",
            "InvalidStateError",
        );
        return;
    }
    let stream = v8::Local::<v8::Object>::try_from(
        get_private_value(scope, object, STREAM).expect("MediaRecorder stream"),
    )
    .expect("native MediaStream");
    if !media_streams::is_active(scope, stream) {
        throw_dom_exception_value(scope, "The stream is inactive.", "NotSupportedError");
        return;
    }
    if get_private_value(scope, object, KEY_FRAME_COUNT).is_some_and(|value| !value.is_undefined())
        && get_private_value(scope, object, KEY_FRAME_DURATION)
            .is_some_and(|value| !value.is_undefined())
    {
        throw_dom_exception_value(
            scope,
            "Only one video keyframe interval may be specified.",
            "NotSupportedError",
        );
        return;
    }
    throw_dom_exception_value(
        scope,
        "No recording encoder is available.",
        "NotSupportedError",
    );
}

fn stop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = media_streams::target(scope, args.this());
    if !is_inactive(scope, object) {
        throw_dom_exception_value(
            scope,
            "No recording encoder is available.",
            "NotSupportedError",
        );
    }
}

fn require_recording<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = media_streams::target(scope, args.this());
    if is_inactive(scope, object) {
        throw_dom_exception_value(scope, "MediaRecorder is inactive.", "InvalidStateError");
    } else {
        throw_dom_exception_value(
            scope,
            "No recording encoder is available.",
            "NotSupportedError",
        );
    }
}
