use crate::{
    util::{v8_string, v8_string_from_utf16_units},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, WebApiValue};

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
impl HardwareAcceleration {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoPreference => "no-preference",
            Self::PreferHardware => "prefer-hardware",
            Self::PreferSoftware => "prefer-software",
        }
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "AlphaOption")]
enum AlphaOption {
    #[webidl(token = "discard")]
    Discard,
    #[webidl(token = "keep")]
    Keep,
}
impl AlphaOption {
    fn as_str(self) -> &'static str {
        match self {
            Self::Discard => "discard",
            Self::Keep => "keep",
        }
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "LatencyMode")]
enum LatencyMode {
    #[webidl(token = "quality")]
    Quality,
    #[webidl(token = "realtime")]
    Realtime,
}
impl LatencyMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Quality => "quality",
            Self::Realtime => "realtime",
        }
    }
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
impl VideoEncoderBitrateMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Quantizer => "quantizer",
        }
    }
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
impl VideoColorPrimaries {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bt709 => "bt709",
            Self::Bt470bg => "bt470bg",
            Self::Smpte170m => "smpte170m",
            Self::Bt2020 => "bt2020",
            Self::Smpte432 => "smpte432",
        }
    }
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
impl VideoTransferCharacteristics {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bt709 => "bt709",
            Self::Smpte170m => "smpte170m",
            Self::Srgb => "iec61966-2-1",
            Self::Linear => "linear",
            Self::Pq => "pq",
            Self::Hlg => "hlg",
        }
    }
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
impl VideoMatrixCoefficients {
    fn as_str(self) -> &'static str {
        match self {
            Self::Rgb => "rgb",
            Self::Bt709 => "bt709",
            Self::Bt470bg => "bt470bg",
            Self::Smpte170m => "smpte170m",
            Self::Bt2020Ncl => "bt2020-ncl",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoColorSpaceInit")]
struct VideoColorSpaceInit {
    #[webidl(nullable)]
    full_range: Option<bool>,
    #[webidl(nullable, converter = "enum")]
    matrix: Option<VideoMatrixCoefficients>,
    #[webidl(nullable, converter = "enum")]
    primaries: Option<VideoColorPrimaries>,
    #[webidl(nullable, converter = "enum")]
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
#[webidl(prefix = "VideoDecoderConfig")]
struct VideoDecoderConfig<'s> {
    #[webidl(converter = "raw")]
    coded_height: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(converter = "raw")]
    coded_width: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(required, converter = "raw")]
    codec: webidl::DomString16,
    #[webidl(dictionary)]
    color_space: Option<VideoColorSpaceInit>,
    #[webidl(converter = "raw")]
    description: Option<Description<'s>>,
    #[webidl(converter = "raw")]
    display_aspect_height: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(converter = "raw")]
    display_aspect_width: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(default = false)]
    flip: bool,
    #[webidl(converter = "enum", default = HardwareAcceleration::NoPreference)]
    hardware_acceleration: HardwareAcceleration,
    #[webidl(default = false)]
    optimize_for_latency: bool,
    #[webidl(converter = "double", default = 0.0)]
    rotation: f64,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoEncoderConfig")]
struct VideoEncoderConfig {
    #[webidl(converter = "enum", default = AlphaOption::Discard)]
    alpha: AlphaOption,
    #[webidl(converter = "raw")]
    bitrate: Option<webidl::EnforceRangeUnsignedLongLong>,
    #[webidl(converter = "enum", default = VideoEncoderBitrateMode::Variable)]
    bitrate_mode: VideoEncoderBitrateMode,
    #[webidl(required, converter = "raw")]
    codec: webidl::DomString16,
    #[webidl(converter = "raw")]
    content_hint: Option<webidl::DomString16>,
    #[webidl(converter = "raw")]
    display_height: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(converter = "raw")]
    display_width: Option<webidl::EnforceRangeUnsignedLong>,
    #[webidl(converter = "double")]
    framerate: Option<f64>,
    #[webidl(converter = "enum", default = HardwareAcceleration::NoPreference)]
    hardware_acceleration: HardwareAcceleration,
    #[webidl(required, converter = "raw")]
    height: webidl::EnforceRangeUnsignedLong,
    #[webidl(converter = "enum", default = LatencyMode::Quality)]
    latency_mode: LatencyMode,
    #[webidl(converter = "raw")]
    scalability_mode: Option<webidl::DomString16>,
    #[webidl(required, converter = "raw")]
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
fn property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: impl WebApiValue<'s>,
) {
    let key = v8_string(scope, name).expect("video config key");
    let value = value.to_v8_value(scope).expect("video config value");
    assert_eq!(
        object.create_data_property(scope, key.into(), value),
        Some(true)
    );
}
fn string_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &webidl::DomString16,
) {
    let value = v8_string_from_utf16_units(scope, &value.0).expect("video config string");
    property(scope, object, name, value);
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
    let clone = v8::Object::new(scope);
    string_property(scope, clone, "codec", &config.codec);
    for (name, value) in [
        ("codedHeight", config.coded_height),
        ("codedWidth", config.coded_width),
        ("displayAspectHeight", config.display_aspect_height),
        ("displayAspectWidth", config.display_aspect_width),
    ] {
        if let Some(value) = value {
            property(scope, clone, name, value.0);
        }
    }
    property(
        scope,
        clone,
        "hardwareAcceleration",
        config.hardware_acceleration.as_str(),
    );
    property(
        scope,
        clone,
        "optimizeForLatency",
        config.optimize_for_latency,
    );
    property(scope, clone, "rotation", config.rotation);
    property(scope, clone, "flip", config.flip);
    if let Some(description) = config.description {
        let context = webidl::Context::member("VideoDecoderConfig", "description");
        let bytes = match webidl::convert::<webidl::BufferSource>(scope, description.0, context) {
            Ok(bytes) => bytes.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        };
        let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
        let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
        property(scope, clone, "description", buffer);
    }
    if let Some(color_space) = config.color_space {
        let color = v8::Object::new(scope);
        for (name, value) in [
            (
                "primaries",
                color_space.primaries.map(VideoColorPrimaries::as_str),
            ),
            (
                "transfer",
                color_space
                    .transfer
                    .map(VideoTransferCharacteristics::as_str),
            ),
            (
                "matrix",
                color_space.matrix.map(VideoMatrixCoefficients::as_str),
            ),
        ] {
            let value: v8::Local<v8::Value> = match value {
                Some(value) => v8_string(scope, value).expect("video color enum").into(),
                None => v8::null(scope).into(),
            };
            property(scope, color, name, value);
        }
        let full_range: v8::Local<v8::Value> = match color_space.full_range {
            Some(value) => v8::Boolean::new(scope, value).into(),
            None => v8::null(scope).into(),
        };
        property(scope, color, "fullRange", full_range);
        property(scope, clone, "colorSpace", color);
    }
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
    let clone = v8::Object::new(scope);
    string_property(scope, clone, "codec", &config.codec);
    property(scope, clone, "width", config.width.0);
    property(scope, clone, "height", config.height.0);
    for (name, value) in [
        ("displayHeight", config.display_height),
        ("displayWidth", config.display_width),
    ] {
        if let Some(value) = value {
            property(scope, clone, name, value.0);
        }
    }
    if let Some(value) = config.bitrate {
        property(scope, clone, "bitrate", value.0);
    }
    if let Some(value) = config.framerate {
        property(scope, clone, "framerate", value);
    }
    if let Some(value) = config.scalability_mode {
        string_property(scope, clone, "scalabilityMode", &value);
    }
    if let Some(value) = config.content_hint {
        string_property(scope, clone, "contentHint", &value);
    }
    property(
        scope,
        clone,
        "hardwareAcceleration",
        config.hardware_acceleration.as_str(),
    );
    property(scope, clone, "alpha", config.alpha.as_str());
    property(scope, clone, "bitrateMode", config.bitrate_mode.as_str());
    property(scope, clone, "latencyMode", config.latency_mode.as_str());
    resolve_unsupported(scope, clone, rv);
}
