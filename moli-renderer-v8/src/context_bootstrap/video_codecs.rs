use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use webidl::{SnapshotPrototype, WebIdlDictionarySerialize};

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "HardwareAcceleration")]
enum HardwareAcceleration {
    #[webidl(token = "no-preference")]
    NoPreference,
    #[webidl(token = "prefer-hardware")]
    PreferHardware,
    #[webidl(token = "prefer-software")]
    PreferSoftware,
}
#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "AlphaOption")]
enum AlphaOption {
    #[webidl(token = "discard")]
    Discard,
    #[webidl(token = "keep")]
    Keep,
}
#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "LatencyMode")]
enum LatencyMode {
    #[webidl(token = "quality")]
    Quality,
    #[webidl(token = "realtime")]
    Realtime,
}
#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "VideoEncoderBitrateMode")]
enum VideoEncoderBitrateMode {
    #[webidl(token = "variable")]
    Variable,
    #[webidl(token = "constant")]
    Constant,
    #[webidl(token = "quantizer")]
    Quantizer,
}
#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "VideoColorPrimaries")]
enum VideoColorPrimaries {
    #[webidl(token = "bt709")]
    Bt709,
    #[webidl(token = "bt470bg")]
    Bt470bg,
    #[webidl(token = "smpte170m")]
    Smpte170m,
    #[webidl(token = "bt2020")]
    Bt2020,
    #[webidl(token = "smpte432")]
    Smpte432,
}
#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "VideoTransferCharacteristics")]
enum VideoTransferCharacteristics {
    #[webidl(token = "bt709")]
    Bt709,
    #[webidl(token = "smpte170m")]
    Smpte170m,
    #[webidl(token = "iec61966-2-1")]
    Srgb,
    #[webidl(token = "linear")]
    Linear,
    #[webidl(token = "pq")]
    Pq,
    #[webidl(token = "hlg")]
    Hlg,
}
#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "VideoMatrixCoefficients")]
enum VideoMatrixCoefficients {
    #[webidl(token = "rgb")]
    Rgb,
    #[webidl(token = "bt709")]
    Bt709,
    #[webidl(token = "bt470bg")]
    Bt470bg,
    #[webidl(token = "smpte170m")]
    Smpte170m,
    #[webidl(token = "bt2020-ncl")]
    Bt2020Ncl,
}
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoColorSpaceInit", snapshot)]
struct VideoColorSpaceInit {
    #[webidl(nullable)]
    #[webidl(snapshot_order = 3)]
    full_range: Option<bool>,
    #[webidl(nullable, converter = "enum")]
    #[webidl(snapshot_order = 2)]
    matrix: Option<VideoMatrixCoefficients>,
    #[webidl(nullable, converter = "enum")]
    #[webidl(snapshot_order = 0)]
    primaries: Option<VideoColorPrimaries>,
    #[webidl(nullable, converter = "enum")]
    #[webidl(snapshot_order = 1)]
    transfer: Option<VideoTransferCharacteristics>,
}

struct Description<'s>(v8::Local<'s, v8::Value>);

impl<'s> webidl::WebIdlConverter<'s> for Description<'s> {
    type Options = ();
    fn convert(
        _scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if value.is_array_buffer()
            || value.is_shared_array_buffer()
            || value.is_typed_array()
            || value.is_data_view()
        {
            Ok(Self(value))
        } else {
            Err(webidl::WebIdlError::cannot_convert(
                context,
                "AllowSharedBufferSource",
            ))
        }
    }
}

impl<'s> Description<'s> {
    fn is_detached(&self, scope: &mut v8::PinScope<'s, '_>) -> bool {
        if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(self.0) {
            return buffer.was_detached();
        }
        if self.0.is_shared_array_buffer() {
            return false;
        }
        v8::Local::<v8::ArrayBufferView>::try_from(self.0)
            .ok()
            .is_none_or(|view| {
                if view
                    .get_backing_store()
                    .is_some_and(|store| store.is_shared())
                {
                    return false;
                }
                view.buffer(scope)
                    .is_none_or(|buffer| buffer.was_detached())
            })
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoDecoderConfig", snapshot)]
struct VideoDecoderConfig<'s> {
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 1)]
    coded_height: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 2)]
    coded_width: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(required, converter = "raw")]
    #[webidl(snapshot_order = 0)]
    codec: webidl::DomString16,
    #[webidl(dictionary)]
    #[webidl(snapshot_order = 10)]
    color_space: Option<VideoColorSpaceInit>,
    #[webidl(converter = "raw", snapshot_with = description_snapshot, snapshot_kind = webidl::SnapshotKind::BufferSource)]
    #[webidl(snapshot_order = 9)]
    description: Option<Description<'s>>,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 3)]
    display_aspect_height: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 4)]
    display_aspect_width: Option<webidl::EnforceRangeUnsignedLong>,
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
struct VideoEncoderConfig {
    #[webidl(converter = "enum", default = AlphaOption::Discard)]
    #[webidl(snapshot_order = 10)]
    alpha: AlphaOption,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 5)]
    bitrate: Option<webidl::EnforceRangeUnsignedLongLong>,
    #[webidl(converter = "enum", default = VideoEncoderBitrateMode::Variable)]
    #[webidl(snapshot_order = 11)]
    bitrate_mode: VideoEncoderBitrateMode,
    #[webidl(required, converter = "raw")]
    #[webidl(snapshot_order = 0)]
    codec: webidl::DomString16,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 8)]
    content_hint: Option<webidl::DomString16>,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 3)]
    display_height: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 4)]
    display_width: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(converter = "double")]
    #[webidl(snapshot_order = 6)]
    framerate: Option<f64>,
    #[webidl(converter = "enum", default = HardwareAcceleration::NoPreference)]
    #[webidl(snapshot_order = 9)]
    hardware_acceleration: HardwareAcceleration,
    #[webidl(required, converter = "raw")]
    #[webidl(snapshot_order = 2)]
    height: webidl::EnforceRangeUnsignedLong,
    #[webidl(converter = "enum", default = LatencyMode::Quality)]
    #[webidl(snapshot_order = 12)]
    latency_mode: LatencyMode,
    #[webidl(converter = "raw")]
    #[webidl(snapshot_order = 7)]
    scalability_mode: Option<webidl::DomString16>,
    #[webidl(required, converter = "raw")]
    #[webidl(snapshot_order = 1)]
    width: webidl::EnforceRangeUnsignedLong,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoDecoder.isConfigSupported")]
struct DecoderSupportArgs<'s> {
    #[webidl(required, dictionary)]
    config: VideoDecoderConfig<'s>,
}
#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VideoEncoder.isConfigSupported")]
struct EncoderSupportArgs {
    #[webidl(required, dictionary)]
    config: VideoEncoderConfig,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VideoDecoder, enumerable)]
struct VideoDecoderStatics {
    #[webapi(static_method, length = 1, returns_promise, callback = decoder_is_config_supported)]
    is_config_supported: (),
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VideoEncoder, enumerable)]
struct VideoEncoderStatics {
    #[webapi(static_method, length = 1, returns_promise, callback = encoder_is_config_supported)]
    is_config_supported: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct VideoSupport<'s> {
    supported: bool,
    config: v8::Local<'s, v8::Object>,
}

pub(super) fn install_video_codec_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    match name {
        "VideoDecoder" => VideoDecoderStatics::initialize_template(scope, template),
        "VideoEncoder" => VideoEncoderStatics::initialize_template(scope, template),
        _ => {}
    }
}

fn nonempty_codec(codec: &webidl::DomString16) -> bool {
    codec
        .0
        .iter()
        .any(|unit| !matches!(*unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
}
fn valid_dimensions(
    a: Option<webidl::EnforceRangeUnsignedLong>,
    b: Option<webidl::EnforceRangeUnsignedLong>,
) -> bool {
    a.is_some() == b.is_some()
        && a.is_none_or(|value| value.0 != 0)
        && b.is_none_or(|value| value.0 != 0)
}
fn description_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &Description<'s>,
    _: SnapshotPrototype,
) -> Option<v8::Local<'s, v8::Value>> {
    webidl::buffer_snapshot(scope, value.0)
}

fn resolve_unsupported<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    config: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let support = VideoSupport::new(false, config)
        .bind(scope)
        .expect("video support result");
    let resolver = v8::PromiseResolver::new(scope).expect("video support promise");
    let _ = resolver.resolve(scope, support.into());
    rv.set(resolver.get_promise(scope).into());
}

fn decoder_is_config_supported<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DecoderSupportArgs>(scope, &args) else {
        return;
    };
    let config = parsed.config;
    if !nonempty_codec(&config.codec)
        || !valid_dimensions(config.coded_width, config.coded_height)
        || !valid_dimensions(config.display_aspect_width, config.display_aspect_height)
        || config
            .description
            .as_ref()
            .is_some_and(|description| description.is_detached(scope))
    {
        crate::context_bootstrap::throw_type_error(scope, "Invalid VideoDecoderConfig.");
        return;
    }
    let Some(clone) = config.to_js_snapshot(scope, SnapshotPrototype::Ordinary) else {
        return;
    };
    resolve_unsupported(scope, clone, rv);
}

fn encoder_is_config_supported<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<EncoderSupportArgs>(scope, &args) else {
        return;
    };
    let config = parsed.config;
    if !nonempty_codec(&config.codec)
        || config.width.0 == 0
        || config.height.0 == 0
        || config.display_width.is_some_and(|value| value.0 == 0)
        || config.display_height.is_some_and(|value| value.0 == 0)
    {
        crate::context_bootstrap::throw_type_error(scope, "Invalid VideoEncoderConfig.");
        return;
    }
    let Some(clone) = config.to_js_snapshot(scope, SnapshotPrototype::Ordinary) else {
        return;
    };
    resolve_unsupported(scope, clone, rv);
}
