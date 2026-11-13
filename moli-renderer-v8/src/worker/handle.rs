//! Worker-side message types and parent-facing handle.

use crate::diagnostics::{ErrorReportOutcome, WorkerExceptionDiagnostic};
use crate::protocol_types::SubresourceNetworkRecord;
use crate::runtime::WorkerInspectorError;
use crate::runtime::{
    RendererRuntimeInspectorMessage, ServiceWorkerClientFocus, ServiceWorkerClientFocusResult,
    ServiceWorkerClientMessage, ServiceWorkerClientNavigate, ServiceWorkerClientNavigateResult,
    ServiceWorkerClientQuery, ServiceWorkerClientQueryResult, ServiceWorkerClientsOpenWindow,
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
#[cfg(feature = "devtools")]
use crate::worker::inspector_task_runner::WorkerInspectorTaskMode;
use crate::worker::inspector_task_runner::WorkerInspectorTaskRunner;
use anyhow::Result;
use moli_crypto::sha256_hex;
use moli_fetch::ResponseHead;
use moli_shared_worker::SharedWorkerInstanceId;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;
#[cfg(test)]
use tokio::sync::mpsc::error::TryRecvError;
#[cfg(test)]
use tokio::sync::oneshot;

#[cfg(feature = "devtools")]
mod devtools;
use url::Url;

/// How to determine a worker's secure-context state during startup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum WorkerSecureContextPolicy {
    /// Starts without a creator, including restored service workers.
    #[default]
    ScriptUrl,
    /// Use the creator's security state as a prerequisite for the worker.
    Inherited(bool),
}

impl WorkerSecureContextPolicy {
    pub(crate) fn resolve(self, script_url: &Url) -> bool {
        match self {
            Self::ScriptUrl => {
                !matches!(script_url.scheme(), "blob" | "data")
                    && moli_url::is_potentially_trustworthy_url(script_url)
            }
            Self::Inherited(secure) => {
                // Blob/data workers inherit the creator's state even though a
                // standalone data URL is not itself trustworthy.
                secure
                    && (matches!(script_url.scheme(), "blob" | "data")
                        || moli_url::is_potentially_trustworthy_url(script_url))
            }
        }
    }
}

/// Message from the parent context to the worker.
#[derive(Debug)]
pub(crate) enum WorkerMessage {
    /// Release bootstrap after the native owner has registered the Worker.
    Start,
    ServiceWorkerRouterRulesRegistered {
        request_id: u64,
        result: Result<(), String>,
    },
    /// A `postMessage` payload encoded via V8 structured clone.
    Post(V8StructuredClonePayload),
    /// Dispatch a ServiceWorker message at this client's container, retaining
    /// the source version and origin rather than using the parent message port.
    ServiceWorkerClientMessage(Box<crate::types::ServiceWorkerClientMessageData>),
    /// A DOM manipulation task belonging to an encoded RTP transformer.
    RtpTransform(super::rtp_tasks::RtpTransformTask),
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
    /// Apply a registration snapshot and its events on this service worker loop.
    ServiceWorkerRegistrationTask(
        Box<crate::service_worker_runtime::ServiceWorkerRegistrationTask>,
    ),
    /// Settle a successful container lookup in a later worker task.
    ServiceWorkerRegistrationQueryResult {
        request_id: u64,
        result: Box<crate::types::ServiceWorkerRegistrationQueryResult>,
    },
    ServiceWorkerUnregisterResult {
        request_id: u64,
        result: bool,
    },
    /// Settle a register/update promise on the requesting worker loop.
    ServiceWorkerUpdateResult {
        request_id: u64,
        result: Box<
            Result<
                crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot,
                crate::service_worker_runtime::ServiceWorkerRegistrationError,
            >,
        >,
    },
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
    /// Resolve skipWaiting() after its owning runtime has tried activation.
    ServiceWorkerSkipWaitingResult {
        request_id: u64,
    },
    /// Settle clients.claim() after the owning runtime has applied the claim.
    ServiceWorkerClientsClaimResult {
        request_id: u64,
        claimed: bool,
    },
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
    /// Run one task from the worker's networking task source.
    RunNetworkingTask,
    /// Run one task from the worker's font loading task source.
    RunFontLoadingTask,
    /// Run one browser-owned WebCodecs completion.
    RunCodecTask,
    /// Settle one ImageBitmap result on its owning worker event loop.
    RunBitmapTask(u64),
    /// Complete one native WebTransport operation on the networking task source.
    RunWebTransportTask(u64),
    RunWebLocksTask(u64, moli_storage_service::WebLockEvent),
    /// Settle an encoded OffscreenCanvas file on its owning worker event loop.
    RunCanvasBlobTask(u64),
    /// Dispatch a CSP violation queued while the current worker was still evaluating script.
    DispatchContentSecurityPolicyViolation(
        Box<crate::content_security_policy::ContentSecurityPolicyUrlViolation>,
    ),
    /// Deliver a local XHR failure after its already queued CSP violations.
    DispatchXhrFailure {
        xhr_id: u32,
        error_text: String,
        local_request: Box<crate::types::PendingSubresourceFetchInfo>,
    },
    /// A worker spawned from this worker has queued a parent-facing event.
    NestedWorkerEvent {
        worker_id: DedicatedWorkerId,
        message: Box<WorkerToParentMessage>,
    },
    /// Finish a nested main-script fetch on its creator's event loop.
    NestedWorkerScriptLoaded(Box<crate::context_bootstrap::NestedWorkerScriptLoadCompletion>),
    #[cfg(feature = "devtools")]
    /// Owner-thread dispatch or fallback for one queued Inspector task.
    RunInspectorTask(WorkerInspectorTaskMode),
    /// One coalesced fallback wake for the isolate's environment mailbox.
    RunEnvironmentNotification,
    #[cfg(test)]
    /// Inspect worker resource-owner V8 slots from inside the worker thread.
    ResourceOwnerSlotDiagnostics {
        response_tx: oneshot::Sender<Result<WorkerResourceOwnerSlotDiagnostics, String>>,
    },
    /// Update the Network.setExtraHTTPHeaders state visible to worker-owned fetch/XHR.
    SetExtraHttpHeaders(moli_fetch::RequestHeaders),
    /// Update the Network.emulateNetworkConditions offline state visible to worker-owned fetch/XHR.
    SetNetworkOffline(bool),
    /// Update the effective Network.setCacheDisabled state visible to worker-owned fetch/XHR.
    SetCacheDisabled(bool),
    /// Update the Network.setBlockedURLs state visible to worker-owned fetch/XHR.
    SetBlockedUrlPatterns(Vec<String>),
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
    ServiceWorkerRegisterRouterRules {
        request_id: u64,
        rules: Vec<crate::service_worker_runtime::static_router::RouterRule>,
    },
    /// A `postMessage` payload encoded via V8 structured clone.
    Post(V8StructuredClonePayload),
    /// The worker encountered an unhandled error.
    Error(WorkerErrorReport),
    /// A worker `console.*` call that should be surfaced through the owning page until
    /// workers get their own CDP targets.
    Console(WorkerConsoleMessage),
    /// A SharedWorker global is closing and its owner service should remove the instance.
    SharedWorkerClosed,
    #[cfg(feature = "devtools")]
    /// A V8 Inspector response waiting for its Worker target's concrete output
    /// boundary.
    ///
    /// This travels on the same worker-to-parent FIFO as lifecycle and runtime
    /// observations. The host can therefore bind the response to the exact
    /// target-stream cursor that precedes it instead of racing a separate
    /// response channel against target retirement.
    RuntimeInspectorResponse(crate::runtime::RendererRuntimeInspectorResponsePublication),
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
    ServiceWorkerShowNotification(Box<ServiceWorkerShowNotification>),
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
        request_id: u64,
        registration_id: crate::runtime::ServiceWorkerRegistrationId,
        version_id: crate::runtime::ServiceWorkerVersionId,
    },
    /// A Service Worker requested `clients.claim()`.
    ServiceWorkerClientsClaim {
        request_id: u64,
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
    #[cfg(test)]
    PendingSubresourceFetch(WorkerPendingSubresourceFetch),
    /// Worker-owned fetch/XHR was canceled before CDP made a Fetch-domain decision.

    /// Completion signal for a worker-owned fetch/XHR that was continued after interception.
    #[cfg(test)]
    SubresourceContinue(PendingSubresourceContinueEvent),
}

/// The error data used by each owner to dispatch its event and settle the diagnostic.
#[derive(Debug)]
pub(crate) struct WorkerErrorReport {
    pub(crate) message: String,
    pub(crate) filename: String,
    pub(crate) lineno: u32,
    pub(crate) colno: u32,
    pub(crate) event_kind: WorkerParentErrorEventKind,
    pub(crate) phase: WorkerErrorPhase,
    pub(crate) source: WorkerErrorSource,
    pub(super) diagnostic: Option<WorkerExceptionDiagnostic>,
}

impl WorkerErrorReport {
    pub(crate) fn new(
        message: String,
        filename: String,
        (lineno, colno): (u32, u32),
        event_kind: WorkerParentErrorEventKind,
        phase: WorkerErrorPhase,
        source: WorkerErrorSource,
    ) -> Self {
        Self {
            message,
            filename,
            lineno,
            colno,
            event_kind,
            phase,
            source,
            diagnostic: None,
        }
    }

    pub(crate) fn complete(&mut self, outcome: impl FnOnce() -> ErrorReportOutcome) {
        if let Some(diagnostic) = self.diagnostic.take() {
            diagnostic.publish(
                &self.message,
                &self.filename,
                (self.lineno, self.colno),
                outcome,
            );
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct WorkerScriptResource {
    pub(crate) request_url: Url,
    pub(crate) final_url: Url,
    pub(crate) kind: WorkerScriptResourceKind,
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, Vec<u8>)>,
    pub(crate) body_len: usize,
    pub(crate) body_sha256: String,
    pub(crate) response_time_ms: u64,
    pub(crate) mime_type: Option<String>,
    pub(crate) classic_script: Option<WorkerStoredClassicScript>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct WorkerStoredClassicScript {
    pub(crate) source: Arc<str>,
    pub(crate) muted_errors: bool,
    pub(crate) redirect_urls: Vec<Url>,
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
            classic_script: None,
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

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct WorkerPendingSubresourceFetch {
    pub(crate) request_policy:
        Option<(moli_fetch::RequestMode, moli_fetch::RequestCredentialsMode)>,
    pub(crate) fetch_id: u64,
    pub(crate) network_partition_key: Option<String>,
    pub(crate) info: PendingSubresourceFetchInfo,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct WorkerPendingFetchContinue {
    pub(crate) fetch_id: u64,
    pub(crate) url: Url,
    pub(crate) method: String,
    /// Upload bytes after applying explicit overrides, including an empty body.
    pub(crate) body: Option<bytes::Bytes>,
    pub(crate) headers: moli_fetch::RequestHeaders,
    pub(crate) intercept_response: bool,
    pub(crate) handle_auth_requests: bool,
    pub(crate) auth: Option<SubresourceAuthCredentials>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct WorkerPendingXhrContinue {
    pub(crate) xhr_id: u64,
    pub(crate) url: Url,
    pub(crate) method: String,
    /// Upload bytes after applying explicit overrides, including an empty body.
    pub(crate) body: Option<bytes::Bytes>,
    pub(crate) headers: moli_fetch::RequestHeaders,
    pub(crate) intercept_response: bool,
    pub(crate) handle_auth_requests: bool,
    pub(crate) auth: Option<SubresourceAuthCredentials>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkerNetworkPolicy {
    /// Startup policy for APIs guarded by `[SecureContext]`.
    ///
    /// Page-created workers inherit the creator's state, including insecure
    /// ancestors. Native starts without a creator derive it from the script
    /// URL; blob/data workers require an explicit creator state.
    pub(crate) secure_context: WorkerSecureContextPolicy,
    pub(crate) permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub(crate) extra_http_headers: moli_fetch::RequestHeaders,
    pub(crate) network_offline: bool,
    pub(crate) blocked_url_patterns: Vec<String>,
    pub(crate) network_partition_key: Option<String>,
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

/// Shared handle for worker startup, termination and network cancellation.
#[derive(Clone, Debug)]
pub(crate) struct WorkerControlHandle {
    network_cancel: moli_fetch::FetchCancelHandle,
    isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
    worker_tx: mpsc::UnboundedSender<WorkerMessage>,
    inspector_tasks: WorkerInspectorTaskRunner,
}

impl WorkerControlHandle {
    pub(crate) fn new(
        wake_tx: mpsc::UnboundedSender<WorkerMessage>,
        isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
    ) -> Self {
        Self {
            network_cancel: moli_fetch::FetchCancelHandle::new(),
            isolate_handle: Arc::clone(&isolate_handle),
            inspector_tasks: WorkerInspectorTaskRunner::new(wake_tx.clone(), isolate_handle),
            worker_tx: wake_tx,
        }
    }

    pub(crate) fn network_cancel_handle(&self) -> moli_fetch::FetchCancelHandle {
        self.network_cancel.clone()
    }

    pub(crate) fn inspector_tasks(&self) -> &WorkerInspectorTaskRunner {
        &self.inspector_tasks
    }

    fn dispose(&self, error: WorkerInspectorError) {
        self.network_cancel.cancel();
        self.inspector_tasks.dispose(error);
    }

    pub(crate) fn terminate(&self) -> bool {
        self.terminate_with_error(WorkerInspectorError::Closed)
    }

    fn terminate_with_error(&self, error: WorkerInspectorError) -> bool {
        if let Some(handle) = self.isolate_handle.lock().as_ref() {
            handle.terminate_execution();
        }
        self.dispose(error);
        self.worker_tx.send(WorkerMessage::Terminate).is_ok()
    }

    fn start(&self) -> bool {
        self.worker_tx.send(WorkerMessage::Start).is_ok()
    }
}

pub(crate) struct WorkerHandle {
    /// Send messages *to* the worker.
    pub(crate) tx: mpsc::UnboundedSender<WorkerMessage>,
    /// Receive messages *from* the worker.
    rx: Option<mpsc::UnboundedReceiver<WorkerToParentMessage>>,
    /// Join handle for the worker OS thread.
    join_handle: Option<std::thread::JoinHandle<()>>,
    termination_requested: Arc<AtomicBool>,
    control: WorkerControlHandle,
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
        let control = WorkerControlHandle::new(tx.clone(), Arc::clone(&isolate_handle));
        Self::new_with_control(rx, join_handle, termination_requested, control)
    }

    pub(crate) fn new_with_control(
        rx: mpsc::UnboundedReceiver<WorkerToParentMessage>,
        join_handle: std::thread::JoinHandle<()>,
        termination_requested: Arc<AtomicBool>,
        control: WorkerControlHandle,
    ) -> Self {
        Self {
            tx: control.worker_tx.clone(),
            rx: Some(rx),
            join_handle: Some(join_handle),
            termination_requested,
            control,
        }
    }

    fn request_termination(&self) {
        // Publish the lifecycle transition before interrupting V8. The worker
        // event loop can then reject an already-selected task without relying
        // on ordering between cloned mpsc senders.
        self.termination_requested.store(true, Ordering::Release);
        let _ = self
            .control
            .terminate_with_error(WorkerInspectorError::Terminated);
    }

    pub(crate) fn start_evaluation(&self) -> bool {
        self.control.start()
    }

    /// Send a message to the worker (`postMessage`).
    #[cfg(test)]
    pub(crate) fn post_message(&self, payload: V8StructuredClonePayload) {
        self.queue_task(WorkerMessage::Post(payload));
    }

    pub(crate) fn queue_task(&self, task: WorkerMessage) {
        let _ = self.tx.send(task);
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

    pub(crate) fn control_handle(&self) -> WorkerControlHandle {
        self.control.clone()
    }

    pub(crate) fn set_extra_http_headers(&self, headers: &moli_fetch::RequestHeaders) {
        let _ = self
            .tx
            .send(WorkerMessage::SetExtraHttpHeaders(headers.clone()));
    }

    pub(crate) fn set_network_offline(&self, offline: bool) {
        let _ = self.tx.send(WorkerMessage::SetNetworkOffline(offline));
    }

    pub(crate) fn set_cache_disabled(&self, disabled: bool) {
        let _ = self.tx.send(WorkerMessage::SetCacheDisabled(disabled));
    }

    pub(crate) fn set_blocked_url_patterns(&self, patterns: &[String]) {
        let _ = self
            .tx
            .send(WorkerMessage::SetBlockedUrlPatterns(patterns.to_vec()));
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

    pub(crate) fn dispatch_service_worker_skip_waiting_result(&self, request_id: u64) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerSkipWaitingResult { request_id });
    }

    pub(crate) fn dispatch_service_worker_router_rules_result(
        &self,
        request_id: u64,
        result: Result<(), String>,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerRouterRulesRegistered { request_id, result });
    }

    pub(crate) fn dispatch_service_worker_clients_claim_result(
        &self,
        request_id: u64,
        claimed: bool,
    ) {
        let _ = self
            .tx
            .send(WorkerMessage::ServiceWorkerClientsClaimResult {
                request_id,
                claimed,
            });
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

    pub(crate) fn take_receiver(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<WorkerToParentMessage>> {
        self.rx.take()
    }

    #[cfg(test)]
    pub(crate) fn replace_test_receiver(
        &mut self,
        rx: mpsc::UnboundedReceiver<WorkerToParentMessage>,
    ) {
        self.rx = Some(rx);
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

#[cfg(test)]
use crate::protocol_types::{
    PendingSubresourceContinueEvent, PendingSubresourceFetchInfo, SubresourceAuthCredentials,
};

#[cfg(test)]
mod error_report_tests {
    use super::*;
    use crate::diagnostics::{
        DiagnosticEmitter, DiagnosticEvent, DiagnosticSource, ExceptionOrigin,
    };

    #[test]
    fn worker_error_publishes_once_only_when_completed() {
        let diagnostics = DiagnosticEmitter::default();
        let mut receiver = diagnostics.subscribe().expect("first subscription");
        let source = DiagnosticSource::DedicatedWorker { instance_id: 2 };
        for expected in [
            None,
            Some(ErrorReportOutcome::Canceled),
            Some(ErrorReportOutcome::Uncanceled),
        ] {
            {
                let mut report = WorkerErrorReport::new(
                    "worker exception".into(),
                    "worker.js".into(),
                    (1, 2),
                    WorkerParentErrorEventKind::ErrorEvent,
                    WorkerErrorPhase::Runtime,
                    WorkerErrorSource::Runtime,
                );
                report.diagnostic = diagnostics
                    .prepare_worker_exception(source, || Some("at throwFromWorker".into()));
                assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
                if let Some(outcome) = expected {
                    report.complete(|| outcome);
                }
            }
            if let Some(outcome) = expected {
                assert_eq!(
                    receiver.try_recv(),
                    Ok(DiagnosticEvent::RuntimeException {
                        source,
                        origin: ExceptionOrigin::UncaughtException,
                        message: "worker exception".into(),
                        filename: "worker.js".into(),
                        lineno: 1,
                        colno: 2,
                        stack: Some("at throwFromWorker".into()),
                        outcome,
                    })
                );
            }
            assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
        }
    }
}
