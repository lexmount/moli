use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use moli_fetch::FetchConfig;
use moli_websocket::test_support::{
    spawn_header_capture_websocket_server, spawn_set_cookie_websocket_server,
    spawn_text_echo_websocket_server,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::spawn_worker;
use super::{
    WorkerScriptKind, WorkerScriptSource, WorkerSpawnOptions, WorkerTestHandle,
    resource_loader_for_worker_context, spawn_test_worker_with_options,
    spawn_worker_with_request_client, spawn_worker_with_request_client_and_blocked_url_patterns,
    spawn_worker_with_request_client_and_kind,
    spawn_worker_with_request_client_and_kind_and_network_policy,
    spawn_worker_with_request_client_and_network_policy,
    spawn_worker_with_source_and_kind_and_network_policy, worker_test_request_client,
};
use crate::RendererSyntheticResponseBody;
use crate::context_bootstrap::{
    structured_deserialize_value, structured_serialize_value,
    structured_serialize_value_for_post_message,
};
use crate::ensure_v8_for_test as ensure_v8;
use crate::network::ResourceRequestClient;
use crate::protocol_types::{
    PendingSubresourceContinueEvent, SubresourceAuthCredentials, SubresourceAuthScheme,
    SubresourceAuthTarget, SubresourceNetworkOutcome, SubresourceNetworkRecord,
    SubresourceNetworkRequestHandle, SubresourceResourceType,
};
use crate::runtime::{
    MaterializedServiceWorkerFetchResponseHead, ServiceWorkerEventId, ServiceWorkerFetchCompletion,
    ServiceWorkerFetchEvent, ServiceWorkerFetchResult, ServiceWorkerGetNotificationsResult,
    ServiceWorkerLifecycleCompletion, ServiceWorkerLifecycleEvent, ServiceWorkerLifecycleEventKind,
    ServiceWorkerMessageCompletion, ServiceWorkerMessageEvent,
    ServiceWorkerNavigationPreloadFailure, ServiceWorkerNavigationPreloadResponseStarted,
    ServiceWorkerNavigationPreloadStreamChunk, ServiceWorkerNavigationPreloadStreamFinished,
    ServiceWorkerNotificationCompletion, ServiceWorkerNotificationEvent,
    ServiceWorkerNotificationSnapshot, ServiceWorkerPeriodicSyncCompletion,
    ServiceWorkerPeriodicSyncEvent, ServiceWorkerPushCompletion, ServiceWorkerPushEvent,
    ServiceWorkerRegistrationId, ServiceWorkerSyncCompletion, ServiceWorkerSyncEvent,
    ServiceWorkerVersionId,
};
use crate::service_worker_runtime::{
    ServiceWorkerFetchRequest, ServiceWorkerFetchRequestMetadata, ServiceWorkerNotificationAction,
    ServiceWorkerRequestDestination,
};
use crate::structured_clone::V8StructuredClonePayload;
use crate::worker::handle::{
    WorkerErrorPhase, WorkerFetchHandlerType, WorkerHandle, WorkerNetworkPolicy,
    WorkerPendingFetchContinue, WorkerPendingXhrContinue, WorkerToParentMessage,
};

const TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "current_thread")]
async fn worker_loader_shares_backend_without_sharing_page_policy() {
    let creator = ResourceRequestClient::new(&FetchConfig::default()).expect("creator loader");
    creator.set_extra_http_headers(&[("x-owner".to_owned(), "page".to_owned())]);
    let policy = WorkerNetworkPolicy {
        extra_http_headers: vec![("x-owner".to_owned(), "worker".to_owned())],
        network_offline: true,
        ..WorkerNetworkPolicy::default()
    };

    let worker = resource_loader_for_worker_context(
        creator.clone(),
        &policy,
        &super::WorkerGlobalKind::Dedicated {
            name: "test-worker".to_owned(),
        },
        crate::network::RendererResourceTaskRunner::from_current_tokio()
            .expect("worker authority test must own a Tokio runtime"),
    );

    assert!(creator.shares_resource_runtime_with(worker.request_client()));
    assert!(!creator.shares_page_network_policy_with(worker.request_client()));
    assert!(!creator.page_network_policy().snapshot().network_offline());
    assert!(
        worker
            .request_client()
            .page_network_policy()
            .snapshot()
            .network_offline()
    );
}

#[test]
fn data_worker_spawn_options_retain_creator_resource_runtime() {
    let creator = ResourceRequestClient::new(&FetchConfig::default()).expect("creator loader");
    let options = WorkerSpawnOptions::new_with_request_client(
        "postMessage('ready')".to_owned(),
        "data:text/javascript,postMessage('ready')".to_owned(),
        creator.clone(),
    );

    assert!(
        options
            .request_client
            .shares_resource_runtime_with(&creator),
        "a local-script Worker still needs the creator backend for later inside-settings requests"
    );
}

fn dns_failure_fetch_config() -> FetchConfig {
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    config.set_request_timeout_ms(2_000);
    config.set_connect_timeout_ms(Some(500));
    config
}

fn serialize_test_string(value: &str) -> V8StructuredClonePayload {
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = v8::String::new(scope, value)
        .expect("v8 string allocation")
        .into();
    structured_serialize_value(scope, value)
        .expect("test string should serialize through structured clone")
}

fn serialize_test_value(expression: &str) -> V8StructuredClonePayload {
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = v8::String::new(scope, expression).expect("v8 string allocation");
    let script = v8::Script::compile(scope, source, None).expect("test script should compile");
    let value = script.run(scope).expect("test script should evaluate");
    structured_serialize_value(scope, value)
        .expect("test value should serialize through structured clone")
}

fn serialize_test_crypto_value(expression: &str) -> V8StructuredClonePayload {
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
        scope,
        global,
        crate::context_bootstrap::exposed_interfaces::RealmKind::DedicatedWorker,
        true,
    )
    .expect("worker lazy interfaces should install in test context");
    crate::context_bootstrap::initialize_worker_crypto_realm_state(scope, global, true)
        .expect("crypto runtime should install in test context");
    let source = v8::String::new(scope, expression).expect("test script should allocate");
    let script = v8::Script::compile(scope, source, None).expect("test script should compile");
    let mut value = script.run(scope).expect("test script should evaluate");
    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        for _ in 0..8 {
            if promise.state() != v8::PromiseState::Pending {
                break;
            }
            scope.perform_microtask_checkpoint();
            crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
        }
        match promise.state() {
            v8::PromiseState::Fulfilled => value = promise.result(scope),
            v8::PromiseState::Rejected => {
                let reason = promise
                    .result(scope)
                    .to_string(scope)
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "<non-string rejection>".to_owned());
                panic!("test crypto promise rejected: {reason}");
            }
            v8::PromiseState::Pending => panic!("test crypto promise remained pending"),
        }
    }
    structured_serialize_value(scope, value)
        .expect("test crypto value should serialize through structured clone")
}

fn serialize_test_post_message_value(expression: &str) -> V8StructuredClonePayload {
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = v8::String::new(scope, expression).expect("v8 string allocation");
    let script = v8::Script::compile(scope, source, None).expect("test script should compile");
    let value = script.run(scope).expect("test script should evaluate");
    structured_serialize_value_for_post_message(scope, value, None, "Worker")
        .expect("test postMessage value should serialize through structured clone")
}

fn stringify_payload(payload: &V8StructuredClonePayload) -> String {
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = structured_deserialize_value(scope, payload)
        .expect("payload should deserialize through structured clone");
    v8::json::stringify(scope, value)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| value.to_rust_string_lossy(scope))
}

fn inspect_payload(payload: &V8StructuredClonePayload, expression: &str) -> String {
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let value = structured_deserialize_value(scope, payload)
        .expect("payload should deserialize through structured clone");
    let global = context.global(scope);
    let key = v8::String::new(scope, "__wire").expect("wire key");
    let _ = global.set(scope, key.into(), value);
    let source = v8::String::new(scope, expression).expect("inspection expression");
    let script = v8::Script::compile(scope, source, None).expect("inspection should compile");
    let result = script.run(scope).expect("inspection should run");
    result.to_rust_string_lossy(scope)
}

fn inspect_payload_with_image_data(payload: &V8StructuredClonePayload, expression: &str) -> String {
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
        scope,
        global,
        crate::context_bootstrap::exposed_interfaces::RealmKind::DedicatedWorker,
        true,
    )
    .expect("worker lazy interfaces should install in test context");
    let value = structured_deserialize_value(scope, payload)
        .expect("payload should deserialize through structured clone");
    let key = v8::String::new(scope, "__wire").expect("wire key");
    let _ = global.set(scope, key.into(), value);
    let source = v8::String::new(scope, expression).expect("inspection expression");
    let script = v8::Script::compile(scope, source, None).expect("inspection should compile");
    let result = script.run(scope).expect("inspection should run");
    result.to_rust_string_lossy(scope)
}

fn expect_post_json(message: WorkerToParentMessage) -> String {
    match message {
        WorkerToParentMessage::Post(payload) => stringify_payload(&payload),
        other => panic!("expected Post, got {other:?}"),
    }
}

async fn recv_post_json(handle: &mut WorkerHandle) -> String {
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        match message {
            WorkerToParentMessage::Post(payload) => return stringify_payload(&payload),
            WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_) => continue,
            other => panic!("expected Post, got {other:?}"),
        }
    }
}

fn expect_subresource_network_record(message: WorkerToParentMessage) -> SubresourceNetworkRecord {
    match message {
        WorkerToParentMessage::SubresourceNetwork(record)
        | WorkerToParentMessage::WebSocketSubresource(record) => record,
        other => panic!("expected worker subresource network record, got {other:?}"),
    }
}

fn spawn_service_worker_for_test(script_source: &str) -> WorkerTestHandle {
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            script_source.to_owned(),
            "https://example.test/app/sw.js".to_owned(),
        )
        .with_global_kind(crate::worker::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://example.test/app/").unwrap(),
        }),
    )
}

async fn dispatch_service_worker_lifecycle_event_for_test(
    handle: &mut WorkerHandle,
    kind: ServiceWorkerLifecycleEventKind,
    event_id: u64,
) -> ServiceWorkerLifecycleCompletion {
    handle.dispatch_service_worker_lifecycle_event(ServiceWorkerLifecycleEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        kind,
    });
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker lifecycle event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerLifecycleCompleted(completion) => {
                return completion;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for lifecycle completion: {message}"
                );
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
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
    }
}

async fn dispatch_service_worker_fetch_event_for_test(
    handle: &mut WorkerHandle,
    event_id: u64,
) -> ServiceWorkerFetchCompletion {
    let request = service_worker_fetch_request_for_test();
    dispatch_service_worker_fetch_event_with_request_for_test(handle, event_id, request).await
}

fn service_worker_fetch_request_for_test() -> ServiceWorkerFetchRequest {
    ServiceWorkerFetchRequest {
        client_id: crate::service_worker_runtime::ServiceWorkerClientId::from_u64_for_test(7),
        resulting_client_id: None,
        url: url::Url::parse("https://example.test/app/data.txt").unwrap(),
        method: "GET".to_owned(),
        headers: vec![("accept".to_owned(), "text/plain".to_owned())],
        body: None,
        destination: ServiceWorkerRequestDestination::Empty,
        request_mode: moli_fetch::RequestMode::Cors,
        credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
        priority: None,
        is_reload: false,
        metadata: Default::default(),
    }
}

async fn dispatch_service_worker_fetch_event_with_request_for_test(
    handle: &mut WorkerHandle,
    event_id: u64,
    request: ServiceWorkerFetchRequest,
) -> ServiceWorkerFetchCompletion {
    handle.dispatch_service_worker_fetch_event(ServiceWorkerFetchEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        request,
        navigation_preload_sent: false,
    });
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker fetch event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => return completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for fetch completion: {message}"
                );
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
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
    }
}

async fn dispatch_service_worker_message_event_for_test(
    handle: &mut WorkerHandle,
    event_id: u64,
    payload: V8StructuredClonePayload,
) -> ServiceWorkerMessageCompletion {
    dispatch_service_worker_message_event_object_for_test(
        handle,
        ServiceWorkerMessageEvent {
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
            payload,
            window_interaction_allowed: false,
        },
    )
    .await
}

async fn dispatch_service_worker_message_event_object_for_test(
    handle: &mut WorkerHandle,
    event: ServiceWorkerMessageEvent,
) -> ServiceWorkerMessageCompletion {
    handle.dispatch_service_worker_message_event(event);
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker message event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => return completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for message completion: {message}"
                );
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
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
    }
}

async fn dispatch_service_worker_notification_event_for_test(
    handle: &mut WorkerHandle,
    event: ServiceWorkerNotificationEvent,
) -> ServiceWorkerNotificationCompletion {
    handle.dispatch_service_worker_notification_event(event);
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker notification event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerNotificationCompleted(completion) => {
                return completion;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for notification completion: {message}"
                );
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
            | WorkerToParentMessage::ServiceWorkerShowNotification(_)
            | WorkerToParentMessage::ServiceWorkerPushCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
            | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
            | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
            | WorkerToParentMessage::ServiceWorkerSyncCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(_)
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
    }
}

async fn dispatch_service_worker_push_event_for_test(
    handle: &mut WorkerHandle,
    event_id: u64,
    data: Option<Vec<u8>>,
) -> ServiceWorkerPushCompletion {
    handle.dispatch_service_worker_push_event(ServiceWorkerPushEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        data,
    });
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker push event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerPushCompleted(completion) => return completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for push completion: {message}"
                );
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
            | WorkerToParentMessage::ServiceWorkerNotificationCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
            | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
            | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
            | WorkerToParentMessage::ServiceWorkerShowNotification(_)
            | WorkerToParentMessage::ServiceWorkerGetNotifications(_)
            | WorkerToParentMessage::ServiceWorkerSyncRegistration(_)
            | WorkerToParentMessage::ServiceWorkerSyncGetTags(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(_)
            | WorkerToParentMessage::ServiceWorkerSyncCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(_)
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
    }
}

async fn dispatch_service_worker_sync_event_for_test(
    handle: &mut WorkerHandle,
    event_id: u64,
    tag: &str,
) -> ServiceWorkerSyncCompletion {
    handle.dispatch_service_worker_sync_event(ServiceWorkerSyncEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        tag: tag.to_owned(),
        last_chance: false,
    });
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker sync event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerSyncCompleted(completion) => return completion,
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for sync completion: {message}"
                );
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
            | WorkerToParentMessage::ServiceWorkerNotificationCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
            | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
            | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
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
    }
}

async fn dispatch_service_worker_periodic_sync_event_for_test(
    handle: &mut WorkerHandle,
    event_id: u64,
    tag: &str,
) -> ServiceWorkerPeriodicSyncCompletion {
    handle.dispatch_service_worker_periodic_sync_event(ServiceWorkerPeriodicSyncEvent {
        event_id: ServiceWorkerEventId::from_u64_for_worker(event_id),
        registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        tag: tag.to_owned(),
    });
    loop {
        let message = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out waiting for service worker periodic sync event")
            .expect("service worker channel closed");
        match message {
            WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(completion) => {
                return completion;
            }
            WorkerToParentMessage::Error { message, .. } => {
                panic!(
                    "unexpected service worker error while waiting for periodic sync completion: {message}"
                );
            }
            WorkerToParentMessage::Post(_)
            | WorkerToParentMessage::SubresourceNetwork(_)
            | WorkerToParentMessage::PendingSubresourceFetch(_)
            | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
            | WorkerToParentMessage::SubresourceContinue(_)
            | WorkerToParentMessage::WebSocketSubresource(_)
            | WorkerToParentMessage::WebSocketLifecycle(_)
            | WorkerToParentMessage::WebSocketFrame(_)
            | WorkerToParentMessage::Console(_)
            | WorkerToParentMessage::RuntimeInspectorMessages(_)
            | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
            | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
            | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
            | WorkerToParentMessage::ServiceWorkerNotificationCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushCompleted(_)
            | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
            | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
            | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
            | WorkerToParentMessage::ServiceWorkerSyncCompleted(_)
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
    }
}

fn owner_assigned_request_handle(internal_id: u64) -> SubresourceNetworkRequestHandle {
    SubresourceNetworkRequestHandle::new(10_000 + internal_id)
}

fn pending_worker_fetch_continue(
    fetch_id: u32,
    internal_id: u64,
    info: &crate::protocol_types::PendingSubresourceFetchInfo,
    intercept_response: bool,
) -> WorkerPendingFetchContinue {
    WorkerPendingFetchContinue {
        fetch_id,
        internal_id,
        network_request_handle: Some(owner_assigned_request_handle(internal_id)),
        url: info.url.clone(),
        method: info.method.clone(),
        body: info.request_body.clone(),
        headers: info.request_headers.clone(),
        intercept_response,
        handle_auth_requests: false,
        auth: None,
    }
}

fn pending_worker_xhr_continue(
    xhr_id: u32,
    internal_id: u64,
    info: &crate::protocol_types::PendingSubresourceFetchInfo,
    intercept_response: bool,
) -> WorkerPendingXhrContinue {
    WorkerPendingXhrContinue {
        xhr_id,
        internal_id,
        network_request_handle: Some(owner_assigned_request_handle(internal_id)),
        url: info.url.clone(),
        method: info.method.clone(),
        body: info.request_body.clone(),
        headers: info.request_headers.clone(),
        intercept_response,
        handle_auth_requests: false,
        auth: None,
    }
}

fn worker_data_url(source: &str) -> String {
    format!(
        "data:text/javascript,{}",
        percent_encoding::utf8_percent_encode(source, percent_encoding::NON_ALPHANUMERIC)
    )
}

async fn read_http_request_head(
    stream: &mut tokio::net::TcpStream,
) -> Result<String, std::io::Error> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    while !buffer.ends_with(b"\r\n\r\n") {
        let read = tokio::io::AsyncReadExt::read(stream, &mut byte).await?;
        if read == 0 {
            break;
        }
        buffer.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

async fn read_http_request_with_body(
    stream: &mut tokio::net::TcpStream,
) -> Result<String, std::io::Error> {
    let head = read_http_request_head(stream).await?;
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        tokio::io::AsyncReadExt::read_exact(stream, &mut body).await?;
    }
    Ok(format!("{head}{}", String::from_utf8_lossy(&body)))
}

async fn spawn_path_response_http_server(
    responses: Vec<(&'static str, &'static str, &'static str, String, Duration)>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker fetch test server");
    let addr = listener.local_addr().expect("worker fetch server addr");
    let server = tokio::spawn(async move {
        let mut responses = responses
            .into_iter()
            .map(|(path, status, content_type, body, delay)| {
                (
                    path.to_owned(),
                    (status.to_owned(), content_type.to_owned(), body, delay),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        while !responses.is_empty() {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept worker fetch request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker fetch request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("worker fetch request path");
            let (status_line, content_type, body, delay) = responses
                .remove(path)
                .unwrap_or_else(|| panic!("unexpected worker fetch path: {path}"));
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let response = format!(
                "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker fetch response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn unused_local_http_url(path: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind unused worker local http port");
    let addr = listener
        .local_addr()
        .expect("unused worker local http addr");
    drop(listener);
    format!("http://{addr}{path}")
}

async fn spawn_connection_drop_http_server(path: &'static str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker connection-drop http server");
    let addr = listener
        .local_addr()
        .expect("worker connection-drop http addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker connection-drop request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker connection-drop request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker connection-drop request path");
        assert_eq!(request_path, path);
    });
    (format!("http://{addr}"), server)
}

async fn spawn_redirect_loop_http_server(path: &'static str) -> (String, JoinHandle<()>) {
    const REDIRECT_LOOP_REQUESTS: usize = 11;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker redirect-loop http server");
    let addr = listener.local_addr().expect("worker redirect-loop addr");
    let server = tokio::spawn(async move {
        for _ in 0..REDIRECT_LOOP_REQUESTS {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept worker redirect-loop request");
            read_http_request_head(&mut stream)
                .await
                .expect("read worker redirect-loop request");
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {path}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker redirect-loop response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_single_redirect_http_server(
    path: &'static str,
    location: &'static str,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker single-redirect http server");
    let addr = listener.local_addr().expect("worker single-redirect addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker single-redirect request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker single-redirect request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker single-redirect request path");
        assert_eq!(request_path, path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker single-redirect response");
    });
    (format!("http://{addr}"), server)
}

async fn spawn_cross_origin_redirect_without_cors_http_servers(
    source_path: &'static str,
    target_path: &'static str,
) -> (String, String, JoinHandle<()>, JoinHandle<()>) {
    let target_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker redirect target without CORS server");
    let target_addr = target_listener
        .local_addr()
        .expect("worker redirect target without CORS addr");
    let target_base_url = format!("http://{target_addr}");
    let target_server = tokio::spawn(async move {
        let (mut stream, _) = target_listener
            .accept()
            .await
            .expect("accept worker redirect target request");
        read_http_request_head(&mut stream)
            .await
            .expect("read worker redirect target request");
        let body = "worker-cors-denied-target";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker redirect target without CORS response");
    });

    let source_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker redirect source server");
    let source_addr = source_listener
        .local_addr()
        .expect("worker redirect source addr");
    let source_base_url = format!("http://{source_addr}");
    let target_location = format!("{target_base_url}{target_path}");
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept worker redirect source request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker redirect source request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker redirect source request path");
        assert_eq!(path, source_path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker redirect source response");
    });

    (
        source_base_url,
        target_base_url,
        source_server,
        target_server,
    )
}

async fn spawn_cross_origin_redirect_with_cors_http_servers(
    source_path: &'static str,
    target_path: &'static str,
    target_body: &'static str,
) -> (String, String, JoinHandle<()>, JoinHandle<()>) {
    let target_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker redirect target with CORS server");
    let target_addr = target_listener
        .local_addr()
        .expect("worker redirect target with CORS addr");
    let target_base_url = format!("http://{target_addr}");
    let target_server = tokio::spawn(async move {
        let (mut stream, _) = target_listener
            .accept()
            .await
            .expect("accept worker redirect target with CORS request");
        read_http_request_head(&mut stream)
            .await
            .expect("read worker redirect target with CORS request");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            target_body.len(),
            target_body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker redirect target with CORS response");
    });

    let source_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker redirect source with CORS server");
    let source_addr = source_listener
        .local_addr()
        .expect("worker redirect source with CORS addr");
    let source_base_url = format!("http://{source_addr}");
    let target_location = format!("{target_base_url}{target_path}");
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept worker redirect source with CORS request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read worker redirect source with CORS request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("worker redirect source with CORS request path");
        assert_eq!(path, source_path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker redirect source with CORS response");
    });

    (
        source_base_url,
        target_base_url,
        source_server,
        target_server,
    )
}

async fn spawn_raw_path_response_http_server(
    responses: Vec<(&'static str, String, Duration)>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind raw worker fetch test server");
    let addr = listener.local_addr().expect("raw worker fetch server addr");
    let server = tokio::spawn(async move {
        let mut responses = responses
            .into_iter()
            .map(|(path, response, delay)| (path.to_owned(), (response, delay)))
            .collect::<std::collections::HashMap<_, _>>();
        while !responses.is_empty() {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept raw worker fetch request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read raw worker fetch request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("raw worker fetch request path");
            let (response, delay) = responses
                .remove(path)
                .unwrap_or_else(|| panic!("unexpected raw worker fetch path: {path}"));
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write raw worker fetch response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_basic_auth_http_server(
    path: &'static str,
    realm: &'static str,
    success_body: &'static str,
    expected_requests: usize,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind worker auth test server");
    let addr = listener.local_addr().expect("worker auth server addr");
    let server = tokio::spawn(async move {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().await.expect("accept worker auth request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker auth request");
            let request_path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("worker auth request path");
            assert_eq!(request_path, path);
            let authorization = request.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("authorization")
                    .then(|| value.trim().to_owned())
            });
            let response = if authorization.as_deref() == Some("Basic dXNlcjpwYXNz") {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    success_body.len(),
                    success_body
                )
            } else {
                let body = "auth required";
                format!(
                    "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"{realm}\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
            };
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker auth response");
        }
    });
    (format!("http://{addr}"), server)
}

fn server_basic_auth_credentials() -> SubresourceAuthCredentials {
    SubresourceAuthCredentials {
        target: SubresourceAuthTarget::Server,
        scheme: SubresourceAuthScheme::Basic,
        username: "user".to_owned(),
        password: "pass".to_owned(),
    }
}

#[test]
fn worker_broadcast_channel_unknown_script_url_is_not_marked_third_party() {
    let registry = super::new_broadcast_channel_registry();
    let worker_storage_key = super::worker_global_storage_key(
        None,
        None,
        Some("https://app.example".to_owned()),
        None,
        &registry,
        &super::WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
    );

    let key = super::worker_broadcast_channel_storage_key(
        None,
        &worker_storage_key,
        &super::WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
    );

    assert_eq!(key, worker_storage_key);
    assert_eq!(key.origin(), "null");
    assert_eq!(key.top_level_site(), "https://app.example");
    assert!(key.opaque_nonce().is_some());
    assert_eq!(
        key.partition_relation(),
        moli_storage_key::StoragePartitionRelation::Unknown
    );
    assert!(!key.is_third_party_partitioned());
    assert!(moli_storage_key::serialized_storage_key_has_opaque_origin(
        &key.serialized_storage_key()
    ));
}

#[test]
fn shared_worker_broadcast_channel_uses_injected_storage_key() {
    let registry = super::new_broadcast_channel_registry();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://top.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::ThirdParty,
    );
    let script_url = url::Url::parse("https://app.example/shared-worker.js").unwrap();
    let global_kind = super::WorkerGlobalKind::Shared {
        name: "shared".to_owned(),
        storage_key: storage_key.clone(),
    };
    let worker_storage_key = super::worker_global_storage_key(
        Some(&script_url),
        None,
        Some("https://ignored.example".to_owned()),
        None,
        &registry,
        &global_kind,
    );

    let key = super::worker_broadcast_channel_storage_key(
        Some(&script_url),
        &worker_storage_key,
        &global_kind,
    );

    assert_eq!(worker_storage_key, storage_key);
    assert_eq!(key, storage_key);
    assert_eq!(
        key.serialized_storage_key(),
        "storage-key:v1;origin=https://app.example;top-level-site=https://top.example"
    );
}

#[test]
fn shared_worker_data_url_broadcast_channel_uses_script_opaque_origin() {
    let registry = super::new_broadcast_channel_registry();
    let constructor_storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://top.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::ThirdParty,
    );
    let script_url = url::Url::parse("data:text/javascript,onconnect=function(){}").unwrap();
    let global_kind = super::WorkerGlobalKind::Shared {
        name: "shared".to_owned(),
        storage_key: constructor_storage_key.clone(),
    };
    let storage_key = super::worker_global_storage_key(
        Some(&script_url),
        None,
        Some("https://top.example".to_owned()),
        Some(constructor_storage_key.clone()),
        &registry,
        &global_kind,
    );

    let broadcast_key =
        super::worker_broadcast_channel_storage_key(Some(&script_url), &storage_key, &global_kind);

    assert_ne!(broadcast_key, constructor_storage_key);
    assert_eq!(broadcast_key, storage_key);
    assert_eq!(broadcast_key.origin(), "null");
    assert_eq!(broadcast_key.top_level_site(), "https://top.example");
    assert!(broadcast_key.opaque_nonce().is_some());
    assert_eq!(storage_key.origin(), "null");
    assert!(storage_key.opaque_nonce().is_some());
    assert!(moli_storage_key::serialized_storage_key_has_opaque_origin(
        &storage_key.serialized_storage_key()
    ));
}

#[test]
fn dedicated_worker_broadcast_channel_can_inherit_creator_storage_key() {
    let registry = super::new_broadcast_channel_registry();
    let creator_storage_key = moli_storage_key::MoliStorageKey::new(
        "null".to_owned(),
        "https://top.example".to_owned(),
        Some(moli_storage_key::OpaqueOriginNonce::new(7)),
        moli_storage_key::StoragePartitionRelation::Unknown,
    );
    let script_url = url::Url::parse("blob:null/worker-script").unwrap();
    let global_kind = super::WorkerGlobalKind::Dedicated {
        name: "dedicated".to_owned(),
    };
    let storage_key = super::worker_global_storage_key(
        Some(&script_url),
        None,
        Some("https://top.example".to_owned()),
        Some(creator_storage_key.clone()),
        &registry,
        &global_kind,
    );

    let broadcast_key =
        super::worker_broadcast_channel_storage_key(Some(&script_url), &storage_key, &global_kind);

    assert_eq!(storage_key, creator_storage_key);
    assert_eq!(broadcast_key, creator_storage_key);
}

#[test]
fn data_url_dedicated_worker_broadcast_channel_uses_script_opaque_storage_key() {
    let registry = super::new_broadcast_channel_registry();
    let creator_storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://top.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::ThirdParty,
    );
    let script_url = url::Url::parse("data:text/javascript,postMessage('ready')").unwrap();
    let global_kind = super::WorkerGlobalKind::Dedicated {
        name: "dedicated".to_owned(),
    };
    let worker_storage_key = super::worker_global_storage_key(
        Some(&script_url),
        None,
        Some("https://top.example".to_owned()),
        Some(creator_storage_key.clone()),
        &registry,
        &global_kind,
    );

    let broadcast_key = super::worker_broadcast_channel_storage_key(
        Some(&script_url),
        &worker_storage_key,
        &global_kind,
    );

    assert_eq!(broadcast_key, worker_storage_key);
    assert_eq!(broadcast_key.origin(), "null");
    assert_ne!(broadcast_key, creator_storage_key);
    assert!(broadcast_key.opaque_nonce().is_some());
}

#[test]
fn service_worker_storage_apis_use_explicit_registration_storage_key() {
    let registry = super::new_broadcast_channel_registry();
    let registration_storage_key = moli_storage_key::MoliStorageKey::new(
        "https://cdn.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::ThirdParty,
    );
    let script_url = url::Url::parse("https://cdn.example/sw.js").unwrap();
    let global_kind = super::WorkerGlobalKind::Service {
        registration_id: crate::runtime::ServiceWorkerRegistrationId::from_u64_for_test(1),
        version_id: crate::runtime::ServiceWorkerVersionId::from_u64_for_test(1),
        scope_url: url::Url::parse("https://cdn.example/").unwrap(),
    };

    let worker_storage_key = super::worker_global_storage_key(
        Some(&script_url),
        Some(registration_storage_key.clone()),
        Some("https://ignored.example".to_owned()),
        None,
        &registry,
        &global_kind,
    );
    let broadcast_key = super::worker_broadcast_channel_storage_key(
        Some(&script_url),
        &worker_storage_key,
        &global_kind,
    );

    assert_eq!(worker_storage_key, registration_storage_key);
    assert_eq!(broadcast_key, registration_storage_key);
    assert_eq!(
        worker_storage_key.serialized_storage_key(),
        "storage-key:v1;origin=https://cdn.example;top-level-site=https://app.example"
    );
}

// ─── Basic tests ────────────────────────────────────────────────────

mod lazy_storage;
mod lifecycle;
mod modules;
mod network;
mod postmessage;
