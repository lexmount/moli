//! MediaSession frontend state, independent of platform media controls.
//!
//! Capture control methods expose the Promise boundary and reject until a
//! device/UI backend is available. Handler records use V8-traced slots so
//! callbacks can retain their session without creating Rust rooting cycles.

use super::{native_target, slot_getter};
use crate::{
    native_bridge::{OwnerDispatchScope, document::document_is_fully_active, throw_dom_exception},
    util::{
        context_host_ptr_from_context_slot, get_private_value, set_private_value, throw_type_error,
        v8_string, v8str,
    },
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const PLAYBACK: &str = "__moliMediaSessionPlaybackState";
const HANDLERS: &str = "__moliMediaSessionActionHandlers";
const POSITION: &str = "__moliMediaSessionPositionState";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaSession, require_prototype)]
struct SessionObject<'s> {
    #[webapi(slot = super::METADATA, init = "null")]
    metadata: (),
    #[webapi(slot = PLAYBACK, constructor_default = "none")]
    playback: &'static str,
    #[webapi(slot = HANDLERS)]
    handlers: v8::Local<'s, v8::Map>,
    #[webapi(slot = POSITION, init = "null")]
    position: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaSession, enumerable, receiver)]
struct SessionPrototype {
    #[webapi(accessor_property, getter = slot_getter, setter = playback_setter, data = v8str(scope, PLAYBACK))]
    playback_state: (),
    #[webapi(method, length = 2, callback = set_action_handler)]
    set_action_handler: (),
    #[webapi(method, length = 0, callback = set_position_state)]
    set_position_state: (),
    #[webapi(method, length = 1, returns_promise, callback = set_capture_active)]
    set_microphone_active: (),
    #[webapi(method, length = 1, returns_promise, callback = set_capture_active)]
    set_camera_active: (),
    #[webapi(method, length = 1, returns_promise, callback = set_capture_active)]
    set_screenshare_active: (),
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "MediaSessionPlaybackState", rename_all = "lowercase")]
enum PlaybackState {
    None,
    Paused,
    Playing,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "MediaSessionAction", rename_all = "lowercase")]
enum Action {
    Play,
    Pause,
    SeekBackward,
    SeekForward,
    PreviousTrack,
    NextTrack,
    SkipAd,
    Stop,
    SeekTo,
    ToggleMicrophone,
    ToggleCamera,
    ToggleScreenshare,
    Hangup,
    PreviousSlide,
    NextSlide,
    EnterPictureInPicture,
    VoiceActivity,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession.playbackState")]
struct PlaybackSetterArgs {
    #[webidl(default = "undefined")]
    value: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession.setActionHandler")]
struct ActionArgs {
    #[webidl(required, converter = "enum")]
    action: Action,
    #[webidl(required, nullable, converter = "callback_function")]
    handler: Option<webidl::WebIdlCallbackFunction>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaPositionState")]
struct PositionState {
    #[webidl(converter = "unrestricted_double")]
    duration: Option<f64>,
    #[webidl(converter = "double")]
    playback_rate: Option<f64>,
    #[webidl(converter = "double")]
    position: Option<f64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession.setPositionState")]
struct PositionArgs {
    #[webidl(dictionary, default = PositionState::default())]
    state: PositionState,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession capture control")]
struct CaptureArgs {
    #[webidl(required)]
    _active: bool,
}

pub(super) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> anyhow::Result<v8::Local<'s, v8::Object>> {
    SessionObject::new(v8::Map::new(scope))
        .bind(scope)
        .map_err(Into::into)
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    SessionPrototype::initialize_prototype_template(scope, prototype);
}

fn playback_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PlaybackSetterArgs>(scope, &args) else {
        return;
    };
    // WebIDL enum attribute setters ignore invalid strings after conversion;
    // operation arguments instead use the enum converter's TypeError path.
    if <PlaybackState as webidl::WebIdlEnum>::parse_token(&parsed.value).is_none() {
        return;
    }
    let target = native_target(scope, args.this());
    let Some(value) = v8_string(scope, &parsed.value) else {
        return;
    };
    set_private_value(scope, target, PLAYBACK, value.into());
}

fn set_action_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ActionArgs>(scope, &args) else {
        return;
    };
    let target = native_target(scope, args.this());
    let handlers = get_private_value(scope, target, HANDLERS).expect("MediaSession handler map");
    let handlers = v8::Local::<v8::Map>::try_from(handlers).expect("MediaSession handler map");
    let key = v8::Integer::new(scope, parsed.action as i32);
    if let Some(callback) = parsed.handler {
        let value = callback.value(scope);
        let relevant = callback.relevant_context(scope).global(scope);
        let incumbent = callback.incumbent_context(scope).global(scope);
        let record =
            v8::Array::new_with_elements(scope, &[value, relevant.into(), incumbent.into()]);
        let _ = handlers.set(scope, key.into(), record.into());
    } else {
        let _ = handlers.delete(scope, key.into());
    }
}

fn set_position_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PositionArgs>(scope, &args) else {
        return;
    };
    let state = parsed.state;
    let target = native_target(scope, args.this());
    if state.duration.is_none() && state.playback_rate.is_none() && state.position.is_none() {
        set_private_value(scope, target, POSITION, v8::null(scope).into());
        return;
    }
    let Some(duration) = state
        .duration
        .filter(|value| !value.is_nan() && *value >= 0.0)
    else {
        throw_type_error(
            scope,
            "Media position state requires a nonnegative duration.",
        );
        return;
    };
    let position = state.position.unwrap_or(0.0);
    let rate = state.playback_rate.unwrap_or(1.0);
    if position < 0.0 || position > duration || rate == 0.0 {
        throw_type_error(scope, "Media position or playback rate is invalid.");
        return;
    }
    let values = [duration, rate, position].map(|value| v8::Number::new(scope, value).into());
    let snapshot = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, target, POSITION, snapshot.into());
}

fn session_is_fully_active(
    scope: &mut v8::PinScope<'_, '_>,
    target: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(context) = target.get_creation_context(scope) else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_from_context_slot(context) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    let Some(identity) = host.window_execution_context_identity_for_access_check(context) else {
        return false;
    };
    if !host.window_execution_context_identity_is_current(identity) {
        return false;
    }
    let document = match identity.dispatch_scope() {
        OwnerDispatchScope::Top => Some(host.document_handle()),
        OwnerDispatchScope::Child(frame) => host.child_browsing_context_document_handle(frame),
        OwnerDispatchScope::LightweightPopup(id) if host.lightweight_popup_is_open(id) => {
            host.lightweight_popup_document_handle(id)
        }
        OwnerDispatchScope::LightweightPopup(_) => None,
    };
    document.is_some_and(|document| document_is_fully_active(host, document))
}

fn set_capture_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if webidl::parse_args::<CaptureArgs>(scope, &args).is_none() {
        return;
    }
    let target = native_target(scope, args.this());
    if !session_is_fully_active(scope, target) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The document is not fully active.",
        );
        return;
    }
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "Media capture controls are not implemented.",
    );
}
