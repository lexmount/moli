use super::super::*;
use crate::web_api_interfaces;
use crate::{util::get_private_value, webidl};
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
#[webapi(interface = web_api_interfaces::MediaCapabilities, enumerable)]
struct MediaCapabilitiesPrototypeDeclaration {
    #[webapi(method, length = 1, callback = media_capabilities_decoding_info_callback)]
    decoding_info: (),

    #[webapi(method, length = 1, callback = media_capabilities_encoding_info_callback)]
    encoding_info: (),
}

#[derive(WebApiObject)]
#[webapi(record)]
struct MediaCapabilitiesDecodingInfoDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    supported: bool,

    #[webapi(data_property, enumerable)]
    smooth: bool,

    #[webapi(data_property, enumerable)]
    power_efficient: bool,

    #[webapi(data_property, enumerable)]
    key_system_access: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    configuration: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(record)]
struct MediaCapabilitiesEncodingInfoDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    supported: bool,

    #[webapi(data_property, enumerable)]
    smooth: bool,

    #[webapi(data_property, enumerable)]
    power_efficient: bool,

    #[webapi(data_property, enumerable)]
    configuration: v8::Local<'scope, v8::Object>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "MediaDecodingType", rename_all = "kebab-case")]
enum MediaDecodingType {
    File,
    MediaSource,
    Webrtc,
}

impl MediaDecodingType {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::MediaSource => "media-source",
            Self::Webrtc => "webrtc",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "MediaEncodingType")]
enum MediaEncodingType {
    Record,
    Webrtc,
}

impl MediaEncodingType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::Webrtc => "webrtc",
        }
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

impl HdrMetadataType {
    fn as_str(self) -> &'static str {
        match self {
            Self::SmpteSt2086 => "smpteSt2086",
            Self::SmpteSt2094_10 => "smpteSt2094-10",
            Self::SmpteSt2094_40 => "smpteSt2094-40",
        }
    }
}

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "ColorGamut")]
enum ColorGamut {
    Srgb,
    P3,
    Rec2020,
}

impl ColorGamut {
    fn as_str(self) -> &'static str {
        match self {
            Self::Srgb => "srgb",
            Self::P3 => "p3",
            Self::Rec2020 => "rec2020",
        }
    }
}

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "TransferFunction")]
enum TransferFunction {
    Srgb,
    Pq,
    Hlg,
}

impl TransferFunction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Srgb => "srgb",
            Self::Pq => "pq",
            Self::Hlg => "hlg",
        }
    }
}

#[derive(Clone, Copy, Debug, webidl::WebIdlEnum)]
#[webidl(name = "MediaKeysRequirement", rename_all = "kebab-case")]
enum MediaKeysRequirement {
    Required,
    Optional,
    NotAllowed,
}

impl MediaKeysRequirement {
    fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::NotAllowed => "not-allowed",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AudioConfiguration")]
struct AudioConfiguration {
    #[webidl(required)]
    content_type: String,
    channels: Option<String>,
    bitrate: Option<u64>,
    samplerate: Option<u32>,
    spatial_rendering: Option<bool>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "VideoConfiguration")]
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
#[webidl(prefix = "KeySystemTrackConfiguration")]
struct KeySystemTrackConfiguration {
    #[webidl(default = "")]
    robustness: String,
    #[webidl(with = optional_nullable_dom_string_member)]
    encryption_scheme: Option<Option<String>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaCapabilitiesKeySystemConfiguration")]
struct MediaCapabilitiesKeySystemConfiguration {
    #[webidl(required)]
    key_system: String,
    #[webidl(default = "")]
    init_data_type: String,
    #[webidl(converter = "enum", default = MediaKeysRequirement::Optional)]
    distinctive_identifier: MediaKeysRequirement,
    #[webidl(converter = "enum", default = MediaKeysRequirement::Optional)]
    persistent_state: MediaKeysRequirement,
    #[webidl(with = optional_dom_string_sequence_member)]
    session_types: Option<Vec<String>>,
    #[webidl(with = optional_key_system_track_member)]
    audio: Option<KeySystemTrackConfiguration>,
    #[webidl(with = optional_key_system_track_member)]
    video: Option<KeySystemTrackConfiguration>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaDecodingConfiguration")]
struct MediaDecodingConfiguration {
    #[webidl(with = optional_audio_configuration_member)]
    audio: Option<AudioConfiguration>,
    #[webidl(name = "keySystemConfiguration", with = optional_key_system_configuration_member)]
    key_system_configuration: Option<MediaCapabilitiesKeySystemConfiguration>,
    #[webidl(name = "type", required, converter = "enum")]
    decoding_type: MediaDecodingType,
    #[webidl(with = optional_video_configuration_member)]
    video: Option<VideoConfiguration>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaEncodingConfiguration")]
struct MediaEncodingConfiguration {
    #[webidl(with = optional_audio_configuration_member)]
    audio: Option<AudioConfiguration>,
    #[webidl(name = "type", required, converter = "enum")]
    encoding_type: MediaEncodingType,
    #[webidl(with = optional_video_configuration_member)]
    video: Option<VideoConfiguration>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaCapabilities.decodingInfo")]
struct DecodingInfoArgs {
    #[webidl(required, with = media_decoding_configuration_arg)]
    configuration: MediaDecodingConfiguration,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaCapabilities.encodingInfo")]
struct EncodingInfoArgs {
    #[webidl(required, with = media_encoding_configuration_arg)]
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
    if !media_capabilities_receiver_branded(scope, receiver) {
        let reason = type_error_value(scope, "Illegal invocation");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
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

    if configuration.key_system_configuration.is_some() {
        let (name, message) = if media_capabilities_is_worker(scope, receiver) {
            (
                "InvalidStateError",
                "Encrypted media capabilities are unavailable in workers",
            )
        } else if !media_capabilities_is_secure_context(scope, receiver) {
            (
                "SecurityError",
                "Encrypted media capabilities require a secure context",
            )
        } else {
            let Some(configuration_object) = decoding_configuration_object(scope, &configuration)
            else {
                rv.set_undefined();
                return;
            };
            let Ok(info) = MediaCapabilitiesDecodingInfoDeclaration::new(
                false,
                false,
                false,
                v8::null(scope).into(),
                configuration_object,
            )
            .bind(scope) else {
                rv.set_undefined();
                return;
            };
            set_resolved_promise(scope, &mut rv, info.into());
            return;
        };
        let reason = crate::context_bootstrap::new_dom_exception_value(scope, message, name);
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }

    let Some(configuration_object) = decoding_configuration_object(scope, &configuration) else {
        rv.set_undefined();
        return;
    };
    let supported =
        configuration_supported(configuration.audio.as_ref(), configuration.video.as_ref());
    let Ok(info) = MediaCapabilitiesDecodingInfoDeclaration::new(
        supported,
        false,
        false,
        v8::null(scope).into(),
        configuration_object,
    )
    .bind(scope) else {
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
    if !media_capabilities_receiver_branded(scope, args.this()) {
        let reason = type_error_value(scope, "Illegal invocation");
        set_rejected_promise(scope, &mut rv, reason);
        return;
    }
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
    let Some(configuration_object) = encoding_configuration_object(scope, &configuration) else {
        rv.set_undefined();
        return;
    };
    let supported =
        configuration_supported(configuration.audio.as_ref(), configuration.video.as_ref());
    let Ok(info) = MediaCapabilitiesEncodingInfoDeclaration::new(
        supported,
        false,
        false,
        configuration_object,
    )
    .bind(scope) else {
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

fn media_capabilities_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::MediaCapabilities::is_instance(scope, receiver)
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

fn media_decoding_configuration_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> std::result::Result<MediaDecodingConfiguration, webidl::WebIdlError> {
    required_dictionary_argument::<MediaDecodingConfiguration>(
        scope,
        args,
        index,
        "MediaCapabilities.decodingInfo",
    )
}

fn media_encoding_configuration_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> std::result::Result<MediaEncodingConfiguration, webidl::WebIdlError> {
    required_dictionary_argument::<MediaEncodingConfiguration>(
        scope,
        args,
        index,
        "MediaCapabilities.encodingInfo",
    )
}

fn required_dictionary_argument<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> std::result::Result<T, webidl::WebIdlError>
where
    T: webidl::WebIdlDictionary<'s>,
{
    if args.length() <= index {
        return Err(webidl::WebIdlError::missing_required(
            webidl::Context::argument(prefix, (index + 1) as usize),
        ));
    }
    let context = webidl::Context::argument(prefix, (index + 1) as usize);
    let object = dictionary_object_or_empty(scope, args.get(index), context)?;
    webidl::parse_dictionary_object::<T>(scope, object)
}

fn dictionary_object_or_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> std::result::Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    if value.is_null_or_undefined() {
        return Ok(v8::Object::new(scope));
    }
    webidl::dictionary_value(value, context)?
        .ok_or_else(|| webidl::WebIdlError::custom_message("The value is not a dictionary"))
}

fn optional_dictionary_member<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    prefix: &'static str,
) -> std::result::Result<Option<T>, webidl::WebIdlError>
where
    T: webidl::WebIdlDictionary<'s>,
{
    let context = webidl::Context::member(prefix, name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    let value = dictionary_object_or_empty(scope, value, context)?;
    webidl::parse_dictionary_object::<T>(scope, value).map(Some)
}

fn optional_audio_configuration_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<Option<AudioConfiguration>, webidl::WebIdlError> {
    optional_dictionary_member(scope, object, name, "MediaConfiguration")
}

fn optional_video_configuration_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<Option<VideoConfiguration>, webidl::WebIdlError> {
    optional_dictionary_member(scope, object, name, "MediaConfiguration")
}

fn optional_key_system_configuration_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<Option<MediaCapabilitiesKeySystemConfiguration>, webidl::WebIdlError> {
    optional_dictionary_member(scope, object, name, "MediaDecodingConfiguration")
}

fn optional_key_system_track_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<Option<KeySystemTrackConfiguration>, webidl::WebIdlError> {
    optional_dictionary_member(
        scope,
        object,
        name,
        "MediaCapabilitiesKeySystemConfiguration",
    )
}

fn optional_dom_string_sequence_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<Option<Vec<String>>, webidl::WebIdlError> {
    let context = webidl::Context::member("MediaCapabilitiesKeySystemConfiguration", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    webidl::convert::<webidl::Sequence<webidl::DomString>>(scope, value, context)
        .map(|sequence| Some(sequence.0.into_iter().map(|value| value.0).collect()))
}

fn optional_nullable_dom_string_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> std::result::Result<Option<Option<String>>, webidl::WebIdlError> {
    let context = webidl::Context::member("KeySystemTrackConfiguration", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    if value.is_null() {
        return Ok(Some(None));
    }
    webidl::convert::<webidl::DomString>(scope, value, context).map(|value| Some(Some(value.0)))
}

fn validate_decoding_configuration(
    configuration: &MediaDecodingConfiguration,
) -> std::result::Result<(), &'static str> {
    validate_media_configuration(
        configuration.audio.as_ref(),
        configuration.video.as_ref(),
        configuration.decoding_type == MediaDecodingType::Webrtc,
        true,
    )?;
    if let Some(key_system) = configuration.key_system_configuration.as_ref() {
        if configuration.decoding_type == MediaDecodingType::Webrtc {
            return Err("keySystemConfiguration is not valid for WebRTC decoding");
        }
        if key_system.audio.is_some() && configuration.audio.is_none() {
            return Err("keySystemConfiguration.audio requires an audio configuration");
        }
        if key_system.video.is_some() && configuration.video.is_none() {
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
        configuration.audio.as_ref(),
        configuration.video.as_ref(),
        configuration.encoding_type == MediaEncodingType::Webrtc,
        false,
    )
}

fn validate_media_configuration(
    audio: Option<&AudioConfiguration>,
    video: Option<&VideoConfiguration>,
    is_webrtc: bool,
    is_decoding: bool,
) -> std::result::Result<(), &'static str> {
    if audio.is_none() && video.is_none() {
        return Err("The configuration needs at least one audio or video member");
    }
    if let Some(audio) = audio
        && !valid_media_content_type(&audio.content_type, "audio/", is_webrtc)
    {
        return Err("The audio configuration dictionary is not valid");
    }
    if let Some(video) = video {
        if !video.framerate.is_finite() || video.framerate <= 0.0 {
            return Err("The video configuration framerate must be finite and positive");
        }
        if !valid_media_content_type(&video.content_type, "video/", is_webrtc) {
            return Err("The video configuration dictionary is not valid");
        }
        if (!is_webrtc || is_decoding) && video.scalability_mode.is_some() {
            return Err("scalabilityMode is only valid for WebRTC encoding");
        }
        if (!is_webrtc || !is_decoding) && video.spatial_scalability.is_some() {
            return Err("spatialScalability is only valid for WebRTC decoding");
        }
    }
    Ok(())
}

fn valid_media_content_type(content_type: &str, prefix: &str, is_webrtc: bool) -> bool {
    if content_type.trim_end().ends_with(';') {
        return false;
    }
    let Some(parsed) = moli_web_mime::parse_mime(content_type) else {
        return false;
    };
    let Some(essence) = moli_web_mime::mime_essence(content_type) else {
        return false;
    };
    if !essence.starts_with(prefix) {
        return false;
    }
    if is_webrtc {
        return true;
    }
    let mut parameters = parsed.params();
    let Some((name, codecs)) = parameters.next() else {
        return false;
    };
    if parameters.next().is_some() || !name.as_str().eq_ignore_ascii_case("codecs") {
        return false;
    }
    let codecs = codecs.as_str();
    let mut codecs = codecs
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(codec) = codecs.next() else {
        return false;
    };
    if codecs.next().is_some() {
        return false;
    }
    codec_matches_media_kind(codec, prefix)
}

fn codec_matches_media_kind(codec: &str, prefix: &str) -> bool {
    let codec = codec.to_ascii_lowercase();
    let audio = [
        "aac", "ac-3", "ec-3", "flac", "mp3", "mp4a", "opus", "vorbis",
    ];
    let video = [
        "av01", "avc1", "avc3", "hev1", "hvc1", "theora", "vp8", "vp9", "vp09",
    ];
    let matches = |known: &[&str]| {
        known.iter().any(|value| {
            codec == *value
                || codec
                    .strip_prefix(*value)
                    .is_some_and(|suffix| suffix.starts_with('.') || suffix.starts_with('-'))
        })
    };
    if prefix == "audio/" {
        !matches(&video)
    } else {
        !matches(&audio)
    }
}

fn configuration_supported(
    audio: Option<&AudioConfiguration>,
    video: Option<&VideoConfiguration>,
) -> bool {
    audio.is_none_or(|audio| {
        moli_web_mime::media_mime_support(&audio.content_type)
            != moli_web_mime::MediaMimeSupport::Unsupported
    }) && video.is_none_or(|video| {
        moli_web_mime::media_mime_support(&video.content_type)
            != moli_web_mime::MediaMimeSupport::Unsupported
    })
}

fn decoding_configuration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    configuration: &MediaDecodingConfiguration,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = media_configuration_object(
        scope,
        configuration.audio.as_ref(),
        configuration.video.as_ref(),
    )?;
    set_string_property(scope, object, "type", configuration.decoding_type.as_str())?;
    if let Some(key_system) = configuration.key_system_configuration.as_ref() {
        let value = key_system_configuration_object(scope, key_system)?;
        set_object_property(scope, object, "keySystemConfiguration", value)?;
    }
    Some(object)
}

fn encoding_configuration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    configuration: &MediaEncodingConfiguration,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = media_configuration_object(
        scope,
        configuration.audio.as_ref(),
        configuration.video.as_ref(),
    )?;
    set_string_property(scope, object, "type", configuration.encoding_type.as_str())?;
    Some(object)
}

fn media_configuration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    audio: Option<&AudioConfiguration>,
    video: Option<&VideoConfiguration>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    if let Some(audio) = audio {
        let audio = audio_configuration_object(scope, audio)?;
        set_object_property(scope, object, "audio", audio)?;
    }
    if let Some(video) = video {
        let video = video_configuration_object(scope, video)?;
        set_object_property(scope, object, "video", video)?;
    }
    Some(object)
}

fn audio_configuration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    configuration: &AudioConfiguration,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    set_string_property(scope, object, "contentType", &configuration.content_type)?;
    if let Some(value) = configuration.channels.as_deref() {
        set_string_property(scope, object, "channels", value)?;
    }
    if let Some(value) = configuration.bitrate {
        set_number_property(scope, object, "bitrate", value as f64)?;
    }
    if let Some(value) = configuration.samplerate {
        set_number_property(scope, object, "samplerate", f64::from(value))?;
    }
    if let Some(value) = configuration.spatial_rendering {
        set_bool_property(scope, object, "spatialRendering", value)?;
    }
    Some(object)
}

fn video_configuration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    configuration: &VideoConfiguration,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    set_string_property(scope, object, "contentType", &configuration.content_type)?;
    set_number_property(scope, object, "width", f64::from(configuration.width))?;
    set_number_property(scope, object, "height", f64::from(configuration.height))?;
    set_number_property(scope, object, "bitrate", configuration.bitrate as f64)?;
    set_number_property(scope, object, "framerate", configuration.framerate)?;
    if let Some(value) = configuration.has_alpha_channel {
        set_bool_property(scope, object, "hasAlphaChannel", value)?;
    }
    if let Some(value) = configuration.hdr_metadata_type {
        set_string_property(scope, object, "hdrMetadataType", value.as_str())?;
    }
    if let Some(value) = configuration.color_gamut {
        set_string_property(scope, object, "colorGamut", value.as_str())?;
    }
    if let Some(value) = configuration.transfer_function {
        set_string_property(scope, object, "transferFunction", value.as_str())?;
    }
    if let Some(value) = configuration.scalability_mode.as_deref() {
        set_string_property(scope, object, "scalabilityMode", value)?;
    }
    if let Some(value) = configuration.spatial_scalability {
        set_bool_property(scope, object, "spatialScalability", value)?;
    }
    Some(object)
}

fn key_system_configuration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    configuration: &MediaCapabilitiesKeySystemConfiguration,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    set_string_property(scope, object, "keySystem", &configuration.key_system)?;
    set_string_property(scope, object, "initDataType", &configuration.init_data_type)?;
    set_string_property(
        scope,
        object,
        "distinctiveIdentifier",
        configuration.distinctive_identifier.as_str(),
    )?;
    set_string_property(
        scope,
        object,
        "persistentState",
        configuration.persistent_state.as_str(),
    )?;
    if let Some(session_types) = configuration.session_types.as_ref() {
        let values = session_types
            .iter()
            .map(|value| v8_string(scope, value).map(v8::Local::into))
            .collect::<Option<Vec<v8::Local<'s, v8::Value>>>>()?;
        let array = v8::Array::new(scope, values.len() as i32);
        for (index, value) in values.into_iter().enumerate() {
            array.set_index(scope, index as u32, value)?;
        }
        set_value_property(scope, object, "sessionTypes", array.into())?;
    }
    if let Some(audio) = configuration.audio.as_ref() {
        let value = key_system_track_configuration_object(scope, audio)?;
        set_object_property(scope, object, "audio", value)?;
    }
    if let Some(video) = configuration.video.as_ref() {
        let value = key_system_track_configuration_object(scope, video)?;
        set_object_property(scope, object, "video", value)?;
    }
    Some(object)
}

fn key_system_track_configuration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    configuration: &KeySystemTrackConfiguration,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    set_string_property(scope, object, "robustness", &configuration.robustness)?;
    if let Some(value) = configuration.encryption_scheme.as_ref() {
        let value = value
            .as_deref()
            .and_then(|value| v8_string(scope, value).map(v8::Local::into))
            .unwrap_or_else(|| v8::null(scope).into());
        set_value_property(scope, object, "encryptionScheme", value)?;
    }
    Some(object)
}

fn set_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
    value: &str,
) -> Option<()> {
    let value = v8_string(scope, value)?;
    set_value_property(scope, object, name, value.into())
}

fn set_number_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
    value: f64,
) -> Option<()> {
    let value = v8::Number::new(scope, value);
    set_value_property(scope, object, name, value.into())
}

fn set_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
    value: bool,
) -> Option<()> {
    let value = v8::Boolean::new(scope, value);
    set_value_property(scope, object, name, value.into())
}

fn set_object_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
    value: v8::Local<'_, v8::Object>,
) -> Option<()> {
    set_value_property(scope, object, name, value.into())
}

fn set_value_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
    value: v8::Local<'_, v8::Value>,
) -> Option<()> {
    object
        .set(scope, v8_string(scope, name)?.into(), value)
        .filter(|set| *set)
        .map(|_| ())
}
