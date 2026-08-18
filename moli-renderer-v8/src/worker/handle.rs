//! Worker-side message types and parent-facing handle.

use crate::RendererSyntheticResponseBody;
use crate::protocol_types::{
    PendingSubresourceContinueEvent, PendingSubresourceFetchInfo, SubresourceAuthCredentials,
    SubresourceNetworkRecord, SubresourceNetworkRequestHandle, SubresourceResourceType,
    WebSocketFrameDirection, WebSocketFrameOpcode,
};
use crate::runtime::{
    RendererRuntimeInspectorMessage, RendererRuntimeInspectorResponseSender,
    ServiceWorkerClientFocus, ServiceWorkerClientFocusResult, ServiceWorkerClientMessage,
    ServiceWorkerClientNavigate, ServiceWorkerClientNavigateResult, ServiceWorkerClientQuery,
    ServiceWorkerClientQueryResult, ServiceWorkerClientsOpenWindow,
    ServiceWorkerClientsOpenWindowResult, ServiceWorkerCloseNotification,
    ServiceWorkerFetchCompletion, ServiceWorkerFetchEvent, ServiceWorkerFetchStreamChunk,
    ServiceWorkerFetchStreamStarted, ServiceWorkerGetNotifications,
    ServiceWorkerGetNotificationsResult, ServiceWorkerLifecycleCompletion,
    ServiceWorkerLifecycleEvent, ServiceWorkerMessageCompletion, ServiceWorkerMessageEvent,
    ServiceWorkerNavigationPreloadFailure, ServiceWorkerNavigationPreloadResponseStarted,
    ServiceWorkerNavigationPreloadStreamChunk, ServiceWorkerNavigationPreloadStreamFinished,
    ServiceWorkerNotificationCompletion, ServiceWorkerNotificationEvent,
    ServiceWorkerPeriodicSyncCompletion, ServiceWorkerPeriodicSyncEvent,
    ServiceWorkerPeriodicSyncGetTags, ServiceWorkerPeriodicSyncGetTagsResult,
    ServiceWorkerPeriodicSyncRegistration, ServiceWorkerPeriodicSyncRegistrationResult,
    ServiceWorkerPeriodicSyncUnregistration, ServiceWorkerPeriodicSyncUnregistrationResult,
    ServiceWorkerPushCompletion, ServiceWorkerPushEvent, ServiceWorkerPushGetSubscription,
    ServiceWorkerPushGetSubscriptionResult, ServiceWorkerPushSubscribe,
    ServiceWorkerPushSubscribeResult, ServiceWorkerPushUnsubscribe,
    ServiceWorkerPushUnsubscribeResult, ServiceWorkerShowNotification,
    ServiceWorkerShowNotificationResult, ServiceWorkerSyncCompletion, ServiceWorkerSyncEvent,
    ServiceWorkerSyncGetTags, ServiceWorkerSyncGetTagsResult, ServiceWorkerSyncRegistration,
    ServiceWorkerSyncRegistrationResult, ServiceWorkerWorkerMessage,
};
use crate::structured_clone::V8StructuredClonePayload;
use crate::types::{BroadcastChannelId, DedicatedWorkerId, MessagePortId, NetworkBodySourceId};
use crate::worker::inspector_task_runner::{WorkerInspectorTaskMode, WorkerInspectorTaskRunner};
use moli_crypto::sha256_hex;
use moli_fetch::{RequestCredentialsMode, ResponseHead};
use moli_shared_worker::SharedWorkerInstanceId;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[cfg(test)]
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{mpsc, oneshot};
use url::Url;

pub(crate) fn worker_secure_context_for_script_url(
    script_url: &Url,
    creator_secure_context: bool,
) -> bool {
    match script_url.scheme() {
        // Blink's secure-context tests expect workers created from blob URLs to
        // inherit the creator context's HTTPS state. Chromium also treats data
        // workers created by a secure page as secure, even though a standalone
        // data URL is not itself a trustworthy URL.
        "blob" | "data" => creator_secure_context,
        _ => moli_url::is_potentially_trustworthy_url(script_url),
    }
}

/// Message from the parent context to the worker.
#[derive(Debug)]
pub(crate) enum WorkerMessage {
    /// A `postMessage` payload encoded via V8 structured clone.
    Post(V8StructuredClonePayload),
    /// A transferred `MessagePort` has queued work ready for delivery.
    MessagePortWake(MessagePortId),
    /// A SharedWorker client connected with a freshly entangled worker-side port.
    SharedWorkerConnect(MessagePortId),
    /// A `BroadcastChannel` has queued work ready for delivery.
    BroadcastChannelWake(BroadcastChannelId),
    /// Dispatch a Service Worker lifecycle event such as `install` or `activate`.
    ServiceWorkerLifecycleEvent(Box<ServiceWorkerLifecycleEvent>),
    /// Dispatch a Service Worker `fetch` event.
    ServiceWorkerFetchEvent(Box<ServiceWorkerFetchEvent>),
    /// Resolve a Service Worker `FetchEvent.preloadResponse` with a streamed response.
    ServiceWorkerNavigationPreloadResponseStarted(
        Box<ServiceWorkerNavigationPreloadResponseStarted>,
    ),
    /// Forward a streamed Service Worker navigation preload body chunk.
    ServiceWorkerNavigationPreloadStreamChunk(ServiceWorkerNavigationPreloadStreamChunk),
    /// Close or error a streamed Service Worker navigation preload response body.
    ServiceWorkerNavigationPreloadStreamFinished(Box<ServiceWorkerNavigationPreloadStreamFinished>),
    /// Reject a Service Worker `FetchEvent.preloadResponse` before response headers arrive.
    ServiceWorkerNavigationPreloadFailure(Box<ServiceWorkerNavigationPreloadFailure>),
    /// Cancel a streamed Service Worker `fetch` response body reader.
    ServiceWorkerFetchStreamCancel {
        event_id: crate::runtime::ServiceWorkerEventId,
        body_source_id: NetworkBodySourceId,
    },
    /// Abort the Service Worker `FetchEvent.request.signal` for a caller-aborted fetch.
    ServiceWorkerFetchRequestSignalAbort {
        event_id: crate::runtime::ServiceWorkerEventId,
        reason: Option<V8StructuredClonePayload>,
    },
    /// Dispatch a Service Worker `message` event.
    ServiceWorkerMessageEvent(Box<ServiceWorkerMessageEvent>),
    /// Dispatch a Service Worker `notificationclick` event.
    ServiceWorkerNotificationEvent(Box<ServiceWorkerNotificationEvent>),
    /// Dispatch a Service Worker `push` event.
    ServiceWorkerPushEvent(Box<ServiceWorkerPushEvent>),
    /// Dispatch a Service Worker `sync` event.
    ServiceWorkerSyncEvent(Box<ServiceWorkerSyncEvent>),
    /// Dispatch a Service Worker `periodicsync` event.
    ServiceWorkerPeriodicSyncEvent(Box<ServiceWorkerPeriodicSyncEvent>),
    /// Dispatch `navigator.serviceWorker` `controllerchange` in this worker client.
    ServiceWorkerControllerChange,
    /// Resolve a Service Worker `SyncManager.register()` request in the worker.
    ServiceWorkerSyncRegistrationResult(ServiceWorkerSyncRegistrationResult),
    /// Resolve a Service Worker `SyncManager.getTags()` request in the worker.
    ServiceWorkerSyncGetTagsResult(ServiceWorkerSyncGetTagsResult),
    /// Resolve a Service Worker `PeriodicSyncManager.register()` request in the worker.
    ServiceWorkerPeriodicSyncRegistrationResult(ServiceWorkerPeriodicSyncRegistrationResult),
    /// Resolve a Service Worker `PeriodicSyncManager.getTags()` request in the worker.
    ServiceWorkerPeriodicSyncGetTagsResult(ServiceWorkerPeriodicSyncGetTagsResult),
    /// Resolve a Service Worker `PeriodicSyncManager.unregister()` request in the worker.
    ServiceWorkerPeriodicSyncUnregistrationResult(ServiceWorkerPeriodicSyncUnregistrationResult),
    /// Resolve a Service Worker `PushManager.subscribe()` request in the worker.
    ServiceWorkerPushSubscribeResult(ServiceWorkerPushSubscribeResult),
    /// Resolve a Service Worker `PushManager.getSubscription()` request in the worker.
    ServiceWorkerPushGetSubscriptionResult(ServiceWorkerPushGetSubscriptionResult),
    /// Resolve a Service Worker `PushSubscription.unsubscribe()` request in the worker.
    ServiceWorkerPushUnsubscribeResult(ServiceWorkerPushUnsubscribeResult),
    /// Resolve a Service Worker `clients.get()` / `clients.matchAll()` query in the worker.
    ServiceWorkerClientQueryResult(ServiceWorkerClientQueryResult),
    /// Resolve a Service Worker `WindowClient.navigate()` request in the worker.
    ServiceWorkerClientNavigateResult(ServiceWorkerClientNavigateResult),
    /// Resolve a Service Worker `WindowClient.focus()` request in the worker.
    ServiceWorkerClientFocusResult(ServiceWorkerClientFocusResult),
    /// Resolve a Service Worker `clients.openWindow()` request in the worker.
    ServiceWorkerClientsOpenWindowResult(ServiceWorkerClientsOpenWindowResult),
    /// Resolve a Service Worker `ServiceWorkerRegistration.showNotification()` request in the worker.
    ServiceWorkerShowNotificationResult(ServiceWorkerShowNotificationResult),
    /// Resolve a Service Worker `ServiceWorkerRegistration.getNotifications()` request in the worker.
    ServiceWorkerGetNotificationsResult(ServiceWorkerGetNotificationsResult),
    /// Run the worker's queued unhandled promise rejection notification task.
    DispatchPendingPromiseRejections,
    /// A worker spawned from this worker has queued a parent-facing event.
    NestedWorkerEvent {
        worker_id: DedicatedWorkerId,
        message: Box<WorkerToParentMessage>,
    },
    /// Owner-thread dispatch or fallback for one queued Inspector task.
    RunInspectorTask(WorkerInspectorTaskMode),
    #[cfg(test)]
    /// Inspect worker resource-owner V8 slots from inside the worker thread.
    ResourceOwnerSlotDiagnostics {
        response_tx: oneshot::Sender<Result<WorkerResourceOwnerSlotDiagnostics, String>>,
    },
    /// Update the Network.setExtraHTTPHeaders state visible to worker-owned fetch/XHR.
    SetExtraHttpHeaders(Vec<(String, String)>),
    /// Update the Network.emulateNetworkConditions offline state visible to worker-owned fetch/XHR.
    SetNetworkOffline(bool),
    /// Update the Network.setBlockedURLs state visible to worker-owned fetch/XHR.
    SetBlockedUrlPatterns(Vec<String>),
    /// Update Fetch domain subresource interception visible to worker-owned fetch/XHR.
    SetFetchSubresourceInterception {
        enabled: bool,
        resource_type: Option<SubresourceResourceType>,
    },
    /// Continue a worker-owned fetch() that was paused for Fetch domain interception.
    ContinuePendingFetch(WorkerPendingFetchContinue),
    /// Continue a worker-owned XHR that was paused for Fetch domain interception.
    ContinuePendingXhr(WorkerPendingXhrContinue),
    /// Continue a worker-owned CSP report that was paused for Fetch domain interception.
    ContinuePendingCspReport(WorkerPendingFetchContinue),
    /// Continue a worker-owned fetch() response that was paused for Fetch domain interception.
    ContinuePendingFetchResponse {
        request: WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    },
    /// Continue a worker-owned XHR response that was paused for Fetch domain interception.
    ContinuePendingXhrResponse {
        request: WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    },
    /// Fail a worker-owned fetch() that was paused for Fetch domain interception.
    FailPendingFetch {
        request: WorkerPendingFetchContinue,
        error_text: String,
    },
    /// Fail a worker-owned XHR that was paused for Fetch domain interception.
    FailPendingXhr {
        request: WorkerPendingXhrContinue,
        error_text: String,
    },
    /// Fail a worker-owned CSP report that was paused for Fetch domain interception.
    FailPendingCspReport {
        request: WorkerPendingFetchContinue,
        error_text: String,
    },
    /// Fail a worker-owned fetch() that was paused for Fetch domain auth handling.
    FailPendingFetchAuth {
        request: WorkerPendingFetchContinue,
        error_text: String,
    },
    /// Fail a worker-owned XHR that was paused for Fetch domain auth handling.
    FailPendingXhrAuth {
        request: WorkerPendingXhrContinue,
        error_text: String,
    },
    /// Fail a worker-owned fetch() response that was paused for Fetch domain interception.
    FailPendingFetchResponse {
        request: WorkerPendingFetchContinue,
        error_text: String,
    },
    /// Fail a worker-owned XHR response that was paused for Fetch domain interception.
    FailPendingXhrResponse {
        request: WorkerPendingXhrContinue,
        error_text: String,
    },
    /// Fulfill a worker-owned fetch() that was paused for Fetch domain interception.
    FulfillPendingFetch {
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    /// Fulfill a worker-owned XHR that was paused for Fetch domain interception.
    FulfillPendingXhr {
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    /// Fulfill a worker-owned CSP report that was paused for Fetch domain interception.
    FulfillPendingCspReport {
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    /// Fulfill a worker-owned fetch() response that was paused for Fetch domain interception.
    FulfillPendingFetchResponse {
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    /// Fulfill a worker-owned XHR response that was paused for Fetch domain interception.
    FulfillPendingXhrResponse {
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    /// Request the worker to terminate.
    Terminate,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerResourceOwnerSlotDiagnostics {
    pub(crate) context_slot_has_owner: bool,
    pub(crate) current_owner_matches_context: bool,
    pub(crate) isolate_slot_has_owner: bool,
    pub(crate) opfs_owner_state_materialized: bool,
    pub(crate) materialized_interfaces: Vec<(&'static str, usize)>,
    pub(crate) storage_constructor_materializations: usize,
    pub(crate) storage_manager_materialized: bool,
    pub(crate) storage_bucket_manager_materialized: bool,
}

/// Message from the worker back to the parent context.
#[derive(Debug)]
pub(crate) enum WorkerToParentMessage {
    /// A `postMessage` payload encoded via V8 structured clone.
    Post(V8StructuredClonePayload),
    /// The worker encountered an unhandled error.
    Error {
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
        phase: WorkerErrorPhase,
        source: WorkerErrorSource,
    },
    /// A worker `console.*` call that should be surfaced through the owning page until
    /// workers get their own CDP targets.
    Console(WorkerConsoleMessage),
    /// A SharedWorker global is closing and its owner service should remove the instance.
    SharedWorkerClosed,
    /// A SharedWorker V8 Inspector response waiting for the worker target's
    /// concrete output boundary.
    ///
    /// This travels on the same worker-to-parent FIFO as `SharedWorkerClosed`.
    /// The host can therefore bind the response to the exact worker-stream
    /// cursor that precedes it instead of racing a separate response channel
    /// against target retirement.
    SharedWorkerRuntimeInspectorResponse(
        crate::runtime::RendererRuntimeInspectorResponsePublication,
    ),
    /// A Service Worker lifecycle event finished dispatch and all `waitUntil()` promises.
    ServiceWorkerLifecycleCompleted(ServiceWorkerLifecycleCompletion),
    /// A Service Worker fetch event finished dispatch and `respondWith()` settled or fell back.
    ServiceWorkerFetchCompleted(ServiceWorkerFetchCompletion),
    /// A Service Worker fetch event started forwarding a streaming `respondWith()` body.
    ServiceWorkerFetchStreamStarted(ServiceWorkerFetchStreamStarted),
    /// A Service Worker streaming `respondWith()` body produced a chunk.
    ServiceWorkerFetchStreamChunk(ServiceWorkerFetchStreamChunk),
    /// A Service Worker message event finished dispatch and all `waitUntil()` promises.
    ServiceWorkerMessageCompleted(ServiceWorkerMessageCompletion),
    /// A Service Worker notification event finished dispatch and all `waitUntil()` promises.
    ServiceWorkerNotificationCompleted(ServiceWorkerNotificationCompletion),
    /// A Service Worker push event finished dispatch and all `waitUntil()` promises.
    ServiceWorkerPushCompleted(ServiceWorkerPushCompletion),
    /// A Service Worker sync event finished dispatch and all `waitUntil()` promises.
    ServiceWorkerSyncCompleted(ServiceWorkerSyncCompletion),
    /// A Service Worker periodic sync event finished dispatch and all `waitUntil()` promises.
    ServiceWorkerPeriodicSyncCompleted(ServiceWorkerPeriodicSyncCompletion),
    /// A Service Worker requested display/recording of a notification.
    ServiceWorkerShowNotification(ServiceWorkerShowNotification),
    /// A Service Worker requested stored notification snapshots.
    ServiceWorkerGetNotifications(ServiceWorkerGetNotifications),
    /// A Service Worker requested registration of a one-shot background sync tag.
    ServiceWorkerSyncRegistration(ServiceWorkerSyncRegistration),
    /// A Service Worker requested current one-shot background sync tags.
    ServiceWorkerSyncGetTags(ServiceWorkerSyncGetTags),
    /// A Service Worker requested registration of a periodic background sync tag.
    ServiceWorkerPeriodicSyncRegistration(ServiceWorkerPeriodicSyncRegistration),
    /// A Service Worker requested current periodic background sync tags.
    ServiceWorkerPeriodicSyncGetTags(ServiceWorkerPeriodicSyncGetTags),
    /// A Service Worker requested removal of a periodic background sync tag.
    ServiceWorkerPeriodicSyncUnregistration(ServiceWorkerPeriodicSyncUnregistration),
    /// A Service Worker requested a push subscription.
    ServiceWorkerPushSubscribe(ServiceWorkerPushSubscribe),
    /// A Service Worker requested the current push subscription.
    ServiceWorkerPushGetSubscription(ServiceWorkerPushGetSubscription),
    /// A Service Worker requested push subscription deletion.
    ServiceWorkerPushUnsubscribe(ServiceWorkerPushUnsubscribe),
    /// A Service Worker requested closing a stored notification.
    ServiceWorkerCloseNotification(ServiceWorkerCloseNotification),
    /// A Service Worker client object requested delivery to a controlled window client.
    ServiceWorkerClientMessage(ServiceWorkerClientMessage),
    /// A Service Worker object requested delivery to another worker version.
    ServiceWorkerWorkerMessage(ServiceWorkerWorkerMessage),
    /// A Service Worker requested live client snapshots.
    ServiceWorkerClientQuery(ServiceWorkerClientQuery),
    /// A Service Worker WindowClient requested page-owner navigation.
    ServiceWorkerClientNavigate(ServiceWorkerClientNavigate),
    /// A Service Worker WindowClient requested page-owner focus.
    ServiceWorkerClientFocus(ServiceWorkerClientFocus),
    /// A Service Worker requested page-owner window creation.
    ServiceWorkerClientsOpenWindow(ServiceWorkerClientsOpenWindow),
    /// A Service Worker requested `skipWaiting()`.
    ServiceWorkerSkipWaiting {
        registration_id: crate::runtime::ServiceWorkerRegistrationId,
        version_id: crate::runtime::ServiceWorkerVersionId,
    },
    /// A Service Worker requested `clients.claim()`.
    ServiceWorkerClientsClaim {
        registration_id: crate::runtime::ServiceWorkerRegistrationId,
        version_id: crate::runtime::ServiceWorkerVersionId,
    },
    /// A Service Worker install-time import fetch produced a script resource record.
    ServiceWorkerImportedScriptLoaded {
        registration_id: crate::runtime::ServiceWorkerRegistrationId,
        version_id: crate::runtime::ServiceWorkerVersionId,
        resource: WorkerScriptResource,
    },
    /// Deferred CDP Runtime inspector messages produced by later worker tasks.
    RuntimeInspectorMessages(Vec<WorkerRuntimeInspectorMessageBatch>),
    /// Worker-owned subresource activity that should be surfaced through the page/CDP host.
    SubresourceNetwork(SubresourceNetworkRecord),
    /// Worker-owned fetch/XHR that should be paused by the page/CDP Fetch domain.
    PendingSubresourceFetch(WorkerPendingSubresourceFetch),
    /// Worker-owned fetch/XHR was canceled before CDP made a Fetch-domain decision.
    PendingSubresourceFetchCanceled { fetch_id: u32, error_text: String },
    /// Completion signal for a worker-owned fetch/XHR that was continued after interception.
    SubresourceContinue(PendingSubresourceContinueEvent),
    /// Worker-owned WebSocket handshake activity. The embedded socket id is worker-local; the
    /// parent remaps it before exposing it through page-level CDP state.
    WebSocketSubresource(SubresourceNetworkRecord),
    /// Worker-owned WebSocket lifecycle activity.
    WebSocketLifecycle(WorkerWebSocketLifecycleEvent),
    /// Worker-owned WebSocket frame activity.
    WebSocketFrame(WorkerWebSocketFrameEvent),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct WorkerScriptResource {
    pub(crate) request_url: Url,
    pub(crate) final_url: Url,
    pub(crate) kind: WorkerScriptResourceKind,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body_len: usize,
    pub(crate) body_sha256: String,
    pub(crate) response_time_ms: u64,
    pub(crate) mime_type: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum WorkerScriptResourceKind {
    JavaScript,
    CssModule,
    JsonModule,
    TextModule,
    WebAssemblyModule,
}

impl WorkerScriptResourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::JavaScript => "javascript",
            Self::CssModule => "css-module",
            Self::JsonModule => "json-module",
            Self::TextModule => "text-module",
            Self::WebAssemblyModule => "webassembly-module",
        }
    }
}

impl WorkerScriptResource {
    pub(crate) fn from_response_parts(
        request_url: Url,
        head: &ResponseHead,
        body_bytes: &[u8],
        response_time_ms: u64,
    ) -> Self {
        let body_sha256 = sha256_hex(body_bytes);
        let mime_type = moli_web_mime::response_header_value(&head.headers, "content-type");
        Self {
            request_url,
            final_url: head.final_url.clone(),
            kind: WorkerScriptResourceKind::JavaScript,
            status: head.status,
            headers: head.headers.clone(),
            body_len: body_bytes.len(),
            body_sha256,
            response_time_ms,
            mime_type,
        }
    }

    pub(crate) fn with_kind(mut self, kind: WorkerScriptResourceKind) -> Self {
        self.kind = kind;
        self
    }
}

#[derive(Debug)]
pub(crate) struct WorkerBootstrapCompletion {
    pub(crate) result: Result<WorkerBootstrapSuccess, WorkerBootstrapFailure>,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) enum WorkerFetchHandlerType {
    #[default]
    NoHandler,
    NotSkippable,
    EmptyFetchHandler,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WorkerBootstrapSuccess {
    pub(crate) service_worker_fetch_handler_type: WorkerFetchHandlerType,
}

#[derive(Debug)]
pub(crate) struct WorkerBootstrapFailure {
    pub(crate) message: String,
    pub(crate) filename: String,
    pub(crate) lineno: u32,
    pub(crate) colno: u32,
    pub(crate) event_kind: WorkerParentErrorEventKind,
    pub(crate) phase: WorkerErrorPhase,
    pub(crate) source: WorkerErrorSource,
}

impl WorkerBootstrapCompletion {
    pub(crate) fn success(success: WorkerBootstrapSuccess) -> Self {
        Self {
            result: Ok(success),
        }
    }

    pub(crate) fn failure(failure: WorkerBootstrapFailure) -> Self {
        Self {
            result: Err(failure),
        }
    }
}

impl WorkerBootstrapFailure {
    pub(crate) fn from_exception_report(
        report: &crate::exception_reporting::V8ExceptionReport,
        script_url: &str,
        event_kind: WorkerParentErrorEventKind,
        phase: WorkerErrorPhase,
        source: WorkerErrorSource,
    ) -> Self {
        Self {
            message: report.summary.clone(),
            filename: report
                .source
                .clone()
                .unwrap_or_else(|| script_url.to_owned()),
            lineno: report.line.unwrap_or(0) as u32,
            colno: report.column.unwrap_or(0) as u32,
            event_kind,
            phase,
            source,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkerRuntimeInspectorMessageBatch {
    pub(crate) inspector_session_id: Option<String>,
    pub(crate) messages: Vec<RendererRuntimeInspectorMessage>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerConsoleMessage {
    pub(crate) message: String,
    pub(crate) args: Vec<Value>,
    pub(crate) stack: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerParentErrorEventKind {
    Event,
    ErrorEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerErrorPhase {
    Bootstrap,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerErrorSource {
    Runtime,
    InitialScriptEvaluation,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerPendingSubresourceFetch {
    /// Opaque worker-local request id. Historically named `fetch_id`; also used for XHR pauses.
    pub(crate) fetch_id: u32,
    /// Request-time authority. Parent-side CDP continuation must use this
    /// captured client instead of resolving whichever Worker or Document is
    /// current when the command eventually arrives.
    pub(crate) load: crate::network::loads::ResourceLoadLease,
    pub(crate) credentials_mode: RequestCredentialsMode,
    pub(crate) request_mode: moli_fetch::RequestMode,
    pub(crate) network_partition_key: Option<String>,
    pub(crate) info: PendingSubresourceFetchInfo,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerPendingFetchContinue {
    pub(crate) fetch_id: u32,
    pub(crate) internal_id: u64,
    pub(crate) network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub(crate) url: Url,
    pub(crate) method: String,
    pub(crate) body: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) intercept_response: bool,
    pub(crate) handle_auth_requests: bool,
    pub(crate) auth: Option<SubresourceAuthCredentials>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerPendingXhrContinue {
    pub(crate) xhr_id: u32,
    pub(crate) internal_id: u64,
    pub(crate) network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub(crate) url: Url,
    pub(crate) method: String,
    pub(crate) body: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) intercept_response: bool,
    pub(crate) handle_auth_requests: bool,
    pub(crate) auth: Option<SubresourceAuthCredentials>,
}

#[derive(Debug, Clone)]
pub(crate) enum WorkerWebSocketLifecycleEvent {
    Open {
        socket_id: u64,
        document_url: Url,
        url: Url,
    },
    Error {
        socket_id: u64,
        document_url: Url,
        url: Url,
        error_text: String,
    },
    Closing {
        socket_id: u64,
        document_url: Url,
        url: Url,
    },
    Close {
        socket_id: u64,
        document_url: Url,
        url: Url,
        code: u16,
        reason: String,
        was_clean: bool,
    },
}

impl WorkerWebSocketLifecycleEvent {
    pub(crate) fn socket_id(&self) -> u64 {
        match self {
            Self::Open { socket_id, .. }
            | Self::Error { socket_id, .. }
            | Self::Closing { socket_id, .. }
            | Self::Close { socket_id, .. } => *socket_id,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerWebSocketFrameEvent {
    pub(crate) socket_id: u64,
    pub(crate) document_url: Url,
    pub(crate) url: Url,
    pub(crate) direction: WebSocketFrameDirection,
    pub(crate) opcode: WebSocketFrameOpcode,
    pub(crate) payload_length: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkerNetworkPolicy {
    /// Secure-context state for APIs guarded by `[SecureContext]`.
    ///
    /// Dedicated workers normally derive this from their script URL, but
    /// blob/data workers inherit it from the creator context instead.
    pub(crate) secure_context: bool,
    pub(crate) permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub(crate) extra_http_headers: Vec<(String, String)>,
    pub(crate) network_offline: bool,
    pub(crate) blocked_url_patterns: Vec<String>,
    pub(crate) network_partition_key: Option<String>,
    pub(crate) fetch_subresource_interception_enabled: bool,
    pub(crate) fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
}

#[derive(Debug)]
pub(crate) enum WorkerRuntimeEvent {
    Message {
        worker_id: DedicatedWorkerId,
        message: Box<WorkerToParentMessage>,
    },
    SharedWorkerMessage {
        instance_id: SharedWorkerInstanceId,
        message: Box<WorkerToParentMessage>,
    },
    /// Relay terminal ordered behind all DedicatedWorker host-bridge records.
    HostBridgeDrained { worker_id: DedicatedWorkerId },
}

impl WorkerRuntimeEvent {
    pub(crate) fn dedicated_worker_id(&self) -> Option<DedicatedWorkerId> {
        match self {
            Self::Message { worker_id, .. } | Self::HostBridgeDrained { worker_id } => {
                Some(*worker_id)
            }
            Self::SharedWorkerMessage { .. } => None,
        }
    }
}

/// Handle held by the parent (main-frame) context to communicate with a
/// running worker.
#[derive(Clone, Debug)]
pub(crate) struct WorkerDevToolsHandle {
    worker_tx: mpsc::UnboundedSender<WorkerMessage>,
    inspector_tasks: WorkerInspectorTaskRunner,
}

impl WorkerDevToolsHandle {
    pub(crate) fn new(
        wake_tx: mpsc::UnboundedSender<WorkerMessage>,
        isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
    ) -> Self {
        Self {
            inspector_tasks: WorkerInspectorTaskRunner::new(wake_tx.clone(), isolate_handle),
            worker_tx: wake_tx,
        }
    }

    pub(crate) fn inspector_tasks(&self) -> &WorkerInspectorTaskRunner {
        &self.inspector_tasks
    }

    pub(crate) fn dispatch_runtime_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        response_tx: oneshot::Sender<Result<Vec<RendererRuntimeInspectorMessage>, String>>,
    ) -> bool {
        self.inspector_tasks.append_protocol_message(
            inspector_session_id,
            raw_json,
            deferred_response,
            response_tx,
        )
    }

    pub(crate) fn attach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.inspector_tasks.append_attach(inspector_session_id)
    }

    pub(crate) fn detach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.inspector_tasks.append_detach(inspector_session_id)
    }

    pub(crate) fn run_if_waiting_for_debugger(&self) -> bool {
        self.inspector_tasks.append_run_if_waiting_for_debugger()
    }

    pub(crate) fn dispose(&self, message: &str) {
        self.inspector_tasks.dispose(message);
    }

    pub(crate) fn terminate_for_devtools(&self) -> bool {
        self.dispose("Worker closed before Inspector task dispatch");
        self.worker_tx.send(WorkerMessage::Terminate).is_ok()
    }
}

pub(crate) struct WorkerHandle {
    /// Send messages *to* the worker.
    pub(crate) tx: mpsc::UnboundedSender<WorkerMessage>,
    /// Receive messages *from* the worker.
    rx: Option<mpsc::UnboundedReceiver<WorkerToParentMessage>>,
    /// Join handle for the worker OS thread.
    join_handle: Option<std::thread::JoinHandle<()>>,
    isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
    termination_requested: Arc<AtomicBool>,
    devtools: WorkerDevToolsHandle,
}

impl WorkerHandle {
    #[cfg(test)]
    pub(crate) fn new(
        tx: mpsc::UnboundedSender<WorkerMessage>,
        rx: mpsc::UnboundedReceiver<WorkerToParentMessage>,
        join_handle: std::thread::JoinHandle<()>,
        isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
    ) -> Self {
        Self::new_with_termination_requested(
            tx,
            rx,
            join_handle,
            isolate_handle,
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_termination_requested(
        tx: mpsc::UnboundedSender<WorkerMessage>,
        rx: mpsc::UnboundedReceiver<WorkerToParentMessage>,
        join_handle: std::thread::JoinHandle<()>,
        isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
        termination_requested: Arc<AtomicBool>,
    ) -> Self {
        let devtools = WorkerDevToolsHandle::new(tx.clone(), Arc::clone(&isolate_handle));
        Self::new_with_termination_requested_and_devtools(
            tx,
            rx,
            join_handle,
            isolate_handle,
            termination_requested,
            devtools,
        )
    }

    pub(crate) fn new_with_termination_requested_and_devtools(
        tx: mpsc::UnboundedSender<WorkerMessage>,
        rx: mpsc::UnboundedReceiver<WorkerToParentMessage>,
        join_handle: std::thread::JoinHandle<()>,
        isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
        termination_requested: Arc<AtomicBool>,
        devtools: WorkerDevToolsHandle,
    ) -> Self {
        Self {
            tx,
            rx: Some(rx),
            join_handle: Some(join_handle),
            isolate_handle,
            termination_requested,
            devtools,
        }
    }

    fn terminate_execution_if_ready(&self) {
        if let Some(handle) = self.isolate_handle.lock().as_ref() {
            handle.terminate_execution();
        }
    }

    fn request_termination(&self) {
        // Publish the lifecycle transition before interrupting V8. The worker
        // event loop can then reject an already-selected task without relying
        // on ordering between cloned mpsc senders.
        self.devtools
            .dispose("Worker terminated before Inspector task dispatch");
        self.termination_requested.store(true, Ordering::Release);
        self.terminate_execution_if_ready();
        let _ = self.tx.send(WorkerMessage::Terminate);
    }

    /// Send a message to the worker (`postMessage`).
    pub(crate) fn post_message(&self, payload: V8StructuredClonePayload) {
        let _ = self.tx.send(WorkerMessage::Post(payload));
    }

    /// Ask the worker to terminate.
    pub(crate) fn terminate(&self) {
        self.request_termination();
    }

    pub(crate) fn terminate_and_join(mut self) {
        self.request_termination();
        if let Some(join_handle) = self.join_handle.take()
            && join_handle.thread().id() != std::thread::current().id()
        {
            let _ = join_handle.join();
        }
    }

    pub(crate) fn dispatch_runtime_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<RendererRuntimeInspectorResponseSender>,
        response_tx: oneshot::Sender<Result<Vec<RendererRuntimeInspectorMessage>, String>>,
    ) -> bool {
        self.devtools.dispatch_runtime_protocol_message(
            inspector_session_id,
            raw_json,
            deferred_response,
            response_tx,
        )
    }

    #[cfg(test)]
    pub(crate) fn attach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.devtools
            .attach_runtime_inspector_session(inspector_session_id)
    }

    pub(crate) fn detach_runtime_inspector_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> bool {
        self.devtools
            .detach_runtime_inspector_session(inspector_session_id)
    }

    pub(crate) fn run_if_waiting_for_debugger_for_devtools(&self) -> bool {
        self.devtools.run_if_waiting_for_debugger()
    }

    pub(crate) fn devtools_handle(&self) -> WorkerDevToolsHandle {
        self.devtools.clone()
    }

    pub(crate) fn set_extra_http_headers(&self, headers: &[(String, String)]) {
        let _ = self
            .tx
            .send(WorkerMessage::SetExtraHttpHeaders(headers.to_vec()));
    }

    pub(crate) fn set_network_offline(&self, offline: bool) {
        let _ = self.tx.send(WorkerMessage::SetNetworkOffline(offline));
    }

    pub(crate) fn set_blocked_url_patterns(&self, patterns: &[String]) {
        let _ = self
            .tx
            .send(WorkerMessage::SetBlockedUrlPatterns(patterns.to_vec()));
    }

    pub(crate) fn set_fetch_subresource_interception(
        &self,
        enabled: bool,
        resource_type: Option<SubresourceResourceType>,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::SetFetchSubresourceInterception {
                enabled,
                resource_type,
            });
    }

    pub(crate) fn dispatch_service_worker_lifecycle_event(
        &self,
        event: ServiceWorkerLifecycleEvent,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerLifecycleEvent(Box::new(event)));
    }

    pub(crate) fn dispatch_service_worker_fetch_event(&self, event: ServiceWorkerFetchEvent) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerFetchEvent(Box::new(event)));
    }

    pub(crate) fn start_service_worker_navigation_preload_response(
        &self,
        started: ServiceWorkerNavigationPreloadResponseStarted,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerNavigationPreloadResponseStarted(Box::new(started)));
    }

    pub(crate) fn enqueue_service_worker_navigation_preload_chunk(
        &self,
        chunk: ServiceWorkerNavigationPreloadStreamChunk,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerNavigationPreloadStreamChunk(
                chunk,
            ));
    }

    pub(crate) fn finish_service_worker_navigation_preload_stream(
        &self,
        finished: ServiceWorkerNavigationPreloadStreamFinished,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerNavigationPreloadStreamFinished(
                Box::new(finished),
            ));
    }

    pub(crate) fn fail_service_worker_navigation_preload(
        &self,
        failure: ServiceWorkerNavigationPreloadFailure,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerNavigationPreloadFailure(
                Box::new(failure),
            ));
    }

    pub(crate) fn cancel_service_worker_fetch_stream(
        &self,
        event_id: crate::runtime::ServiceWorkerEventId,
        body_source_id: NetworkBodySourceId,
    ) {
        let _ = self.tx.send(WorkerMessage::ServiceWorkerFetchStreamCancel {
            event_id,
            body_source_id,
        });
    }

    pub(crate) fn abort_service_worker_fetch_request_signal(
        &self,
        event_id: crate::runtime::ServiceWorkerEventId,
        reason: Option<V8StructuredClonePayload>,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerFetchRequestSignalAbort { event_id, reason });
    }

    pub(crate) fn dispatch_service_worker_message_event(&self, event: ServiceWorkerMessageEvent) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerMessageEvent(Box::new(event)));
    }

    pub(crate) fn dispatch_service_worker_notification_event(
        &self,
        event: ServiceWorkerNotificationEvent,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerNotificationEvent(Box::new(
                event,
            )));
    }

    pub(crate) fn dispatch_service_worker_push_event(&self, event: ServiceWorkerPushEvent) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerPushEvent(Box::new(event)));
    }

    pub(crate) fn dispatch_service_worker_sync_event(&self, event: ServiceWorkerSyncEvent) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerSyncEvent(Box::new(event)));
    }

    pub(crate) fn dispatch_service_worker_periodic_sync_event(
        &self,
        event: ServiceWorkerPeriodicSyncEvent,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerPeriodicSyncEvent(Box::new(
                event,
            )));
    }

    pub(crate) fn dispatch_service_worker_sync_registration_result(
        &self,
        result: ServiceWorkerSyncRegistrationResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerSyncRegistrationResult(result));
    }

    pub(crate) fn dispatch_service_worker_sync_get_tags_result(
        &self,
        result: ServiceWorkerSyncGetTagsResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerSyncGetTagsResult(result));
    }

    pub(crate) fn dispatch_service_worker_periodic_sync_registration_result(
        &self,
        result: ServiceWorkerPeriodicSyncRegistrationResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerPeriodicSyncRegistrationResult(
                result,
            ));
    }

    pub(crate) fn dispatch_service_worker_periodic_sync_get_tags_result(
        &self,
        result: ServiceWorkerPeriodicSyncGetTagsResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerPeriodicSyncGetTagsResult(
                result,
            ));
    }

    pub(crate) fn dispatch_service_worker_periodic_sync_unregistration_result(
        &self,
        result: ServiceWorkerPeriodicSyncUnregistrationResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerPeriodicSyncUnregistrationResult(result));
    }

    pub(crate) fn dispatch_service_worker_push_subscribe_result(
        &self,
        result: ServiceWorkerPushSubscribeResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerPushSubscribeResult(result));
    }

    pub(crate) fn dispatch_service_worker_push_get_subscription_result(
        &self,
        result: ServiceWorkerPushGetSubscriptionResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerPushGetSubscriptionResult(
                result,
            ));
    }

    pub(crate) fn dispatch_service_worker_push_unsubscribe_result(
        &self,
        result: ServiceWorkerPushUnsubscribeResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerPushUnsubscribeResult(result));
    }

    pub(crate) fn dispatch_service_worker_client_query_result(
        &self,
        result: ServiceWorkerClientQueryResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerClientQueryResult(result));
    }

    pub(crate) fn dispatch_service_worker_client_navigate_result(
        &self,
        result: ServiceWorkerClientNavigateResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerClientNavigateResult(result));
    }

    pub(crate) fn dispatch_service_worker_client_focus_result(
        &self,
        result: ServiceWorkerClientFocusResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerClientFocusResult(result));
    }

    pub(crate) fn dispatch_service_worker_clients_open_window_result(
        &self,
        result: ServiceWorkerClientsOpenWindowResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerClientsOpenWindowResult(result));
    }

    pub(crate) fn dispatch_service_worker_get_notifications_result(
        &self,
        result: ServiceWorkerGetNotificationsResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerGetNotificationsResult(result));
    }

    pub(crate) fn dispatch_service_worker_show_notification_result(
        &self,
        result: ServiceWorkerShowNotificationResult,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerShowNotificationResult(result));
    }

    pub(crate) fn continue_pending_fetch(&self, request: WorkerPendingFetchContinue) {
        let _ = self.tx.send(WorkerMessage::ContinuePendingFetch(request));
    }

    pub(crate) fn continue_pending_xhr(&self, request: WorkerPendingXhrContinue) {
        let _ = self.tx.send(WorkerMessage::ContinuePendingXhr(request));
    }

    pub(crate) fn continue_pending_csp_report(&self, request: WorkerPendingFetchContinue) {
        let _ = self
            .tx
            .send(WorkerMessage::ContinuePendingCspReport(request));
    }

    pub(crate) fn continue_pending_fetch_response(
        &self,
        request: WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) {
        let _ = self.tx.send(WorkerMessage::ContinuePendingFetchResponse {
            request,
            response_code,
            response_headers,
        });
    }

    pub(crate) fn continue_pending_xhr_response(
        &self,
        request: WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) {
        let _ = self.tx.send(WorkerMessage::ContinuePendingXhrResponse {
            request,
            response_code,
            response_headers,
        });
    }

    pub(crate) fn fail_pending_fetch(
        &self,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) {
        let _ = self.tx.send(WorkerMessage::FailPendingFetch {
            request,
            error_text,
        });
    }

    pub(crate) fn fail_pending_xhr(&self, request: WorkerPendingXhrContinue, error_text: String) {
        let _ = self.tx.send(WorkerMessage::FailPendingXhr {
            request,
            error_text,
        });
    }

    pub(crate) fn fail_pending_csp_report(
        &self,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) {
        let _ = self.tx.send(WorkerMessage::FailPendingCspReport {
            request,
            error_text,
        });
    }

    pub(crate) fn fail_pending_fetch_auth(
        &self,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) {
        let _ = self.tx.send(WorkerMessage::FailPendingFetchAuth {
            request,
            error_text,
        });
    }

    pub(crate) fn fail_pending_xhr_auth(
        &self,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) {
        let _ = self.tx.send(WorkerMessage::FailPendingXhrAuth {
            request,
            error_text,
        });
    }

    pub(crate) fn fail_pending_fetch_response(
        &self,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) {
        let _ = self.tx.send(WorkerMessage::FailPendingFetchResponse {
            request,
            error_text,
        });
    }

    pub(crate) fn fail_pending_xhr_response(
        &self,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) {
        let _ = self.tx.send(WorkerMessage::FailPendingXhrResponse {
            request,
            error_text,
        });
    }

    pub(crate) fn fulfill_pending_fetch(
        &self,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) {
        let _ = self.tx.send(WorkerMessage::FulfillPendingFetch {
            request,
            response_code,
            response_headers,
            response_body,
        });
    }

    pub(crate) fn fulfill_pending_xhr(
        &self,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) {
        let _ = self.tx.send(WorkerMessage::FulfillPendingXhr {
            request,
            response_code,
            response_headers,
            response_body,
        });
    }

    pub(crate) fn fulfill_pending_csp_report(
        &self,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) {
        let _ = self.tx.send(WorkerMessage::FulfillPendingCspReport {
            request,
            response_code,
            response_headers,
            response_body,
        });
    }

    pub(crate) fn fulfill_pending_fetch_response(
        &self,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) {
        let _ = self.tx.send(WorkerMessage::FulfillPendingFetchResponse {
            request,
            response_code,
            response_headers,
            response_body,
        });
    }

    pub(crate) fn fulfill_pending_xhr_response(
        &self,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) {
        let _ = self.tx.send(WorkerMessage::FulfillPendingXhrResponse {
            request,
            response_code,
            response_headers,
            response_body,
        });
    }

    pub(crate) fn take_receiver(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<WorkerToParentMessage>> {
        self.rx.take()
    }

    #[cfg(test)]
    pub(crate) async fn recv(&mut self) -> Option<WorkerToParentMessage> {
        self.rx.as_mut()?.recv().await
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Result<WorkerToParentMessage, TryRecvError> {
        match self.rx.as_mut() {
            Some(rx) => rx.try_recv(),
            None => Err(TryRecvError::Disconnected),
        }
    }

    #[cfg(test)]
    pub(crate) async fn resource_owner_slot_diagnostics(
        &self,
    ) -> Result<WorkerResourceOwnerSlotDiagnostics, String> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(WorkerMessage::ResourceOwnerSlotDiagnostics { response_tx })
            .map_err(|_| "worker closed before resource-owner slot diagnostics".to_owned())?;
        response_rx
            .await
            .map_err(|_| "worker closed without resource-owner slot diagnostics".to_owned())?
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        // Signal termination so the worker thread can exit, but do not
        // synchronously join here. Render-side teardown must not block on a
        // worker thread finishing its event loop.
        self.request_termination();
        let _ = self.join_handle.take();
    }
}
