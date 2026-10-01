//! Service Worker extendable events and their pending dispatch state.

use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::ExtendableEvent,
    constructor_callback = extendable_event_constructor_callback,
    constructor_length = 1
)]
struct ExtendableEventTemplateDeclaration {
    #[webapi(method = "waitUntil", callback = extendable_event_wait_until_callback, length = 1)]
    wait_until: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::ExtendableMessageEvent,
    constructor_callback = extendable_message_event_constructor_callback,
    constructor_length = 1
)]
struct ExtendableMessageEventTemplateDeclaration {}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ExtendableEventConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "ExtendableEvent")]
    extendable_event: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ExtendableMessageEventConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "ExtendableMessageEvent")]
    extendable_message_event: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ExtendableEvent, prototype = "Object")]
struct ExtendableEventStateDeclaration {
    #[webapi(data_property, enumerable)]
    composed: bool,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::ExtendableMessageEvent,
    prototype = "Object"
)]
struct ExtendableMessageEventStateDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    origin: String,

    #[webapi(data_property = "lastEventId", enumerable)]
    last_event_id: String,

    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}

pub(in crate::worker) struct PendingServiceWorkerLifecycleEvent {
    pub(in crate::worker) completion: ServiceWorkerLifecycleCompletion,
    pub(in crate::worker) pending_wait_until_count: usize,
    pub(in crate::worker) dispatch_finished: bool,
}

pub(in crate::worker) struct PendingServiceWorkerMessageEvent {
    pub(in crate::worker) completion: ServiceWorkerMessageCompletion,
    pub(in crate::worker) pending_wait_until_count: usize,
    pub(in crate::worker) dispatch_finished: bool,
    pub(in crate::worker) window_interaction_allowed: bool,
}

pub(in crate::worker) struct PendingServiceWorkerNotificationEvent {
    pub(in crate::worker) completion: crate::runtime::ServiceWorkerNotificationCompletion,
    pub(in crate::worker) pending_wait_until_count: usize,
    pub(in crate::worker) dispatch_finished: bool,
    pub(in crate::worker) window_interaction_allowed: bool,
}

pub(in crate::worker) struct PendingServiceWorkerPushEvent {
    pub(in crate::worker) completion: crate::runtime::ServiceWorkerPushCompletion,
    pub(in crate::worker) pending_wait_until_count: usize,
    pub(in crate::worker) dispatch_finished: bool,
}

pub(in crate::worker) struct PendingServiceWorkerSyncEvent {
    pub(in crate::worker) completion: crate::runtime::ServiceWorkerSyncCompletion,
    pub(in crate::worker) pending_wait_until_count: usize,
    pub(in crate::worker) dispatch_finished: bool,
}

pub(in crate::worker) struct PendingServiceWorkerPeriodicSyncEvent {
    pub(in crate::worker) completion: crate::runtime::ServiceWorkerPeriodicSyncCompletion,
    pub(in crate::worker) pending_wait_until_count: usize,
    pub(in crate::worker) dispatch_finished: bool,
}

pub(in crate::worker) struct PendingServiceWorkerFetchEvent {
    pub(in crate::worker) completion: ServiceWorkerFetchCompletion,
    pub(in crate::worker) handled_resolver: Option<v8::Global<v8::PromiseResolver>>,
    pub(in crate::worker) request_signal_id: Option<u32>,
    pub(in crate::worker) request_mode: RequestMode,
    pub(in crate::worker) request_destination: ServiceWorkerRequestDestination,
    pub(in crate::worker) pending_respond_with_response:
        Option<crate::network_host::MaterializedResponseHead>,
    pub(in crate::worker) pending_respond_with_stream_body_source_id: Option<NetworkBodySourceId>,
    pub(in crate::worker) pending_respond_with_stream_cancel_handle: Option<v8::Global<v8::Object>>,
    pub(in crate::worker) respond_with_called: bool,
    pub(in crate::worker) pending_respond_with: bool,
    pub(in crate::worker) pending_wait_until_count: usize,
    pub(in crate::worker) dispatch_finished: bool,
}

impl PendingServiceWorkerFetchEvent {
    pub(in crate::worker) fn fallback(
        completion: ServiceWorkerFetchCompletion,
        request_mode: RequestMode,
        request_destination: ServiceWorkerRequestDestination,
    ) -> Self {
        Self {
            completion: ServiceWorkerFetchCompletion {
                result: ServiceWorkerFetchResult::Fallback,
                ..completion
            },
            handled_resolver: None,
            request_signal_id: None,
            request_mode,
            request_destination,
            pending_respond_with_response: None,
            pending_respond_with_stream_body_source_id: None,
            pending_respond_with_stream_cancel_handle: None,
            respond_with_called: false,
            pending_respond_with: false,
            pending_wait_until_count: 0,
            dispatch_finished: false,
        }
    }
}

pub(in crate::worker) struct PendingServiceWorkerNavigationPreload {
    pub(in crate::worker) owner: crate::service_worker_runtime::ServiceWorkerRunOwner,
    pub(in crate::worker) _promise: v8::Global<v8::Promise>,
    pub(in crate::worker) resolver: Option<v8::Global<v8::PromiseResolver>>,
    pub(in crate::worker) body_source_id: Option<NetworkBodySourceId>,
}

pub(super) fn install_service_worker_extendable_event_constructors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let extendable_template = ExtendableEventTemplateDeclaration::build(scope);
    let extendable_ctor = extendable_template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build ExtendableEvent constructor"))?;
    let extendable_proto = constructor_prototype(scope, extendable_ctor, "ExtendableEvent")?;
    set_worker_to_string_tag(scope, extendable_proto, "ExtendableEvent");
    if let Some(event_ctor) = global_constructor_object(scope, "Event") {
        let _ = extendable_ctor.set_prototype(scope, event_ctor.into());
    }
    if let Some(event_proto) = global_constructor_prototype(scope, "Event") {
        let _ = extendable_proto.set_prototype(scope, event_proto.into());
    }
    ExtendableEventConstructorGlobalDeclaration::new(extendable_ctor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize ExtendableEvent global: {error}"))?;

    let message_template = ExtendableMessageEventTemplateDeclaration::build(scope);
    let message_ctor = message_template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build ExtendableMessageEvent constructor"))?;
    let message_proto = constructor_prototype(scope, message_ctor, "ExtendableMessageEvent")?;
    set_worker_to_string_tag(scope, message_proto, "ExtendableMessageEvent");
    let _ = message_ctor.set_prototype(scope, extendable_ctor.into());
    let _ = message_proto.set_prototype(scope, extendable_proto.into());
    ExtendableMessageEventConstructorGlobalDeclaration::new(message_ctor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize ExtendableMessageEvent global: {error}"))?;

    ensure_worker_interface_constructor(scope, "ServiceWorker")?;
    web_api_interfaces::WindowClient::DESCRIPTOR.register(scope)?;
    ensure_worker_interface_constructor(scope, "Client")?;
    ensure_worker_interface_constructor(scope, "WindowClient")?;
    if let Some(client_ctor) = global_constructor_object(scope, "Client")
        && let Some(window_ctor) = global_constructor_object(scope, "WindowClient")
    {
        let _ = window_ctor.set_prototype(scope, client_ctor.into());
    }
    if let Some(client_proto) = global_constructor_prototype(scope, "Client")
        && let Some(window_proto) = global_constructor_prototype(scope, "WindowClient")
    {
        let _ = window_proto.set_prototype(scope, client_proto.into());
    }

    Ok(())
}

fn extendable_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableEvent': Please use the 'new' operator.",
        );
        return;
    }
    let Some(event_type) = extendable_event_type_argument(scope, &args, "ExtendableEvent") else {
        return;
    };
    let Some((bubbles, cancelable, composed)) =
        extendable_event_init_flags(scope, &args, "ExtendableEvent")
    else {
        return;
    };
    initialize_extendable_event_object(
        scope,
        args.this(),
        &event_type,
        bubbles,
        cancelable,
        composed,
    );
    rv.set(args.this().into());
}

fn extendable_message_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': Please use the 'new' operator.",
        );
        return;
    }
    let Some(event_type) = extendable_event_type_argument(scope, &args, "ExtendableMessageEvent")
    else {
        return;
    };
    let init = extendable_event_init_object(&args);
    let Some((bubbles, cancelable, composed)) =
        extendable_event_init_flags(scope, &args, "ExtendableMessageEvent")
    else {
        return;
    };
    let Some(data) = extendable_message_event_data(scope, init) else {
        return;
    };
    let Some(origin) = extendable_message_event_string_member(scope, init, "origin", "") else {
        return;
    };
    let Some(last_event_id) =
        extendable_message_event_string_member(scope, init, "lastEventId", "")
    else {
        return;
    };
    let Some(source) = extendable_message_event_source(scope, init) else {
        return;
    };
    let Some(ports) = extendable_message_event_ports(scope, init) else {
        return;
    };

    let event = args.this();
    initialize_extendable_event_object(scope, event, &event_type, bubbles, cancelable, composed);
    let _ = ExtendableMessageEventStateDeclaration::new(data, origin, last_event_id, source, ports)
        .initialize(scope, event);
    rv.set(event.into());
}

fn extendable_event_type_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor_name: &'static str,
) -> Option<String> {
    if args.length() == 0 {
        throw_type_error(
            scope,
            &format!("Failed to construct '{constructor_name}': 1 argument required."),
        );
        return None;
    }
    webidl::argument::<webidl::DomString>(
        scope,
        args,
        0,
        webidl::Context::argument(constructor_name, 1),
    )
    .map(Into::into)
    .map_or_else(
        |error| {
            webidl::throw_error(scope, &error);
            None
        },
        Some,
    )
}

fn extendable_event_init_object<'s>(
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    if args.length() <= 1 {
        return None;
    }
    let value = args.get(1);
    if value.is_null_or_undefined() || !value.is_object() {
        None
    } else {
        v8::Local::<v8::Object>::try_from(value).ok()
    }
}

fn extendable_event_init_flags<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor_name: &'static str,
) -> Option<(bool, bool, bool)> {
    let init = extendable_event_init_object(args);
    Some((
        extendable_event_bool_member(scope, init, constructor_name, "bubbles")?,
        extendable_event_bool_member(scope, init, constructor_name, "cancelable")?,
        extendable_event_bool_member(scope, init, constructor_name, "composed")?,
    ))
}

fn extendable_event_bool_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    prefix: &'static str,
    key: &'static str,
) -> Option<bool> {
    let Some(init) = init else {
        return Some(false);
    };
    let value =
        match webidl::property_result(scope, init, key, webidl::Context::member(prefix, key)) {
            Ok(Some(value)) if !value.is_undefined() => value,
            Ok(_) => return Some(false),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
    webidl::convert::<webidl::Boolean>(scope, value, webidl::Context::member(prefix, key))
        .map(Into::into)
        .map_or_else(
            |error| {
                webidl::throw_error(scope, &error);
                None
            },
            Some,
        )
}

fn initialize_extendable_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) {
    // EventTarget dispatch and inherited Event methods require the base
    // event's internal state, in addition to its visible properties.
    crate::context_bootstrap::initialize_event_object(
        scope, event, event_type, bubbles, cancelable,
    );
    ExtendableEventStateDeclaration::new(composed)
        .initialize(scope, event)
        .expect("extendable event state should initialize");
}

fn extendable_message_event_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(init) = init else {
        return Some(v8::null(scope).into());
    };
    match webidl::property_result(
        scope,
        init,
        "data",
        webidl::Context::member("ExtendableMessageEvent", "data"),
    ) {
        Ok(Some(value)) if !value.is_undefined() => Some(value),
        Ok(_) => Some(v8::null(scope).into()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn extendable_message_event_string_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    key: &'static str,
    default: &'static str,
) -> Option<String> {
    let Some(init) = init else {
        return Some(default.to_owned());
    };
    let value = match webidl::property_result(
        scope,
        init,
        key,
        webidl::Context::member("ExtendableMessageEvent", key),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(default.to_owned()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("ExtendableMessageEvent", key),
    )
    .map(Into::into)
    .map_or_else(
        |error| {
            webidl::throw_error(scope, &error);
            None
        },
        Some,
    )
}

fn extendable_message_event_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(init) = init else {
        return Some(v8::null(scope).into());
    };
    let value = match webidl::property_result(
        scope,
        init,
        "source",
        webidl::Context::member("ExtendableMessageEvent", "source"),
    ) {
        Ok(Some(value)) if !value.is_null_or_undefined() => value,
        Ok(_) => return Some(v8::null(scope).into()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': member source is not of type Client, ServiceWorker, or MessagePort.",
        );
        return None;
    };
    if extendable_message_event_source_object_is_valid(scope, object) {
        Some(value)
    } else {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': member source is not of type Client, ServiceWorker, or MessagePort.",
        );
        None
    }
}

fn extendable_message_event_source_object_is_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    crate::context_bootstrap::message_port_id_from_object(scope, object).is_some()
        || get_private_value(scope, object, SERVICE_WORKER_CLIENT_ID_SLOT).is_some()
        || (get_private_value(scope, object, SERVICE_WORKER_VERSION_ID_SLOT).is_some()
            && object
                .get(scope, v8str(scope, "scriptURL").into())
                .is_some_and(|value| value.is_string()))
}

fn extendable_message_event_ports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Array>> {
    let Some(init) = init else {
        return Some(frozen_empty_worker_array(scope));
    };
    let value = match webidl::property_result(
        scope,
        init,
        "ports",
        webidl::Context::member("ExtendableMessageEvent", "ports"),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(frozen_empty_worker_array(scope)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Ok(source) = v8::Local::<v8::Array>::try_from(value) else {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': member ports is not a sequence<MessagePort>.",
        );
        return None;
    };
    let ports = v8::Array::new(scope, source.length() as i32);
    for index in 0..source.length() {
        let port = source.get_index(scope, index)?;
        let Ok(port_object) = v8::Local::<v8::Object>::try_from(port) else {
            throw_type_error(
                scope,
                "Failed to construct 'ExtendableMessageEvent': member ports contains a non-MessagePort value.",
            );
            return None;
        };
        if crate::context_bootstrap::message_port_id_from_object(scope, port_object).is_none() {
            throw_type_error(
                scope,
                "Failed to construct 'ExtendableMessageEvent': member ports contains a non-MessagePort value.",
            );
            return None;
        }
        let _ = ports.set_index(scope, index, port);
    }
    let _ = ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    Some(ports)
}

fn frozen_empty_worker_array<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, 0);
    let _ = array.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    array
}

fn extendable_event_wait_until_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let exception = worker_dom_exception_value(
        scope,
        "ExtendableEvent.waitUntil() was called outside an active event dispatch.",
        "InvalidStateError",
    );
    scope.throw_exception(exception);
}
