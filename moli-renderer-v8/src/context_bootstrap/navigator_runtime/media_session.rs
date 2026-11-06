//! Headless media action registration and playback state. Registrations belong
//! to the Navigator's realm and retain callbacks through traced native slots;
//! they do not synthesize media-key events or control an operating-system player.

use super::super::*;
use crate::util::{get_private_value, set_private_value};
use crate::v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction;
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const HANDLERS: &str = "__moliMediaSessionHandlers";
const PLAYBACK_STATE: &str = "__moliMediaSessionPlaybackState";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaSession)]
struct MediaSessionObjectDeclaration {
    #[webapi(slot = HANDLERS, init = "null_object")]
    handlers: (),
    #[webapi(slot = PLAYBACK_STATE)]
    playback_state: &'static str,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaSession, enumerable, receiver)]
struct MediaSessionPrototypeDeclaration {
    #[webapi(method, length = 2, callback = set_action_handler)]
    set_action_handler: (),
    #[webapi(accessor_property, getter = playback_state, setter = set_playback_state)]
    playback_state: (),
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "MediaSessionAction")]
enum MediaSessionAction {
    Play,
    Pause,
    Seekbackward,
    Seekforward,
    Previoustrack,
    Nexttrack,
    Skipad,
    Stop,
    Seekto,
    Togglemicrophone,
    Togglecamera,
    Hangup,
    Previousslide,
    Nextslide,
    Enterpictureinpicture,
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "MediaSessionPlaybackState")]
enum PlaybackState {
    None,
    Paused,
    Playing,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession.setActionHandler")]
struct SetActionHandlerArgs {
    #[webidl(required, converter = "enum")]
    action: MediaSessionAction,
    #[webidl(required, nullable, converter = "callback_function")]
    handler: Option<webidl::WebIdlCallbackFunction>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession.playbackState")]
struct PlaybackStateArgs {
    #[webidl(required)]
    value: String,
}

pub(super) fn build<'s>(scope: &mut v8::PinScope<'s, '_>) -> Result<v8::Local<'s, v8::Object>> {
    MediaSessionObjectDeclaration::new("none")
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind MediaSession object: {error}"))
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "MediaSession" {
        MediaSessionPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

fn set_action_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SetActionHandlerArgs>(scope, &args) else {
        return;
    };
    let handlers = get_private_value(scope, args.this(), HANDLERS)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("MediaSession has native action handlers");
    let action = v8str(
        scope,
        webidl::WebIdlEnumSnapshot::snapshot_token(&parsed.action),
    );
    if let Some(handler) = parsed.handler {
        let callback = V8TracedWebIdlCallbackFunction::from_callback(scope, &handler).into_object();
        let _ = handlers.create_data_property(scope, action.into(), callback.into());
    } else {
        let _ = handlers.delete(scope, action.into());
    }
}

fn playback_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), PLAYBACK_STATE) {
        rv.set(value);
    }
}

fn set_playback_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PlaybackStateArgs>(scope, &args) else {
        return;
    };
    // Like other Web IDL enum attributes, invalid tokens leave the state intact.
    if let Some(state) = <PlaybackState as webidl::WebIdlEnum>::parse_token(&parsed.value) {
        let value = v8str(scope, webidl::WebIdlEnumSnapshot::snapshot_token(&state));
        set_private_value(scope, args.this(), PLAYBACK_STATE, value.into());
    }
}
