//! WorkerGlobalScope runtime state and native queries for the current worker.

use super::*;

pub(crate) const WORKER_STATE_SLOT: &str = "__workerState";

pub(crate) struct WorkerGlobalState {
    /// Weak handles and native cleanups owned by this worker isolate. Worker
    /// teardown clears this registry before `OwnedIsolate` is destroyed.
    pub(crate) v8_finalizers: crate::v8_finalizer::V8FinalizerRegistry,
    pub(crate) resource_timing_buffers: crate::native_bridge::SharedResourceTimingBufferRegistry,
    /// Channel to send messages back to the parent.
    pub(in crate::worker) parent_tx: mpsc::UnboundedSender<WorkerToParentMessage>,
    /// Internal wake channel used by worker-owned async runtime surfaces.
    pub(crate) worker_wake_tx: mpsc::UnboundedSender<crate::worker::handle::WorkerMessage>,
    /// Shared lifecycle bit published by `Worker.terminate()` before V8 is
    /// interrupted. Worker-side task delivery must not rely on cross-thread
    /// timing between that interrupt and already-selected work.
    pub(crate) termination_requested: Arc<AtomicBool>,
    /// Whether `close()` has been called.
    pub(in crate::worker) closed: bool,
    /// HTML's per-global guard against recursively reporting an error event.
    pub(in crate::worker) in_error_reporting_mode: bool,
    /// Timer id counter.
    pub(in crate::worker) next_timer_id: u32,
    /// Browser font tasks use a separate queue and cannot be canceled by timer IDs.
    pub(in crate::worker) font_tasks:
        std::collections::VecDeque<super::super::timer_callback::WorkerTimerCallback>,
    /// Networking completions are browser tasks, outside the author timer ID space.
    pub(in crate::worker) networking_tasks:
        super::super::networking_tasks::WorkerNetworkingTaskQueue,
    /// Codec completions cannot be canceled by author timer IDs.
    pub(in crate::worker) codec_tasks: super::super::codec_tasks::WorkerCodecTaskQueue,
    pub(in crate::worker) canvas_blob_tasks: super::super::canvas_blob_tasks::WorkerCanvasBlobTasks,
    /// Inside-settings resource authority for every request owned by this
    /// WorkerGlobalScope. Even data/blob workers retain the creator's browser
    /// backend so later fetch/XHR/module/WebSocket work has an exact owner.
    pub(in crate::worker) loader: crate::network::context::WorkerResourceLoader,
    /// Whether this runtime is a dedicated or shared worker global.
    pub(in crate::worker) global_kind: crate::worker::thread::WorkerGlobalKind,
    /// Whether this worker was constructed as a classic or module worker.
    pub(in crate::worker) script_kind: crate::worker::thread::WorkerScriptKind,
    /// Worker global URL and settings base, unchanged by importScripts().
    /// Imported scripts carry their separate import base in V8 ScriptOrigin.
    pub(in crate::worker) current_script_url: Option<Url>,
    /// Version-specific classic ServiceWorker script resources.
    pub(in crate::worker) service_worker_script_resources:
        HashMap<Url, crate::worker::WorkerScriptResource>,
    pub(in crate::worker) service_worker_updated_script_resources:
        crate::worker::WorkerScriptUpdateResources,
    pub(in crate::worker) service_worker_can_import_new_scripts: bool,
    /// Referrer policy parsed from the top-level worker script response.
    pub(in crate::worker) referrer_policy: Option<String>,
    /// CSP policies from outside settings used for module static imports.
    pub(in crate::worker) module_static_import_content_security_policies: Vec<String>,
    /// Enforce CSP policies parsed from the top-level worker script response.
    pub(in crate::worker) content_security_policies: Vec<String>,
    pub(in crate::worker) content_security_policy_snapshot:
        Option<crate::content_security_policy::InheritedContentSecurityPolicy>,
    /// Report-only CSP policies parsed from the top-level worker script response.
    pub(in crate::worker) content_security_report_only_policies: Vec<String>,
    /// Reporting API endpoints parsed from the top-level worker script response.
    pub(in crate::worker) content_security_reporting_endpoints:
        ContentSecurityPolicyReportingEndpoints,
    /// Whether this worker global is a secure context for `[SecureContext]` APIs.
    pub(in crate::worker) secure_context: bool,
    /// Network.setExtraHTTPHeaders headers inherited from the owning page/CDP session.
    pub(in crate::worker) extra_http_headers: moli_fetch::RequestHeaders,
    /// Permission overrides inherited from the owning page/CDP session.
    pub(in crate::worker) permission_overrides:
        Vec<crate::protocol_types::PermissionOverrideRegistration>,
    /// Network.emulateNetworkConditions offline state inherited from the owning page/CDP session.
    pub(in crate::worker) network_offline: bool,
    /// Network.setBlockedURLs patterns inherited from the owning page/CDP session.
    pub(in crate::worker) blocked_url_patterns: Vec<String>,
    /// Current document network/cache partition key inherited by worker-owned requests.
    pub(in crate::worker) network_partition_key: Option<String>,
    /// COEP/DIP policy container state inherited by worker-owned subresource requests.
    pub(in crate::worker) policy_context: crate::types::SubresourcePolicyContext,
    /// Fetch domain subresource interception inherited from the owning page/CDP session.
    pub(in crate::worker) fetch_subresource_interception_enabled: bool,
    pub(in crate::worker) fetch_subresource_interception_resource_type:
        Option<SubresourceResourceType>,
    /// Async fetch completions routed back onto the worker event loop.
    pub(in crate::worker) fetch_completion_tx: mpsc::UnboundedSender<WorkerFetchEvent>,
    /// Promise resolvers pending async fetch completion.
    pub(in crate::worker) pending_fetches: HashMap<u32, PendingWorkerFetch>,
    /// Worker-local pending body sources for headers-first worker fetch.
    pub(crate) pending_network_body_sources:
        HashMap<NetworkBodySourceId, crate::network_host::PendingNetworkBodySourceState>,
    pub(crate) pending_network_body_clones: HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    /// Fetch id counter.
    pub(in crate::worker) next_fetch_id: u32,
    /// Async XHR completions routed back onto the worker event loop.
    pub(in crate::worker) xhr_completion_tx: mpsc::UnboundedSender<WorkerXhrEvent>,
    /// In-flight worker XHR requests keyed by internal id.
    pub(in crate::worker) pending_xhrs: HashMap<u32, PendingWorkerXhr>,
    /// Worker XHR id counter.
    pub(in crate::worker) next_xhr_id: u32,
    /// Worker-owned CSP report requests paused for Fetch domain request-stage interception.
    pub(in crate::worker) pending_csp_reports: HashMap<u32, PendingWorkerCspReport>,
    /// Worker-local TextDecoder state. TextEncoder is stateless, but TextDecoder
    /// can stream and needs decoder state tied to this worker's isolate.
    pub(crate) text_codecs: TextCodecStore,
    /// Worker-local AbortController/AbortSignal runtime state.
    pub(in crate::worker) abort: Rc<RefCell<WorkerAbortStore>>,
    /// Worker-thread view of the creator's browser-context/partition runtime.
    pub(in crate::worker) worker_context_runtime: crate::runtime::RendererWorkerContextRuntime,
    /// Browser-context Service Worker owner inherited by real worker clients.
    pub(in crate::worker) service_worker_runtime:
        Option<crate::service_worker_runtime::ServiceWorkerRuntimeService>,
    /// Live Service Worker client id for DedicatedWorker/SharedWorker globals.
    pub(in crate::worker) service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    /// Owner-scoped MessagePort state shared with page and nested worker contexts.
    pub(in crate::worker) message_port_registry: SharedMessagePortRegistry,
    /// Worker-owned MessagePort wrappers keyed by runtime port id.
    pub(in crate::worker) message_port_wrappers:
        HashMap<MessagePortId, WorkerMessagePortWrapperEntry>,
    /// SharedWorker connect-event ports whose lifetime is owned by the shared worker runtime.
    pub(in crate::worker) shared_worker_connection_ports: HashSet<MessagePortId>,
    /// Worker-owned BroadcastChannel wrappers keyed by runtime channel id.
    pub(in crate::worker) broadcast_channel_registry: SharedBroadcastChannelRegistry,
    pub(in crate::worker) broadcast_channel_storage_key: MoliStorageKey,
    pub(in crate::worker) storage_key: MoliStorageKey,
    pub(in crate::worker) broadcast_channel_wrappers:
        HashMap<BroadcastChannelId, v8::Global<v8::Object>>,
    /// IndexedDB storage manager inherited from the owning browser context.
    pub(in crate::worker) indexed_db_manager:
        Option<crate::context_bootstrap::WeakIndexedDbManager>,
    /// Storage Buckets registry inherited from the owning browser context.
    pub(in crate::worker) storage_bucket_store:
        Option<crate::context_bootstrap::SharedStorageBucketStore>,
    /// Async WebSocket transport events routed back onto this worker loop.
    pub(in crate::worker) websocket_event_tx: tokio::sync::mpsc::Sender<WebSocketEvent>,
    /// Worker-owned classic WebSocket connections keyed by socket id.
    pub(in crate::worker) websockets: HashMap<u64, WorkerWebSocketState>,
    /// Worker-local WebSocket id counter.
    pub(in crate::worker) next_websocket_id: u64,
    /// Worker-local dedicated workers created through `new Worker(...)`.
    pub(in crate::worker) next_nested_worker_id: u64,
    pub(in crate::worker) nested_worker_wrappers:
        HashMap<DedicatedWorkerId, v8::Global<v8::Object>>,
    /// Heavyweight WebCrypto completions routed back onto the worker event loop.
    pub(in crate::worker) webcrypto_completion_tx: mpsc::UnboundedSender<WorkerWebCryptoCompletion>,
    /// In-flight worker WebCrypto blocking tasks keyed by task id.
    pub(in crate::worker) pending_webcrypto: HashMap<u64, PendingWorkerWebCryptoTask>,
    /// Worker WebCrypto task id counter (never zero so 0 can mean "unset").
    pub(in crate::worker) next_webcrypto_task_id: u64,
    /// OPFS completions routed from the partition-owned storage IO sequence.
    pub(in crate::worker) opfs_completion_tx: mpsc::UnboundedSender<WorkerOpfsCompletion>,
    pub(in crate::worker) opfs_owner_state: Option<WorkerOpfsOwnerState>,
    /// In-flight Service Worker `periodicsync` events keyed by runtime event id.
    pub(in crate::worker) pending_service_worker_periodic_sync_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerPeriodicSyncEvent>,
    /// In-flight Service Worker `clients.get()` / `clients.matchAll()` queries keyed by request id.
    pub(in crate::worker) pending_service_worker_client_queries:
        HashMap<u64, PendingServiceWorkerClientQuery>,
    /// In-flight Service Worker `WindowClient.navigate()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_client_navigates:
        HashMap<u64, PendingServiceWorkerClientNavigate>,
    /// In-flight Service Worker `WindowClient.focus()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_client_focuses:
        HashMap<u64, PendingServiceWorkerClientFocus>,
    /// In-flight Service Worker `clients.openWindow()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_clients_open_windows:
        HashMap<u64, PendingServiceWorkerClientsOpenWindow>,
    /// In-flight Service Worker `registration.update()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_updates: HashMap<u64, PendingServiceWorkerUpdate>,
    pub(in crate::worker) service_worker_update_request_ids: WorkerServiceWorkerRequestIdAllocator,
    /// In-flight Service Worker `registration.showNotification()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_show_notifications:
        HashMap<u64, PendingServiceWorkerShowNotification>,
    /// In-flight Service Worker `registration.getNotifications()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_get_notifications:
        HashMap<u64, PendingServiceWorkerGetNotifications>,
    /// In-flight Service Worker `registration.sync.register()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_sync_registrations:
        HashMap<u64, PendingServiceWorkerSyncRegistration>,
    /// In-flight Service Worker `registration.sync.getTags()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_sync_get_tags:
        HashMap<u64, PendingServiceWorkerSyncGetTags>,
    /// In-flight Service Worker `registration.periodicSync.register()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_periodic_sync_registrations:
        HashMap<u64, PendingServiceWorkerPeriodicSyncRegistration>,
    /// In-flight Service Worker `registration.periodicSync.getTags()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_periodic_sync_get_tags:
        HashMap<u64, PendingServiceWorkerPeriodicSyncGetTags>,
    /// In-flight Service Worker `registration.periodicSync.unregister()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_periodic_sync_unregistrations:
        HashMap<u64, PendingServiceWorkerPeriodicSyncUnregistration>,
    /// In-flight Service Worker `registration.pushManager.subscribe()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_push_subscriptions:
        HashMap<u64, PendingServiceWorkerPushSubscribe>,
    /// In-flight Service Worker `registration.pushManager.getSubscription()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_push_get_subscriptions:
        HashMap<u64, PendingServiceWorkerPushGetSubscription>,
    /// In-flight Service Worker `PushSubscription.unsubscribe()` requests keyed by request id.
    pub(in crate::worker) pending_service_worker_push_unsubscriptions:
        HashMap<u64, PendingServiceWorkerPushUnsubscribe>,
    pub(in crate::worker) service_worker_client_query_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_client_navigate_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_client_focus_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_clients_open_window_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_show_notification_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_get_notifications_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_sync_registration_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_sync_get_tags_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_periodic_sync_registration_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_periodic_sync_get_tags_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_periodic_sync_unregistration_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_push_subscription_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_push_get_subscription_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(in crate::worker) service_worker_push_unsubscription_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    /// Count of active Service Worker events that allow window interaction.
    pub(in crate::worker) service_worker_window_interaction_allowed_count: usize,
    /// Service Worker lifecycle events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(in crate::worker) pending_service_worker_lifecycle_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerLifecycleEvent>,
    /// Service Worker fetch events currently waiting for dispatch, `respondWith()`,
    /// and minimal `waitUntil()` promises to finish.
    pub(in crate::worker) pending_service_worker_fetch_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerFetchEvent>,
    /// Navigation preload promises and response bodies stay alive until preload complete/error.
    pub(in crate::worker) pending_service_worker_navigation_preloads:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerNavigationPreload>,
    /// Service Worker message events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(in crate::worker) pending_service_worker_message_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerMessageEvent>,
    /// Service Worker notification events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(in crate::worker) pending_service_worker_notification_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerNotificationEvent>,
    /// Service Worker push events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(in crate::worker) pending_service_worker_push_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerPushEvent>,
    /// Service Worker sync events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(in crate::worker) pending_service_worker_sync_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerSyncEvent>,
}

pub(crate) fn worker_service_worker_control_state(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::runtime::ServiceWorkerControlState> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let runtime = state.service_worker_runtime.as_ref()?;
    let client_id = state.service_worker_client_id?;
    runtime.matching_controller_for_client(client_id)
}

/// Returns the native secure-context policy, independently of public bindings.
pub(crate) fn worker_realm_secure_context_available(scope: &mut v8::PinScope<'_, '_>) -> bool {
    get_worker_state(scope).is_some_and(|state| state.borrow().secure_context)
}

/// Retrieve the `WorkerGlobalState` from a callback scope.
pub(crate) fn get_worker_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<Rc<RefCell<WorkerGlobalState>>> {
    let global = scope.get_current_context().global(scope);
    let val = get_private_value(scope, global, WORKER_STATE_SLOT)?;
    let external = v8::Local::<v8::External>::try_from(val).ok()?;
    let ptr = external.value() as *const RefCell<WorkerGlobalState>;
    // Safety: the Rc is kept alive by the worker thread for the lifetime of
    // the context.  We increment the ref-count here so we get our own Rc.
    unsafe {
        Rc::increment_strong_count(ptr);
        Some(Rc::from_raw(ptr))
    }
}

pub(crate) fn worker_global_is_closed(scope: &mut v8::PinScope<'_, '_>) -> bool {
    get_worker_state(scope).is_some_and(|state| state.borrow().closed)
}

pub(crate) fn worker_termination_requested(scope: &mut v8::PinScope<'_, '_>) -> bool {
    get_worker_state(scope)
        .is_some_and(|state| state.borrow().termination_requested.load(Ordering::Acquire))
}

pub(crate) fn worker_storage_key(scope: &mut v8::PinScope<'_, '_>) -> Option<MoliStorageKey> {
    Some(get_worker_state(scope)?.borrow().storage_key.clone())
}

pub(crate) fn worker_storage_partition_identity(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::runtime::RendererStoragePartitionIdentity> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .worker_context_runtime
            .storage_partition_identity(),
    )
}

pub(crate) fn worker_referrer_policy(scope: &mut v8::PinScope<'_, '_>) -> Option<String> {
    get_worker_state(scope)?.borrow().referrer_policy.clone()
}

pub(crate) fn worker_current_script_url(scope: &mut v8::PinScope<'_, '_>) -> Option<Url> {
    get_worker_state(scope)?.borrow().current_script_url.clone()
}

pub(crate) fn worker_exception_report_target(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<(mpsc::UnboundedSender<WorkerToParentMessage>, String)> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let script_url = state
        .current_script_url
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    Some((state.parent_tx.clone(), script_url))
}

pub(crate) fn worker_uses_shared_worker_agent_cluster(scope: &mut v8::PinScope<'_, '_>) -> bool {
    get_worker_state(scope).is_some_and(|state| {
        matches!(
            state.borrow().global_kind,
            crate::worker::thread::WorkerGlobalKind::Shared { .. }
        )
    })
}

pub(super) fn worker_close_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(state) = get_worker_state(scope) {
        state.borrow_mut().closed = true;
        close_worker_owned_broadcast_channels(&state);
    }
}

pub(crate) fn worker_content_security_policy_snapshot(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::content_security_policy::InheritedContentSecurityPolicy> {
    Some(content_security_policy::worker_policy_snapshot(
        &get_worker_state(scope)?.borrow(),
    ))
}
