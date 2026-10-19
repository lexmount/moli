use super::super::*;
use super::encrypted_media::MediaKeysRequirement;
use crate::web_api_interfaces;
use crate::{
    util::get_private_value,
    webidl::{self, SnapshotPrototype, WebIdlDictionarySerialize},
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const MEDIA_CAPABILITIES_SECURE_CONTEXT_SLOT: &str = "__moliMediaCapabilitiesSecureContext";
const MEDIA_CAPABILITIES_WORKER_SLOT: &str = "__moliMediaCapabilitiesWorker";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaCapabilities)]
struct MediaCapabilitiesObjectDeclaration {
    #[webapi(slot = MEDIA_CAPABILITIES_SECURE_CONTEXT_SLOT)]
    secure_context: bool,

    #[webapi(slot = MEDIA_CAPABILITIES_WORKER_SLOT)]
    worker: bool,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaCapabilities, enumerable, receiver)]
struct MediaCapabilitiesPrototypeDeclaration {
    #[webapi(method, length = 1, callback = media_capabilities_decoding_info_callback, returns_promise)]
    decoding_info: (),

    #[webapi(method, length = 1, callback = media_capabilities_encoding_info_callback, returns_promise)]
    encoding_info: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaCapabilitiesInfo", snapshot)]
struct MediaCapabilitiesInfo {
    #[webidl(required)]
    power_efficient: bool,
    #[webidl(required)]
    smooth: bool,
    #[webidl(required)]
    supported: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaCapabilitiesDecodingInfo", snapshot)]
struct MediaCapabilitiesDecodingInfo<'s> {
    #[webidl(inherit)]
    info: MediaCapabilitiesInfo,
    #[webidl(required, dictionary)]
    configuration: MediaDecodingConfiguration,
    #[webidl(required, nullable, interface = web_api_interfaces::MediaKeySystemAccess)]
    key_system_access: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaCapabilitiesEncodingInfo", snapshot)]
struct MediaCapabilitiesEncodingInfo {
    #[webidl(inherit)]
    info: MediaCapabilitiesInfo,
    #[webidl(required, dictionary)]
    configuration: MediaEncodingConfiguration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "MediaDecodingType", rename_all = "kebab-case")]
enum MediaDecodingType {
    File,
    MediaSource,
    Webrtc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "MediaEncodingType")]
enum MediaEncodingType {
    Record,
    Webrtc,
}

#[derive(Clone, Copy)]
enum MediaConfigurationType {
    Decoding(MediaDecodingType),
    Encoding(MediaEncodingType),
}

impl MediaConfigurationType {
    fn is_webrtc(self) -> bool {
        matches!(
            self,
            Self::Decoding(MediaDecodingType::Webrtc) | Self::Encoding(MediaEncodingType::Webrtc)
        )
    }
}

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "HdrMetadataType")]
enum HdrMetadataType {
    #[webidl(token = "smpteSt2086")]
    SmpteSt2086,
    #[webidl(token = "smpteSt2094-10")]
    SmpteSt2094_10,
    #[webidl(token = "smpteSt2094-40")]
    SmpteSt2094_40,
}

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "ColorGamut")]
enum ColorGamut {
    Srgb,
    P3,
    Rec2020,
}

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "TransferFunction")]
enum TransferFunction {
    Srgb,
    Pq,
    Hlg,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AudioConfiguration", snapshot)]
struct AudioConfiguration {
    #[webidl(required)]
    content_type: String,
    channels: Option<String>,
    bitrate: Option<u64>,
    samplerate: Option<u32>,
    spatial_rendering: Option<bool>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoConfiguration", snapshot)]
struct VideoConfiguration {
    #[webidl(required)]
    content_type: String,
    #[webidl(required)]
    width: u32,
    #[webidl(required)]
    height: u32,
    #[webidl(required)]
    bitrate: u64,
    #[webidl(required, converter = "double")]
    framerate: f64,
    has_alpha_channel: Option<bool>,
    #[webidl(converter = "enum")]
    hdr_metadata_type: Option<HdrMetadataType>,
    #[webidl(converter = "enum")]
    color_gamut: Option<ColorGamut>,
    #[webidl(converter = "enum")]
    transfer_function: Option<TransferFunction>,
    scalability_mode: Option<String>,
    spatial_scalability: Option<bool>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "KeySystemTrackConfiguration", snapshot)]
struct KeySystemTrackConfiguration {
    #[webidl(converter = "raw", default = webidl::DomString16(Vec::new()))]
    robustness: webidl::DomString16,
    #[webidl(nullable, converter = "raw")]
    encryption_scheme: Option<webidl::DomString16>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaCapabilitiesKeySystemConfiguration", snapshot)]
struct MediaCapabilitiesKeySystemConfiguration {
    #[webidl(required)]
    key_system: String,
    #[webidl(default = "")]
    init_data_type: String,
    #[webidl(converter = "enum", default = MediaKeysRequirement::Optional)]
    distinctive_identifier: MediaKeysRequirement,
    #[webidl(converter = "enum", default = MediaKeysRequirement::Optional)]
    persistent_state: MediaKeysRequirement,
    #[webidl(sequence)]
    session_types: Option<Vec<String>>,
    #[webidl(dictionary)]
    audio: Option<KeySystemTrackConfiguration>,
    #[webidl(dictionary)]
    video: Option<KeySystemTrackConfiguration>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaConfiguration", snapshot)]
struct MediaConfiguration {
    #[webidl(dictionary)]
    audio: Option<AudioConfiguration>,
    #[webidl(dictionary)]
    video: Option<VideoConfiguration>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaDecodingConfiguration", snapshot)]
struct MediaDecodingConfiguration {
    #[webidl(inherit)]
    media: MediaConfiguration,
    #[webidl(name = "keySystemConfiguration", dictionary)]
    key_system_configuration: Option<MediaCapabilitiesKeySystemConfiguration>,
    #[webidl(name = "type", required, converter = "enum")]
    decoding_type: MediaDecodingType,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaEncodingConfiguration", snapshot)]
struct MediaEncodingConfiguration {
    #[webidl(inherit)]
    media: MediaConfiguration,
    #[webidl(name = "type", required, converter = "enum")]
    encoding_type: MediaEncodingType,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaCapabilities.decodingInfo")]
struct DecodingInfoArgs {
    #[webidl(required, dictionary)]
    configuration: MediaDecodingConfiguration,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaCapabilities.encodingInfo")]
struct EncodingInfoArgs {
    #[webidl(required, dictionary)]
    configuration: MediaEncodingConfiguration,
}

pub(super) fn install_media_capabilities_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "MediaCapabilities" {
        MediaCapabilitiesPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

pub(super) fn build_media_capabilities_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    secure_context: bool,
    worker: bool,
) -> Result<v8::Local<'s, v8::Object>> {
    MediaCapabilitiesObjectDeclaration::new(secure_context, worker)
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind MediaCapabilities object: {error}"))
}

fn media_capabilities_decoding_info_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let parsed = match try_parse_promise_args::<DecodingInfoArgs>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    let configuration = parsed.configuration;
    if let Err(message) = validate_decoding_configuration(&configuration) {
        let reason = type_error_value(scope, message);
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }

    let supported = if configuration.key_system_configuration.is_some() {
        if media_capabilities_is_worker(scope, receiver) {
            let reason = crate::context_bootstrap::new_dom_exception_value(
                scope,
                "Encrypted media capabilities are unavailable in workers",
                "InvalidStateError",
            );
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
        if !media_capabilities_is_secure_context(scope, receiver) {
            let reason = crate::context_bootstrap::new_dom_exception_value(
                scope,
                "Encrypted media capabilities require a secure context",
                "SecurityError",
            );
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
        false
    } else {
        configuration_supported(
            &configuration.media,
            MediaConfigurationType::Decoding(configuration.decoding_type),
        )
    };
    let info = MediaCapabilitiesDecodingInfo {
        info: MediaCapabilitiesInfo {
            power_efficient: false,
            smooth: false,
            supported,
        },
        configuration,
        key_system_access: None,
    };
    let Some(info) = info.to_js_snapshot(scope, SnapshotPrototype::Ordinary) else {
        rv.set_undefined();
        return;
    };
    set_resolved_promise(scope, &mut rv, info.into());
}

fn media_capabilities_encoding_info_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let parsed = match try_parse_promise_args::<EncodingInfoArgs>(scope, &args) {
        Ok(parsed) => parsed,
        Err(reason) => {
            set_rejected_promise(scope, &mut rv, reason);
            return;
        }
    };
    let configuration = parsed.configuration;
    if let Err(message) = validate_encoding_configuration(&configuration) {
        let reason = type_error_value(scope, message);
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
    let supported = configuration_supported(
        &configuration.media,
        MediaConfigurationType::Encoding(configuration.encoding_type),
    );
    let info = MediaCapabilitiesEncodingInfo {
        info: MediaCapabilitiesInfo {
            power_efficient: false,
            smooth: false,
            supported,
        },
        configuration,
    };
    let Some(info) = info.to_js_snapshot(scope, SnapshotPrototype::Ordinary) else {
        rv.set_undefined();
        return;
    };
    set_resolved_promise(scope, &mut rv, info.into());
}

fn try_parse_promise_args<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> std::result::Result<T, v8::Local<'s, v8::Value>>
where
    T: webidl::WebIdlArguments<'s>,
{
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match webidl::try_parse_args::<T>(&mut scope, args) {
        Ok(parsed) => Ok(parsed),
        Err(error) if error.is_pending_exception() => Err(scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&scope).into())),
        Err(error) => Err(type_error_value(&mut scope, &error.to_string())),
    }
}

fn type_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let message = v8_string(scope, message).unwrap_or_else(|| v8::String::empty(scope));
    v8::Exception::type_error(scope, message)
}

fn set_rejected_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.reject(scope, reason);
    rv.set(promise.into());
}

fn set_resolved_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.resolve(scope, value);
    rv.set(promise.into());
}

fn media_capabilities_is_secure_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, MEDIA_CAPABILITIES_SECURE_CONTEXT_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn media_capabilities_is_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, MEDIA_CAPABILITIES_WORKER_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn validate_decoding_configuration(
    configuration: &MediaDecodingConfiguration,
) -> std::result::Result<(), &'static str> {
    validate_media_configuration(
        &configuration.media,
        MediaConfigurationType::Decoding(configuration.decoding_type),
    )?;
    if let Some(key_system) = configuration.key_system_configuration.as_ref() {
        if configuration.decoding_type == MediaDecodingType::Webrtc {
            return Err("keySystemConfiguration is not valid for WebRTC decoding");
        }
        if key_system.audio.is_some() && configuration.media.audio.is_none() {
            return Err("keySystemConfiguration.audio requires an audio configuration");
        }
        if key_system.video.is_some() && configuration.media.video.is_none() {
            return Err("keySystemConfiguration.video requires a video configuration");
        }
        if key_system.session_types.as_ref().is_some_and(|types| {
            types
                .iter()
                .any(|value| !matches!(value.as_str(), "temporary" | "persistent-license"))
        }) {
            return Err("keySystemConfiguration contains an invalid session type");
        }
    }
    Ok(())
}

fn validate_encoding_configuration(
    configuration: &MediaEncodingConfiguration,
) -> std::result::Result<(), &'static str> {
    validate_media_configuration(
        &configuration.media,
        MediaConfigurationType::Encoding(configuration.encoding_type),
    )
}

fn validate_media_configuration(
    configuration: &MediaConfiguration,
    configuration_type: MediaConfigurationType,
) -> std::result::Result<(), &'static str> {
    if configuration.audio.is_none() && configuration.video.is_none() {
        return Err("The configuration needs at least one audio or video member");
    }
    let is_webrtc = configuration_type.is_webrtc();
    if let Some(audio) = &configuration.audio
        && !moli_web_mime::is_valid_media_capabilities_mime_type(
            &audio.content_type,
            moli_web_mime::MediaKind::Audio,
            is_webrtc,
        )
    {
        return Err("The audio configuration dictionary is not valid");
    }
    if let Some(video) = &configuration.video {
        if !video.framerate.is_finite() || video.framerate <= 0.0 {
            return Err("The video configuration framerate must be finite and positive");
        }
        if !matches!(
            configuration_type,
            MediaConfigurationType::Decoding(
                MediaDecodingType::File | MediaDecodingType::MediaSource
            )
        ) && (video.hdr_metadata_type.is_some()
            || video.color_gamut.is_some()
            || video.transfer_function.is_some())
        {
            return Err("HDR and color options are only valid for file or media-source decoding");
        }
        if !matches!(
            configuration_type,
            MediaConfigurationType::Encoding(MediaEncodingType::Webrtc)
        ) && video.scalability_mode.is_some()
        {
            return Err("scalabilityMode is only valid for WebRTC encoding");
        }
        if matches!(configuration_type, MediaConfigurationType::Encoding(_))
            && video.spatial_scalability.is_some()
        {
            return Err("spatialScalability is only valid for decoding");
        }
        if !moli_web_mime::is_valid_media_capabilities_mime_type(
            &video.content_type,
            moli_web_mime::MediaKind::Video,
            is_webrtc,
        ) {
            return Err("The video configuration dictionary is not valid");
        }
    }
    Ok(())
}

fn configuration_supported(
    configuration: &MediaConfiguration,
    configuration_type: MediaConfigurationType,
) -> bool {
    // File MIME support does not provide an RTP codec backend.
    if configuration_type.is_webrtc() {
        return false;
    }
    configuration.audio.as_ref().is_none_or(|audio| {
        moli_web_mime::media_mime_support(&audio.content_type)
            != moli_web_mime::MediaMimeSupport::Unsupported
    }) && configuration.video.as_ref().is_none_or(|video| {
        moli_web_mime::media_mime_support(&video.content_type)
            != moli_web_mime::MediaMimeSupport::Unsupported
            && video_codec_color_supported(video)
    })
}

fn video_codec_color_supported(video: &VideoConfiguration) -> bool {
    let Some(codec) = moli_web_mime::mime_parameter(&video.content_type, "codecs") else {
        return true;
    };
    let Ok(color_space) = moli_web_mime::parse_video_codec_color_space(
        codec.trim_matches(|value: char| value.is_ascii_whitespace()),
    ) else {
        return false;
    };
    let Some(color_space) = color_space else {
        return true;
    };
    // Compare each supplied member independently. An unspecified CICP component
    // leaves that requirement to the configuration rather than inventing a color.
    video.color_gamut.is_none_or(|gamut| {
        color_space.primaries.is_none_or(|primaries| match gamut {
            ColorGamut::Srgb => primaries == 1,
            ColorGamut::P3 => matches!(primaries, 11 | 12),
            ColorGamut::Rec2020 => primaries == 9,
        })
    }) && video.transfer_function.is_none_or(|transfer| {
        color_space
            .transfer
            .is_none_or(|characteristics| match transfer {
                // BT.709 is the codec default for conventional SDR; 13 explicitly
                // signals the IEC 61966-2-1 (sRGB) transfer characteristic.
                TransferFunction::Srgb => matches!(characteristics, 1 | 13),
                TransferFunction::Pq => characteristics == 16,
                TransferFunction::Hlg => characteristics == 18,
            })
    })
}
