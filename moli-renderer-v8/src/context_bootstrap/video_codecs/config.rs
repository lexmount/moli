//! Shared WebIDL conversion and configuration snapshots. Backend support is
//! deliberately separate from dictionary validity: unsupported codecs still
//! return converted, independently owned configuration records.

use super::super::video_color_space::VideoColorSpaceInit;
use crate::{blob::array_buffer_from_bytes, webidl};
use webidl::{SnapshotPrototype, WebIdlDictionarySerialize};

fn codec_is_empty(codec: &webidl::DomString16) -> bool {
    codec.0.iter().all(|unit| matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
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
            webidl::AllowSharedBufferSource::View(view) => {
                if view.get_backing_store().is_some_and(|store| store.is_shared()) {
                    false
                } else {
                    view.buffer(scope).is_none_or(|buffer| buffer.was_detached())
                }
            },
        }
    }
}
fn description_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>, value: &Description<'s>, _: SnapshotPrototype,
) -> Option<v8::Local<'s, v8::Value>> {
        // Copy only after all dictionary getters and validity checks complete.
        let bytes = value.0.to_vec(scope);
        array_buffer_from_bytes(scope, bytes).map(Into::into)
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

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "AlphaOption")]
pub(super) enum AlphaOption {
    #[webidl(token = "discard")]
    Discard,
    #[webidl(token = "keep")]
    Keep,
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

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "LatencyMode")]
pub(super) enum LatencyMode {
    #[webidl(token = "quality")]
    Quality,
    #[webidl(token = "realtime")]
    Realtime,
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "AvcBitstreamFormat")]
pub(super) enum AvcBitstreamFormat {
    #[webidl(token = "avc")]
    Avc,
    #[webidl(token = "annexb")]
    Annexb,
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "HevcBitstreamFormat")]
pub(super) enum HevcBitstreamFormat {
    #[webidl(token = "hevc")]
    Hevc,
    #[webidl(token = "annexb")]
    Annexb,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AvcConfig", snapshot)]
pub(super) struct AvcConfig {
    #[webidl(converter = "enum", default = AvcBitstreamFormat::Avc)]
    format: AvcBitstreamFormat,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "HevcConfig", snapshot)]
pub(super) struct HevcConfig {
    #[webidl(converter = "enum", default = HevcBitstreamFormat::Hevc)]
    format: HevcBitstreamFormat,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoDecoderConfig", snapshot)]
pub(super) struct DecoderConfig<'s> {
    #[webidl(required, converter = "raw")]
    #[webidl(snapshot_order = 0)]
    codec: webidl::DomString16,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webidl(snapshot_order = 1)]
    coded_height: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webidl(snapshot_order = 2)]
    coded_width: Option<u32>,
    #[webidl(dictionary)]
    #[webidl(snapshot_order = 10)]
    color_space: Option<VideoColorSpaceInit>,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_with = description_snapshot, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    #[webidl(snapshot_order = 9)]
    description: Option<Description<'s>>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webidl(snapshot_order = 3)]
    display_aspect_height: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webidl(snapshot_order = 4)]
    display_aspect_width: Option<u32>,
    #[webidl(default = false)]
    #[webidl(snapshot_order = 8)]
    flip: bool,
    #[webidl(converter = "enum", default = HardwareAcceleration::NoPreference)]
    #[webidl(snapshot_order = 5)]
    hardware_acceleration: HardwareAcceleration,
    #[webidl(default = false)]
    #[webidl(snapshot_order = 6)]
    optimize_for_latency: bool,
    #[webidl(converter = "double", default = 0.0)]
    #[webidl(snapshot_order = 7)]
    rotation: f64,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoEncoderConfig", snapshot)]
pub(super) struct EncoderConfig {
    #[webidl(converter = "enum", default = AlphaOption::Discard)]
    #[webidl(snapshot_order = 10)]
    alpha: AlphaOption,
    #[webidl(dictionary)]
    #[webidl(snapshot_order = 13)]
    avc: Option<AvcConfig>,
    #[webidl(converter = "enforce_range_unsigned_long_long")]
    #[webidl(snapshot_order = 5)]
    bitrate: Option<u64>,
    #[webidl(converter = "enum", default = BitrateMode::Variable)]
    #[webidl(snapshot_order = 11)]
    bitrate_mode: BitrateMode,
    #[webidl(required, converter = "raw")]
    #[webidl(snapshot_order = 0)]
    codec: webidl::DomString16,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 8)]
    content_hint: Option<webidl::DomString16>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webidl(snapshot_order = 3)]
    display_height: Option<u32>,
    #[webidl(converter = "enforce_range_unsigned_long")]
    #[webidl(snapshot_order = 4)]
    display_width: Option<u32>,
    #[webidl(converter = "double")]
    #[webidl(snapshot_order = 6)]
    framerate: Option<f64>,
    #[webidl(converter = "enum", default = HardwareAcceleration::NoPreference)]
    #[webidl(snapshot_order = 9)]
    hardware_acceleration: HardwareAcceleration,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    #[webidl(snapshot_order = 2)]
    height: u32,
    #[webidl(dictionary)]
    #[webidl(snapshot_order = 14)]
    hevc: Option<HevcConfig>,
    #[webidl(converter = "enum", default = LatencyMode::Quality)]
    #[webidl(snapshot_order = 12)]
    latency_mode: LatencyMode,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 7)]
    scalability_mode: Option<webidl::DomString16>,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    #[webidl(snapshot_order = 1)]
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
