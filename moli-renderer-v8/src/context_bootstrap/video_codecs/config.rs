//! Shared WebIDL conversion and configuration snapshots. Backend support is
//! deliberately separate from dictionary validity: unsupported codecs still
//! return converted, independently owned configuration records.

use super::super::video_color_space::VideoColorSpaceInit;
use crate::{blob::array_buffer_from_bytes, webidl};
use moli_webapi_declare::{WebApiObject, WebApiValue};

pub(super) struct Text(webidl::DomString16);
impl<'s> webidl::WebIdlConverter<'s> for Text {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::convert::<webidl::DomString16>(scope, value, context).map(Self)
    }
}
impl<'s> WebApiValue<'s> for Text {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        v8::String::new_from_two_byte(scope, &self.0.0, v8::NewStringType::Normal).map(Into::into)
    }
}
fn codec_is_empty(codec: &Text) -> bool {
    codec
        .0
        .0
        .iter()
        .all(|unit| matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
}

pub(super) struct Description<'s>(webidl::AllowSharedBufferSource<'s>);
impl<'s> webidl::WebIdlConverter<'s> for Description<'s> {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::convert::<webidl::AllowSharedBufferSource<'s>>(scope, value, context).map(Self)
    }
}
impl Description<'_> {
    fn is_detached(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        match &self.0 {
            webidl::AllowSharedBufferSource::Buffer(buffer) => buffer.was_detached(),
            webidl::AllowSharedBufferSource::Shared(_) => false,
            webidl::AllowSharedBufferSource::View(view) => view
                .buffer(scope)
                .is_none_or(|buffer| buffer.was_detached()),
        }
    }
}
impl<'s> WebApiValue<'s> for Description<'s> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        // Copy only after all dictionary getters and validity checks complete.
        let bytes = self.0.to_vec(scope);
        array_buffer_from_bytes(scope, bytes).map(Into::into)
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "HardwareAcceleration")]
pub(super) enum HardwareAcceleration {
    #[webidl(token = "no-preference")]
    NoPreference,
    #[webidl(token = "prefer-hardware")]
    PreferHardware,
    #[webidl(token = "prefer-software")]
    PreferSoftware,
}
impl<'s> WebApiValue<'s> for HardwareAcceleration {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let label = match self {
            Self::NoPreference => "no-preference",
            Self::PreferHardware => "prefer-hardware",
            Self::PreferSoftware => "prefer-software",
        };
        label.to_v8_value(scope)
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "AlphaOption")]
pub(super) enum AlphaOption {
    #[webidl(token = "discard")]
    Discard,
    #[webidl(token = "keep")]
    Keep,
}
impl<'s> WebApiValue<'s> for AlphaOption {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let label = match self {
            Self::Discard => "discard",
            Self::Keep => "keep",
        };
        label.to_v8_value(scope)
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "BitrateMode")]
pub(super) enum BitrateMode {
    #[webidl(token = "variable")]
    Variable,
    #[webidl(token = "constant")]
    Constant,
    #[webidl(token = "quantizer")]
    Quantizer,
}
impl<'s> WebApiValue<'s> for BitrateMode {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let label = match self {
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Quantizer => "quantizer",
        };
        label.to_v8_value(scope)
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "LatencyMode")]
pub(super) enum LatencyMode {
    #[webidl(token = "quality")]
    Quality,
    #[webidl(token = "realtime")]
    Realtime,
}
impl<'s> WebApiValue<'s> for LatencyMode {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let label = match self {
            Self::Quality => "quality",
            Self::Realtime => "realtime",
        };
        label.to_v8_value(scope)
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "AvcBitstreamFormat")]
pub(super) enum AvcBitstreamFormat {
    #[webidl(token = "avc")]
    Avc,
    #[webidl(token = "annexb")]
    Annexb,
}
impl<'s> WebApiValue<'s> for AvcBitstreamFormat {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let label = match self {
            Self::Avc => "avc",
            Self::Annexb => "annexb",
        };
        label.to_v8_value(scope)
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "HevcBitstreamFormat")]
pub(super) enum HevcBitstreamFormat {
    #[webidl(token = "hevc")]
    Hevc,
    #[webidl(token = "annexb")]
    Annexb,
}
impl<'s> WebApiValue<'s> for HevcBitstreamFormat {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let label = match self {
            Self::Hevc => "hevc",
            Self::Annexb => "annexb",
        };
        label.to_v8_value(scope)
    }
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "AvcConfig")]
#[webapi(plain, enumerable)]
pub(super) struct AvcConfig {
    #[webidl(converter = "enum", default = AvcBitstreamFormat::Avc)]
    #[webapi(data_property)]
    format: AvcBitstreamFormat,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "HevcConfig")]
#[webapi(plain, enumerable)]
pub(super) struct HevcConfig {
    #[webidl(converter = "enum", default = HevcBitstreamFormat::Hevc)]
    #[webapi(data_property)]
    format: HevcBitstreamFormat,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "VideoDecoderConfig")]
#[webapi(plain, enumerable)]
pub(super) struct DecoderConfig<'s> {
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    codec: Text,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webapi(data_property)]
    coded_height: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webapi(data_property)]
    coded_width: Option<u32>,
    #[webidl(dictionary)]
    #[webapi(data_property)]
    color_space: Option<VideoColorSpaceInit>,
    #[webidl(converter = "raw")]
    #[webapi(data_property)]
    description: Option<Description<'s>>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webapi(data_property)]
    display_aspect_height: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webapi(data_property)]
    display_aspect_width: Option<u32>,
    #[webidl(default = false)]
    #[webapi(data_property)]
    flip: bool,
    #[webidl(converter = "enum", default = HardwareAcceleration::NoPreference)]
    #[webapi(data_property)]
    hardware_acceleration: HardwareAcceleration,
    #[webidl(default = false)]
    #[webapi(data_property)]
    optimize_for_latency: bool,
    #[webidl(converter = "double", default = 0.0)]
    #[webapi(data_property)]
    rotation: f64,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "VideoEncoderConfig")]
#[webapi(plain, enumerable)]
pub(super) struct EncoderConfig {
    #[webidl(converter = "enum", default = AlphaOption::Discard)]
    #[webapi(data_property)]
    alpha: AlphaOption,
    #[webidl(dictionary)]
    #[webapi(data_property)]
    avc: Option<AvcConfig>,
    #[webidl(converter = "enforce_range_unsigned_long_long")]
    #[webapi(data_property)]
    bitrate: Option<u64>,
    #[webidl(converter = "enum", default = BitrateMode::Variable)]
    #[webapi(data_property)]
    bitrate_mode: BitrateMode,
    #[webidl(required, converter = "raw")]
    #[webapi(data_property)]
    codec: Text,
    #[webidl(converter = "raw")]
    #[webapi(data_property)]
    content_hint: Option<Text>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webapi(data_property)]
    display_height: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webapi(data_property)]
    display_width: Option<u32>,
    #[webidl(converter = "double")]
    #[webapi(data_property)]
    framerate: Option<f64>,
    #[webidl(converter = "enum", default = HardwareAcceleration::NoPreference)]
    #[webapi(data_property)]
    hardware_acceleration: HardwareAcceleration,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    #[webapi(data_property)]
    height: u32,
    #[webidl(dictionary)]
    #[webapi(data_property)]
    hevc: Option<HevcConfig>,
    #[webidl(converter = "enum", default = LatencyMode::Quality)]
    #[webapi(data_property)]
    latency_mode: LatencyMode,
    #[webidl(converter = "raw")]
    #[webapi(data_property)]
    scalability_mode: Option<Text>,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    #[webapi(data_property)]
    width: u32,
}

impl DecoderConfig<'_> {
    pub(super) fn is_valid(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        !codec_is_empty(&self.codec)
            && valid_pair(self.coded_width, self.coded_height)
            && valid_pair(self.display_aspect_width, self.display_aspect_height)
            && self
                .description
                .as_ref()
                .is_none_or(|description| !description.is_detached(scope))
    }
}
fn valid_pair(first: Option<u32>, second: Option<u32>) -> bool {
    match (first, second) {
        (None, None) => true,
        (Some(a), Some(b)) => a != 0 && b != 0,
        _ => false,
    }
}
impl EncoderConfig {
    pub(super) fn is_valid(&self) -> bool {
        !codec_is_empty(&self.codec)
            && self.width != 0
            && self.height != 0
            && self.display_width != Some(0)
            && self.display_height != Some(0)
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoEncoder quantizer options")]
struct QuantizerOptions {
    #[webidl(name = "quantizer", nullable, converter = "unsigned_short")]
    _quantizer: Option<u16>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoEncoderEncodeOptions")]
pub(super) struct EncodeOptions {
    #[webidl(name = "av1", dictionary)]
    _av1: Option<QuantizerOptions>,
    #[webidl(name = "avc", dictionary)]
    _avc: Option<QuantizerOptions>,
    #[webidl(name = "hevc", dictionary)]
    _hevc: Option<QuantizerOptions>,
    #[webidl(name = "keyFrame", default = false)]
    _key_frame: bool,
    #[webidl(name = "vp9", dictionary)]
    _vp9: Option<QuantizerOptions>,
}
