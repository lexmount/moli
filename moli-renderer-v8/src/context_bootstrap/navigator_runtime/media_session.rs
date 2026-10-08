//! Headless media action registration and playback state. Registrations belong
//! to the Window and retain callbacks through traced native slots;
//! they do not synthesize media-key events or control an operating-system player.

use super::super::media_metadata::METADATA;
use super::super::*;
use crate::native_bridge::{
    OwnerDispatchScope, document::document_is_fully_active, throw_dom_exception,
};
use crate::util::{get_private_value, set_private_value};
use crate::v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction;
use crate::{web_api_interfaces, webidl};
use moli_webapi::navigator_options::{PlaybackState, PlaybackStateArgs, SetActionHandlerArgs};
use moli_webapi::window_values::{
    MEDIA_SESSION_HANDLERS_SLOT as HANDLERS, MEDIA_SESSION_PLAYBACK_STATE_SLOT as PLAYBACK_STATE,
    MEDIA_SESSION_POSITION_STATE_SLOT as POSITION,
    MEDIA_SESSION_STATE_SLOT as STATE, new_media_session, new_media_session_state,
};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaSession, enumerable)]
struct MediaSessionPrototypeDeclaration {
    #[webapi(method, length = 2, callback = set_action_handler)]
    set_action_handler: (),
    #[webapi(accessor_property, getter = playback_state, setter = set_playback_state)]
    playback_state: (),
    #[webapi(accessor_property, getter = metadata, setter = set_metadata)]
    metadata: (),
    #[webapi(method, length = 0, callback = set_position_state)]
    set_position_state: (),
    #[webapi(method, length = 1, returns_promise, callback = set_capture_active)]
    set_microphone_active: (),
    #[webapi(method, length = 1, returns_promise, callback = set_capture_active)]
    set_camera_active: (),
    #[webapi(method, length = 1, returns_promise, callback = set_capture_active)]
    set_screenshare_active: (),
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
            let state = new_media_session_state(target_scope)
                .map_err(|error| anyhow!("failed to bind MediaSession state: {error}"))?;
            v8::Global::new(target_scope, state)
        };
        let state = v8::Local::new(scope, &state);
        set_private_value(scope, anchor, STATE, state.into());
        state
    };
    new_media_session(scope, state)
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

fn metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let state = session_state(scope, args.this());
    let value =
        get_private_value(scope, state, METADATA).expect("MediaSession has Window-owned metadata");
    rv.set(value);
}

fn set_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<MetadataSetterArgs>(scope, &args) else {
        return;
    };
    let value = parsed
        .value
        .map_or_else(|| v8::null(scope).into(), Into::into);
    let state = session_state(scope, args.this());
    super::super::media_metadata::set_session_metadata(scope, state, value);
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

fn set_position_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PositionArgs>(scope, &args) else {
        return;
    };
    let state = parsed.state;
    let target = session_state(scope, args.this());
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
    let Some(host_ptr) = crate::util::context_host_ptr_from_context_slot(context) else {
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
    let target = args.this();
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

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession.metadata")]
struct MetadataSetterArgs<'s> {
    #[webidl(nullable, interface = web_api_interfaces::MediaMetadata)]
    value: Option<v8::Local<'s, v8::Object>>,
}
