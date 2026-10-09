//! Remote Playback frontend for a UA without device discovery or a connection
//! backend. The specification's lifetime-unavailable branch resolves a watch
//! without an id, then queues one false availability notification.

use super::{media_queries, shared};
use crate::{
    document_runtime::DomHandle,
    host::report_event_callback_exception,
    native_bridge::{
        JsContextHost, document::document_is_fully_active,
        node_runtime_and_handle_from_object_or_detached, throw_dom_exception,
    },
    util::{get_private_object, get_private_value, set_private_value, v8str},
    web_api_interfaces, webidl,
    window_webidl_callback::{
        WindowWebIdlCallbackFunction, WindowWebIdlCallbackFunctionOutcome,
        invoke_window_webidl_callback_function,
    },
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const ASSOCIATED: &str = "__moliMediaRemotePlayback";
const MEDIA: &str = "__moliRemotePlaybackMedia";
const LISTENERS: &str = "__moliRemotePlaybackListeners";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RemotePlayback, require_prototype)]
struct RemotePlaybackObject<'s> {
    #[webapi(slot = MEDIA)]
    media: v8::Local<'s, v8::Object>,
    #[webapi(slot = shared::SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS)]
    event_target: (),
    #[webapi(slot = shared::SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RemotePlayback, enumerable, receiver)]
struct RemotePlaybackPrototype {
    #[webapi(method, returns_promise, length = 1, callback = watch_availability)]
    watch_availability: (),
    #[webapi(method, returns_promise, length = 0, callback = cancel_watch_availability)]
    cancel_watch_availability: (),
    #[webapi(method, returns_promise, length = 0, callback = prompt)]
    prompt: (),
    #[webapi(accessor_property, getter = state)]
    state: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "connecting"))]
    onconnecting: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "connect"))]
    onconnect: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "disconnect"))]
    ondisconnect: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RemotePlayback.watchAvailability")]
struct WatchArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RemotePlayback.cancelWatchAvailability")]
struct CancelArgs {
    #[webidl(converter = "long")]
    id: Option<i32>,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name == "RemotePlayback" {
        RemotePlaybackPrototype::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver)
        .expect("RemotePlayback receiver was validated")
}

pub(crate) fn media_remote_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let media = target(scope, args.this());
    if let Some(remote) = get_private_object(scope, media, ASSOCIATED) {
        rv.set(remote.into());
        return;
    }
    let context = media
        .get_creation_context(scope)
        .expect("native media element has a creation realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let remote = RemotePlaybackObject::new(media)
        .bind(scope)
        .expect("native RemotePlayback object should bind");
    set_private_value(scope, media, ASSOCIATED, remote.into());
    rv.set(remote.into());
}

fn media_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let remote = target(scope, receiver);
    let media = get_private_object(scope, remote, MEDIA)
        .expect("RemotePlayback retains its native media element");
    node_runtime_and_handle_from_object_or_detached(scope, media).ok()
}

fn enabled_media_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Some((host_ptr, media)) = media_owner(scope, receiver) else {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The media element is no longer available.",
        );
        return None;
    };
    if unsafe { &*host_ptr }
        .dom_host()
        .node(media)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.has_attribute("disableremoteplayback"))
    {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "Remote playback is disabled for this media element.",
        );
        return None;
    }
    Some((host_ptr, media))
}

fn resolved_undefined<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("RemotePlayback promise should allocate");
    resolver.resolve(scope, v8::undefined(scope).into());
    resolver.get_promise(scope)
}

fn watch_availability<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WatchArgs>(scope, &args) else {
        return;
    };
    let Some((host_ptr, media)) = enabled_media_owner(scope, args.this()) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let callback = RemotePlaybackAvailabilityTask {
        callback: WindowWebIdlCallbackFunction::new(scope, host, parsed.callback),
    };
    // The lifetime-unavailable algorithm does not register a callback or
    // allocate an id. Exact Window/Document admission handles retired owners.
    host.queue_remote_playback_availability(scope, media, callback);
    rv.set(resolved_undefined(scope).into());
}

fn cancel_watch_availability<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CancelArgs>(scope, &args) else {
        return;
    };
    if enabled_media_owner(scope, args.this()).is_none() {
        return;
    }
    // There are no registered monitors in the lifetime-unavailable branch.
    if parsed.id.is_some() {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "No availability callback has this id.",
        );
        return;
    }
    rv.set(resolved_undefined(scope).into());
}

fn prompt<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((host_ptr, media)) = enabled_media_owner(scope, args.this()) else {
        return;
    };
    let host = unsafe { &*host_ptr };
    let active_owner = host.owner_dispatch_scope_for_node(media).filter(|_| {
        host.dom_host()
            .node(media)
            .and_then(|node| node.owner_document())
            .is_some_and(|document| document_is_fully_active(host, document))
    });
    if active_owner.is_none_or(|owner| {
        !host.window_has_transient_user_activation(owner)
            && !host.protocol_user_gesture_activation()
    }) {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "Remote playback requires an active window with transient user activation.",
        );
        return;
    }
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "Remote playback connections are unavailable on this platform.",
    );
}

fn state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8str(scope, "disconnected").into());
}

fn handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let remote = target(scope, args.this());
    let event = args.data().to_rust_string_lossy(scope);
    rv.set(
        get_private_value(scope, remote, handler_slot(&event))
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let remote = target(scope, args.this());
    let event = args.data().to_rust_string_lossy(scope);
    let slot = handler_slot(&event);
    let active = args.get(0).is_object();
    let handler = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, remote, slot, handler);
    media_queries::simple_object_event_set_ordered_handler(
        scope, remote, LISTENERS, &event, slot, active,
    );
}

fn handler_slot(event: &str) -> &'static str {
    match event {
        "connecting" => "__moliRemotePlaybackOnconnecting",
        "connect" => "__moliRemotePlaybackOnconnect",
        "disconnect" => "__moliRemotePlaybackOndisconnect",
        _ => unreachable!("RemotePlayback handler callback data"),
    }
}

/// One initial unavailable notification, on the media element event task
/// source. Callback realm/incumbent context and retired-realm checks use the
/// shared Web IDL callback infrastructure.
pub(crate) struct RemotePlaybackAvailabilityTask {
    callback: WindowWebIdlCallbackFunction,
}

impl RemotePlaybackAvailabilityTask {
    pub(crate) fn invoke(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> bool {
        let callback = self.callback.prepare(scope);
        let relevant_identity = callback.relevant_identity();
        let receiver = v8::undefined(scope).into();
        let available = v8::Boolean::new(scope, false).into();
        match invoke_window_webidl_callback_function(
            scope,
            host_ptr,
            "RemotePlaybackAvailabilityCallback",
            "RemotePlayback availability callback threw",
            "RemotePlayback availability callback",
            &callback,
            receiver,
            &[available],
        ) {
            WindowWebIdlCallbackFunctionOutcome::Returned => true,
            WindowWebIdlCallbackFunctionOutcome::Threw(report) => {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "remoteplayback",
                    relevant_identity,
                    None,
                    &report,
                );
                true
            }
            WindowWebIdlCallbackFunctionOutcome::Retired => false,
        }
    }
}
