//! WebGL context creation failure when no rendering backend is available.
//!
//! Interface exposure is not evidence of a working GL implementation. Until a
//! backend can compile, draw and read back pixels, neither canvas factory may
//! create a context or lock out the caller's fallback to another context type.

use super::*;
use crate::{
    context_bootstrap::{
        dispatch_simple_event_target_event,
        events::{initialize_event_object, mark_event_trusted},
    },
    util::{get_private_value, v8str},
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const STATUS_MESSAGE_SLOT: &str = "__moliWebGlContextEventStatusMessage";
pub(crate) const WEBGL_BACKEND_UNAVAILABLE: &str =
    "Could not create a WebGL context: no rendering backend is available.";
pub(crate) const WEBGL_CONTEXT_TYPE_CONFLICT: &str =
    "Canvas has an existing context of a different type";

#[derive(WebApiObject)]
#[webapi(interface = "WebGLContextEvent")]
struct WebGlContextEventState {
    #[webapi(slot = STATUS_MESSAGE_SLOT)]
    status_message: String,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "WebGLContextEventInit")]
struct WebGlContextEventInit {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(name = "statusMessage", default = String::new())]
    status_message: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebGLContextEvent")]
struct WebGlContextEventArgs {
    #[webidl(required)]
    event_type: String,
    #[webidl(with = parse_init)]
    init: WebGlContextEventInit,
}

fn parse_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<WebGlContextEventInit, webidl::WebIdlError> {
    webidl::parse_dictionary(
        scope,
        args.get(index),
        webidl::Context::argument("WebGLContextEvent", (index + 1) as usize),
    )
    .map(|init| init.unwrap_or_default())
}

pub(crate) fn webgl_context_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'WebGLContextEvent': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<WebGlContextEventArgs>(scope, &args) else {
        return;
    };
    let event = args.this();
    initialize_event_object(
        scope,
        event,
        &parsed.event_type,
        parsed.init.bubbles,
        parsed.init.cancelable,
    );
    event.set(
        scope,
        v8str(scope, "composed").into(),
        v8::Boolean::new(scope, parsed.init.composed).into(),
    );
    WebGlContextEventState::new(parsed.init.status_message)
        .initialize(scope, event)
        .expect("WebGLContextEvent state should initialize");
    rv.set(event.into());
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "WebGLContextEvent", enumerable)]
struct WebGlContextEventTemplate {
    #[webapi(accessor_property, getter = status_message_getter)]
    status_message: (),
}

fn status_message_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(value) = get_private_value(scope, args.this(), STATUS_MESSAGE_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

pub(super) fn install_webgl_context_event_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    WebGlContextEventTemplate::initialize_prototype_template(scope, prototype);
}

pub(crate) fn dispatch_webgl_creation_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    status_message: &str,
) {
    let Some(prototype) = global_constructor_prototype(scope, "WebGLContextEvent") else {
        return;
    };
    let event = v8::Object::new(scope);
    if event.set_prototype(scope, prototype.into()) != Some(true) {
        return;
    }
    WebGlContextEventState::new(status_message.to_owned())
        .initialize(scope, event)
        .expect("WebGLContextEvent state should initialize");
    let event_type = "webglcontextcreationerror";
    initialize_event_object(scope, event, event_type, false, true);
    mark_event_trusted(scope, event);
    if let Ok((host, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, canvas)
    {
        element::dispatch_public_event(scope, host, handle, event);
    } else {
        dispatch_simple_event_target_event(
            scope,
            canvas,
            super::offscreen::OFFSCREEN_CANVAS_LISTENERS_SLOT,
            event_type,
            event,
        );
    }
}
