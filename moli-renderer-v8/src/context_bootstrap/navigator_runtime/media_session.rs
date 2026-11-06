//! Headless media action registration and playback state. Registrations belong
//! to the Window and retain callbacks through traced native slots;
//! they do not synthesize media-key events or control an operating-system player.

use super::super::*;
use crate::util::{get_private_value, set_private_value};
use crate::v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction;
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const HANDLERS: &str = "__moliMediaSessionHandlers";
const PLAYBACK_STATE: &str = "__moliMediaSessionPlaybackState";
const STATE: &str = "__moliMediaSessionState";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaSession)]
struct MediaSessionObjectDeclaration<'s> {
    #[webapi(slot = STATE)]
    state: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct MediaSessionStateDeclaration {
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
    Togglescreenshare,
    Hangup,
    Previousslide,
    Nextslide,
    Enterpictureinpicture,
    Voiceactivity,
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

pub(super) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Object>,
    child: Option<crate::document_runtime::DomHandle>,
    popup: Option<u64>,
) -> Result<v8::Local<'s, v8::Object>> {
    let owner = child
        .map(crate::native_bridge::OwnerDispatchScope::Child)
        .or_else(|| popup.map(crate::native_bridge::OwnerDispatchScope::LightweightPopup))
        .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
    let context = scope.get_current_context();
    let default_context =
        crate::util::context_host_ptr_from_context_slot(context).and_then(|host| {
            let host = unsafe { &*host };
            let identity = host.window_execution_context_identity_for_access_check(context)?;
            if identity.dispatch_scope() != owner {
                return None;
            }
            host.window_execution_context(scope, identity.owner(), identity.dispatch_scope())
                .map(|(_, context)| context)
        });
    // A shared state object lives in the Window's default realm, so merely
    // accessing mediaSession cannot keep an otherwise unused isolated world alive.
    // Unmanaged/legacy facade Navigators retain their own backing as the owner.
    let anchor = default_context
        .map(|context| context.get_extras_binding_object(scope))
        .unwrap_or(backing);
    let state = if let Some(state) = get_private_value(scope, anchor, STATE)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        state
    } else {
        let context = default_context.unwrap_or(context);
        let state = {
            let target_scope = &mut v8::ContextScope::new(scope, context);
            let state = MediaSessionStateDeclaration::new("none")
                .bind(target_scope)
                .map_err(|error| anyhow!("failed to bind MediaSession state: {error}"))?;
            v8::Global::new(target_scope, state)
        };
        let state = v8::Local::new(scope, &state);
        set_private_value(scope, anchor, STATE, state.into());
        state
    };
    MediaSessionObjectDeclaration::new(state)
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind MediaSession object: {error}"))
}

fn session_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    session: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_value(scope, session, STATE)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("MediaSession has Window-owned state")
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
    let state = session_state(scope, args.this());
    let handlers = get_private_value(scope, state, HANDLERS)
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
    let state = session_state(scope, args.this());
    if let Some(value) = get_private_value(scope, state, PLAYBACK_STATE) {
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
        let state = session_state(scope, args.this());
        set_private_value(scope, state, PLAYBACK_STATE, value.into());
    }
}
