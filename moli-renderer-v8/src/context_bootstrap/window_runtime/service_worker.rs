use super::*;
use crate::context_bootstrap::navigator_runtime::{
    SERVICE_WORKER_OWNER_TOKEN_SLOT, service_worker_owner_token_value,
};
use crate::document_runtime::DomHandle;
use crate::native_bridge::OwnerDispatchScope;
use crate::service_worker_runtime::{
    ServiceWorkerNavigationPreloadState, ServiceWorkerNavigationPreloadStateError,
    ServiceWorkerRegistrationError, ServiceWorkerUpdateViaCache,
};
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use crate::worker::WorkerScriptKind;
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject, WebApiObjectDeclaration};

const SERVICE_WORKER_REGISTRATION_SCOPE_SLOT: &str = "__moliServiceWorkerRegistrationScope";
const SERVICE_WORKER_REGISTRATION_EVENTS_SLOT: &str = "__moliServiceWorkerRegistrationEvents";
const SERVICE_WORKER_REGISTRATION_WORKER_SLOT: &str = "__moliServiceWorkerRegistrationWorker";
const SERVICE_WORKER_REGISTRATION_WORKERS_SLOT: &str = "__moliServiceWorkerRegistrationWorkers";
const SERVICE_WORKER_SYNC_MANAGER_SCOPE_SLOT: &str = "__moliServiceWorkerSyncManagerScope";
const SERVICE_WORKER_PERIODIC_SYNC_MANAGER_SCOPE_SLOT: &str =
    "__moliServiceWorkerPeriodicSyncManagerScope";
const SERVICE_WORKER_PUSH_MANAGER_SCOPE_SLOT: &str = "__moliServiceWorkerPushManagerScope";
const SERVICE_WORKER_PUSH_SUBSCRIPTION_SCOPE_SLOT: &str =
    "__moliServiceWorkerPushSubscriptionScope";
const SERVICE_WORKER_NAVIGATION_PRELOAD_MANAGER_SCOPE_SLOT: &str =
    "__moliServiceWorkerNavigationPreloadManagerScope";
const SERVICE_WORKER_WORKER_EVENTS_SLOT: &str = "__moliServiceWorkerEvents";
const SERVICE_WORKER_WORKER_REGISTRATION_SLOT: &str = "__moliServiceWorkerWorkerRegistration";
const SERVICE_WORKER_WORKER_VERSION_ID_SLOT: &str = "__moliServiceWorkerVersionId";
const SERVICE_WORKER_CONTAINER_ONMESSAGE_SLOT: &str = "__moliServiceWorkerContainerOnmessage";
const SERVICE_WORKER_CONTAINER_ONMESSAGEERROR_SLOT: &str =
    "__moliServiceWorkerContainerOnmessageerror";
const SERVICE_WORKER_CONTAINER_ONCONTROLLERCHANGE_SLOT: &str =
    "__moliServiceWorkerContainerOncontrollerchange";
const SERVICE_WORKER_CONTAINER_LISTENERS_SLOT: &str = "__moliServiceWorkerContainerEvents";
const SERVICE_WORKER_CONTAINER_REGISTRATIONS_SLOT: &str =
    "__moliServiceWorkerContainerRegistrations";
const SERVICE_WORKER_CONTAINER_CONTROLLER_SLOT: &str = "__moliServiceWorkerContainerController";

#[derive(WebApiObject)]
#[webapi(interface = "ServiceWorkerRegistration")]
struct ServiceWorkerRegistrationObjectDeclaration<'scope> {
    #[webapi(data_property, readonly)]
    scope: String,

    #[webapi(data_property = "updateViaCache", readonly)]
    update_via_cache: &'static str,

    #[webapi(accessor_property, getter = service_worker_registration_installing_getter_callback)]
    installing: (),

    #[webapi(accessor_property, getter = service_worker_registration_waiting_getter_callback)]
    waiting: (),

    #[webapi(accessor_property, getter = service_worker_registration_active_getter_callback)]
    active: (),

    #[webapi(method, callback = navigator_service_worker_unregister_callback, length = 0)]
    unregister: (),

    #[webapi(
        method = "showNotification",
        callback = service_worker_registration_show_notification_callback,
        length = 1
    )]
    show_notification: (),

    #[webapi(
        method = "getNotifications",
        callback = service_worker_registration_get_notifications_callback,
        length = 0
    )]
    get_notifications: (),

    #[webapi(data_property, readonly)]
    sync: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(data_property = "periodicSync", readonly)]
    periodic_sync: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(data_property = "pushManager", readonly)]
    push_manager: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(data_property = "navigationPreload", readonly)]
    navigation_preload: Option<v8::Local<'scope, v8::Object>>,

    #[webapi(
        method = "addEventListener",
        callback = service_worker_registration_add_event_listener_callback,
        length = 2
    )]
    add_event_listener: (),
    #[webapi(
        method = "removeEventListener",
        callback = simple_event_target_remove_event_listener_callback,
        length = 2
    )]
    remove_event_listener: (),
    #[webapi(
        method = "dispatchEvent",
        callback = simple_event_target_dispatch_event_callback,
        length = 1
    )]
    dispatch_event: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerNavigationPreloadStateDeclaration {
    #[webapi(data_property, enumerable)]
    enabled: bool,

    #[webapi(data_property = "headerValue", enumerable)]
    header_value: String,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerPushSubscriptionOptionsDeclaration<'scope> {
    #[webapi(data_property = "userVisibleOnly", readonly)]
    user_visible_only: bool,

    #[webapi(data_property = "applicationServerKey", readonly)]
    application_server_key: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "SyncManager")]
struct ServiceWorkerSyncManagerDeclaration {
    #[webapi(
        method = "register",
        callback = service_worker_sync_manager_register_callback,
        length = 1
    )]
    register: (),

    #[webapi(
        method = "getTags",
        callback = service_worker_sync_manager_get_tags_callback,
        length = 0
    )]
    get_tags: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "PeriodicSyncManager")]
struct ServiceWorkerPeriodicSyncManagerDeclaration {
    #[webapi(
        method = "register",
        callback = service_worker_periodic_sync_manager_register_callback,
        length = 1
    )]
    register: (),

    #[webapi(
        method = "getTags",
        callback = service_worker_periodic_sync_manager_get_tags_callback,
        length = 0
    )]
    get_tags: (),

    #[webapi(
        method = "unregister",
        callback = service_worker_periodic_sync_manager_unregister_callback,
        length = 1
    )]
    unregister: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "PushManager")]
struct ServiceWorkerPushManagerDeclaration {
    #[webapi(
        method = "subscribe",
        callback = service_worker_push_manager_subscribe_callback,
        length = 0
    )]
    subscribe: (),

    #[webapi(
        method = "getSubscription",
        callback = service_worker_push_manager_get_subscription_callback,
        length = 0
    )]
    get_subscription: (),

    #[webapi(
        method = "permissionState",
        callback = service_worker_push_manager_permission_state_callback,
        length = 0
    )]
    permission_state: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "NavigationPreloadManager")]
struct ServiceWorkerNavigationPreloadManagerDeclaration {
    #[webapi(
        method,
        callback = service_worker_navigation_preload_manager_enable_callback,
        length = 0
    )]
    enable: (),

    #[webapi(
        method,
        callback = service_worker_navigation_preload_manager_disable_callback,
        length = 0
    )]
    disable: (),

    #[webapi(
        method = "setHeaderValue",
        callback = service_worker_navigation_preload_manager_set_header_value_callback,
        length = 1
    )]
    set_header_value: (),

    #[webapi(
        method = "getState",
        callback = service_worker_navigation_preload_manager_get_state_callback,
        length = 0
    )]
    get_state: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "PushSubscription")]
struct ServiceWorkerPushSubscriptionDeclaration<'scope> {
    #[webapi(data_property, readonly)]
    endpoint: String,

    #[webapi(data_property = "expirationTime", readonly)]
    expiration_time: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, readonly)]
    options: v8::Local<'scope, v8::Object>,

    #[webapi(method, callback = service_worker_push_subscription_unsubscribe_callback, length = 0)]
    unsubscribe: (),

    #[webapi(
        method = "toJSON",
        callback = service_worker_push_subscription_to_json_callback,
        length = 0
    )]
    to_json: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "ServiceWorker")]
struct ServiceWorkerObjectDeclaration {
    #[webapi(data_property = "scriptURL", readonly)]
    script_url: String,

    #[webapi(data_property, readonly)]
    state: &'static str,

    #[webapi(method = "postMessage", callback = service_worker_post_message_callback, length = 1)]
    post_message: (),

    #[webapi(
        method = "addEventListener",
        callback = service_worker_worker_add_event_listener_callback,
        length = 2
    )]
    add_event_listener: (),

    #[webapi(
        method = "removeEventListener",
        callback = simple_event_target_remove_event_listener_callback,
        length = 2
    )]
    remove_event_listener: (),

    #[webapi(
        method = "dispatchEvent",
        callback = simple_event_target_dispatch_event_callback,
        length = 1
    )]
    dispatch_event: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerMessageEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    origin: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerSimpleEventDeclaration<'scope> {
    #[webapi(data_property = "type", enumerable)]
    event_type: v8::Local<'scope, v8::String>,
}

fn bind_declared_service_worker_object<'s, D>(
    scope: &mut v8::PinScope<'s, '_>,
    declaration: &D,
) -> v8::Local<'s, v8::Object>
where
    D: WebApiObjectDeclaration<'s>,
{
    match declaration.bind(scope) {
        Ok(object) => object,
        Err(_) => {
            let object = v8::Object::new(scope);
            let _ = declaration.initialize(scope, object);
            object
        }
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "BackgroundSyncOptions")]
struct BackgroundSyncOptions {
    #[webidl(
        name = "minInterval",
        converter = "enforce_range_unsigned_long_long",
        default = 0
    )]
    min_interval: u64,
}

#[derive(Clone, Copy)]
enum ServiceWorkerRegistrationPhase<'a> {
    Snapshot(&'a crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot),
}

pub(in crate::context_bootstrap) fn navigator_service_worker_register_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(scope, resolver, "failed to get native bridge");
        rv.set(promise.into());
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let owner = service_worker_container_owner_scope(scope, args.this());
    let Some(request_context) = host.service_worker_window_request_context(owner) else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "service worker document is no longer current",
        );
        rv.set(promise.into());
        return;
    };
    let Some(script_url) = service_worker_script_url(scope, request_context.document_url(), &args)
    else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to resolve service worker script URL",
        );
        rv.set(promise.into());
        return;
    };
    let script_kind = match service_worker_script_kind(scope, &args) {
        Ok(script_kind) => script_kind,
        Err(message) => {
            reject_service_worker_promise(scope, resolver, message);
            rv.set(promise.into());
            return;
        }
    };
    let update_via_cache = match service_worker_update_via_cache(scope, &args) {
        Ok(update_via_cache) => update_via_cache,
        Err(message) => {
            reject_service_worker_promise(scope, resolver, message);
            rv.set(promise.into());
            return;
        }
    };
    let scope_url =
        service_worker_scope_url(scope, request_context.document_url(), &script_url, &args);
    let Some(request_client) = host
        .document_resource_loader_for_window_owner(request_context.owner().window_document_owner())
        .map(|loader| loader.request_client().clone())
    else {
        reject_service_worker_promise(
            scope,
            resolver,
            "The ServiceWorker registration Document is no longer active.",
        );
        rv.set(promise.into());
        return;
    };
    let (request_id, document_owner, completion_tx) =
        host.register_pending_service_worker_register(scope, resolver, request_context.owner());
    host.start_service_worker_runtime(
        script_url,
        scope_url,
        script_kind,
        update_via_cache,
        &request_context,
        request_client,
        request_id,
        document_owner,
        completion_tx,
    );
    rv.set(promise.into());
}

pub(crate) fn settle_service_worker_register_completion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    owner: OwnerDispatchScope,
    result: std::result::Result<
        crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot,
        ServiceWorkerRegistrationError,
    >,
) {
    match result {
        Ok(state) => {
            let script_url = state
                .active()
                .or_else(|| state.waiting())
                .or_else(|| state.installing())
                .map(|version| version.script_url().as_str())
                .unwrap_or_else(|| state.scope_url().as_str());
            let registration =
                if let Some(container) = service_worker_container_for_owner(scope, owner) {
                    build_service_worker_registration_object_for_container(
                        scope,
                        container,
                        state.scope_url().as_str(),
                        script_url,
                        ServiceWorkerRegistrationPhase::Snapshot(&state),
                    )
                } else {
                    build_service_worker_registration_object(
                        scope,
                        owner,
                        state.scope_url().as_str(),
                        script_url,
                        ServiceWorkerRegistrationPhase::Snapshot(&state),
                    )
                };
            let _ = resolver.resolve(scope, registration.into());
        }
        Err(error) => {
            reject_service_worker_registration_promise(scope, resolver, error);
        }
    }
}

pub(crate) fn settle_service_worker_ready_completion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    owner: OwnerDispatchScope,
    registration_snapshot: crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot,
) {
    let script_url = service_worker_registration_snapshot_script_url(&registration_snapshot);
    let registration = if let Some(container) = service_worker_container_for_owner(scope, owner) {
        build_service_worker_registration_object_for_container(
            scope,
            container,
            registration_snapshot.scope_url().as_str(),
            script_url,
            ServiceWorkerRegistrationPhase::Snapshot(&registration_snapshot),
        )
    } else {
        build_service_worker_registration_object(
            scope,
            owner,
            registration_snapshot.scope_url().as_str(),
            script_url,
            ServiceWorkerRegistrationPhase::Snapshot(&registration_snapshot),
        )
    };
    let _ = resolver.resolve(scope, registration.into());
}

/// Whether one internal ServiceWorker event-dispatch pass entered callback
/// code.
///
/// The dispatch helper reports only the body fact. It does not decide when
/// the surrounding scheduler task performs its microtask checkpoint or
/// callback follow-up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerInternalEventCallbackDispatchEffect {
    CallbackBodyDispatched,
    NoCallbackBodyDispatched,
}

impl ServiceWorkerInternalEventCallbackDispatchEffect {
    fn merge(self, other: Self) -> Self {
        if matches!(self, Self::CallbackBodyDispatched)
            || matches!(other, Self::CallbackBodyDispatched)
        {
            Self::CallbackBodyDispatched
        } else {
            Self::NoCallbackBodyDispatched
        }
    }
}

pub(crate) fn dispatch_service_worker_lifecycle_notification(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut crate::native_bridge::JsContextHost,
    notification: crate::types::ServiceWorkerLifecycleNotification,
) -> ServiceWorkerInternalEventCallbackDispatchEffect {
    let mut callback_effect =
        ServiceWorkerInternalEventCallbackDispatchEffect::NoCallbackBodyDispatched;
    let registrations = host.service_worker_registration_watchers_for_lifecycle(
        scope,
        notification.registration.scope_url(),
        &notification.storage_key,
        notification.document_owner,
    );
    for (owner, registration) in registrations {
        let previous_owner_context = owner.enter(scope);
        for event in &notification.events {
            match event {
                crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound => {
                    if let Some(installing) = notification.registration.installing() {
                        let _ = service_worker_registration_worker_for_snapshot_version(
                            scope,
                            registration,
                            installing,
                        );
                    }
                    callback_effect = callback_effect.merge(dispatch_service_worker_simple_event(
                        scope,
                        registration,
                        SERVICE_WORKER_REGISTRATION_EVENTS_SLOT,
                        "updatefound",
                    ));
                }
                crate::types::ServiceWorkerLifecycleClientEvent::WorkerStateChanged {
                    version_id,
                    state,
                } => {
                    if let Some(worker) = service_worker_registration_cached_worker_for_version(
                        scope,
                        registration,
                        *version_id,
                    ) {
                        service_worker_worker_set_state(scope, worker, state);
                        callback_effect =
                            callback_effect.merge(dispatch_service_worker_simple_event(
                                scope,
                                worker,
                                SERVICE_WORKER_WORKER_EVENTS_SLOT,
                                "statechange",
                            ));
                    }
                }
            }
        }
        owner.restore(scope, previous_owner_context);
    }
    callback_effect
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientMessageDispatchEffect {
    MessageDispatched {
        callback_effect: ServiceWorkerClientMessageCallbackDispatchEffect,
    },
    MessageErrorDispatched {
        callback_effect: ServiceWorkerClientMessageCallbackDispatchEffect,
    },
    /// The exact Window client remained current, but dispatch could not
    /// produce an event target/event pair. This deliberately does not claim
    /// whether the container was absent or event construction failed.
    CurrentTargetProducedNoDispatchableEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientMessageCallbackDispatchEffect {
    /// At least one ordered handler/listener was registered for this event
    /// type when dispatch began.
    CallbackDispatched,
    /// The event target and event were valid, but dispatch began without a
    /// matching ordered handler/listener.
    CurrentTargetHadNoCallback,
}

impl ServiceWorkerClientMessageDispatchEffect {
    const fn dispatched(self) -> bool {
        matches!(
            self,
            Self::MessageDispatched { .. } | Self::MessageErrorDispatched { .. }
        )
    }
}

pub(crate) fn dispatch_service_worker_client_message_body(
    scope: &mut v8::PinScope<'_, '_>,
    owner: crate::native_bridge::OwnerDispatchScope,
    completion: crate::types::ServiceWorkerClientMessageCompletion,
) -> ServiceWorkerClientMessageDispatchEffect {
    let Some(container) = service_worker_container_for_owner(scope, owner) else {
        return ServiceWorkerClientMessageDispatchEffect::CurrentTargetProducedNoDispatchableEvent;
    };
    let Some(context) = container.get_creation_context(scope) else {
        return ServiceWorkerClientMessageDispatchEffect::CurrentTargetProducedNoDispatchableEvent;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let previous_owner_context = owner.enter(scope);
    let effect =
        dispatch_service_worker_client_message_in_owner_scope(scope, container, completion);
    if effect.dispatched() {
        owner.defer_restore(scope, previous_owner_context);
    } else {
        owner.restore(scope, previous_owner_context);
    }
    effect
}

fn dispatch_service_worker_client_message_in_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
    completion: crate::types::ServiceWorkerClientMessageCompletion,
) -> ServiceWorkerClientMessageDispatchEffect {
    let source = service_worker_container_cached_worker_for_version(
        scope,
        container,
        completion.source_version_id,
    )
    .unwrap_or_else(|| {
        let source = build_service_worker_object(
            scope,
            completion.source_script_url.as_str(),
            completion.source_state,
        );
        service_worker_worker_set_version_id(scope, source, completion.source_version_id);
        source
    });
    if !crate::context_bootstrap::runtime_message_allowed_for_current_target(
        scope,
        &completion.payload,
    ) {
        return if let Some(callback_effect) =
            dispatch_service_worker_client_messageerror(scope, container, source.into())
        {
            ServiceWorkerClientMessageDispatchEffect::MessageErrorDispatched { callback_effect }
        } else {
            ServiceWorkerClientMessageDispatchEffect::CurrentTargetProducedNoDispatchableEvent
        };
    }
    let Some((data, ports)) =
        crate::context_bootstrap::structured_deserialize_value_for_message_event(
            scope,
            &completion.payload,
        )
    else {
        return if let Some(callback_effect) =
            dispatch_service_worker_client_messageerror(scope, container, source.into())
        {
            ServiceWorkerClientMessageDispatchEffect::MessageErrorDispatched { callback_effect }
        } else {
            ServiceWorkerClientMessageDispatchEffect::CurrentTargetProducedNoDispatchableEvent
        };
    };
    let event = new_service_worker_message_event(
        scope,
        "message",
        data,
        completion
            .source_script_url
            .origin()
            .ascii_serialization()
            .as_str(),
        source.into(),
        ports,
    );
    if let Some(event) = event {
        let callback_effect =
            service_worker_client_message_callback_dispatch_effect(scope, container, "message");
        let _ = dispatch_simple_event_target_event(
            scope,
            container,
            SERVICE_WORKER_CONTAINER_LISTENERS_SLOT,
            "message",
            event,
        );
        return ServiceWorkerClientMessageDispatchEffect::MessageDispatched { callback_effect };
    }
    ServiceWorkerClientMessageDispatchEffect::CurrentTargetProducedNoDispatchableEvent
}

fn dispatch_service_worker_client_messageerror<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Value>,
) -> Option<ServiceWorkerClientMessageCallbackDispatchEffect> {
    let origin = crate::context_bootstrap::current_runtime_message_origin(scope)
        .unwrap_or_else(|| "null".to_owned());
    let event = new_service_worker_message_event(
        scope,
        "messageerror",
        v8::null(scope).into(),
        &origin,
        source,
        v8::Array::new(scope, 0),
    );
    if let Some(event) = event {
        let callback_effect = service_worker_client_message_callback_dispatch_effect(
            scope,
            container,
            "messageerror",
        );
        let _ = dispatch_simple_event_target_event(
            scope,
            container,
            SERVICE_WORKER_CONTAINER_LISTENERS_SLOT,
            "messageerror",
            event,
        );
        return Some(callback_effect);
    }
    None
}

fn service_worker_client_message_callback_dispatch_effect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> ServiceWorkerClientMessageCallbackDispatchEffect {
    if crate::context_bootstrap::simple_object_event_listeners_snapshot(
        scope,
        container,
        SERVICE_WORKER_CONTAINER_LISTENERS_SLOT,
        event_type,
    )
    .is_empty()
    {
        ServiceWorkerClientMessageCallbackDispatchEffect::CurrentTargetHadNoCallback
    } else {
        ServiceWorkerClientMessageCallbackDispatchEffect::CallbackDispatched
    }
}

pub(crate) fn dispatch_service_worker_controller_change(
    scope: &mut v8::PinScope<'_, '_>,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> ServiceWorkerInternalEventCallbackDispatchEffect {
    let previous_owner_context = owner.enter(scope);
    let Some(container) = service_worker_container_for_owner(scope, owner) else {
        owner.restore(scope, previous_owner_context);
        return ServiceWorkerInternalEventCallbackDispatchEffect::NoCallbackBodyDispatched;
    };
    let callback_effect = dispatch_service_worker_simple_event(
        scope,
        container,
        SERVICE_WORKER_CONTAINER_LISTENERS_SLOT,
        "controllerchange",
    );
    owner.defer_restore(scope, previous_owner_context);
    callback_effect
}

fn service_worker_container_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: crate::native_bridge::OwnerDispatchScope,
) -> Option<v8::Local<'s, v8::Object>> {
    let window = match owner {
        crate::native_bridge::OwnerDispatchScope::Top => scope.get_current_context().global(scope),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            let host_ptr = context_host_ptr_from_global_bridge(scope)?;
            unsafe { &mut *host_ptr }.child_browsing_context_window_wrapper(scope, handle)?
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            let host_ptr = context_host_ptr_from_global_bridge(scope)?;
            unsafe { &*host_ptr }.lightweight_popup_window(scope, popup_id)?
        }
    };
    service_worker_container_from_window(scope, window)
}

fn service_worker_container_from_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let navigator = window
        .get(scope, v8str(scope, "navigator").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    navigator
        .get(scope, v8str(scope, "serviceWorker").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn service_worker_container_registration_cache<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    if let Some(cache) = get_private_value(
        scope,
        container,
        SERVICE_WORKER_CONTAINER_REGISTRATIONS_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return cache;
    }
    let cache = v8::Array::new(scope, 0);
    set_private_value(
        scope,
        container,
        SERVICE_WORKER_CONTAINER_REGISTRATIONS_SLOT,
        cache.into(),
    );
    cache
}

fn service_worker_container_cached_registration_for_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
    scope_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache = service_worker_container_registration_cache(scope, container);
    for index in 0..cache.length() {
        let Some(registration) = cache
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        let Some(value) =
            object_own_hidden_value(scope, registration, SERVICE_WORKER_REGISTRATION_SCOPE_SLOT)
        else {
            continue;
        };
        if value
            .to_string(scope)
            .is_some_and(|value| value.to_rust_string_lossy(scope) == scope_url)
        {
            return Some(registration);
        }
    }
    None
}

fn remember_service_worker_container_registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
    registration: v8::Local<'s, v8::Object>,
) {
    let Some(scope_value) =
        object_own_hidden_value(scope, registration, SERVICE_WORKER_REGISTRATION_SCOPE_SLOT)
    else {
        return;
    };
    let Some(scope_url) = scope_value.to_string(scope) else {
        return;
    };
    let scope_url = scope_url.to_rust_string_lossy(scope);
    if service_worker_container_cached_registration_for_scope(scope, container, &scope_url)
        .is_some()
    {
        return;
    }
    let cache = service_worker_container_registration_cache(scope, container);
    array_push_value(scope, cache, registration.into());
}

fn service_worker_container_cached_worker_for_version<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
    version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache = service_worker_container_registration_cache(scope, container);
    for index in 0..cache.length() {
        let Some(registration) = cache
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if let Some(worker) =
            service_worker_registration_cached_worker_for_version(scope, registration, version_id)
        {
            return Some(worker);
        }
    }
    None
}

pub(in crate::context_bootstrap) fn navigator_service_worker_message_handler_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), SERVICE_WORKER_CONTAINER_ONMESSAGE_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(in crate::context_bootstrap) fn navigator_service_worker_message_handler_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_ONMESSAGE_SLOT,
        stored,
    );
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_LISTENERS_SLOT,
        "message",
        SERVICE_WORKER_CONTAINER_ONMESSAGE_SLOT,
        stored.is_function(),
    );
}

pub(in crate::context_bootstrap) fn navigator_service_worker_messageerror_handler_getter_callback<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = get_private_value(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_ONMESSAGEERROR_SLOT,
    )
    .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(in crate::context_bootstrap) fn navigator_service_worker_messageerror_handler_setter_callback<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_ONMESSAGEERROR_SLOT,
        stored,
    );
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_LISTENERS_SLOT,
        "messageerror",
        SERVICE_WORKER_CONTAINER_ONMESSAGEERROR_SLOT,
        stored.is_function(),
    );
}

pub(in crate::context_bootstrap) fn navigator_service_worker_controllerchange_handler_getter_callback<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = get_private_value(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_ONCONTROLLERCHANGE_SLOT,
    )
    .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(in crate::context_bootstrap) fn navigator_service_worker_controllerchange_handler_setter_callback<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = args.get(0);
    let stored = if value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_ONCONTROLLERCHANGE_SLOT,
        stored,
    );
    simple_object_event_set_ordered_handler(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_LISTENERS_SLOT,
        "controllerchange",
        SERVICE_WORKER_CONTAINER_ONCONTROLLERCHANGE_SLOT,
        stored.is_function(),
    );
}

pub(in crate::context_bootstrap) fn navigator_service_worker_get_registration_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let value: v8::Local<'_, v8::Value> = if let Some(host_ptr) =
        context_host_ptr_from_global_bridge(scope)
    {
        let host = unsafe { &mut *host_ptr };
        let owner = service_worker_container_owner_scope(scope, args.this());
        let state = host
            .service_worker_window_request_context(owner)
            .and_then(|request_context| {
                service_worker_client_url(scope, request_context.document_url(), &args).and_then(
                    |client_url| {
                        host.service_worker_registration_for_client(&request_context, &client_url)
                    },
                )
            });
        if let Some(state) = state {
            build_service_worker_registration_object_for_container(
                scope,
                args.this(),
                state.scope_url().as_str(),
                service_worker_registration_snapshot_script_url(&state),
                ServiceWorkerRegistrationPhase::Snapshot(&state),
            )
            .into()
        } else {
            v8::undefined(scope).into()
        }
    } else {
        v8::undefined(scope).into()
    };
    let _ = resolver.resolve(scope, value);
    rv.set(resolver.get_promise(scope).into());
}

pub(in crate::context_bootstrap) fn navigator_service_worker_get_registrations_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let owner = service_worker_container_owner_scope(scope, args.this());
    let states = context_host_ptr_from_global_bridge(scope)
        .and_then(|host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let request_context = host.service_worker_window_request_context(owner)?;
            Some(host.service_worker_registrations(&request_context))
        })
        .unwrap_or_default();
    let registrations = v8::Array::new(scope, states.len() as i32);
    for (index, state) in states.into_iter().enumerate() {
        let registration = build_service_worker_registration_object_for_container(
            scope,
            args.this(),
            state.scope_url().as_str(),
            service_worker_registration_snapshot_script_url(&state),
            ServiceWorkerRegistrationPhase::Snapshot(&state),
        );
        let value: v8::Local<'_, v8::Value> = registration.into();
        let _ = registrations.set_index(scope, index as u32, value);
    }
    let _ = resolver.resolve(scope, registrations.into());
    rv.set(resolver.get_promise(scope).into());
}

pub(in crate::context_bootstrap) fn navigator_service_worker_controller_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let owner = service_worker_container_owner_scope(scope, args.this());
    let state = context_host_ptr_from_global_bridge(scope)
        .and_then(|host_ptr| {
            unsafe { &*host_ptr }.service_worker_control_state_for_window_owner(owner)
        })
        .or_else(|| crate::worker::worker_service_worker_control_state(scope));
    let Some(state) = state else {
        rv.set(v8::null(scope).into());
        return;
    };
    let cached_for_version = state.active_version_id().and_then(|version_id| {
        service_worker_container_cached_worker_for_version(scope, args.this(), version_id).or_else(
            || {
                get_private_value(scope, args.this(), SERVICE_WORKER_CONTAINER_CONTROLLER_SLOT)
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                    .filter(|worker| {
                        service_worker_worker_matches_version(scope, *worker, version_id)
                    })
            },
        )
    });
    let controller = cached_for_version
        .unwrap_or_else(|| build_service_worker_controller_object(scope, &state, owner));
    service_worker_worker_set_state(scope, controller, "activated");
    service_worker_worker_set_owner_scope(scope, controller, owner);
    set_private_value(
        scope,
        args.this(),
        SERVICE_WORKER_CONTAINER_CONTROLLER_SLOT,
        controller.into(),
    );
    rv.set(controller.into());
}

fn service_worker_container_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
) -> OwnerDispatchScope {
    service_worker_owner_scope_from_object(scope, container)
}

fn navigator_service_worker_unregister_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    let scope_url = service_worker_registration_scope_from_this(scope, args.this());
    let active_worker = service_worker_registration_active_worker(scope, args.this());
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        settle_service_worker_unregister_completion(
            scope,
            resolver,
            Some(args.this()),
            active_worker,
            false,
        );
        rv.set(promise.into());
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let owner = service_worker_owner_scope_from_object(scope, args.this());
    let Some(request_context) = host.service_worker_window_request_context(owner) else {
        settle_service_worker_unregister_completion(
            scope,
            resolver,
            Some(args.this()),
            active_worker,
            false,
        );
        rv.set(promise.into());
        return;
    };
    let (request_id, document_owner, completion_tx) = host
        .register_pending_service_worker_unregister(
            scope,
            resolver,
            args.this(),
            active_worker,
            request_context.owner(),
        );
    let start = match scope_url.as_ref() {
        Some(scope_url) => host.unregister_service_worker_scope(
            &request_context,
            scope_url,
            request_id,
            document_owner,
            completion_tx,
        ),
        None => host.unregister_service_worker_control(
            &request_context,
            request_id,
            document_owner,
            completion_tx,
        ),
    };
    if let crate::service_worker_runtime::ServiceWorkerUnregisterStart::Completed(removed) = start {
        let _ = host.take_pending_service_worker_unregister(request_id);
        settle_service_worker_unregister_completion(
            scope,
            resolver,
            Some(args.this()),
            active_worker,
            removed,
        );
    }
    rv.set(promise.into());
}

fn service_worker_registration_show_notification_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(title) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("ServiceWorkerRegistration.showNotification", 1),
        "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(options) = crate::context_bootstrap::notification_options_payload(scope, args.get(1))
    else {
        return;
    };
    let Some(scope_url) = service_worker_registration_scope_from_this(scope, args.this()) else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for showNotification",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    if host.permission_state("notifications") != "granted" {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': notification permission has not been granted.",
        );
        return;
    }
    if !host.show_service_worker_notification(
        &scope_url,
        title.0,
        options.tag,
        options.metadata,
        options.actions,
        options.data,
    ) {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': no active Service Worker registration is available.",
        );
        return;
    }
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

fn service_worker_registration_get_notifications_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = crate::context_bootstrap::notification_get_options_tag(scope, args.get(0))
    else {
        return;
    };
    let Some(scope_url) = service_worker_registration_scope_from_this(scope, args.this()) else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'getNotifications' on 'ServiceWorkerRegistration': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for getNotifications",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    let notifications = host.service_worker_notifications(&scope_url, tag.as_deref());
    let array = v8::Array::new(scope, notifications.len() as i32);
    for (index, notification) in notifications.iter().enumerate() {
        if let Some(object) =
            crate::context_bootstrap::build_notification_object_from_snapshot(scope, notification)
        {
            let _ = array.set_index(scope, index as u32, object.into());
        }
    }
    let _ = resolver.resolve(scope, array.into());
}

fn service_worker_navigation_preload_manager_enable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    service_worker_navigation_preload_manager_set_enabled(scope, args, rv, true);
}

fn service_worker_navigation_preload_manager_disable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    service_worker_navigation_preload_manager_set_enabled(scope, args, rv, false);
}

fn service_worker_navigation_preload_manager_set_enabled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    enabled: bool,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute navigation preload operation: registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for navigation preload",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    match host.set_service_worker_navigation_preload_enabled(&scope_url, enabled) {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(error) => reject_service_worker_navigation_preload_state_error(scope, resolver, error),
    }
}

fn service_worker_navigation_preload_manager_set_header_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let header_value = match service_worker_navigation_preload_header_value(scope, &args) {
        Ok(header_value) => header_value,
        Err(error) => {
            reject_service_worker_promise_with_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    if !service_worker_navigation_preload_valid_header_value(&header_value) {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "The string provided to setHeaderValue is not a valid HTTP header field value.",
        );
        return;
    }
    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'setHeaderValue' on 'NavigationPreloadManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for NavigationPreloadManager.setHeaderValue",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    match host.set_service_worker_navigation_preload_header_value(&scope_url, header_value) {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(error) => reject_service_worker_navigation_preload_state_error(scope, resolver, error),
    }
}

fn service_worker_navigation_preload_manager_get_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'getState' on 'NavigationPreloadManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for NavigationPreloadManager.getState",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    let Some(state) = host.service_worker_navigation_preload_state(&scope_url) else {
        reject_service_worker_navigation_preload_state_error(
            scope,
            resolver,
            ServiceWorkerNavigationPreloadStateError::InvalidState,
        );
        return;
    };
    let state_object = build_service_worker_navigation_preload_state_object(scope, &state);
    let _ = resolver.resolve(scope, state_object.into());
}

fn service_worker_sync_manager_register_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("SyncManager.register", 1),
        "Failed to execute 'register' on 'SyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(scope_url) = service_worker_sync_manager_scope_from_this(scope, args.this()) else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'register' on 'SyncManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for background sync",
        );
        return;
    };
    if service_worker_background_sync_permission_state(scope) != "granted" {
        reject_service_worker_promise_with_dom_exception(
            scope,
            resolver,
            "Background Sync permission has not been granted.",
            "NotAllowedError",
        );
        return;
    }
    let host = unsafe { &*host_ptr };
    if !host.register_service_worker_sync(&scope_url, tag.0) {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'register' on 'SyncManager': no active Service Worker registration is available.",
        );
        return;
    }
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

fn service_worker_sync_manager_get_tags_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(scope_url) = service_worker_sync_manager_scope_from_this(scope, args.this()) else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'getTags' on 'SyncManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for background sync",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    let tags = host.service_worker_sync_tags(&scope_url);
    let array = v8::Array::new(scope, tags.len() as i32);
    for (index, tag) in tags.iter().enumerate() {
        if let Some(value) = v8_string(scope, tag) {
            let _ = array.set_index(scope, index as u32, value.into());
        }
    }
    let _ = resolver.resolve(scope, array.into());
}

fn service_worker_periodic_sync_manager_register_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("PeriodicSyncManager.register", 1),
        "Failed to execute 'register' on 'PeriodicSyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(options) = service_worker_periodic_sync_options(scope, args.get(1)) else {
        return;
    };
    let Some(scope_url) = service_worker_periodic_sync_manager_scope_from_this(scope, args.this())
    else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'register' on 'PeriodicSyncManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for periodic background sync",
        );
        return;
    };
    if service_worker_periodic_sync_permission_state(scope) != "granted" {
        reject_service_worker_promise_with_dom_exception(
            scope,
            resolver,
            "Periodic Background Sync permission has not been granted.",
            "NotAllowedError",
        );
        return;
    }
    let host = unsafe { &*host_ptr };
    if !host.register_service_worker_periodic_sync(&scope_url, tag.0, options.min_interval) {
        reject_service_worker_promise_with_dom_exception(
            scope,
            resolver,
            "Registration failed - no active Service Worker",
            "InvalidStateError",
        );
        return;
    }
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

fn service_worker_periodic_sync_manager_get_tags_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(scope_url) = service_worker_periodic_sync_manager_scope_from_this(scope, args.this())
    else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'getTags' on 'PeriodicSyncManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for periodic background sync",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    let tags = host.service_worker_periodic_sync_tags(&scope_url);
    let array = v8::Array::new(scope, tags.len() as i32);
    for (index, tag) in tags.iter().enumerate() {
        if let Some(value) = v8_string(scope, tag) {
            let _ = array.set_index(scope, index as u32, value.into());
        }
    }
    let _ = resolver.resolve(scope, array.into());
}

fn service_worker_periodic_sync_manager_unregister_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("PeriodicSyncManager.unregister", 1),
        "Failed to execute 'unregister' on 'PeriodicSyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(scope_url) = service_worker_periodic_sync_manager_scope_from_this(scope, args.this())
    else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'unregister' on 'PeriodicSyncManager': registration scope is unavailable.",
        );
        return;
    };
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        let _ = host.unregister_service_worker_periodic_sync(&scope_url, &tag.0);
    }
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

fn service_worker_periodic_sync_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<BackgroundSyncOptions> {
    match webidl::parse_dictionary::<BackgroundSyncOptions>(
        scope,
        value,
        webidl::Context::argument("PeriodicSyncManager.register", 2),
    ) {
        Ok(options) => Some(options.unwrap_or_default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn service_worker_background_sync_permission_state(scope: &mut v8::PinScope<'_, '_>) -> String {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return "prompt".to_owned();
    };
    let host = unsafe { &*host_ptr };
    if !moli_url::is_potentially_trustworthy_url(host.document_url()) {
        return "denied".to_owned();
    }
    match host.permission_state("background-sync") {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

fn service_worker_periodic_sync_permission_state(scope: &mut v8::PinScope<'_, '_>) -> String {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return "prompt".to_owned();
    };
    let host = unsafe { &*host_ptr };
    if !moli_url::is_potentially_trustworthy_url(host.document_url()) {
        return "denied".to_owned();
    }
    match host.permission_state("periodic-background-sync") {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

fn service_worker_periodic_sync_manager_scope_from_this(
    scope: &mut v8::PinScope<'_, '_>,
    this: v8::Local<'_, v8::Object>,
) -> Option<url::Url> {
    let value =
        object_own_hidden_value(scope, this, SERVICE_WORKER_PERIODIC_SYNC_MANAGER_SCOPE_SLOT)?;
    let scope_string = value.to_string(scope)?.to_rust_string_lossy(scope);
    url::Url::parse(&scope_string).ok()
}

fn service_worker_navigation_preload_manager_scope_from_this(
    scope: &mut v8::PinScope<'_, '_>,
    this: v8::Local<'_, v8::Object>,
) -> Option<url::Url> {
    let value = object_own_hidden_value(
        scope,
        this,
        SERVICE_WORKER_NAVIGATION_PRELOAD_MANAGER_SCOPE_SLOT,
    )?;
    let scope_string = value.to_string(scope)?.to_rust_string_lossy(scope);
    url::Url::parse(&scope_string).ok()
}

fn service_worker_navigation_preload_header_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<String, webidl::WebIdlError> {
    let context = webidl::Context::argument("NavigationPreloadManager.setHeaderValue", 1);
    if args.length() <= 0 {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::ByteString>(scope, args.get(0), context).map(Into::into)
}

fn service_worker_navigation_preload_valid_header_value(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch as u32 <= 0xff && !matches!(ch, '\0' | '\r' | '\n'))
}

fn service_worker_push_manager_subscribe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    if service_worker_push_permission_state(scope) != "granted" {
        reject_service_worker_promise_with_dom_exception(
            scope,
            resolver,
            "Push permission has not been granted.",
            "NotAllowedError",
        );
        return;
    }
    let Some(scope_url) = service_worker_push_manager_scope_from_this(scope, args.this()) else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'subscribe' on 'PushManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for PushManager.subscribe",
        );
        return;
    };
    let user_visible_only = service_worker_push_subscribe_user_visible_only(scope, args.get(0));
    let host = unsafe { &*host_ptr };
    let Some(subscription) = host.subscribe_service_worker_push(&scope_url, user_visible_only)
    else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'subscribe' on 'PushManager': no active Service Worker registration is available.",
        );
        return;
    };
    let value = build_service_worker_push_subscription_object(scope, &scope_url, &subscription)
        .map(v8::Local::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let _ = resolver.resolve(scope, value);
}

fn service_worker_push_manager_get_subscription_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(scope_url) = service_worker_push_manager_scope_from_this(scope, args.this()) else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'getSubscription' on 'PushManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for PushManager.getSubscription",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    let value = host
        .service_worker_push_subscription(&scope_url)
        .and_then(|subscription| {
            build_service_worker_push_subscription_object(scope, &scope_url, &subscription)
        })
        .map(v8::Local::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let _ = resolver.resolve(scope, value);
}

fn service_worker_push_manager_permission_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let state = service_worker_push_permission_state(scope);
    let value = v8_string(scope, &state)
        .map(v8::Local::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.resolve(scope, value);
}

fn service_worker_push_permission_state(scope: &mut v8::PinScope<'_, '_>) -> String {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return "prompt".to_owned();
    };
    let host = unsafe { &*host_ptr };
    if !moli_url::is_potentially_trustworthy_url(host.document_url()) {
        return "denied".to_owned();
    }
    match host.permission_state("notifications") {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

fn service_worker_push_subscribe_user_visible_only(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    options
        .get(scope, v8str(scope, "userVisibleOnly").into())
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn settle_service_worker_unregister_completion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    registration: Option<v8::Local<'s, v8::Object>>,
    active_worker: Option<v8::Local<'s, v8::Object>>,
    removed: bool,
) {
    let retained_controller =
        registration
            .zip(active_worker)
            .is_some_and(|(registration, active_worker)| {
                let owner = service_worker_owner_scope_from_object(scope, registration);
                let active_version_id =
                    service_worker_worker_version_id_value(scope, active_worker);
                context_host_ptr_from_global_bridge(scope)
                    .and_then(|host_ptr| {
                        unsafe { &*host_ptr }.service_worker_control_state_for_window_owner(owner)
                    })
                    .is_some_and(|state| state.active_version_id() == active_version_id)
            });
    if removed && !retained_controller {
        if let Some(registration) = registration {
            service_worker_registration_clear_workers(scope, registration);
        }
        if let Some(active_worker) = active_worker {
            service_worker_worker_set_state(scope, active_worker, "redundant");
            dispatch_service_worker_simple_event(
                scope,
                active_worker,
                SERVICE_WORKER_WORKER_EVENTS_SLOT,
                "statechange",
            );
        }
    }
    let _ = resolver.resolve(scope, v8::Boolean::new(scope, removed).into());
}

fn reject_service_worker_promise(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    message: &str,
) {
    let message = v8_string(scope, message).unwrap_or_else(|| v8_string(scope, "").unwrap());
    let error = v8::Exception::error(scope, message);
    let _ = resolver.reject(scope, error);
}

fn reject_service_worker_promise_with_type_error(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    message: &str,
) {
    let message = v8_string(scope, message).unwrap_or_else(|| v8_string(scope, "").unwrap());
    let error = v8::Exception::type_error(scope, message);
    let _ = resolver.reject(scope, error);
}

fn reject_service_worker_promise_with_dom_exception(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    message: &str,
    name: &str,
) {
    let error = crate::context_bootstrap::new_dom_exception_value(scope, message, name);
    let _ = resolver.reject(scope, error);
}

fn reject_service_worker_registration_promise(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    error: ServiceWorkerRegistrationError,
) {
    if error.kind.rejects_as_type_error_for_update() {
        reject_service_worker_promise_with_type_error(scope, resolver, &error.message);
    } else {
        reject_service_worker_promise_with_dom_exception(
            scope,
            resolver,
            &error.message,
            error.kind.dom_exception_name(),
        );
    }
}

fn reject_service_worker_navigation_preload_state_error(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    error: ServiceWorkerNavigationPreloadStateError,
) {
    match error {
        ServiceWorkerNavigationPreloadStateError::InvalidState => {
            reject_service_worker_promise_with_dom_exception(
                scope,
                resolver,
                "Registration failed - no active Service Worker",
                "InvalidStateError",
            );
        }
        ServiceWorkerNavigationPreloadStateError::StorageFailure => {
            reject_service_worker_promise_with_type_error(
                scope,
                resolver,
                "Failed to persist navigation preload state.",
            );
        }
    }
}

fn service_worker_script_url(
    scope: &mut v8::PinScope<'_, '_>,
    document_url: &url::Url,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<url::Url> {
    let script = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))?;
    let mut script_url = document_url.join(&script).ok()?;
    script_url.set_fragment(None);
    Some(script_url)
}

fn service_worker_scope_url(
    scope: &mut v8::PinScope<'_, '_>,
    document_url: &url::Url,
    script_url: &url::Url,
    args: &v8::FunctionCallbackArguments<'_>,
) -> url::Url {
    if args.length() > 1
        && let Ok(options) = v8::Local::<v8::Object>::try_from(args.get(1))
        && let Some(scope_value) = options.get(scope, v8str(scope, "scope").into())
        && !scope_value.is_null_or_undefined()
        && let Some(scope_string) = scope_value.to_string(scope)
        && let Ok(mut scope_url) = document_url.join(&scope_string.to_rust_string_lossy(scope))
    {
        scope_url.set_fragment(None);
        return scope_url;
    }
    default_service_worker_scope_url(script_url)
}

fn service_worker_script_kind(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Result<WorkerScriptKind, &'static str> {
    if args.length() <= 1 {
        return Ok(WorkerScriptKind::Classic);
    }
    let value = args.get(1);
    if value.is_null_or_undefined() {
        return Ok(WorkerScriptKind::Classic);
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(WorkerScriptKind::Classic);
    };
    let Some(type_value) = options.get(scope, v8str(scope, "type").into()) else {
        return Ok(WorkerScriptKind::Classic);
    };
    if type_value.is_null_or_undefined() {
        return Ok(WorkerScriptKind::Classic);
    }
    let Some(type_string) = type_value.to_string(scope) else {
        return Err("failed to parse service worker type");
    };
    match type_string.to_rust_string_lossy(scope).as_str() {
        "classic" => Ok(WorkerScriptKind::Classic),
        "module" => Ok(WorkerScriptKind::Module),
        _ => Err("invalid service worker type"),
    }
}

fn service_worker_update_via_cache(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Result<ServiceWorkerUpdateViaCache, &'static str> {
    if args.length() <= 1 {
        return Ok(ServiceWorkerUpdateViaCache::default());
    }
    let value = args.get(1);
    if value.is_null_or_undefined() {
        return Ok(ServiceWorkerUpdateViaCache::default());
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(ServiceWorkerUpdateViaCache::default());
    };
    let Some(update_via_cache_value) = options.get(scope, v8str(scope, "updateViaCache").into())
    else {
        return Ok(ServiceWorkerUpdateViaCache::default());
    };
    if update_via_cache_value.is_null_or_undefined() {
        return Ok(ServiceWorkerUpdateViaCache::default());
    }
    let Some(update_via_cache_string) = update_via_cache_value.to_string(scope) else {
        return Err("failed to parse service worker updateViaCache");
    };
    ServiceWorkerUpdateViaCache::parse_webidl_token(
        &update_via_cache_string.to_rust_string_lossy(scope),
    )
    .ok_or("invalid service worker updateViaCache")
}

fn service_worker_client_url(
    scope: &mut v8::PinScope<'_, '_>,
    document_url: &url::Url,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<url::Url> {
    let client = if args.length() > 0 && !args.get(0).is_null_or_undefined() {
        args.get(0)
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))?
    } else {
        String::new()
    };
    let mut client_url = document_url.join(&client).ok()?;
    client_url.set_fragment(None);
    Some(client_url)
}

fn default_service_worker_scope_url(script_url: &url::Url) -> url::Url {
    let mut scope_url = script_url.clone();
    let path = script_url.path();
    let scope_path = path.rfind('/').map(|index| &path[..=index]).unwrap_or("/");
    scope_url.set_path(scope_path);
    scope_url.set_query(None);
    scope_url.set_fragment(None);
    scope_url
}

fn service_worker_registration_snapshot_script_url(
    snapshot: &crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot,
) -> &str {
    snapshot
        .active()
        .or_else(|| snapshot.waiting())
        .or_else(|| snapshot.installing())
        .map(|version| version.script_url().as_str())
        .unwrap_or_else(|| snapshot.scope_url().as_str())
}

fn service_worker_registration_snapshot_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
) -> Option<crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot> {
    let scope_url = service_worker_registration_scope_from_this(scope, registration)?;
    let owner = service_worker_owner_scope_from_object(scope, registration);
    context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
        let host = unsafe { &mut *host_ptr };
        let request_context = host.service_worker_window_request_context(owner)?;
        host.service_worker_registration_for_client(&request_context, &scope_url)
    })
}

fn service_worker_registration_worker_for_phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
    phase: &'static str,
) -> v8::Local<'s, v8::Value> {
    let Some(snapshot) = service_worker_registration_snapshot_from_object(scope, registration)
    else {
        return v8::null(scope).into();
    };
    let version = match phase {
        "installing" => snapshot.installing(),
        "waiting" => snapshot.waiting(),
        "active" => snapshot.active(),
        _ => None,
    };
    let Some(version) = version else {
        return v8::null(scope).into();
    };
    service_worker_registration_worker_for_snapshot_version(scope, registration, version)
}

fn service_worker_registration_worker_for_snapshot_version<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
    version: &crate::service_worker_runtime::ServiceWorkerVersionSnapshot,
) -> v8::Local<'s, v8::Value> {
    if let Some(worker) = service_worker_registration_cached_worker_for_version(
        scope,
        registration,
        version.version_id(),
    ) {
        service_worker_worker_set_state(scope, worker, version.state());
        define_non_enumerable_value_property(
            scope,
            worker,
            SERVICE_WORKER_WORKER_REGISTRATION_SLOT,
            registration.into(),
        );
        return worker.into();
    }
    let worker = build_service_worker_object_for_version(scope, version);
    define_non_enumerable_value_property(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_WORKER_SLOT,
        worker.into(),
    );
    define_non_enumerable_value_property(
        scope,
        worker,
        SERVICE_WORKER_WORKER_REGISTRATION_SLOT,
        registration.into(),
    );
    remember_service_worker_registration_worker(scope, registration, worker);
    worker.into()
}

fn service_worker_registration_installing_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(service_worker_registration_worker_for_phase(
        scope,
        args.this(),
        "installing",
    ));
}

fn service_worker_registration_waiting_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(service_worker_registration_worker_for_phase(
        scope,
        args.this(),
        "waiting",
    ));
}

fn service_worker_registration_active_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(service_worker_registration_worker_for_phase(
        scope,
        args.this(),
        "active",
    ));
}

fn build_service_worker_registration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: OwnerDispatchScope,
    scope_url: &str,
    script_url: &str,
    phase: ServiceWorkerRegistrationPhase<'_>,
) -> v8::Local<'s, v8::Object> {
    ensure_service_worker_constructor(scope, "ServiceWorkerRegistration");
    let resolved = resolve_service_worker_registration_phase(scope, None, script_url, phase);
    let sync_manager = build_service_worker_sync_manager(scope, scope_url);
    let periodic_sync_manager = build_service_worker_periodic_sync_manager(scope, scope_url);
    let push_manager = build_service_worker_push_manager(scope, scope_url);
    let navigation_preload = build_service_worker_navigation_preload_manager(scope, scope_url);
    let declaration = ServiceWorkerRegistrationObjectDeclaration::new(
        scope_url.to_owned(),
        resolved.update_via_cache,
        sync_manager,
        periodic_sync_manager,
        push_manager,
        navigation_preload,
    );
    let registration = bind_declared_service_worker_object(scope, &declaration);
    mark_service_worker_registration_event_target(scope, registration);
    if let Some(scope_value) = v8_string(scope, scope_url) {
        define_non_enumerable_value_property(
            scope,
            registration,
            SERVICE_WORKER_REGISTRATION_SCOPE_SLOT,
            scope_value.into(),
        );
    }
    service_worker_worker_set_owner_scope(scope, registration, owner);
    define_non_enumerable_value_property(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_WORKER_SLOT,
        resolved.hidden_worker.into(),
    );
    define_non_enumerable_value_property(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_WORKERS_SLOT,
        v8::Array::new(scope, 0).into(),
    );
    remember_service_worker_registration_workers(scope, registration, resolved);
    watch_service_worker_registration_object_lifecycle(scope, owner, scope_url, registration);
    registration
}

fn build_service_worker_registration_object_for_container<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
    scope_url: &str,
    script_url: &str,
    phase: ServiceWorkerRegistrationPhase<'_>,
) -> v8::Local<'s, v8::Object> {
    let owner = service_worker_container_owner_scope(scope, container);
    if let Some(registration) =
        service_worker_container_cached_registration_for_scope(scope, container, scope_url)
    {
        service_worker_worker_set_owner_scope(scope, registration, owner);
        update_service_worker_registration_object(scope, registration, script_url, phase);
        watch_service_worker_registration_object_lifecycle(scope, owner, scope_url, registration);
        return registration;
    }
    let registration =
        build_service_worker_registration_object(scope, owner, scope_url, script_url, phase);
    remember_service_worker_container_registration(scope, container, registration);
    registration
}

fn watch_service_worker_registration_object_lifecycle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: OwnerDispatchScope,
    scope_url: &str,
    registration: v8::Local<'s, v8::Object>,
) {
    let Ok(scope_url) = url::Url::parse(scope_url) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    unsafe { &mut *host_ptr }.watch_service_worker_registration_lifecycle(
        scope,
        owner,
        scope_url,
        registration,
    );
}

struct ResolvedServiceWorkerRegistrationPhase<'s> {
    update_via_cache: &'static str,
    hidden_worker: v8::Local<'s, v8::Object>,
    cached_workers: Vec<v8::Local<'s, v8::Object>>,
}

fn resolve_service_worker_registration_phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: Option<v8::Local<'s, v8::Object>>,
    script_url: &str,
    phase: ServiceWorkerRegistrationPhase<'_>,
) -> ResolvedServiceWorkerRegistrationPhase<'s> {
    match phase {
        ServiceWorkerRegistrationPhase::Snapshot(snapshot) => {
            let mut cached_workers = Vec::new();
            let installing_worker = snapshot.installing().map(|version| {
                service_worker_registration_worker_for_snapshot_version_with_cache(
                    scope,
                    registration,
                    version,
                )
            });
            let waiting_worker = snapshot.waiting().map(|version| {
                service_worker_registration_worker_for_snapshot_version_with_cache(
                    scope,
                    registration,
                    version,
                )
            });
            let active_worker = snapshot.active().map(|version| {
                service_worker_registration_worker_for_snapshot_version_with_cache(
                    scope,
                    registration,
                    version,
                )
            });
            cached_workers.extend(installing_worker);
            cached_workers.extend(waiting_worker);
            cached_workers.extend(active_worker);
            let hidden_worker = active_worker
                .or(waiting_worker)
                .or(installing_worker)
                .or_else(|| {
                    registration.and_then(|registration| {
                        service_worker_registration_hidden_worker(scope, registration)
                    })
                })
                .unwrap_or_else(|| build_service_worker_object(scope, script_url, "redundant"));
            ResolvedServiceWorkerRegistrationPhase {
                update_via_cache: snapshot.update_via_cache().as_str(),
                hidden_worker,
                cached_workers,
            }
        }
    }
}

fn service_worker_registration_worker_for_snapshot_version_with_cache<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: Option<v8::Local<'s, v8::Object>>,
    version: &crate::service_worker_runtime::ServiceWorkerVersionSnapshot,
) -> v8::Local<'s, v8::Object> {
    if let Some(registration) = registration
        && let Some(worker) = service_worker_registration_cached_worker_for_version(
            scope,
            registration,
            version.version_id(),
        )
    {
        service_worker_worker_set_state(scope, worker, version.state());
        return worker;
    }
    build_service_worker_object_for_version(scope, version)
}

fn update_service_worker_registration_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
    script_url: &str,
    phase: ServiceWorkerRegistrationPhase<'_>,
) {
    let resolved =
        resolve_service_worker_registration_phase(scope, Some(registration), script_url, phase);
    set_service_worker_readonly_value(
        scope,
        registration,
        "updateViaCache",
        v8str(scope, resolved.update_via_cache).into(),
    );
    define_non_enumerable_value_property(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_WORKER_SLOT,
        resolved.hidden_worker.into(),
    );
    remember_service_worker_registration_workers(scope, registration, resolved);
}

fn remember_service_worker_registration_workers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
    resolved: ResolvedServiceWorkerRegistrationPhase<'s>,
) {
    let owner = service_worker_owner_scope_from_object(scope, registration);
    for worker in resolved.cached_workers {
        service_worker_worker_set_owner_scope(scope, worker, owner);
        remember_service_worker_registration_worker(scope, registration, worker);
        define_non_enumerable_value_property(
            scope,
            worker,
            SERVICE_WORKER_WORKER_REGISTRATION_SLOT,
            registration.into(),
        );
    }
    service_worker_worker_set_owner_scope(scope, resolved.hidden_worker, owner);
    define_non_enumerable_value_property(
        scope,
        resolved.hidden_worker,
        SERVICE_WORKER_WORKER_REGISTRATION_SLOT,
        registration.into(),
    );
}

fn build_service_worker_sync_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    scope_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let declaration = ServiceWorkerSyncManagerDeclaration::new();
    let sync_manager = bind_declared_service_worker_object(scope, &declaration);
    let scope_value = v8_string(scope, scope_url)?;
    define_non_enumerable_value_property(
        scope,
        sync_manager,
        SERVICE_WORKER_SYNC_MANAGER_SCOPE_SLOT,
        scope_value.into(),
    );
    Some(sync_manager)
}

fn build_service_worker_periodic_sync_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    scope_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let declaration = ServiceWorkerPeriodicSyncManagerDeclaration::new();
    let periodic_sync_manager = bind_declared_service_worker_object(scope, &declaration);
    let scope_value = v8_string(scope, scope_url)?;
    define_non_enumerable_value_property(
        scope,
        periodic_sync_manager,
        SERVICE_WORKER_PERIODIC_SYNC_MANAGER_SCOPE_SLOT,
        scope_value.into(),
    );
    Some(periodic_sync_manager)
}

fn build_service_worker_push_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    scope_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let declaration = ServiceWorkerPushManagerDeclaration::new();
    let push_manager = bind_declared_service_worker_object(scope, &declaration);
    let scope_value = v8_string(scope, scope_url)?;
    define_non_enumerable_value_property(
        scope,
        push_manager,
        SERVICE_WORKER_PUSH_MANAGER_SCOPE_SLOT,
        scope_value.into(),
    );
    Some(push_manager)
}

fn build_service_worker_navigation_preload_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    scope_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    ensure_service_worker_constructor(scope, "NavigationPreloadManager");
    let declaration = ServiceWorkerNavigationPreloadManagerDeclaration::new();
    let navigation_preload = bind_declared_service_worker_object(scope, &declaration);
    let scope_value = v8_string(scope, scope_url)?;
    define_non_enumerable_value_property(
        scope,
        navigation_preload,
        SERVICE_WORKER_NAVIGATION_PRELOAD_MANAGER_SCOPE_SLOT,
        scope_value.into(),
    );
    Some(navigation_preload)
}

fn build_service_worker_navigation_preload_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &ServiceWorkerNavigationPreloadState,
) -> v8::Local<'s, v8::Object> {
    let declaration = ServiceWorkerNavigationPreloadStateDeclaration::new(
        state.enabled,
        state.header_value.clone(),
    );
    bind_declared_service_worker_object(scope, &declaration)
}

fn build_service_worker_push_subscription_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    scope_url: &url::Url,
    snapshot: &crate::service_worker_runtime::ServiceWorkerPushSubscriptionSnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    let options = ServiceWorkerPushSubscriptionOptionsDeclaration::new(
        snapshot.user_visible_only,
        v8::null(scope).into(),
    )
    .bind(scope)
    .ok()?;
    let subscription = ServiceWorkerPushSubscriptionDeclaration {
        endpoint: snapshot.endpoint.clone(),
        expiration_time: v8::null(scope).into(),
        options,
        unsubscribe: (),
        to_json: (),
    }
    .bind(scope)
    .ok()?;
    let scope_value = v8_string(scope, scope_url.as_str())?;
    define_non_enumerable_value_property(
        scope,
        subscription,
        SERVICE_WORKER_PUSH_SUBSCRIPTION_SCOPE_SLOT,
        scope_value.into(),
    );
    Some(subscription)
}

fn service_worker_push_subscription_unsubscribe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some(scope_url) = service_worker_push_subscription_scope_from_this(scope, args.this())
    else {
        reject_service_worker_promise_with_type_error(
            scope,
            resolver,
            "Failed to execute 'unsubscribe' on 'PushSubscription': registration scope is unavailable.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_service_worker_promise(
            scope,
            resolver,
            "failed to get native bridge for PushSubscription.unsubscribe",
        );
        return;
    };
    let host = unsafe { &*host_ptr };
    let unsubscribed = host.unsubscribe_service_worker_push(&scope_url);
    let _ = resolver.resolve(scope, v8::Boolean::new(scope, unsubscribed).into());
}

fn service_worker_push_subscription_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let object = ObjectLiteralDeclaration::bind(scope);
    for name in ["endpoint", "expirationTime", "options"] {
        let value = args
            .this()
            .get(scope, v8str(scope, name).into())
            .unwrap_or_else(|| v8::undefined(scope).into());
        object.set_string_property(scope, name, value);
    }
    rv.set(object.into_value());
}

fn service_worker_registration_scope_from_this(
    scope: &mut v8::PinScope<'_, '_>,
    registration: v8::Local<'_, v8::Object>,
) -> Option<url::Url> {
    let value =
        object_own_hidden_value(scope, registration, SERVICE_WORKER_REGISTRATION_SCOPE_SLOT)?;
    let scope_string = value.to_string(scope)?.to_rust_string_lossy(scope);
    url::Url::parse(&scope_string).ok()
}

fn service_worker_sync_manager_scope_from_this(
    scope: &mut v8::PinScope<'_, '_>,
    sync_manager: v8::Local<'_, v8::Object>,
) -> Option<url::Url> {
    let value =
        object_own_hidden_value(scope, sync_manager, SERVICE_WORKER_SYNC_MANAGER_SCOPE_SLOT)?;
    let scope_string = value.to_string(scope)?.to_rust_string_lossy(scope);
    url::Url::parse(&scope_string).ok()
}

fn service_worker_push_manager_scope_from_this(
    scope: &mut v8::PinScope<'_, '_>,
    push_manager: v8::Local<'_, v8::Object>,
) -> Option<url::Url> {
    let value =
        object_own_hidden_value(scope, push_manager, SERVICE_WORKER_PUSH_MANAGER_SCOPE_SLOT)?;
    let scope_string = value.to_string(scope)?.to_rust_string_lossy(scope);
    url::Url::parse(&scope_string).ok()
}

fn service_worker_push_subscription_scope_from_this(
    scope: &mut v8::PinScope<'_, '_>,
    subscription: v8::Local<'_, v8::Object>,
) -> Option<url::Url> {
    let value = object_own_hidden_value(
        scope,
        subscription,
        SERVICE_WORKER_PUSH_SUBSCRIPTION_SCOPE_SLOT,
    )?;
    let scope_string = value.to_string(scope)?.to_rust_string_lossy(scope);
    url::Url::parse(&scope_string).ok()
}

fn build_service_worker_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    script_url: &str,
    state: &'static str,
) -> v8::Local<'s, v8::Object> {
    ensure_service_worker_constructor(scope, "ServiceWorker");
    let declaration = ServiceWorkerObjectDeclaration::new(script_url.to_owned(), state);
    let worker = bind_declared_service_worker_object(scope, &declaration);
    mark_service_worker_worker_event_target(scope, worker);
    worker
}

fn build_service_worker_object_for_version<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    version: &crate::service_worker_runtime::ServiceWorkerVersionSnapshot,
) -> v8::Local<'s, v8::Object> {
    let worker = build_service_worker_object(scope, version.script_url().as_str(), version.state());
    service_worker_worker_set_version_id(scope, worker, version.version_id());
    worker
}

fn ensure_service_worker_constructor(scope: &mut v8::PinScope<'_, '_>, name: &'static str) {
    let global = scope.get_current_context().global(scope);
    if global
        .get(scope, v8str(scope, name).into())
        .is_some_and(|value| !value.is_undefined())
    {
        return;
    }
    let template = v8::FunctionTemplate::builder(illegal_constructor_callback)
        .length(0)
        .build(scope);
    template.set_class_name(v8str(scope, name));
    let Some(constructor) = template.get_function(scope) else {
        return;
    };
    let _ = define_global_value(scope, global, name, constructor.into());
    if let Some(prototype) = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        let _ = prototype.define_own_property(
            scope,
            v8::Symbol::get_to_string_tag(scope).into(),
            v8str(scope, name).into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
    }
}

fn build_service_worker_controller_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &crate::runtime::ServiceWorkerControlState,
    owner: OwnerDispatchScope,
) -> v8::Local<'s, v8::Object> {
    let controller = build_service_worker_object(scope, state.script_url().as_str(), "activated");
    if let Some(version_id) = state.active_version_id() {
        service_worker_worker_set_version_id(scope, controller, version_id);
    }
    service_worker_worker_set_owner_scope(scope, controller, owner);
    set_service_worker_container_value(
        scope,
        controller,
        "state",
        v8str(scope, "activated").into(),
    );
    controller
}

pub(in crate::context_bootstrap) fn install_initial_service_worker_ready_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    service_worker: v8::Local<'s, v8::Object>,
) {
    ensure_service_worker_constructor(scope, "ServiceWorker");
    ensure_service_worker_constructor(scope, "ServiceWorkerRegistration");
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    let owner = service_worker_container_owner_scope(scope, service_worker);
    let state = context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
        unsafe { &*host_ptr }.service_worker_control_state_for_window_owner(owner)
    });
    if let Some(state) = state.as_ref() {
        let snapshot =
            crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot::from_active_control_for_binding(
                state.clone(),
            );
        let registration = build_service_worker_registration_object_for_container(
            scope,
            service_worker,
            state.scope_url().as_str(),
            state.script_url().as_str(),
            ServiceWorkerRegistrationPhase::Snapshot(&snapshot),
        );
        let _ = resolver.resolve(scope, registration.into());
    } else if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        if let Some(request_context) = host.service_worker_window_request_context(owner) {
            host.install_pending_service_worker_ready(scope, resolver, request_context);
            host.watch_pending_service_worker_ready();
        }
    }
    set_service_worker_container_value(scope, service_worker, "ready", promise.into());
}

fn set_service_worker_container_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(name) = v8_string(scope, name) else {
        return;
    };
    let _ = object.set(scope, name.into(), value);
}

fn set_service_worker_readonly_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    let _ = object.define_own_property(
        scope,
        v8str(scope, name).into(),
        value,
        v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY,
    );
}

fn mark_service_worker_registration_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
) {
    mark_simple_event_target_slot(scope, registration, SERVICE_WORKER_REGISTRATION_EVENTS_SLOT);
}

fn mark_service_worker_worker_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) {
    mark_simple_event_target_slot(scope, worker, SERVICE_WORKER_WORKER_EVENTS_SLOT);
}

fn service_worker_registration_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    simple_event_target_add_event_listener_callback(scope, args, rv);
}

fn service_worker_worker_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    simple_event_target_add_event_listener_callback(scope, args, rv);
}

fn service_worker_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'ServiceWorker': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(version_id) = service_worker_worker_version_id_value(scope, args.this()) else {
        return;
    };
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(payload) = crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        args.get(0),
        transfer_arg,
        "ServiceWorker",
    ) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let owner = service_worker_worker_owner_scope(scope, args.this());
    let _ = unsafe { &*host_ptr }.dispatch_service_worker_message(version_id, owner, payload);
}

fn service_worker_worker_set_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    state: &'static str,
) {
    set_service_worker_readonly_value(scope, worker, "state", v8str(scope, state).into());
}

fn service_worker_worker_set_version_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
) {
    if let Some(value) = v8_string(scope, &version_id.as_u64().to_string()) {
        define_non_enumerable_value_property(
            scope,
            worker,
            SERVICE_WORKER_WORKER_VERSION_ID_SLOT,
            value.into(),
        );
    }
}

pub(in crate::context_bootstrap) fn service_worker_object_set_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    owner: OwnerDispatchScope,
) {
    let owner_token = service_worker_owner_token_value(scope, owner);
    set_private_value(scope, object, SERVICE_WORKER_OWNER_TOKEN_SLOT, owner_token);
}

fn service_worker_worker_set_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    owner: OwnerDispatchScope,
) {
    service_worker_object_set_owner_scope(scope, worker, owner);
}

fn service_worker_worker_owner_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) -> OwnerDispatchScope {
    service_worker_owner_scope_from_object(scope, worker)
}

fn service_worker_owner_scope_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> OwnerDispatchScope {
    let Some(token) = get_private_value(scope, object, SERVICE_WORKER_OWNER_TOKEN_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return OwnerDispatchScope::Top;
    };
    if let Some(raw) = token.strip_prefix("child:")
        && let Ok(index) = raw.parse::<usize>()
    {
        return OwnerDispatchScope::Child(DomHandle::new(index));
    }
    if let Some(raw) = token.strip_prefix("popup:")
        && let Ok(popup_id) = raw.parse::<u64>()
        && popup_id != 0
    {
        return OwnerDispatchScope::LightweightPopup(popup_id);
    }
    OwnerDispatchScope::Top
}

fn service_worker_worker_version_id(
    scope: &mut v8::PinScope<'_, '_>,
    worker: v8::Local<'_, v8::Object>,
) -> Option<String> {
    object_own_hidden_value(scope, worker, SERVICE_WORKER_WORKER_VERSION_ID_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn service_worker_worker_version_id_value(
    scope: &mut v8::PinScope<'_, '_>,
    worker: v8::Local<'_, v8::Object>,
) -> Option<crate::service_worker_runtime::ServiceWorkerVersionId> {
    let raw = service_worker_worker_version_id(scope, worker)?
        .parse::<u64>()
        .ok()?;
    Some(crate::service_worker_runtime::ServiceWorkerVersionId::from_u64_for_binding(raw))
}

fn service_worker_worker_matches_version(
    scope: &mut v8::PinScope<'_, '_>,
    worker: v8::Local<'_, v8::Object>,
    version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
) -> bool {
    service_worker_worker_version_id(scope, worker)
        .is_some_and(|worker_version_id| worker_version_id == version_id.as_u64().to_string())
}

fn service_worker_registration_worker_cache<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    if let Some(cache) = object_own_hidden_value(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_WORKERS_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return cache;
    }
    let cache = v8::Array::new(scope, 0);
    define_non_enumerable_value_property(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_WORKERS_SLOT,
        cache.into(),
    );
    cache
}

fn service_worker_registration_cached_worker_for_version<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
    version_id: crate::service_worker_runtime::ServiceWorkerVersionId,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache = service_worker_registration_worker_cache(scope, registration);
    for index in 0..cache.length() {
        let Some(worker) = cache
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if service_worker_worker_matches_version(scope, worker, version_id) {
            return Some(worker);
        }
    }
    service_worker_registration_hidden_worker(scope, registration)
        .filter(|worker| service_worker_worker_matches_version(scope, *worker, version_id))
}

fn remember_service_worker_registration_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
    worker: v8::Local<'s, v8::Object>,
) {
    let owner = service_worker_owner_scope_from_object(scope, registration);
    service_worker_worker_set_owner_scope(scope, worker, owner);
    let Some(version_id) = service_worker_worker_version_id(scope, worker) else {
        return;
    };
    let cache = service_worker_registration_worker_cache(scope, registration);
    for index in 0..cache.length() {
        let Some(existing) = cache
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if service_worker_worker_version_id(scope, existing).as_deref() == Some(version_id.as_str())
        {
            return;
        }
    }
    array_push_value(scope, cache, worker.into());
}

fn service_worker_registration_active_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    registration
        .get(scope, v8str(scope, "active").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn service_worker_registration_hidden_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    object_own_hidden_value(scope, registration, SERVICE_WORKER_REGISTRATION_WORKER_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn service_worker_registration_clear_workers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
) {
    let null_value: v8::Local<'_, v8::Value> = v8::null(scope).into();
    set_service_worker_registration_worker_values(
        scope,
        registration,
        null_value,
        null_value,
        null_value,
    );
}

fn set_service_worker_registration_worker_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: v8::Local<'s, v8::Object>,
    installing: v8::Local<'s, v8::Value>,
    waiting: v8::Local<'s, v8::Value>,
    active: v8::Local<'s, v8::Value>,
) {
    set_service_worker_readonly_value(scope, registration, "installing", installing);
    set_service_worker_readonly_value(scope, registration, "waiting", waiting);
    set_service_worker_readonly_value(scope, registration, "active", active);
}

fn dispatch_service_worker_simple_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &'static str,
) -> ServiceWorkerInternalEventCallbackDispatchEffect {
    let callback_effect = if crate::context_bootstrap::simple_object_event_listeners_snapshot(
        scope, target, slot_name, event_type,
    )
    .is_empty()
    {
        ServiceWorkerInternalEventCallbackDispatchEffect::NoCallbackBodyDispatched
    } else {
        ServiceWorkerInternalEventCallbackDispatchEffect::CallbackBodyDispatched
    };
    let event = v8::Object::new(scope);
    let _ =
        ServiceWorkerSimpleEventDeclaration::new(v8str(scope, event_type)).initialize(scope, event);
    dispatch_simple_event_target_event(scope, target, slot_name, event_type, event);
    callback_effect
}

fn new_service_worker_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    origin: &str,
    source: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let message_ctor = global
        .get(scope, v8str(scope, "MessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let init = ServiceWorkerMessageEventInitDeclaration {
        data,
        origin: v8_string(scope, origin)?,
        source,
        ports,
    }
    .bind(scope)
    .expect("ServiceWorker MessageEvent init declaration should bind");
    let event_type = v8_string(scope, event_type)?;
    message_ctor.new_instance(scope, &[event_type.into(), init.into()])
}
