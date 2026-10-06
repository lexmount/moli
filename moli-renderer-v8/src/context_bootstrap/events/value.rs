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
use moli_webapi_declare::WebApiFunctionTemplate;

const NAME_SLOT: &str = "__moliValueEventName";
const ELAPSED_TIME_SLOT: &str = "__moliValueEventElapsedTime";
const PSEUDO_ELEMENT_SLOT: &str = "__moliValueEventPseudoElement";
const BLOB_SLOT: &str = "__moliValueEventBlob";
const STATUS_MESSAGE_SLOT: &str = "__moliWebGlContextEventStatusMessage";
const TIMECODE_SLOT: &str = "__moliValueEventTimecode";
const TOOL_NAME_SLOT: &str = "__moliToolEventName";
const GAMEPAD_SLOT: &str = "__moliValueEventGamepad";
const INIT_DATA_SLOT: &str = "__moliMediaEncryptedEventInitData";
const INIT_DATA_TYPE_SLOT: &str = "__moliMediaEncryptedEventInitDataType";
const MESSAGE_SLOT: &str = "__moliMediaKeyMessageEventMessage";
const MESSAGE_TYPE_SLOT: &str = "__moliMediaKeyMessageEventMessageType";

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
        }
    }

    fn length(self) -> i32 {
        match self {
            Self::Blob | Self::MediaKeyMessage => 2,
            _ => 1,
        }
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AnimationEvent, enumerable, receiver)]
struct AnimationEventPrototypeDeclaration {
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
    #[webidl(name = "elapsedTime", converter = "double", default = 0.0)]
    elapsed_time: f64,
    #[webidl(name = "propertyName", with = string_member)]
    property_name: v8::Local<'s, v8::String>,
    #[webidl(name = "pseudoElement", with = string_member)]
    pseudo_element: v8::Local<'s, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::GamepadEvent, enumerable, receiver)]
struct GamepadEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = v8str(scope, GAMEPAD_SLOT))]
    gamepad: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "GamepadEventInit")]
struct GamepadEventInit<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
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
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(name = "initData", nullable)]
    init_data: Option<v8::Local<'s, v8::ArrayBuffer>>,
    #[webidl(name = "initDataType", with = string_member)]
    init_data_type: v8::Local<'s, v8::String>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaKeyMessageEventInit")]
struct MediaKeyMessageEventInit<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(required)]
    message: v8::Local<'s, v8::ArrayBuffer>,
    #[webidl(name = "messageType", required, converter = "enum")]
    message_type: MediaKeyMessageType,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "GamepadEventInit")]
struct GamepadEventInit<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
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
                set_event_private_value(scope, state, NAME_SLOT, parsed.animation_name.into());
                set_event_private_value(
                    scope,
                    state,
                    ELAPSED_TIME_SLOT,
                    v8::Number::new(scope, parsed.elapsed_time).into(),
                );
                set_event_private_value(
                    scope,
                    state,
                    PSEUDO_ELEMENT_SLOT,
                    parsed.pseudo_element.into(),
                );
                (
                    parsed.base.bubbles,
                    parsed.base.cancelable,
                    parsed.base.composed,
                )
            }
            ValueEventKind::Transition => {
                let parsed =
                    webidl::parse_dictionary_object::<TransitionEventInit>(scope, dictionary)?;
                set_event_private_value(scope, state, NAME_SLOT, parsed.property_name.into());
                set_event_private_value(
                    scope,
                    state,
                    ELAPSED_TIME_SLOT,
                    v8::Number::new(scope, parsed.elapsed_time).into(),
                );
                set_event_private_value(
                    scope,
                    state,
                    PSEUDO_ELEMENT_SLOT,
                    parsed.pseudo_element.into(),
                );
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
                (parsed.bubbles, parsed.cancelable, parsed.composed)
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
                (parsed.bubbles, parsed.cancelable, parsed.composed)
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
                (parsed.bubbles, parsed.cancelable, parsed.composed)
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
