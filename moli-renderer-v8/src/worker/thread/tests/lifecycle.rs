use super::*;
use crate::runtime::RendererBrowserContextRuntime;
use crate::runtime::{ServiceWorkerNotificationMetadata, ServiceWorkerShowNotificationResult};
use crate::service_worker_runtime::ServiceWorkerNotificationEventKind;
use crate::worker::WorkerGlobalKind;

async fn dispatch_service_worker_lifecycle_event_and_console_for_test(
    handle: &mut crate::worker::WorkerHandle,
    kind: ServiceWorkerLifecycleEventKind,
    event_id: u64,
) -> (ServiceWorkerLifecycleCompletion, String) {
    handle.dispatch_service_worker_lifecycle_event(ServiceWorkerLifecycleEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        kind,
    });

    let mut completion = None;
    let mut console = None;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for lifecycle event and console")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(message) => {
                completion = Some(message);
            }
            WorkerToParentMessage::Console(message) => {
                console = Some(message.message);
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error while waiting for lifecycle: {message}");
            }
            other => panic!("unexpected message while waiting for lifecycle console: {other:?}"),
        }
        if let (Some(completion), Some(console)) = (completion.as_ref(), console.as_ref()) {
            return (completion.clone(), console.clone());
        }
    }
}

async fn dispatch_service_worker_message_event_and_console_for_test(
    handle: &mut crate::worker::WorkerHandle,
    event_id: u64,
    data: V8StructuredClonePayload,
) -> (ServiceWorkerMessageCompletion, String) {
    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: data,
        window_interaction_allowed: false,
    });

    let mut completion = None;
    let mut console = None;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for message event and console")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerMessageCompleted(message) => {
                completion = Some(message);
            }
            WorkerToParentMessage::Console(message) => {
                console = Some(message.message);
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error while waiting for message: {message}");
            }
            other => panic!("unexpected message while waiting for message console: {other:?}"),
        }
        if let (Some(completion), Some(console)) = (completion.as_ref(), console.as_ref()) {
            return (completion.clone(), console.clone());
        }
    }
}

async fn dispatch_service_worker_fetch_event_and_handled_console_for_test(
    handle: &mut crate::worker::WorkerHandle,
    event_id: u64,
    request: ServiceWorkerFetchRequest,
) -> (ServiceWorkerFetchCompletion, String) {
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        request,
        navigation_preload_sent: false,
    });

    let mut completion = None;
    let mut handled_console = None;
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for handled fetch event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(message) => {
                completion = Some(message);
            }
            WorkerToParentMessage::Console(message) => {
                handled_console = Some(message.message);
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!("unexpected service worker error while waiting for handled: {message}");
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
            | WorkerToParentMessage::ServiceWorkerNotificationCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
            | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
            | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
            | WorkerToParentMessage::ServiceWorkerSyncCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(_)
            | WorkerToParentMessage::ServiceWorkerShowNotification(_)
            | WorkerToParentMessage::ServiceWorkerGetNotifications(_)
            | WorkerToParentMessage::ServiceWorkerSyncRegistration(_)
            | WorkerToParentMessage::ServiceWorkerSyncGetTags(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(_)
            | WorkerToParentMessage::ServiceWorkerCloseNotification(_)
            | WorkerToParentMessage::ServiceWorkerClientMessage(_)
            | WorkerToParentMessage::ServiceWorkerWorkerMessage(_)
            | WorkerToParentMessage::ServiceWorkerClientQuery(_)
            | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
            | WorkerToParentMessage::ServiceWorkerClientFocus(_)
            | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
            | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
            | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
            | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
            | WorkerToParentMessage::RuntimeInspectorResponse(_)
            | WorkerToParentMessage::SharedWorkerClosed => {}
        }
        if let (Some(completion), Some(handled_console)) =
            (completion.as_ref(), handled_console.as_ref())
        {
            return (completion.clone(), handled_console.clone());
        }
    }
}

fn worker_client_snapshot_for_test(
    controlled: bool,
) -> crate::runtime::ServiceWorkerClientSnapshot {
    let mut snapshot = crate::runtime::ServiceWorkerClientSnapshot::dedicated_worker_for_test(
        crate::runtime::ServiceWorkerClientId::from_u64_for_test(77),
        url::Url::parse("https://example.test/app/dedicated-worker.js").unwrap(),
        controlled,
    );
    snapshot.exposed_id = "client-000000000000004d".to_owned();
    snapshot
}

// ─── setInterval ────────────────────────────────────────────────────

// ─── Multiple concurrent workers ────────────────────────────────────

// ─── Console ────────────────────────────────────────────────────────

// ─── Self reference ─────────────────────────────────────────────────

// ─── Empty worker stays alive until parent drops handle ────────────

// ─── Syntax error ───────────────────────────────────────────────────

// ─── Drop terminates ────────────────────────────────────────────────

mod background_apis_and_worker_events;
mod client_lifecycle_and_sync;
mod fetch_events_and_messages;
mod messages_clients_and_windows;
mod worker_and_fetch_lifecycle;
mod worker_errors_and_teardown;
