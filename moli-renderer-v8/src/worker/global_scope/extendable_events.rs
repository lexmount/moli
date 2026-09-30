//! Service Worker extendable events and their pending dispatch state.
use moli_webapi::events::service_worker::{
    extendable_event_constructor_callback, extendable_message_event_constructor_callback,
};

use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::ExtendableEvent,
    constructor_callback = extendable_event_constructor_callback,
    constructor_length = 1
)]
struct ExtendableEventTemplateDeclaration {
    #[webapi(method = "waitUntil", callback = super::super::thread::service_worker_wait_until_callback, length = 1)]
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
        extendable_ctor
            .set_prototype(scope, event_ctor.into())
            .filter(|set| *set)
            .ok_or_else(|| anyhow!("failed to inherit ExtendableEvent constructor"))?;
    }
    if let Some(event_proto) = global_constructor_prototype(scope, "Event") {
        extendable_proto
            .set_prototype(scope, event_proto.into())
            .filter(|set| *set)
            .ok_or_else(|| anyhow!("failed to inherit ExtendableEvent prototype"))?;
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
    message_ctor
        .set_prototype(scope, extendable_ctor.into())
        .filter(|set| *set)
        .ok_or_else(|| anyhow!("failed to inherit ExtendableMessageEvent constructor"))?;
    message_proto
        .set_prototype(scope, extendable_proto.into())
        .filter(|set| *set)
        .ok_or_else(|| anyhow!("failed to inherit ExtendableMessageEvent prototype"))?;
    ExtendableMessageEventConstructorGlobalDeclaration::new(message_ctor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize ExtendableMessageEvent global: {error}"))?;

    web_api_interfaces::WindowClient::DESCRIPTOR.register(scope)?;
    ensure_worker_interface_constructor(scope, "Client")?;
    ensure_worker_interface_constructor(scope, "WindowClient")?;
    if let Some(client_ctor) = global_constructor_object(scope, "Client")
        && let Some(window_ctor) = global_constructor_object(scope, "WindowClient")
    {
        window_ctor
            .set_prototype(scope, client_ctor.into())
            .filter(|set| *set)
            .ok_or_else(|| anyhow!("failed to inherit WindowClient constructor"))?;
    }
    if let Some(client_proto) = global_constructor_prototype(scope, "Client")
        && let Some(window_proto) = global_constructor_prototype(scope, "WindowClient")
    {
        window_proto
            .set_prototype(scope, client_proto.into())
            .filter(|set| *set)
            .ok_or_else(|| anyhow!("failed to inherit WindowClient prototype"))?;
    }

    Ok(())
}
