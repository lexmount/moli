//! Constructible event payloads that do not require an animation or recording backend.

use super::{
    define_event_property, event_private_value, initialize_event_object_with_type,
    initialize_event_wrapper, new_event_state, set_event_private_value,
};
use crate::context_bootstrap::events::EventInit;
use crate::{
    util::{new_null_prototype_object, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const NAME_SLOT: &str = "__moliValueEventName";
const ELAPSED_TIME_SLOT: &str = "__moliValueEventElapsedTime";
const PSEUDO_ELEMENT_SLOT: &str = "__moliValueEventPseudoElement";
const ANIMATION_SLOT: &str = "__moliValueEventAnimation";
const BLOB_SLOT: &str = "__moliValueEventBlob";
const STATUS_MESSAGE_SLOT: &str = "__moliWebGlContextEventStatusMessage";
const TIMECODE_SLOT: &str = "__moliValueEventTimecode";
const TOOL_NAME_SLOT: &str = "__moliToolEventName";
const GAMEPAD_SLOT: &str = "__moliValueEventGamepad";
const INIT_DATA_SLOT: &str = "__moliMediaEncryptedEventInitData";
const INIT_DATA_TYPE_SLOT: &str = "__moliMediaEncryptedEventInitDataType";
const MESSAGE_SLOT: &str = "__moliMediaKeyMessageEventMessage";
const MESSAGE_TYPE_SLOT: &str = "__moliMediaKeyMessageEventMessageType";
const UTTERANCE_SLOT: &str = "__moliSpeechSynthesisEventUtterance";
const CHAR_INDEX_SLOT: &str = "__moliSpeechSynthesisEventCharIndex";
const CHAR_LENGTH_SLOT: &str = "__moliSpeechSynthesisEventCharLength";
const SPEECH_ERROR_SLOT: &str = "__moliSpeechSynthesisErrorEventError";
const MIDI_DATA_SLOT: &str = "__moliMIDIMessageEventData";
const MIDI_PORT_SLOT: &str = "__moliMIDIConnectionEventPort";
const MEDIA_TRACK_SLOT: &str = "__moliMediaStreamTrackEventTrack";
const PICTURE_IN_PICTURE_WINDOW_SLOT: &str = "__moliPictureInPictureEventWindow";
const PAYMENT_METHOD_NAME_SLOT: &str = "__moliPaymentMethodChangeEventMethodName";
const PAYMENT_METHOD_DETAILS_SLOT: &str = "__moliPaymentMethodChangeEventMethodDetails";

#[derive(WebApiObject)]
#[webapi(fragment)]
struct AnimationEventPayloadDeclaration<'s> {
    #[webapi(slot = ANIMATION_SLOT)]
    animation: v8::Local<'s, v8::Value>,
    #[webapi(slot = NAME_SLOT)]
    name: v8::Local<'s, v8::String>,
    #[webapi(slot = ELAPSED_TIME_SLOT)]
    elapsed_time: f64,
    #[webapi(slot = PSEUDO_ELEMENT_SLOT)]
    pseudo_element: v8::Local<'s, v8::String>,
}

pub(in crate::context_bootstrap) fn construct_css_animation_start_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    name: v8::Local<'s, v8::String>,
    elapsed_time: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = new_event_state(scope);
    super::initialize_event_object(scope, state, "animationstart", true, false);
    AnimationEventPayloadDeclaration::new(animation.into(), name, elapsed_time, v8str(scope, ""))
        .initialize(scope, state)
        .ok()?;
    web_api_interfaces::AnimationEvent::DESCRIPTOR
        .initialize(scope, state)
        .ok()?;
    let event = super::new_event_wrapper(scope, state)?;
    super::mark_event_trusted(scope, event);
    Some(event)
}

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap) enum ValueEventKind {
    Animation,
    Transition,
    Blob,
    WebGlContext,
    ToolActivated,
    ToolCancel,
    Gamepad,
    MediaEncrypted,
    MediaKeyMessage,
    SpeechSynthesis,
    SpeechSynthesisError,
    MidiMessage,
    MidiConnection,
    MediaStreamTrack,
    PaymentRequestUpdate,
    PaymentMethodChange,
    PictureInPicture,
}

impl ValueEventKind {
    fn name(self) -> &'static str {
        match self {
            Self::Animation => "AnimationEvent",
            Self::Transition => "TransitionEvent",
            Self::Blob => "BlobEvent",
            Self::WebGlContext => "WebGLContextEvent",
            Self::ToolActivated => "ToolActivatedEvent",
            Self::ToolCancel => "ToolCancelEvent",
            Self::Gamepad => "GamepadEvent",
            Self::MediaEncrypted => "MediaEncryptedEvent",
            Self::MediaKeyMessage => "MediaKeyMessageEvent",
            Self::SpeechSynthesis => "SpeechSynthesisEvent",
            Self::SpeechSynthesisError => "SpeechSynthesisErrorEvent",
            Self::MidiMessage => "MIDIMessageEvent",
            Self::MidiConnection => "MIDIConnectionEvent",
            Self::MediaStreamTrack => "MediaStreamTrackEvent",
            Self::PaymentRequestUpdate => "PaymentRequestUpdateEvent",
            Self::PaymentMethodChange => "PaymentMethodChangeEvent",
            Self::PictureInPicture => "PictureInPictureEvent",
        }
    }

    fn length(self) -> i32 {
        match self {
            Self::Blob
            | Self::MediaKeyMessage
            | Self::SpeechSynthesis
            | Self::SpeechSynthesisError => 2,
            Self::MediaStreamTrack | Self::PictureInPicture => 2,
            _ => 1,
        }
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AnimationEvent, enumerable, receiver)]
struct AnimationEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, ANIMATION_SLOT))]
    animation: (),
    #[webapi(accessor_property = "animationName", getter = payload_getter, data = v8str(scope, NAME_SLOT))]
    animation_name: (),
    #[webapi(accessor_property = "elapsedTime", getter = payload_getter, data = v8str(scope, ELAPSED_TIME_SLOT))]
    elapsed_time: (),
    #[webapi(accessor_property = "pseudoElement", getter = payload_getter, data = v8str(scope, PSEUDO_ELEMENT_SLOT))]
    pseudo_element: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TransitionEvent, enumerable, receiver)]
struct TransitionEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, ANIMATION_SLOT))]
    animation: (),
    #[webapi(accessor_property = "propertyName", getter = payload_getter, data = v8str(scope, NAME_SLOT))]
    property_name: (),
    #[webapi(accessor_property = "elapsedTime", getter = payload_getter, data = v8str(scope, ELAPSED_TIME_SLOT))]
    elapsed_time: (),
    #[webapi(accessor_property = "pseudoElement", getter = payload_getter, data = v8str(scope, PSEUDO_ELEMENT_SLOT))]
    pseudo_element: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::BlobEvent, enumerable, receiver)]
struct BlobEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, BLOB_SLOT))]
    data: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, TIMECODE_SLOT))]
    timecode: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::WebGLContextEvent, enumerable, receiver)]
struct WebGlContextEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, STATUS_MESSAGE_SLOT))]
    status_message: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ToolActivatedEvent, enumerable, receiver)]
struct ToolActivatedEventPrototypeDeclaration {
    #[webapi(accessor_property = "toolName", getter = payload_getter, data = v8str(scope, TOOL_NAME_SLOT))]
    tool_name: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ToolCancelEvent, enumerable, receiver)]
struct ToolCancelEventPrototypeDeclaration {
    #[webapi(accessor_property = "toolName", getter = payload_getter, data = v8str(scope, TOOL_NAME_SLOT))]
    tool_name: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ToolEventInit")]
struct ToolEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "toolName", with = string_member)]
    tool_name: v8::Local<'s, v8::String>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebGLContextEventInit")]
struct WebGlContextEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(with = string_member)]
    status_message: v8::Local<'s, v8::String>,
}

// Inherited dictionary members precede derived members, whose declaration order
// follows Web IDL's lexicographic conversion order. Preserve DOMString code units.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AnimationEventInit")]
struct AnimationEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, interface = web_api_interfaces::CSSAnimation)]
    animation: Option<v8::Local<'s, v8::Object>>,
    #[webidl(name = "animationName", with = string_member)]
    animation_name: v8::Local<'s, v8::String>,
    #[webidl(name = "elapsedTime", converter = "double", default = 0.0)]
    elapsed_time: f64,
    #[webidl(name = "pseudoElement", with = string_member)]
    pseudo_element: v8::Local<'s, v8::String>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TransitionEventInit")]
struct TransitionEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, interface = web_api_interfaces::CSSTransition)]
    animation: Option<v8::Local<'s, v8::Object>>,
    #[webidl(name = "elapsedTime", converter = "double", default = 0.0)]
    elapsed_time: f64,
    #[webidl(name = "propertyName", with = string_member)]
    property_name: v8::Local<'s, v8::String>,
    #[webidl(name = "pseudoElement", with = string_member)]
    pseudo_element: v8::Local<'s, v8::String>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "BlobEventInit")]
struct BlobEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::Blob)]
    data: v8::Local<'s, v8::Object>,
    #[webidl(converter = "double")]
    timecode: Option<f64>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GamepadEvent, enumerable, receiver)]
struct GamepadEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, GAMEPAD_SLOT))]
    gamepad: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaEncryptedEvent, enumerable, receiver)]
struct MediaEncryptedEventPrototypeDeclaration {
    #[webapi(accessor_property = "initData", getter = payload_getter, data = v8str(scope, INIT_DATA_SLOT))]
    init_data: (),
    #[webapi(accessor_property = "initDataType", getter = payload_getter, data = v8str(scope, INIT_DATA_TYPE_SLOT))]
    init_data_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaKeyMessageEvent, enumerable, receiver)]
struct MediaKeyMessageEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, MESSAGE_SLOT))]
    message: (),
    #[webapi(accessor_property = "messageType", getter = payload_getter, data = v8str(scope, MESSAGE_TYPE_SLOT))]
    message_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PaymentMethodChangeEvent, enumerable, receiver)]
struct PaymentMethodChangeEventPrototypeDeclaration {
    #[webapi(accessor_property = "methodName", getter = payload_getter, data = v8str(scope, PAYMENT_METHOD_NAME_SLOT))]
    method_name: (),
    #[webapi(accessor_property = "methodDetails", getter = payload_getter, data = v8str(scope, PAYMENT_METHOD_DETAILS_SLOT))]
    method_details: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PaymentMethodChangeEventInit")]
struct PaymentMethodChangeEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable)]
    method_details: Option<v8::Local<'s, v8::Object>>,
    #[webidl(with = string_member)]
    method_name: v8::Local<'s, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PaymentRequestUpdateEvent, enumerable, receiver)]
struct PaymentRequestUpdateEventPrototypeDeclaration {
    #[webapi(method, length = 1, callback = payment_update_with_callback)]
    update_with: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PaymentRequestUpdateEvent.updateWith")]
struct PaymentUpdateWithArgs<'s> {
    #[webidl(required)]
    _details_promise: v8::Local<'s, v8::Promise>,
}

fn payment_update_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if webidl::parse_args::<PaymentUpdateWithArgs>(scope, &args).is_none() {
        return;
    }
    // Only author-created payment events are currently produced. They must
    // reject updates even during dispatch, after Web IDL Promise conversion.
    // Trusted payment events require an interactive PaymentRequest backend.
    crate::native_bridge::throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "updateWith requires a trusted event associated with an interactive payment request",
    );
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "MediaKeyMessageType", rename_all = "kebab-case")]
enum MediaKeyMessageType {
    LicenseRequest,
    LicenseRenewal,
    LicenseRelease,
    IndividualizationRequest,
}

impl MediaKeyMessageType {
    fn as_str(self) -> &'static str {
        match self {
            Self::LicenseRequest => "license-request",
            Self::LicenseRenewal => "license-renewal",
            Self::LicenseRelease => "license-release",
            Self::IndividualizationRequest => "individualization-request",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaEncryptedEventInit")]
struct MediaEncryptedEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "initData", nullable)]
    init_data: Option<v8::Local<'s, v8::ArrayBuffer>>,
    #[webidl(name = "initDataType", with = string_member)]
    init_data_type: v8::Local<'s, v8::String>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaKeyMessageEventInit")]
struct MediaKeyMessageEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required)]
    message: v8::Local<'s, v8::ArrayBuffer>,
    #[webidl(name = "messageType", required, converter = "enum")]
    message_type: MediaKeyMessageType,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SpeechSynthesisEvent, enumerable, receiver)]
struct SpeechSynthesisEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, UTTERANCE_SLOT))]
    utterance: (),
    #[webapi(accessor_property = "charIndex", getter = payload_getter, data = v8str(scope, CHAR_INDEX_SLOT))]
    char_index: (),
    #[webapi(accessor_property = "charLength", getter = payload_getter, data = v8str(scope, CHAR_LENGTH_SLOT))]
    char_length: (),
    #[webapi(accessor_property = "elapsedTime", getter = payload_getter, data = v8str(scope, ELAPSED_TIME_SLOT))]
    elapsed_time: (),
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, NAME_SLOT))]
    name: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SpeechSynthesisErrorEvent, enumerable, receiver)]
struct SpeechSynthesisErrorEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, SPEECH_ERROR_SLOT))]
    error: (),
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "SpeechSynthesisErrorCode", rename_all = "kebab-case")]
enum SpeechSynthesisErrorCode {
    Canceled,
    Interrupted,
    AudioBusy,
    AudioHardware,
    Network,
    SynthesisUnavailable,
    SynthesisFailed,
    LanguageUnavailable,
    VoiceUnavailable,
    TextTooLong,
    InvalidArgument,
    NotAllowed,
}

impl SpeechSynthesisErrorCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Canceled => "canceled",
            Self::Interrupted => "interrupted",
            Self::AudioBusy => "audio-busy",
            Self::AudioHardware => "audio-hardware",
            Self::Network => "network",
            Self::SynthesisUnavailable => "synthesis-unavailable",
            Self::SynthesisFailed => "synthesis-failed",
            Self::LanguageUnavailable => "language-unavailable",
            Self::VoiceUnavailable => "voice-unavailable",
            Self::TextTooLong => "text-too-long",
            Self::InvalidArgument => "invalid-argument",
            Self::NotAllowed => "not-allowed",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SpeechSynthesisEventInit")]
struct SpeechSynthesisEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(default = 0)]
    char_index: u32,
    #[webidl(default = 0)]
    char_length: u32,
    #[webidl(converter = "float", default = 0.0)]
    elapsed_time: f32,
    #[webidl(with = string_member)]
    name: v8::Local<'s, v8::String>,
    #[webidl(required, interface = web_api_interfaces::SpeechSynthesisUtterance)]
    utterance: v8::Local<'s, v8::Object>,
}

// Read this derived member from the original dictionary only after every
// inherited member has been converted, without duplicating the base parser.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "SpeechSynthesisErrorEventInit")]
struct SpeechSynthesisErrorEventInit {
    #[webidl(required, converter = "enum")]
    error: SpeechSynthesisErrorCode,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MIDIMessageEvent, enumerable, receiver)]
struct MidiMessageEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, MIDI_DATA_SLOT))]
    data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MIDIConnectionEvent, enumerable, receiver)]
struct MidiConnectionEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, MIDI_PORT_SLOT))]
    port: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaStreamTrackEvent, enumerable, receiver)]
struct MediaStreamTrackEventPrototype {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, MEDIA_TRACK_SLOT))]
    track: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaStreamTrackEventInit")]
struct MediaStreamTrackEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::MediaStreamTrack)]
    track: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PictureInPictureEvent, enumerable, receiver)]
struct PictureInPictureEventPrototype {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, PICTURE_IN_PICTURE_WINDOW_SLOT))]
    picture_in_picture_window: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PictureInPictureEventInit")]
struct PictureInPictureEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::PictureInPictureWindow)]
    picture_in_picture_window: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MIDIMessageEventInit")]
struct MidiMessageEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    data: Option<v8::Local<'s, v8::Uint8Array>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MIDIConnectionEventInit")]
struct MidiConnectionEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(interface = web_api_interfaces::MIDIPort)]
    port: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "GamepadEventInit")]
struct GamepadEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(with = gamepad_member)]
    gamepad: Option<v8::Local<'s, v8::Object>>,
}

fn gamepad_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    let context = webidl::Context::member("GamepadEventInit", name);
    let value = webidl::property_result(scope, object, name, context)?
        .unwrap_or_else(|| v8::undefined(scope).into());
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let gamepad = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| webidl::WebIdlError::cannot_convert(context, "Gamepad"))?;
    if !web_api_interfaces::Gamepad::is_instance(scope, gamepad) {
        return Err(webidl::WebIdlError::cannot_convert(context, "Gamepad"));
    }
    Ok(Some(gamepad))
}

fn string_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::String>, webidl::WebIdlError> {
    let context = webidl::Context::member("EventInit", name);
    let value = webidl::property_result(scope, object, name, context)?;
    match value.filter(|value| !value.is_undefined()) {
        Some(value) => value
            .to_string(scope)
            .ok_or_else(|| webidl::WebIdlError::pending_exception(context)),
        None => Ok(v8str(scope, "")),
    }
}

pub(in crate::context_bootstrap) fn install_value_event_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "AnimationEvent" => {
            AnimationEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "TransitionEvent" => {
            TransitionEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "WebGLContextEvent" => {
            WebGlContextEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "BlobEvent" => {
            BlobEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "ToolActivatedEvent" => {
            ToolActivatedEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "ToolCancelEvent" => {
            ToolCancelEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "GamepadEvent" => {
            GamepadEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "MediaEncryptedEvent" => {
            MediaEncryptedEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "MediaKeyMessageEvent" => {
            MediaKeyMessageEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "PaymentRequestUpdateEvent" => {
            PaymentRequestUpdateEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "PaymentMethodChangeEvent" => {
            PaymentMethodChangeEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "SpeechSynthesisEvent" => {
            SpeechSynthesisEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "SpeechSynthesisErrorEvent" => {
            SpeechSynthesisErrorEventPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "MIDIMessageEvent" => {
            MidiMessageEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "MIDIConnectionEvent" => {
            MidiConnectionEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "MediaStreamTrackEvent" => {
            MediaStreamTrackEventPrototype::initialize_prototype_template(scope, prototype)
        }
        "PictureInPictureEvent" => {
            PictureInPictureEventPrototype::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

fn payload_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let key = args.data().to_rust_string_lossy(scope);
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("value event receiver was validated");
    if let Some(value) = event_private_value(scope, receiver, &key) {
        rv.set(value);
    }
}

pub(in crate::context_bootstrap) fn build_value_event_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: ValueEventKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    v8::FunctionTemplate::builder(value_event_constructor)
        .data(v8::Integer::new(scope, kind as i32).into())
        .length(kind.length())
        .build(scope)
}

fn value_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let kind = match args.data().int32_value(scope) {
        Some(0) => ValueEventKind::Animation,
        Some(1) => ValueEventKind::Transition,
        Some(2) => ValueEventKind::Blob,
        Some(3) => ValueEventKind::WebGlContext,
        Some(4) => ValueEventKind::ToolActivated,
        Some(5) => ValueEventKind::ToolCancel,
        Some(6) => ValueEventKind::Gamepad,
        Some(7) => ValueEventKind::MediaEncrypted,
        Some(8) => ValueEventKind::MediaKeyMessage,
        Some(9) => ValueEventKind::SpeechSynthesis,
        Some(10) => ValueEventKind::SpeechSynthesisError,
        Some(11) => ValueEventKind::MidiMessage,
        Some(12) => ValueEventKind::MidiConnection,
        Some(13) => ValueEventKind::MediaStreamTrack,
        Some(14) => ValueEventKind::PaymentRequestUpdate,
        Some(15) => ValueEventKind::PaymentMethodChange,
        Some(16) => ValueEventKind::PictureInPicture,
        _ => return,
    };
    if !args.is_construct_call() {
        throw_type_error(scope, &format!("{} requires 'new'.", kind.name()));
        return;
    }
    if args.length() < kind.length() {
        throw_type_error(
            scope,
            &format!("{} requires {} arguments.", kind.name(), kind.length()),
        );
        return;
    }
    let Some(event_type) = args.get(0).to_string(scope) else {
        return;
    };
    let state = new_event_state(scope);
    let result = (|| -> Result<(), webidl::WebIdlError> {
        let dictionary =
            webidl::dictionary_arg(&args, 1, webidl::Context::argument(kind.name(), 2))?
                .unwrap_or_else(|| new_null_prototype_object(scope));
        let (bubbles, cancelable, composed) = match kind {
            ValueEventKind::Animation => {
                let parsed =
                    webidl::parse_dictionary_object::<AnimationEventInit>(scope, dictionary)?;
                let animation = parsed
                    .animation
                    .map(Into::into)
                    .unwrap_or_else(|| v8::null(scope).into());
                AnimationEventPayloadDeclaration::new(
                    animation,
                    parsed.animation_name,
                    parsed.elapsed_time,
                    parsed.pseudo_element,
                )
                .initialize(scope, state)
                .expect("animation event payload should initialize");
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::Transition => {
                let parsed =
                    webidl::parse_dictionary_object::<TransitionEventInit>(scope, dictionary)?;
                let animation = parsed
                    .animation
                    .map(Into::into)
                    .unwrap_or_else(|| v8::null(scope).into());
                AnimationEventPayloadDeclaration::new(
                    animation,
                    parsed.property_name,
                    parsed.elapsed_time,
                    parsed.pseudo_element,
                )
                .initialize(scope, state)
                .expect("animation event payload should initialize");
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::WebGlContext => {
                let parsed =
                    webidl::parse_dictionary_object::<WebGlContextEventInit>(scope, dictionary)?;
                set_event_private_value(
                    scope,
                    state,
                    STATUS_MESSAGE_SLOT,
                    parsed.status_message.into(),
                );
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::Blob => {
                let parsed = webidl::parse_dictionary_object::<BlobEventInit>(scope, dictionary)?;
                set_event_private_value(scope, state, BLOB_SLOT, parsed.data.into());
                set_event_private_value(
                    scope,
                    state,
                    TIMECODE_SLOT,
                    v8::Number::new(scope, parsed.timecode.unwrap_or(f64::NAN)).into(),
                );
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::ToolActivated | ValueEventKind::ToolCancel => {
                let parsed = webidl::parse_dictionary_object::<ToolEventInit>(scope, dictionary)?;
                set_event_private_value(scope, state, TOOL_NAME_SLOT, parsed.tool_name.into());
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::Gamepad => {
                let parsed =
                    webidl::parse_dictionary_object::<GamepadEventInit>(scope, dictionary)?;
                let gamepad = parsed
                    .gamepad
                    .map(Into::into)
                    .unwrap_or_else(|| v8::null(scope).into());
                set_event_private_value(scope, state, GAMEPAD_SLOT, gamepad);
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::MediaEncrypted => {
                let parsed =
                    webidl::parse_dictionary_object::<MediaEncryptedEventInit>(scope, dictionary)?;
                let init_data = parsed
                    .init_data
                    .map(Into::into)
                    .unwrap_or_else(|| v8::null(scope).into());
                set_event_private_value(scope, state, INIT_DATA_SLOT, init_data);
                set_event_private_value(
                    scope,
                    state,
                    INIT_DATA_TYPE_SLOT,
                    parsed.init_data_type.into(),
                );
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::MediaKeyMessage => {
                let parsed =
                    webidl::parse_dictionary_object::<MediaKeyMessageEventInit>(scope, dictionary)?;
                set_event_private_value(scope, state, MESSAGE_SLOT, parsed.message.into());
                set_event_private_value(
                    scope,
                    state,
                    MESSAGE_TYPE_SLOT,
                    v8str(scope, parsed.message_type.as_str()).into(),
                );
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::MidiMessage => {
                let parsed =
                    webidl::parse_dictionary_object::<MidiMessageEventInit>(scope, dictionary)?;
                let data = parsed
                    .data
                    .map(Into::into)
                    .unwrap_or_else(|| v8::null(scope).into());
                set_event_private_value(scope, state, MIDI_DATA_SLOT, data);
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::MidiConnection => {
                let parsed =
                    webidl::parse_dictionary_object::<MidiConnectionEventInit>(scope, dictionary)?;
                let port = parsed
                    .port
                    .map(Into::into)
                    .unwrap_or_else(|| v8::null(scope).into());
                set_event_private_value(scope, state, MIDI_PORT_SLOT, port);
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::MediaStreamTrack => {
                let parsed = webidl::parse_dictionary_object::<MediaStreamTrackEventInit>(
                    scope, dictionary,
                )?;
                set_event_private_value(scope, state, MEDIA_TRACK_SLOT, parsed.track.into());
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::PictureInPicture => {
                let parsed = webidl::parse_dictionary_object::<PictureInPictureEventInit>(
                    scope, dictionary,
                )?;
                set_event_private_value(
                    scope,
                    state,
                    PICTURE_IN_PICTURE_WINDOW_SLOT,
                    parsed.picture_in_picture_window.into(),
                );
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::PaymentRequestUpdate => {
                // PaymentRequestUpdateEventInit adds no members to EventInit.
                let parsed = webidl::parse_dictionary_object::<EventInit>(scope, dictionary)?;
                (parsed.bubbles, parsed.cancelable, parsed.composed)
            }
            ValueEventKind::PaymentMethodChange => {
                let parsed = webidl::parse_dictionary_object::<PaymentMethodChangeEventInit>(
                    scope, dictionary,
                )?;
                let details = parsed
                    .method_details
                    .map(Into::into)
                    .unwrap_or_else(|| v8::null(scope).into());
                set_event_private_value(scope, state, PAYMENT_METHOD_DETAILS_SLOT, details);
                set_event_private_value(
                    scope,
                    state,
                    PAYMENT_METHOD_NAME_SLOT,
                    parsed.method_name.into(),
                );
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::SpeechSynthesis | ValueEventKind::SpeechSynthesisError => {
                let parsed =
                    webidl::parse_dictionary_object::<SpeechSynthesisEventInit>(scope, dictionary)?;
                let error = if matches!(kind, ValueEventKind::SpeechSynthesisError) {
                    Some(
                        webidl::parse_dictionary_object::<SpeechSynthesisErrorEventInit>(
                            scope, dictionary,
                        )?
                        .error,
                    )
                } else {
                    None
                };
                set_event_private_value(scope, state, UTTERANCE_SLOT, parsed.utterance.into());
                set_event_private_value(
                    scope,
                    state,
                    CHAR_INDEX_SLOT,
                    v8::Number::new(scope, f64::from(parsed.char_index)).into(),
                );
                set_event_private_value(
                    scope,
                    state,
                    CHAR_LENGTH_SLOT,
                    v8::Number::new(scope, f64::from(parsed.char_length)).into(),
                );
                set_event_private_value(
                    scope,
                    state,
                    ELAPSED_TIME_SLOT,
                    v8::Number::new(scope, f64::from(parsed.elapsed_time)).into(),
                );
                set_event_private_value(scope, state, NAME_SLOT, parsed.name.into());
                if let Some(error) = error {
                    set_event_private_value(
                        scope,
                        state,
                        SPEECH_ERROR_SLOT,
                        v8str(scope, error.as_str()).into(),
                    );
                }
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
        };
        initialize_event_object_with_type(scope, state, event_type, bubbles, cancelable);
        define_event_property(
            scope,
            state,
            "composed",
            v8::Boolean::new(scope, composed).into(),
        );
        Ok(())
    })();
    if let Err(error) = result {
        webidl::throw_error(scope, &error);
        return;
    }
    web_api_interfaces::initialize(scope, state, kind.name())
        .expect("value event brand should initialize");
    if initialize_event_wrapper(scope, args.this(), state).is_some() {
        rv.set(args.this().into());
    }
}
