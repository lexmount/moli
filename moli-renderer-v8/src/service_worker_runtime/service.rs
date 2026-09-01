use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, atomic::Ordering},
};

use url::Url;

use crate::{
    network::ResourceRequestClient,
    page_task_queue::RendererPageServiceWorkerTaskSender,
    runtime::{
        RendererBrowserContextRuntime, RendererRuntimeInspectorMessage,
        RendererServiceWorkerConsoleMessage, RendererServiceWorkerExceptionMessage,
        RendererServiceWorkerFetchDiagnostic, RendererServiceWorkerRunIdentity,
        RendererWorkerContextRuntime,
    },
    structured_clone::V8StructuredClonePayload,
    types::{
        AsyncSubresourceFetchCompletion, ServiceWorkerClientFocusCompletion,
        ServiceWorkerClientFocusRequestCompletion, ServiceWorkerClientNavigateCompletion,
        ServiceWorkerClientNavigateRequestCompletion, ServiceWorkerClientsOpenWindowCompletion,
        ServiceWorkerClientsOpenWindowRequestCompletion, ServiceWorkerLifecycleClientEvent,
        ServiceWorkerReadyCompletion,
    },
    worker::{
        WorkerBootstrapFailure, WorkerConsoleMessage, WorkerErrorPhase, WorkerErrorSource,
        WorkerNetworkPolicy, WorkerParentErrorEventKind, WorkerRuntimeInspectorMessageBatch,
        WorkerScriptKind, WorkerScriptResource,
    },
};

use super::{
    clients::{
        ServiceWorkerClientFrameType, ServiceWorkerClientQuery, ServiceWorkerClientQueryKind,
        ServiceWorkerClientQueryOptions, ServiceWorkerClientQueryResult,
        ServiceWorkerClientQueryType, ServiceWorkerClientSnapshot, ServiceWorkerClientType,
        ServiceWorkerClientVisibilityState, allocate_service_worker_exposed_client_id,
        service_worker_current_url_for_creation_url, service_worker_exposed_client_id,
    },
    diagnostics::{
        ServiceWorkerMainScriptUpdateCheckDiagnostics, ServiceWorkerRegistrationDiagnostics,
        ServiceWorkerRuntimeDiagnostics, ServiceWorkerScriptResourceDiagnostics,
        ServiceWorkerVersionDiagnostics,
    },
    errors::ServiceWorkerRegistrationError,
    events::{
        MaterializedServiceWorkerFetchResponseHead, ServiceWorkerClientFocus,
        ServiceWorkerClientFocusError, ServiceWorkerClientFocusResult, ServiceWorkerClientMessage,
        ServiceWorkerClientNavigate, ServiceWorkerClientNavigateError,
        ServiceWorkerClientNavigateResult, ServiceWorkerClientsOpenWindow,
        ServiceWorkerClientsOpenWindowError, ServiceWorkerClientsOpenWindowResult,
        ServiceWorkerCloseNotification, ServiceWorkerDirectFetchResult,
        ServiceWorkerFetchCompletion, ServiceWorkerFetchDispatch, ServiceWorkerFetchEvent,
        ServiceWorkerFetchResponse, ServiceWorkerFetchResult, ServiceWorkerFetchStreamChunk,
        ServiceWorkerFetchStreamStarted, ServiceWorkerGetNotifications,
        ServiceWorkerGetNotificationsResult, ServiceWorkerLifecycleCompletion,
        ServiceWorkerLifecycleEvent, ServiceWorkerLifecycleEventKind,
        ServiceWorkerMessageCompletion, ServiceWorkerMessageEvent, ServiceWorkerNotificationAction,
        ServiceWorkerNotificationCompletion, ServiceWorkerNotificationEvent,
        ServiceWorkerNotificationEventKind, ServiceWorkerNotificationMetadata,
        ServiceWorkerNotificationSnapshot, ServiceWorkerPeriodicSyncCompletion,
        ServiceWorkerPeriodicSyncEvent, ServiceWorkerPeriodicSyncGetTags,
        ServiceWorkerPeriodicSyncGetTagsResult, ServiceWorkerPeriodicSyncRegistration,
        ServiceWorkerPeriodicSyncRegistrationResult, ServiceWorkerPeriodicSyncUnregistration,
        ServiceWorkerPeriodicSyncUnregistrationResult, ServiceWorkerPushCompletion,
        ServiceWorkerPushEvent, ServiceWorkerPushGetSubscription,
        ServiceWorkerPushGetSubscriptionResult, ServiceWorkerPushSubscribe,
        ServiceWorkerPushSubscribeResult, ServiceWorkerPushSubscriptionSnapshot,
        ServiceWorkerPushUnsubscribe, ServiceWorkerPushUnsubscribeResult,
        ServiceWorkerShowNotification, ServiceWorkerShowNotificationResult,
        ServiceWorkerSyncCompletion, ServiceWorkerSyncEvent, ServiceWorkerSyncGetTags,
        ServiceWorkerSyncGetTagsResult, ServiceWorkerSyncRegistration,
        ServiceWorkerSyncRegistrationResult, ServiceWorkerWorkerMessage,
    },
    functional_events::{
        ServiceWorkerNotificationRecord, ServiceWorkerPeriodicSyncRegistrationRecord,
        ServiceWorkerSyncRegistrationRecord,
    },
    host::{RendererServiceWorkerHost, SharedRendererServiceWorkerHost},
    ids::{
        ServiceWorkerClientId, ServiceWorkerEventId, ServiceWorkerRegistrationId,
        ServiceWorkerVersionId,
    },
    jobs::{
        ServiceWorkerAbortedJob, ServiceWorkerLaunchParams,
        ServiceWorkerMainScriptUpdateCheckStart, ServiceWorkerPendingMainScriptUpdateCheck,
        ServiceWorkerPendingRegisterJob, ServiceWorkerQueuedJob, ServiceWorkerQueuedRegisterJob,
        ServiceWorkerRegisterJob, ServiceWorkerRegistrationKey, ServiceWorkerUnregisterJob,
        ServiceWorkerUnregisterStart, ServiceWorkerVersionLaunchConfig,
    },
    matching::service_worker_scope_matches_url,
    owner_wake::{ServiceWorkerRuntimeOwnerWake, ServiceWorkerRuntimeOwnerWakeSender},
    pending_clear::{
        ServiceWorkerPendingClearAction, execute_pending_clear_locked,
        pending_clear_phase_for_registration_locked, registration_ready_to_delete_locked,
    },
    registration::{
        ServiceWorkerNavigationPreloadState, ServiceWorkerNavigationPreloadStateError,
        ServiceWorkerRegistration, ServiceWorkerUpdateViaCache,
    },
    resource_store::SharedServiceWorkerResourceStore,
    run_owner::ServiceWorkerRunOwner,
    script_loading::{
        LoadedServiceWorkerScript, ServiceWorkerScriptLoadParams, ServiceWorkerScriptResource,
        ServiceWorkerScriptUpdateCheckChange, ServiceWorkerScriptUpdateCheckCompletion,
        ServiceWorkerScriptUpdateCheckFailure, ServiceWorkerScriptUpdateCheckFailureStatus,
        ServiceWorkerScriptUpdateCheckParams, ServiceWorkerScriptUpdateCheckResult,
        load_service_worker_script_update_check,
    },
    service_lane::ServiceWorkerServiceLane,
    snapshots::{
        ServiceWorkerControlState, ServiceWorkerRegistrationSnapshot, ServiceWorkerVersionSnapshot,
    },
    start_completion::ServiceWorkerRuntimeCompletion,
    state::{
        LifecycleProgress, ServiceWorkerClient, ServiceWorkerClientEndpoint,
        ServiceWorkerControllerChangeDelivery, ServiceWorkerDevToolsRelatedPauseOnStartPolicy,
        ServiceWorkerFetchJob, ServiceWorkerLifecycleNotificationDelivery,
        ServiceWorkerLifecycleStart, ServiceWorkerLifecycleWatcher, ServiceWorkerMessageStart,
        ServiceWorkerNotificationStart, ServiceWorkerPeriodicSyncStart, ServiceWorkerPushStart,
        ServiceWorkerQueuedLaunch, ServiceWorkerReadyJob, ServiceWorkerRuntimeInner,
        ServiceWorkerRuntimeState, ServiceWorkerSyncStart, WeakServiceWorkerRuntimeService,
    },
    version::{
        ServiceWorkerFetchHandlerExistence, ServiceWorkerFetchHandlerType,
        ServiceWorkerIdleTimeout, ServiceWorkerIdleTimeoutToken, ServiceWorkerPendingStartEvent,
        ServiceWorkerVersion, ServiceWorkerVersionLifecycleState, ServiceWorkerVersionRunningState,
        ServiceWorkerVersionStartFailure,
    },
};

mod client_registry;
mod client_requests;
mod context_shutdown;
mod devtools_commands;
mod diagnostics_snapshot;
mod event_completion;
mod event_dispatch;
mod event_start;
mod fetch_settlement;
mod functional_requests;
mod idle_scheduling;
mod lifecycle_completion;
mod lifecycle_requests;
mod registration_jobs;
mod registration_lookup;
mod registration_store;
mod runtime_protocol;
mod service_lane_completions;
mod worker_completion;
use client_registry::{
    claim_live_scope_clients_locked, controller_change_deliveries_for_controlled_clients_locked,
    service_worker_client_snapshot, service_worker_client_snapshot_with_controlled,
};
use registration_lookup::{
    active_registration_id_for_scope_locked, select_controller_for_new_client_locked,
    service_worker_registration_can_see_client, service_worker_registration_matches_client,
    service_worker_registration_matches_url,
};
use registration_store::bump_registration_last_update_check_time_locked;

#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
use super::functional_events::ServiceWorkerTagDispatchState;
#[cfg(test)]
use super::resource_store::{
    ServiceWorkerStoredRegistration, new_shared_service_worker_resource_store,
};
#[cfg(test)]
use crate::types::AsyncSubresourceNetworkContext;

const SERVICE_WORKER_DEFAULT_IDLE_DELAY_MS: u64 = 30_000;
const SERVICE_WORKER_JOB_ABORTED_ERROR: &str = "service worker job was aborted";
const SERVICE_WORKER_FORCE_UPDATE_DEVTOOLS_CONSOLE_MESSAGE: &str = concat!(
    "warn: ",
    "Service Worker was updated because \"Update on reload\" was ",
    "checked in the DevTools Application panel."
);
#[cfg(test)]
use super::pending_clear::SERVICE_WORKER_REGISTRATION_DELETED_FETCH_ERROR;

#[derive(Clone)]
pub(crate) struct ServiceWorkerRuntimeService {
    inner: Arc<ServiceWorkerRuntimeInner>,
}

fn service_worker_version_snapshot(
    version: Option<&ServiceWorkerVersion>,
) -> Option<ServiceWorkerVersionSnapshot> {
    let version = version?;
    Some(ServiceWorkerVersionSnapshot::new(
        version.id,
        version.script_url.clone(),
        version.lifecycle_state.as_str(),
    ))
}

fn service_worker_registration_snapshot(
    state: &ServiceWorkerRuntimeState,
    registration: &ServiceWorkerRegistration,
) -> ServiceWorkerRegistrationSnapshot {
    ServiceWorkerRegistrationSnapshot::new(
        registration.id,
        registration.scope_url.clone(),
        registration.update_via_cache,
        registration.navigation_preload_state.clone(),
        service_worker_version_snapshot(
            registration
                .installing_version_id
                .and_then(|version_id| state.versions.get(&version_id)),
        ),
        service_worker_version_snapshot(
            registration
                .waiting_version_id
                .and_then(|version_id| state.versions.get(&version_id)),
        ),
        service_worker_version_snapshot(
            registration
                .active_version_id
                .and_then(|version_id| state.versions.get(&version_id)),
        ),
    )
}

#[cfg(test)]
pub(crate) struct ServiceWorkerRuntimeServiceOwner {
    service: ServiceWorkerRuntimeService,
    browser_context_owner: crate::runtime::RendererBrowserContextRuntimeOwner,
}

#[cfg(test)]
impl ServiceWorkerRuntimeServiceOwner {
    pub(crate) fn request_client(&self) -> ResourceRequestClient {
        ResourceRequestClient::from_browser_resource_runtime(
            self.browser_context_owner
                .owner_access()
                .current_browser_resource_runtime()
                .expect("service test browser resource owner should remain live"),
        )
    }

    pub(crate) fn browser_context_runtime(&self) -> RendererBrowserContextRuntime {
        self.browser_context_owner.handle()
    }

    pub(crate) fn replace_browser_resource_runtime(
        &self,
        registration: crate::network::BrowserResourceRuntimeOwnerRegistration,
    ) -> crate::network::BrowserResourceRuntime {
        self.browser_context_owner
            .replace_browser_resource_runtime(registration)
            .expect("service test replacement should target its fixture owner root")
    }
}

#[cfg(test)]
impl std::ops::Deref for ServiceWorkerRuntimeServiceOwner {
    type Target = ServiceWorkerRuntimeService;

    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

#[cfg(test)]
pub(crate) fn new_service_worker_runtime_service() -> ServiceWorkerRuntimeServiceOwner {
    new_service_worker_runtime_service_with_resource_store(
        new_shared_service_worker_resource_store(),
        RendererWorkerContextRuntime::new(
            crate::message_port_runtime::new_message_port_registry(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
        ),
    )
}

#[cfg(test)]
pub(crate) fn new_service_worker_runtime_service_with_resource_store(
    resource_store: SharedServiceWorkerResourceStore,
    restored_worker_context_runtime: RendererWorkerContextRuntime,
) -> ServiceWorkerRuntimeServiceOwner {
    let browser_context_owner =
        RendererBrowserContextRuntime::new_with_worker_context_and_service_worker_store_for_test(
            restored_worker_context_runtime,
            resource_store,
        );
    let service = browser_context_owner.service_worker_runtime();
    let (target_output_tx, target_output_rx) = crate::runtime::renderer_output_transport_channel();
    browser_context_owner.set_renderer_output_transport_sender(target_output_tx);
    *service.inner.target_output_test_rx.lock() = Some(target_output_rx);
    ServiceWorkerRuntimeServiceOwner {
        service,
        browser_context_owner,
    }
}

pub(crate) fn new_service_worker_runtime_service_with_resource_store_and_browser_resource_runtime_binding(
    resource_store: SharedServiceWorkerResourceStore,
    restored_worker_context_runtime: RendererWorkerContextRuntime,
    browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
    client_id_allocator: super::ids::ServiceWorkerClientIdAllocator,
    browser_context_runtime_id: crate::runtime::RendererBrowserContextRuntimeId,
    output_transport: crate::runtime::RendererOutputTransportSenderSlot,
) -> ServiceWorkerRuntimeService {
    let service = ServiceWorkerRuntimeService {
        inner: Arc::new(ServiceWorkerRuntimeInner::new(
            SERVICE_WORKER_DEFAULT_IDLE_DELAY_MS,
            resource_store,
            restored_worker_context_runtime,
            browser_resource_runtime,
            client_id_allocator,
            browser_context_runtime_id,
            output_transport,
        )),
    };
    service.restore_all_stored_registrations();
    service
}

impl std::fmt::Debug for ServiceWorkerRuntimeService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceWorkerRuntimeService")
            .field("diagnostics", &self.diagnostics_snapshot())
            .finish()
    }
}

impl ServiceWorkerRuntimeService {
    pub(super) fn downgrade(&self) -> WeakServiceWorkerRuntimeService {
        WeakServiceWorkerRuntimeService {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub(super) fn service_lane(&self) -> &ServiceWorkerServiceLane {
        &self.inner.service_lane
    }

    pub(crate) fn add_owner_wake_sender(&self, sender: ServiceWorkerRuntimeOwnerWakeSender) {
        self.inner.owner_wake.add_owner_wake_sender(sender.clone());
        if self.pending_service_lane_event_count() > 0 {
            sender.signal(ServiceWorkerRuntimeOwnerWake::ServiceLane);
        }
    }

    pub(super) fn signal_service_lane_wake(&self) -> bool {
        self.inner.owner_wake.signal_service_lane_wake()
    }

    pub(crate) fn bind_target_output_transport(
        &self,
        transport: crate::runtime::RendererOutputTransportSender,
    ) {
        self.inner
            .state
            .lock()
            .bind_target_output_transport(transport);
    }

    #[cfg(test)]
    fn take_target_output_events_for_test(
        &self,
    ) -> Vec<crate::runtime::RendererServiceWorkerTargetEvent> {
        let mut receiver = self.inner.target_output_test_rx.lock();
        super::target_output_streams::drain_service_worker_target_events_for_test(
            receiver
                .as_mut()
                .expect("ServiceWorker test service must install a concrete output receiver"),
        )
    }

    pub(super) fn running_host_for_version(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> Option<SharedRendererServiceWorkerHost> {
        let state = self.inner.state.lock();
        let version = state.versions.get(&version_id)?;
        let ServiceWorkerVersionRunningState::Running { host } = &version.running_state else {
            return None;
        };
        if !host.has_running_worker() {
            return None;
        }
        Some(host.clone())
    }

    pub(super) fn target_output_journal(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> Option<crate::runtime::RendererTurnOutputJournal> {
        self.inner.state.lock().target_output_journal(version_id)
    }

    pub(super) fn enqueue_target_console_message(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        message: WorkerConsoleMessage,
    ) {
        let mut state = self.inner.state.lock();
        if state.observes_live_target_run(version_id, &run) {
            state.record_target_console_message(
                version_id,
                run,
                RendererServiceWorkerConsoleMessage {
                    message: message.message,
                    args: message.args,
                    stack: message.stack,
                },
            );
        }
    }

    pub(super) fn enqueue_target_exception_message(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        message: String,
        filename: String,
        lineno: u32,
        colno: u32,
        event_kind: WorkerParentErrorEventKind,
        phase: WorkerErrorPhase,
        source: WorkerErrorSource,
    ) {
        let mut state = self.inner.state.lock();
        if state.observes_live_target_run(version_id, &run) {
            state.record_target_exception_message(
                version_id,
                run,
                RendererServiceWorkerExceptionMessage {
                    message,
                    filename,
                    lineno,
                    colno,
                    event_kind: worker_error_event_kind_label(event_kind).to_owned(),
                    phase: worker_error_phase_label(phase).to_owned(),
                    source: worker_error_source_label(source).to_owned(),
                },
            );
        }
    }

    pub(super) fn enqueue_target_fetch_diagnostic(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        diagnostic: RendererServiceWorkerFetchDiagnostic,
    ) {
        let mut state = self.inner.state.lock();
        if !state.observes_live_target_run(version_id, &run) {
            return;
        }
        state.record_target_fetch_diagnostic(version_id, run, diagnostic);
    }

    pub(super) fn enqueue_target_runtime_inspector_messages(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        batches: Vec<WorkerRuntimeInspectorMessageBatch>,
    ) {
        let mut state = self.inner.state.lock();
        if !state.observes_live_target_run(version_id, &run) {
            return;
        }
        for batch in batches {
            let (responses, notifications): (Vec<_>, Vec<_>) = batch
                .messages
                .into_iter()
                .partition(|message| match message {
                    RendererRuntimeInspectorMessage::Protocol(message) => {
                        message.get("id").is_some()
                    }
                    RendererRuntimeInspectorMessage::RuntimeContext(_) => false,
                });
            for message in responses {
                tracing::trace!(
                    version_id = version_id.as_u64(),
                    inspector_session_id = ?batch.inspector_session_id,
                    message = ?message,
                    "dropping stale service worker runtime inspector response without a deferred callback"
                );
            }
            state.record_target_runtime_inspector_messages(
                version_id,
                run.clone(),
                batch.inspector_session_id,
                notifications,
            );
        }
    }
}

fn worker_error_event_kind_label(kind: WorkerParentErrorEventKind) -> &'static str {
    match kind {
        WorkerParentErrorEventKind::Event => "event",
        WorkerParentErrorEventKind::ErrorEvent => "error_event",
    }
}

fn worker_error_phase_label(phase: WorkerErrorPhase) -> &'static str {
    match phase {
        WorkerErrorPhase::Bootstrap => "bootstrap",
        WorkerErrorPhase::Runtime => "runtime",
    }
}

fn worker_error_source_label(source: WorkerErrorSource) -> &'static str {
    match source {
        WorkerErrorSource::Runtime => "runtime",
        WorkerErrorSource::InitialScriptEvaluation => "initial_script_evaluation",
    }
}

fn registration_ready_to_activate_locked(
    state: &ServiceWorkerRuntimeState,
    registration: &ServiceWorkerRegistration,
) -> bool {
    if registration.active_version_id.is_none() {
        return true;
    }
    if !registration.controlled_client_ids.is_empty() {
        return false;
    }
    let Some(active_version) = registration
        .active_version_id
        .and_then(|active_version_id| state.versions.get(&active_version_id))
    else {
        return false;
    };
    active_version.in_flight_event_count == 0
        && active_version.pending_start_events.is_empty()
        && active_version.pending_activation_fetch_events.is_empty()
}

fn service_worker_push_subscription_snapshot(
    registration_id: ServiceWorkerRegistrationId,
    user_visible_only: bool,
) -> ServiceWorkerPushSubscriptionSnapshot {
    ServiceWorkerPushSubscriptionSnapshot {
        endpoint: format!(
            "https://moli.invalid/service-worker/push/{}",
            registration_id.as_u64()
        ),
        user_visible_only,
    }
}

fn service_worker_notifications_for_registration_locked(
    state: &ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
    tag: Option<&str>,
) -> Vec<ServiceWorkerNotificationSnapshot> {
    state
        .notification_records
        .iter()
        .filter(|record| record.registration_id == registration_id)
        .filter(|record| tag.is_none_or(|tag| record.tag == tag))
        .map(|record| ServiceWorkerNotificationSnapshot {
            id: record.id,
            registration_id: record.registration_id,
            title: record.title.clone(),
            tag: record.tag.clone(),
            metadata: record.metadata.clone(),
            actions: record.actions.clone(),
            data: record.data.clone(),
        })
        .collect()
}

fn current_epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn lifecycle_notifications_for_registration_locked(
    state: &ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
    events: Vec<ServiceWorkerLifecycleClientEvent>,
) -> Vec<ServiceWorkerLifecycleNotificationDelivery> {
    if events.is_empty() {
        return Vec::new();
    }
    let Some(registration) = state.registrations.get(&registration_id) else {
        return Vec::new();
    };
    let snapshot = service_worker_registration_snapshot(state, registration);
    state
        .lifecycle_watchers
        .iter()
        .filter(|watcher| {
            watcher.scope_url == registration.scope_url
                && watcher.storage_key == registration.storage_key
        })
        .cloned()
        .map(|watcher| ServiceWorkerLifecycleNotificationDelivery {
            watcher,
            registration: snapshot.clone(),
            events: events.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ensure_v8_for_test;
    use crate::service_worker_runtime::jobs::{
        ServiceWorkerQueuedUnregisterJob, ServiceWorkerRegisterJobPhase,
        ServiceWorkerRegistrationKey, ServiceWorkerUnregisterJobPhase,
    };
    use crate::service_worker_runtime::{
        ServiceWorkerFetchRequest, ServiceWorkerRequestDestination,
    };

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    fn test_request_client(service: &ServiceWorkerRuntimeServiceOwner) -> ResourceRequestClient {
        service.request_client()
    }

    fn test_resource_task_runner() -> crate::network::RendererResourceTaskRunner {
        crate::network::RendererResourceTaskRunner::for_test()
    }

    fn test_run_owner(
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
    ) -> ServiceWorkerRunOwner {
        ServiceWorkerRunOwner::new(version_id, run.clone())
    }

    fn new_loading_test_host(
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
    ) -> SharedRendererServiceWorkerHost {
        RendererServiceWorkerHost::new_loading(&test_run_owner(version_id, run))
    }

    fn new_running_test_host(
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
    ) -> SharedRendererServiceWorkerHost {
        RendererServiceWorkerHost::new_running_without_handle_for_test(&test_run_owner(
            version_id, run,
        ))
    }

    fn new_running_test_host_with_handle(
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        handle: crate::worker::WorkerHandle,
    ) -> SharedRendererServiceWorkerHost {
        RendererServiceWorkerHost::new_running_with_handle_for_test(
            &test_run_owner(version_id, run),
            handle,
        )
    }

    fn async_subresource_completion_queue()
    -> crate::page_task_queue::RendererResourceCompletionTestHarness {
        crate::page_task_queue::RendererResourceCompletionTestHarness::new()
    }

    fn pop_async_subresource_completion(
        queue: &mut crate::page_task_queue::RendererResourceCompletionTestHarness,
    ) -> AsyncSubresourceFetchCompletion {
        match queue.pop_next_async_subresource_event() {
            Some(crate::types::AsyncSubresourceFetchEvent::Completion(completion)) => *completion,
            other => panic!("expected async-subresource completion, got {other:?}"),
        }
    }

    fn expect_direct_fetch_fallback(
        receiver: &mut tokio::sync::oneshot::Receiver<ServiceWorkerDirectFetchResult>,
    ) {
        assert!(matches!(
            receiver.try_recv(),
            Ok(ServiceWorkerDirectFetchResult::Fallback)
        ));
    }

    fn serialize_test_string(value: &str) -> V8StructuredClonePayload {
        use std::pin::pin;

        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let value = v8::String::new(scope, value)
            .expect("v8 string allocation")
            .into();
        crate::context_bootstrap::structured_serialize_value_for_post_message(
            scope, value, None, "Worker",
        )
        .expect("test postMessage value should serialize through structured clone")
    }

    fn test_launch_config(
        service: &ServiceWorkerRuntimeServiceOwner,
        _script_url: &Url,
        scope_url: &Url,
    ) -> ServiceWorkerVersionLaunchConfig {
        let browser_context_runtime = service.browser_context_runtime();
        ServiceWorkerVersionLaunchConfig::restored(
            scope_url.clone(),
            browser_context_runtime.worker_context_runtime(),
            service.inner.browser_resource_runtime.clone(),
        )
    }

    #[test]
    fn restored_launch_config_resolves_the_current_browser_resource_runtime() {
        let service = new_service_worker_runtime_service();
        let launch_config = test_launch_config(
            &service,
            &url("https://example.test/sw.js"),
            &url("https://example.test/"),
        );
        let first_client = launch_config.request_client();

        let replacement_registration = crate::network::BrowserResourceRuntimeOwner::new(
            &moli_fetch::FetchConfig::default(),
            moli_cookie_jar::new_shared_browser_cookie_store(),
        );
        let replacement_runtime =
            service.replace_browser_resource_runtime(replacement_registration);
        let replacement_client = launch_config.request_client();
        assert!(
            replacement_client
                .browser_resource_runtime()
                .shares_state_with(&replacement_runtime)
        );
        assert!(!replacement_client.shares_resource_runtime_with(&first_client));
    }

    fn test_queued_launch(
        service: &ServiceWorkerRuntimeServiceOwner,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        script_url: Url,
        scope_url: Url,
    ) -> ServiceWorkerQueuedLaunch {
        let run = exact_version_run(service, version_id);
        let owner = test_run_owner(version_id, &run);
        let launch_config = test_launch_config(service, &script_url, &scope_url);
        let params = launch_config.to_launch_params(
            registration_id,
            &owner,
            script_url,
            scope_url.clone(),
            ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
            WorkerScriptKind::Classic,
        );
        ServiceWorkerQueuedLaunch {
            params,
            host: new_loading_test_host(version_id, &run),
            lifecycle_notifications: Vec::new(),
            preloaded_script: None,
        }
    }

    fn exact_created_target_run(
        events: &[crate::runtime::RendererServiceWorkerTargetEvent],
        version_id: ServiceWorkerVersionId,
    ) -> crate::runtime::RendererServiceWorkerRunIdentity {
        events
            .iter()
            .find_map(|event| match event {
                crate::runtime::RendererServiceWorkerTargetEvent::Created {
                    info,
                    active_run: Some(active_run),
                } if info.version_id == version_id.as_u64() => Some(active_run.clone()),
                _ => None,
            })
            .expect("a target created for a concrete worker host must expose its exact run")
    }

    fn exact_version_run(
        service: &ServiceWorkerRuntimeServiceOwner,
        version_id: ServiceWorkerVersionId,
    ) -> RendererServiceWorkerRunIdentity {
        service
            .inner
            .state
            .lock()
            .versions
            .get(&version_id)
            .expect("test version must exist")
            .run
            .clone()
    }

    fn insert_registered_version(
        service: &ServiceWorkerRuntimeServiceOwner,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        script_url: Url,
        scope_url: Url,
        controlled_client_documents: impl IntoIterator<Item = Url>,
    ) -> ServiceWorkerControlState {
        let mut state = service.inner.state.lock();
        let controlled_client_ids = controlled_client_documents
            .into_iter()
            .map(|document_url| {
                let client_id = service.inner.client_id_allocator.allocate();
                let current_document_url =
                    service_worker_current_url_for_creation_url(&document_url);
                let storage_key =
                    ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
                state.live_clients.insert(
                    client_id,
                    ServiceWorkerClient {
                        id: client_id,
                        exposed_id: service_worker_exposed_client_id(client_id),
                        creation_url: document_url.clone(),
                        document_url: current_document_url,
                        client_type: ServiceWorkerClientType::Window,
                        frame_type: ServiceWorkerClientFrameType::TopLevel,
                        visibility_state: ServiceWorkerClientVisibilityState::Visible,
                        storage_key,
                        secure_context: true,
                        execution_ready: true,
                        discarded_or_frozen: false,
                        document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(
                            0,
                        )),
                        endpoint: ServiceWorkerClientEndpoint::Page(test_completion_sender()),
                        focused: false,
                    },
                );
                client_id
            })
            .collect::<HashSet<_>>();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: None,
                waiting_version_id: None,
                active_version_id: Some(version_id),
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids,
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: Some(script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                running_state: ServiceWorkerVersionRunningState::Stopped,
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 0,
                run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        ServiceWorkerControlState::new(registration_id, Some(version_id), script_url, scope_url)
    }

    fn test_fetch_request(
        client_id: ServiceWorkerClientId,
        request_url: Url,
    ) -> ServiceWorkerFetchRequest {
        ServiceWorkerFetchRequest {
            client_id,
            resulting_client_id: None,
            url: request_url,
            method: "GET".to_owned(),
            headers: Vec::new(),
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

    fn test_fetch_job(
        service: &ServiceWorkerRuntimeServiceOwner,
        internal_id: u64,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        request_url: Url,
        completion_tx: crate::page_task_queue::RendererResourceCompletionSender,
        cancel_handle: moli_fetch::FetchCancelHandle,
    ) -> ServiceWorkerFetchJob {
        ServiceWorkerFetchJob {
            internal_id,
            owner: Some(test_run_owner(version_id, run)),
            request_url,
            request_method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_body_bytes: None,
            cors_preflight_request_headers: Vec::new(),
            client_id,
            resulting_client_id: None,
            destination: ServiceWorkerRequestDestination::Empty,
            is_reload: false,
            metadata: Default::default(),
            request_mode: moli_fetch::RequestMode::Cors,
            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
            redirect_mode: moli_fetch::RequestRedirectMode::Follow,
            priority: None,
            redirect_chain: Vec::new(),
            redirect_count: 0,
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url,
                resource_type: crate::types::SubresourceResourceType::Fetch,
                policy_context: Default::default(),
            },
            completion_tx,
            request_client: test_request_client(service),
            resource_task_runner: test_resource_task_runner(),
            cancel_handle,
            navigation_preload_cancel_handle: None,
            streaming_body_source_id: None,
            direct_completion_tx: None,
        }
    }

    fn insert_pending_navigation_preload_fetch_job(
        service: &ServiceWorkerRuntimeServiceOwner,
        event_id: ServiceWorkerEventId,
        internal_id: u64,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        request_url: Url,
        completion_tx: crate::page_task_queue::RendererResourceCompletionSender,
        cancel_handle: moli_fetch::FetchCancelHandle,
        navigation_preload_cancel_handle: moli_fetch::FetchCancelHandle,
    ) {
        let mut job = test_fetch_job(
            service,
            internal_id,
            version_id,
            run,
            client_id,
            document_url,
            request_url,
            completion_tx,
            cancel_handle,
        );
        job.navigation_preload_cancel_handle = Some(navigation_preload_cancel_handle);

        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        version.run = run.clone();
        version.in_flight_event_count = 1;
        state.pending_fetch_jobs.insert(event_id, job);
    }

    fn insert_inactive_registration(
        service: &ServiceWorkerRuntimeServiceOwner,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        script_url: Url,
        scope_url: Url,
    ) -> ServiceWorkerControlState {
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: Some(version_id),
                waiting_version_id: None,
                active_version_id: None,
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: Some(script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                running_state: ServiceWorkerVersionRunningState::Stopped,
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 0,
                run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        ServiceWorkerControlState::new(registration_id, None, script_url, scope_url)
    }

    fn snapshot_for_registration(
        service: &ServiceWorkerRuntimeServiceOwner,
        registration_id: ServiceWorkerRegistrationId,
    ) -> ServiceWorkerRegistrationSnapshot {
        let state = service.inner.state.lock();
        let registration = state
            .registrations
            .get(&registration_id)
            .expect("registration should exist");
        service_worker_registration_snapshot(&state, registration)
    }

    fn make_version_persistable(
        service: &ServiceWorkerRuntimeServiceOwner,
        version_id: ServiceWorkerVersionId,
    ) {
        let mut state = service.inner.state.lock();
        let script_url = state
            .versions
            .get(&version_id)
            .expect("version should exist")
            .script_url
            .clone();
        state
            .versions
            .get_mut(&version_id)
            .expect("version should exist")
            .main_script_resource = Some(test_script_resource(&script_url));
    }

    fn client_id_for_document(
        service: &ServiceWorkerRuntimeServiceOwner,
        document_url: &Url,
    ) -> ServiceWorkerClientId {
        let current_document_url = service_worker_current_url_for_creation_url(document_url);
        service
            .inner
            .state
            .lock()
            .live_clients
            .values()
            .find(|client| client.document_url == current_document_url)
            .expect("client should exist")
            .id
    }

    fn register_client_for_test(
        service: &ServiceWorkerRuntimeServiceOwner,
        document_url: Url,
    ) -> ServiceWorkerClientId {
        register_window_client_for_test(
            service,
            document_url,
            ServiceWorkerClientFrameType::TopLevel,
        )
    }

    fn register_window_client_for_test(
        service: &ServiceWorkerRuntimeServiceOwner,
        document_url: Url,
        frame_type: ServiceWorkerClientFrameType,
    ) -> ServiceWorkerClientId {
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
        service.register_client_with_storage_key(
            document_url,
            storage_key,
            frame_type,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
            test_completion_sender(),
        )
    }

    fn register_reserved_window_client_for_test(
        service: &ServiceWorkerRuntimeServiceOwner,
        document_url: Url,
        frame_type: ServiceWorkerClientFrameType,
    ) -> ServiceWorkerClientId {
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
        service.register_reserved_client_with_storage_key(
            document_url,
            storage_key,
            frame_type,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
        )
    }

    fn insert_live_client_record_for_test(
        service: &ServiceWorkerRuntimeServiceOwner,
        creation_url: Url,
        client_type: ServiceWorkerClientType,
    ) -> ServiceWorkerClientId {
        if client_type != ServiceWorkerClientType::Window {
            let storage_key =
                ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&creation_url);
            let (worker_tx, _worker_rx) = tokio::sync::mpsc::unbounded_channel();
            return service.register_worker_client_with_storage_key(
                creation_url,
                storage_key,
                client_type,
                true,
                worker_tx,
            );
        }
        let client_id = service.inner.client_id_allocator.allocate();
        let document_url = service_worker_current_url_for_creation_url(&creation_url);
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
        let (frame_type, visibility_state) = match client_type {
            ServiceWorkerClientType::Window => (
                ServiceWorkerClientFrameType::TopLevel,
                ServiceWorkerClientVisibilityState::Visible,
            ),
            ServiceWorkerClientType::DedicatedWorker | ServiceWorkerClientType::SharedWorker => (
                ServiceWorkerClientFrameType::None,
                ServiceWorkerClientVisibilityState::Hidden,
            ),
        };
        service.inner.state.lock().live_clients.insert(
            client_id,
            ServiceWorkerClient {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
                creation_url,
                document_url,
                client_type,
                frame_type,
                visibility_state,
                storage_key,
                secure_context: true,
                execution_ready: true,
                discarded_or_frozen: false,
                document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
                endpoint: ServiceWorkerClientEndpoint::Page(test_completion_sender()),
                focused: false,
            },
        );
        client_id
    }

    fn test_completion_sender() -> RendererPageServiceWorkerTaskSender {
        crate::page_task_queue::RendererPageServiceWorkerTestHarness::new().sender()
    }

    fn test_worker_context_runtime() -> RendererWorkerContextRuntime {
        RendererWorkerContextRuntime::new(
            crate::message_port_runtime::new_message_port_registry(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
        )
    }

    #[test]
    fn registration_lifecycle_watchers_are_partitioned_by_storage_key() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://lifecycle-partition.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://lifecycle-partition.test/app/worker.js"),
            scope_url.clone(),
            [],
        );

        let deliveries = {
            let mut state = service.inner.state.lock();
            let registration_storage_key = state
                .registrations
                .get(&registration_id)
                .expect("registered service worker")
                .storage_key
                .clone();
            state.lifecycle_watchers.extend([
                ServiceWorkerLifecycleWatcher {
                    scope_url: scope_url.clone(),
                    storage_key: registration_storage_key.clone(),
                    document_owner: crate::native_bridge::WindowDocumentOwner::for_test(7),
                    completion_tx: test_completion_sender(),
                },
                ServiceWorkerLifecycleWatcher {
                    scope_url: scope_url.clone(),
                    storage_key: "different-partition".to_owned(),
                    document_owner: crate::native_bridge::WindowDocumentOwner::for_test(8),
                    completion_tx: test_completion_sender(),
                },
            ]);
            lifecycle_notifications_for_registration_locked(
                &state,
                registration_id,
                vec![ServiceWorkerLifecycleClientEvent::UpdateFound],
            )
        };

        assert_eq!(deliveries.len(), 1);
        assert_eq!(
            deliveries[0].watcher.document_owner,
            crate::native_bridge::WindowDocumentOwner::for_test(7)
        );
        assert_ne!(deliveries[0].watcher.storage_key, "different-partition");
    }

    struct FailingJsonStorePath {
        parent_file: std::path::PathBuf,
    }

    impl FailingJsonStorePath {
        fn new(name: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let parent_file = std::env::temp_dir().join(format!(
                "moli-service-worker-failing-store-{name}-{}-{nonce}",
                std::process::id()
            ));
            std::fs::write(&parent_file, b"not a directory")
                .expect("failing store parent file should be written");
            Self { parent_file }
        }

        fn store_path(&self) -> std::path::PathBuf {
            self.parent_file.join("service-worker-resources.json")
        }
    }

    impl Drop for FailingJsonStorePath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.parent_file);
        }
    }

    struct TempJsonStorePath {
        path: std::path::PathBuf,
    }

    impl TempJsonStorePath {
        fn new(name: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moli-service-worker-store-{name}-{}-{nonce}.json",
                std::process::id()
            ));
            Self { path }
        }

        fn store_path(&self) -> std::path::PathBuf {
            self.path.clone()
        }
    }

    impl Drop for TempJsonStorePath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn test_script_resource(script_url: &Url) -> ServiceWorkerScriptResource {
        ServiceWorkerScriptResource {
            request_url: script_url.clone(),
            final_url: script_url.clone(),
            kind: crate::worker::WorkerScriptResourceKind::JavaScript,
            status: 200,
            headers: vec![("Content-Type".to_owned(), "text/javascript".to_owned())],
            body_len: 3,
            body_sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                .to_owned(),
            response_time_ms: 7,
            mime_type: Some("text/javascript".to_owned()),
        }
    }

    fn test_worker_script_resource(script_url: &Url) -> WorkerScriptResource {
        WorkerScriptResource {
            request_url: script_url.clone(),
            final_url: script_url.clone(),
            kind: crate::worker::WorkerScriptResourceKind::JavaScript,
            status: 200,
            headers: vec![("Content-Type".to_owned(), "text/javascript".to_owned())],
            body_len: 3,
            body_sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                .to_owned(),
            response_time_ms: 7,
            mime_type: Some("text/javascript".to_owned()),
        }
    }

    fn test_loaded_script(script_url: &Url, source: &str) -> LoadedServiceWorkerScript {
        LoadedServiceWorkerScript {
            resource: test_script_resource(script_url),
            source: source.to_owned(),
            response_referrer_policy: None,
            response_content_security_policies: Vec::new(),
            response_content_security_report_only_policies: Vec::new(),
            response_content_security_reporting_endpoints: Default::default(),
        }
    }

    fn test_update_check_result(
        script_url: &Url,
        source: &str,
        imported_script_changed: bool,
    ) -> ServiceWorkerScriptUpdateCheckResult {
        ServiceWorkerScriptUpdateCheckResult {
            main_script: test_loaded_script(script_url, source),
            change: if imported_script_changed {
                ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                    script_url: script_url.join("dep.js").expect("imported script url"),
                }
            } else {
                ServiceWorkerScriptUpdateCheckChange::Identical
            },
        }
    }

    fn test_queued_register_job(
        service: &ServiceWorkerRuntimeServiceOwner,
        script_url: Url,
        scope_url: Url,
    ) -> ServiceWorkerQueuedRegisterJob {
        ServiceWorkerQueuedRegisterJob {
            script_url,
            document_url: scope_url.join("page.html").expect("document url"),
            storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
            scope_url,
            script_kind: WorkerScriptKind::Classic,
            update_via_cache: ServiceWorkerUpdateViaCache::Imports,
            force_bypass_cache: false,
            skip_script_comparison: false,
            skip_waiting_after_install: false,
            force_update_page_load_waiter_ids: Vec::new(),
            request_client: test_request_client(service),
            network_policy: WorkerNetworkPolicy::default(),
            browser_context_runtime: service.browser_context_runtime(),
            broadcast_channel_top_level_site: None,
            indexed_db_manager: None,
            storage_bucket_store: None,
            callbacks: vec![ServiceWorkerRegisterJob {
                request_id: 1,
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(1),
                completion_tx: test_completion_sender(),
            }],
        }
    }

    fn insert_pending_main_script_update_check(
        service: &ServiceWorkerRuntimeServiceOwner,
        registration_id: ServiceWorkerRegistrationId,
        newest_version_id: ServiceWorkerVersionId,
        script_url: Url,
        scope_url: Url,
        request_id: u64,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerVersionId {
        let document_url = scope_url.join("page.html").expect("document url");
        let browser_context_runtime = service.browser_context_runtime();
        let queued_job = ServiceWorkerQueuedRegisterJob {
            script_url: script_url.clone(),
            scope_url: scope_url.clone(),
            document_url,
            storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
            script_kind: WorkerScriptKind::Classic,
            update_via_cache: ServiceWorkerUpdateViaCache::Imports,
            force_bypass_cache: false,
            skip_script_comparison: false,
            skip_waiting_after_install: false,
            force_update_page_load_waiter_ids: Vec::new(),
            request_client: test_request_client(service),
            network_policy: WorkerNetworkPolicy::default(),
            browser_context_runtime,
            broadcast_channel_top_level_site: None,
            indexed_db_manager: None,
            storage_bucket_store: None,
            callbacks: vec![ServiceWorkerRegisterJob {
                request_id,
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(1),
                completion_tx,
            }],
        };
        let mut state = service.inner.state.lock();
        let (new_version_id, _, _, _) = service
            .create_installing_version_locked(&mut state, registration_id, &queued_job, false)
            .expect("pending update check should create installing version");
        state.pending_main_script_update_checks.insert(
            registration_id,
            ServiceWorkerPendingMainScriptUpdateCheck::new(
                queued_job,
                newest_version_id,
                test_script_resource(&script_url).body_sha256,
                new_version_id,
            ),
        );
        new_version_id
    }

    fn insert_starting_version(
        service: &ServiceWorkerRuntimeServiceOwner,
    ) -> (
        Url,
        Url,
        ServiceWorkerRegistrationId,
        ServiceWorkerVersionId,
    ) {
        let script_url = Url::parse("https://example.test/app/sw.js").expect("script url");
        let scope_url = Url::parse("https://example.test/app/").expect("scope url");
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let host = new_running_test_host(version_id, &run);
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: Some(version_id),
                waiting_version_id: None,
                active_version_id: None,
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: None,
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Module,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                running_state: ServiceWorkerVersionRunningState::Starting { host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        (script_url, scope_url, registration_id, version_id)
    }

    fn insert_running_installing_version(
        service: &ServiceWorkerRuntimeServiceOwner,
    ) -> (ServiceWorkerRegistrationId, ServiceWorkerVersionId) {
        let script_url = Url::parse("https://example.test/app/sw.js").expect("script url");
        let scope_url = Url::parse("https://example.test/app/").expect("scope url");
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let host = new_loading_test_host(version_id, &run);
        let mut state = service.inner.state.lock();
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: Some(version_id),
                waiting_version_id: None,
                active_version_id: None,
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: Some(script_url.clone()),
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                running_state: ServiceWorkerVersionRunningState::Running { host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 1,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
        (registration_id, version_id)
    }

    fn insert_starting_version_with_register_job(
        service: &ServiceWorkerRuntimeServiceOwner,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        script_url: Url,
        scope_url: Url,
        request_id: u64,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) {
        let run = RendererServiceWorkerRunIdentity::fresh();
        let host = new_loading_test_host(version_id, &run);
        let mut state = service.inner.state.lock();
        let registration = state
            .registrations
            .entry(registration_id)
            .or_insert_with(|| ServiceWorkerRegistration {
                id: registration_id,
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                installing_version_id: None,
                waiting_version_id: None,
                active_version_id: None,
                pending_unregistration: false,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                last_update_check_time_ms: None,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            });
        registration.script_url = script_url.clone();
        registration.scope_url = scope_url.clone();
        registration.installing_version_id = Some(version_id);
        let mut pending_register_job =
            ServiceWorkerPendingRegisterJob::new(vec![ServiceWorkerRegisterJob {
                request_id,
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(1),
                completion_tx,
            }]);
        pending_register_job.start_current_moli_job();
        registration
            .pending_register_jobs
            .insert(version_id, pending_register_job);
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: script_url.clone(),
                final_script_url: None,
                main_script_resource: None,
                imported_script_resources: Default::default(),
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: WorkerScriptKind::Classic,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                launch_config: test_launch_config(service, &script_url, &scope_url),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                running_state: ServiceWorkerVersionRunningState::Starting { host },
                pending_start_events: VecDeque::new(),
                pending_activation_fetch_events: VecDeque::new(),
                in_flight_event_count: 0,
                run: run.clone(),
                idle_timeout_token: None,
                skip_waiting_requested: false,
                clients_claim_requested: false,
                last_start_error: None,
            },
        );
    }

    fn push_queued_register_job(
        service: &ServiceWorkerRuntimeServiceOwner,
        registration_id: ServiceWorkerRegistrationId,
        script_url: Url,
        scope_url: Url,
        request_id: u64,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) {
        let mut state = service.inner.state.lock();
        let registration_key = state
            .registrations
            .get(&registration_id)
            .map(|registration| registration.key())
            .expect("registration should exist");
        state.job_coordinator.enqueue_register(
            registration_key.clone(),
            ServiceWorkerQueuedRegisterJob {
                script_url,
                scope_url: scope_url.clone(),
                document_url: scope_url.join("page.html").expect("document url"),
                storage_key: registration_key.storage_key.clone(),
                script_kind: WorkerScriptKind::Classic,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                force_bypass_cache: false,
                skip_script_comparison: false,
                skip_waiting_after_install: false,
                force_update_page_load_waiter_ids: Vec::new(),
                request_client: test_request_client(service),
                network_policy: WorkerNetworkPolicy::default(),
                browser_context_runtime: service.browser_context_runtime(),
                broadcast_channel_top_level_site: None,
                indexed_db_manager: None,
                storage_bucket_store: None,
                callbacks: vec![ServiceWorkerRegisterJob {
                    request_id,
                    document_owner: crate::native_bridge::WindowDocumentOwner::for_test(1),
                    completion_tx,
                }],
            },
        );
    }

    fn pop_unregister_completion(
        queue: &mut crate::page_task_queue::RendererPageServiceWorkerTestHarness,
    ) -> crate::types::ServiceWorkerUnregisterCompletion {
        match queue.pop_internal() {
            Some(crate::page_task_queue::RendererServiceWorkerInternalTask::Unregister(
                completion,
            )) => completion,
            other => panic!("expected unregister completion, got {other:?}"),
        }
    }

    fn pop_register_completion(
        queue: &mut crate::page_task_queue::RendererPageServiceWorkerTestHarness,
    ) -> crate::types::ServiceWorkerRegisterCompletion {
        match queue.pop_internal() {
            Some(crate::page_task_queue::RendererServiceWorkerInternalTask::Register(
                completion,
            )) => completion,
            other => panic!("expected register completion, got {other:?}"),
        }
    }

    #[test]
    fn context_shutdown_aborts_pending_and_queued_register_jobs() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let first_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        insert_starting_version_with_register_job(
            &service,
            registration_id,
            first_version_id,
            url("https://example.test/app/worker-v1.js"),
            scope_url.clone(),
            11,
            first_queue.sender(),
        );
        push_queued_register_job(
            &service,
            registration_id,
            url("https://example.test/app/worker-v2.js"),
            scope_url,
            22,
            second_queue.sender(),
        );
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.queued_register_job_count, 1);

        service.terminate_all_for_context_shutdown();

        let first_completion = pop_register_completion(&mut first_queue);
        assert_eq!(first_completion.request_id, 11);
        assert_eq!(
            first_completion.result.err().as_deref(),
            Some(SERVICE_WORKER_JOB_ABORTED_ERROR)
        );
        let second_completion = pop_register_completion(&mut second_queue);
        assert_eq!(second_completion.request_id, 22);
        assert_eq!(
            second_completion.result.err().as_deref(),
            Some(SERVICE_WORKER_JOB_ABORTED_ERROR)
        );
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.queued_register_job_count, 0);
    }

    #[test]
    fn context_shutdown_aborts_queued_unregister_jobs() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let first_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let mut register_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut unregister_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        insert_starting_version_with_register_job(
            &service,
            registration_id,
            first_version_id,
            url("https://example.test/app/worker-v1.js"),
            scope_url.clone(),
            11,
            register_queue.sender(),
        );

        assert_eq!(
            service.start_unregistration(&scope_url, 33, 1, unregister_queue.sender()),
            ServiceWorkerUnregisterStart::Queued
        );
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.queued_unregistration_job_count, 1);

        service.terminate_all_for_context_shutdown();

        let register_completion = pop_register_completion(&mut register_queue);
        assert_eq!(register_completion.request_id, 11);
        assert_eq!(
            register_completion.result.err().as_deref(),
            Some(SERVICE_WORKER_JOB_ABORTED_ERROR)
        );
        let unregister_completion = pop_unregister_completion(&mut unregister_queue);
        assert_eq!(unregister_completion.request_id, 33);
        assert!(!unregister_completion.result);
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.queued_unregistration_job_count, 0);
    }

    #[test]
    fn queued_unregister_job_phase_transitions_to_complete() {
        let mut job = ServiceWorkerQueuedUnregisterJob::new(None);
        assert_eq!(job.phase(), ServiceWorkerUnregisterJobPhase::Initial);
        job.mark_pending();
        assert_eq!(job.phase(), ServiceWorkerUnregisterJobPhase::MarkPending);
        assert_eq!(
            job.send_all(true),
            ServiceWorkerUnregisterJobPhase::Complete
        );
    }

    #[test]
    fn matching_controller_uses_longest_scope() {
        let service = new_service_worker_runtime_service();
        let root = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/root-worker.js"),
            url("https://example.test/"),
            [url("https://example.test/other.html")],
        );
        let app = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(2),
            ServiceWorkerVersionId(2),
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [url("https://example.test/app/page.html")],
        );

        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/app/page.html")),
            Some(app)
        );
        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/other.html")),
            Some(root)
        );
    }

    #[test]
    fn matching_controller_respects_scope_path_boundary() {
        let service = new_service_worker_runtime_service();
        let app = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/app-worker.js"),
            url("https://example.test/app"),
            [
                url("https://example.test/app"),
                url("https://example.test/app/page.html"),
                url("https://example.test/app?query"),
                url("https://example.test/app#fragment"),
            ],
        );

        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/app")),
            Some(app.clone())
        );
        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/app/page.html")),
            Some(app.clone())
        );
        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/app?query")),
            Some(app.clone())
        );
        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/app#fragment")),
            Some(app)
        );
        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/app2/page.html")),
            None
        );
    }

    #[test]
    fn matching_controller_ignores_inactive_registration() {
        let service = new_service_worker_runtime_service();
        insert_inactive_registration(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
        );
        let inactive = snapshot_for_registration(&service, ServiceWorkerRegistrationId(1));

        assert_eq!(
            service.matching_registration_for_client(&url("https://example.test/app/page.html")),
            Some(inactive)
        );
        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/app/page.html")),
            None
        );
        assert_eq!(
            service.matching_controller_for_fetch(
                &url("https://example.test/app/page.html"),
                &url("https://example.test/app/api.json")
            ),
            None
        );
    }

    #[test]
    fn navigation_preload_state_defaults_and_mutation_requires_active_worker() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        insert_inactive_registration(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            scope_url.clone(),
        );

        assert_eq!(service.navigation_preload_state_for_scope(&scope_url), None);
        assert_eq!(
            service.navigation_preload_state_for_scope(&url("https://example.test/other/")),
            None
        );
        assert_eq!(
            service.set_navigation_preload_enabled_for_scope(&scope_url, true),
            Err(ServiceWorkerNavigationPreloadStateError::InvalidState)
        );
        assert_eq!(
            service.set_navigation_preload_header_value_for_scope(
                &scope_url,
                "custom-preload".to_owned()
            ),
            Err(ServiceWorkerNavigationPreloadStateError::InvalidState)
        );
        assert_eq!(
            snapshot_for_registration(&service, registration_id).navigation_preload_state(),
            &ServiceWorkerNavigationPreloadState::default()
        );
    }

    #[test]
    fn navigation_preload_state_updates_active_registration_and_store() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            scope_url.clone(),
            [],
        );
        make_version_persistable(&service, version_id);

        service
            .set_navigation_preload_enabled_for_scope(&scope_url, true)
            .expect("active navigation preload enable should resolve");
        service
            .set_navigation_preload_header_value_for_scope(&scope_url, "custom-preload".to_owned())
            .expect("active navigation preload header update should resolve");

        let expected = ServiceWorkerNavigationPreloadState {
            enabled: true,
            header_value: "custom-preload".to_owned(),
        };
        assert_eq!(
            service.navigation_preload_state_for_scope(&scope_url),
            Some(expected.clone())
        );
        assert_eq!(
            snapshot_for_registration(&service, registration_id).navigation_preload_state(),
            &expected
        );
        let registration_key = ServiceWorkerRegistrationKey {
            scope_url: scope_url.clone(),
            storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
        };
        let stored = service
            .inner
            .resource_store
            .lock()
            .registration_for_key(&registration_key)
            .expect("navigation preload update should persist active registration");
        assert_eq!(stored.navigation_preload_state, expected);
    }

    #[test]
    fn navigation_preload_state_rolls_back_when_store_fails() {
        let temp_store = TempJsonStorePath::new("navigation-preload-state-failure");
        let resource_store =
            crate::new_shared_json_service_worker_resource_store(temp_store.store_path())
                .expect("json resource store should open");
        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            scope_url.clone(),
            [],
        );
        make_version_persistable(&service, version_id);
        resource_store.lock().fail_next_persist_attempts_for_test(2);

        assert_eq!(
            service.set_navigation_preload_enabled_for_scope(&scope_url, true),
            Err(ServiceWorkerNavigationPreloadStateError::StorageFailure)
        );
        assert_eq!(
            service.navigation_preload_state_for_scope(&scope_url),
            Some(ServiceWorkerNavigationPreloadState::default())
        );
        assert_eq!(
            snapshot_for_registration(&service, registration_id).navigation_preload_state(),
            &ServiceWorkerNavigationPreloadState::default()
        );

        service
            .set_navigation_preload_enabled_for_scope(&scope_url, true)
            .expect("navigation preload update should succeed after transient store failure");
        assert!(
            service
                .navigation_preload_state_for_scope(&scope_url)
                .expect("registration should remain visible")
                .enabled
        );
    }

    #[test]
    fn matching_controller_for_fetch_uses_controlled_document_scope() {
        let service = new_service_worker_runtime_service();
        let root = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/root-worker.js"),
            url("https://example.test/"),
            [url("https://example.test/root.html")],
        );
        let app = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(2),
            ServiceWorkerVersionId(2),
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [url("https://example.test/app/page.html")],
        );

        assert_eq!(
            service.matching_controller_for_fetch(
                &url("https://example.test/app/page.html"),
                &url("https://example.test/app/api.json")
            ),
            Some(app.clone())
        );
        assert_eq!(
            service.matching_controller_for_fetch(
                &url("https://example.test/app/page.html"),
                &url("https://example.test/other/api.json")
            ),
            Some(app.clone())
        );
        assert_eq!(
            service.matching_controller_for_fetch(
                &url("https://example.test/app/page.html"),
                &url("https://other.test/api.json")
            ),
            Some(app)
        );
        assert_eq!(
            service.matching_controller_for_fetch(
                &url("https://example.test/root.html"),
                &url("https://example.test/other/api.json")
            ),
            Some(root)
        );
    }

    #[test]
    fn active_registration_does_not_control_existing_live_client_until_claimed() {
        let service = new_service_worker_runtime_service();
        let app_page = url("https://example.test/app/page.html");
        let app_client_id = register_client_for_test(&service, app_page.clone());
        let state = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        assert_eq!(service.matching_controller_for_document(&app_page), None);
        assert_eq!(
            service.matching_controller_for_fetch(&app_page, &url("https://other.test/api.json")),
            None
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.live_client_count, 1);
        assert_eq!(diagnostics.controlled_client_count, 0);

        service.finish_worker_clients_claim_requested(
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
        );
        assert_eq!(
            service.matching_controller_for_client(app_client_id),
            Some(state)
        );
    }

    #[test]
    fn new_client_selects_longest_active_controller_on_registration() {
        let service = new_service_worker_runtime_service();
        let root = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/root-worker.js"),
            url("https://example.test/"),
            [],
        );
        let app = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(2),
            ServiceWorkerVersionId(2),
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        let app_client_id =
            register_client_for_test(&service, url("https://example.test/app/page.html"));
        let root_client_id =
            register_client_for_test(&service, url("https://example.test/other.html"));
        let out_of_scope_client_id =
            register_client_for_test(&service, url("https://other.test/page.html"));

        assert_eq!(
            service.matching_controller_for_client(app_client_id),
            Some(app)
        );
        assert_eq!(
            service.matching_controller_for_client(root_client_id),
            Some(root)
        );
        assert_eq!(
            service.matching_controller_for_client(out_of_scope_client_id),
            None
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.live_client_count, 3);
        assert_eq!(diagnostics.controlled_client_count, 2);
    }

    #[test]
    fn devtools_controlled_window_client_urls_use_current_document_urls() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [
                url("https://example.test/app/page.html#section"),
                url("https://example.test/app/other.html"),
            ],
        );

        assert_eq!(
            service.controlled_window_client_urls_for_version_for_devtools(
                registration_id,
                version_id
            ),
            vec![
                "https://example.test/app/other.html".to_owned(),
                "https://example.test/app/page.html".to_owned(),
            ]
        );
        assert_eq!(
            service
                .controlled_window_client_ids_for_version_for_devtools(registration_id, version_id),
            vec![1, 2]
        );
        assert!(
            service
                .controlled_window_client_urls_for_version_for_devtools(
                    registration_id,
                    ServiceWorkerVersionId(99)
                )
                .is_empty()
        );
        assert!(
            service
                .controlled_window_client_ids_for_version_for_devtools(
                    registration_id,
                    ServiceWorkerVersionId(99)
                )
                .is_empty()
        );
    }

    #[test]
    fn new_controlled_client_queues_service_worker_version_update_for_devtools() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state.record_target_created(registration_id, version_id, script_url, scope_url);
            service.take_target_output_events_for_test();
        }

        register_client_for_test(&service, url("https://example.test/app/page.html"));

        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated {
                    version_id: updated_version_id,
                    status,
                } if *updated_version_id == version_id.as_u64()
                    && *status == crate::runtime::RendererServiceWorkerVersionStatus::Activated
            )),
            "newly controlled client should refresh ServiceWorker version projection: {target_events:?}"
        );
    }

    #[test]
    fn local_worker_client_inherits_parent_controller_and_keeps_creation_url() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let parent_url = url("https://example.test/app/page.html");
        let controller = insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/sw.js"),
            url("https://example.test/app/"),
            [parent_url.clone()],
        );
        let parent_client_id = client_id_for_document(&service, &parent_url);
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&parent_url);

        assert_eq!(
            service.register_reserved_worker_client_inheriting_controller_from_client(
                url("data:text/javascript,postMessage('ready')"),
                storage_key.clone(),
                ServiceWorkerClientType::DedicatedWorker,
                true,
                parent_client_id,
            ),
            None,
            "data worker scripts must not inherit a service worker controller"
        );

        let local_worker_url = url("blob:https://example.test/blob-worker");
        let client_id = service
            .register_reserved_worker_client_inheriting_controller_from_client(
                local_worker_url.clone(),
                storage_key,
                ServiceWorkerClientType::DedicatedWorker,
                true,
                parent_client_id,
            )
            .expect("parent blob worker client should inherit controller");
        assert_eq!(
            service.matching_controller_for_client(client_id),
            Some(controller.clone())
        );
        assert_eq!(
            service.matching_controller_for_client_fetch(
                client_id,
                &url("https://example.test/other/api.json")
            ),
            Some(controller.clone())
        );
        let (worker_tx, _worker_rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(service.activate_reserved_worker_client(client_id, worker_tx));

        let query = ServiceWorkerClientQuery {
            request_id: 7,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::Worker,
                },
            },
        };
        let snapshots = service.query_clients(&query);
        assert!(
            snapshots.clients.iter().any(|client| {
                client.id == client_id && client.url == local_worker_url && client.controlled
            }),
            "inherited blob worker client should stay controlled while exposing its creation URL: {:?}",
            snapshots.clients
        );
    }

    #[test]
    fn clients_claim_controls_live_scope_clients_only_for_active_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let app_page = url("https://example.test/app/page.html");
        let other_page = url("https://example.test/other.html");
        let app_client_id = register_client_for_test(&service, app_page.clone());
        register_client_for_test(&service, other_page.clone());
        let state = insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        service.finish_worker_clients_claim_requested(registration_id, ServiceWorkerVersionId(99));
        assert_eq!(service.matching_controller_for_document(&app_page), None);

        service.finish_worker_clients_claim_requested(registration_id, version_id);

        assert_eq!(
            service.matching_controller_for_document(&app_page),
            Some(state.clone())
        );
        assert_eq!(
            service.matching_controller_for_fetch(&app_page, &url("https://other.test/api.json")),
            Some(state)
        );
        assert_eq!(service.matching_controller_for_document(&other_page), None);

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.live_client_count, 2);
        assert_eq!(diagnostics.controlled_client_count, 1);

        service.unregister_client(app_client_id);
        assert_eq!(service.matching_controller_for_document(&app_page), None);
        assert_eq!(service.diagnostics_snapshot().controlled_client_count, 0);
    }

    #[test]
    fn clients_claim_does_not_claim_client_when_longer_registration_matches() {
        let service = new_service_worker_runtime_service();
        let root_registration_id = ServiceWorkerRegistrationId(1);
        let root_version_id = ServiceWorkerVersionId(1);
        let app_registration_id = ServiceWorkerRegistrationId(2);
        let app_version_id = ServiceWorkerVersionId(2);
        insert_registered_version(
            &service,
            root_registration_id,
            root_version_id,
            url("https://example.test/root-worker.js"),
            url("https://example.test/"),
            [],
        );
        let app_state = insert_registered_version(
            &service,
            app_registration_id,
            app_version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        let app_page = url("https://example.test/app/page.html");
        let app_client_id = register_client_for_test(&service, app_page.clone());

        assert_eq!(
            service.matching_controller_for_document(&app_page),
            Some(app_state.clone())
        );

        service.finish_worker_clients_claim_requested(root_registration_id, root_version_id);

        assert_eq!(
            service.matching_controller_for_document(&app_page),
            Some(app_state)
        );
        let state = service.inner.state.lock();
        let root_registration = state.registrations.get(&root_registration_id).unwrap();
        let app_registration = state.registrations.get(&app_registration_id).unwrap();
        assert!(
            !root_registration
                .controlled_client_ids
                .contains(&app_client_id)
        );
        assert!(
            app_registration
                .controlled_client_ids
                .contains(&app_client_id)
        );
    }

    #[test]
    fn clients_claim_replaces_previous_controller_registration() {
        let service = new_service_worker_runtime_service();
        let root_registration_id = ServiceWorkerRegistrationId(1);
        let root_version_id = ServiceWorkerVersionId(1);
        let app_registration_id = ServiceWorkerRegistrationId(2);
        let app_version_id = ServiceWorkerVersionId(2);
        let root_state = insert_registered_version(
            &service,
            root_registration_id,
            root_version_id,
            url("https://example.test/root-worker.js"),
            url("https://example.test/"),
            [],
        );
        let app_page = url("https://example.test/app/page.html");
        let app_client_id = register_client_for_test(&service, app_page.clone());
        let app_state = insert_registered_version(
            &service,
            app_registration_id,
            app_version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        assert_eq!(
            service.matching_controller_for_document(&app_page),
            Some(root_state)
        );

        service.finish_worker_clients_claim_requested(app_registration_id, app_version_id);

        assert_eq!(
            service.matching_controller_for_document(&app_page),
            Some(app_state)
        );
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.live_client_count, 1);
        assert_eq!(diagnostics.controlled_client_count, 1);
        let state = service.inner.state.lock();
        let root_registration = state.registrations.get(&root_registration_id).unwrap();
        let app_registration = state.registrations.get(&app_registration_id).unwrap();
        assert!(
            !root_registration
                .controlled_client_ids
                .contains(&app_client_id)
        );
        assert!(
            app_registration
                .controlled_client_ids
                .contains(&app_client_id)
        );
    }

    #[test]
    fn clients_claim_skips_ineligible_live_window_clients() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let eligible_client_id =
            register_client_for_test(&service, url("https://example.test/app/eligible.html"));
        let not_ready_client_id =
            register_client_for_test(&service, url("https://example.test/app/not-ready.html"));
        let discarded_client_id =
            register_client_for_test(&service, url("https://example.test/app/discarded.html"));
        let insecure_client_id =
            register_client_for_test(&service, url("https://example.test/app/insecure.html"));
        {
            let mut state = service.inner.state.lock();
            state
                .live_clients
                .get_mut(&not_ready_client_id)
                .unwrap()
                .execution_ready = false;
            state
                .live_clients
                .get_mut(&discarded_client_id)
                .unwrap()
                .discarded_or_frozen = true;
            state
                .live_clients
                .get_mut(&insecure_client_id)
                .unwrap()
                .secure_context = false;
        }
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        service.finish_worker_clients_claim_requested(registration_id, version_id);

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(
            registration.controlled_client_ids,
            HashSet::from([eligible_client_id])
        );
    }

    #[test]
    fn reserved_window_client_is_hidden_from_clients_api_until_document_commit() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let control_state = insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        let reserved_creation_url = url("https://example.test/app/reserved.html#pending");
        let reserved_client_id = register_reserved_window_client_for_test(
            &service,
            reserved_creation_url.clone(),
            ServiceWorkerClientFrameType::TopLevel,
        );

        assert_eq!(
            service.matching_controller_for_client(reserved_client_id),
            Some(control_state)
        );

        let hidden_match_all = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 18,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert!(hidden_match_all.clients.is_empty());

        let hidden_get = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 19,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(reserved_client_id),
            },
        });
        assert!(hidden_get.clients.is_empty());

        let committed_creation_url = url("https://example.test/app/committed.html#current");
        assert!(service.update_client_document_with_storage_key(
            reserved_client_id,
            committed_creation_url.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&committed_creation_url,),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(3)),
        ));

        let visible = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 20,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert_eq!(visible.clients.len(), 1);
        assert_eq!(visible.clients[0].id, reserved_client_id);
        assert_eq!(visible.clients[0].url, committed_creation_url);
        assert!(visible.clients[0].controlled);
    }

    #[test]
    fn clients_claim_skips_reserved_window_clients_until_document_commit() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let ready_client_id =
            register_client_for_test(&service, url("https://example.test/app/ready.html"));
        let reserved_creation_url = url("https://example.test/app/reserved.html");
        let reserved_client_id = register_reserved_window_client_for_test(
            &service,
            reserved_creation_url.clone(),
            ServiceWorkerClientFrameType::TopLevel,
        );
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        service.finish_worker_clients_claim_requested(registration_id, version_id);

        {
            let state = service.inner.state.lock();
            let registration = state.registrations.get(&registration_id).unwrap();
            assert_eq!(
                registration.controlled_client_ids,
                HashSet::from([ready_client_id])
            );
            assert!(
                !state
                    .live_clients
                    .get(&reserved_client_id)
                    .expect("reserved client should remain live")
                    .execution_ready
            );
        }

        let hidden = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 21,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(reserved_client_id),
            },
        });
        assert!(hidden.clients.is_empty());

        assert!(service.update_client_document_with_storage_key(
            reserved_client_id,
            reserved_creation_url.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&reserved_creation_url),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(4)),
        ));

        let visible = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 22,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert_eq!(
            visible
                .clients
                .iter()
                .map(|client| client.id)
                .collect::<Vec<_>>(),
            vec![ready_client_id, reserved_client_id]
        );
    }

    #[test]
    fn bypassed_navigation_client_stays_uncontrolled_until_claimed() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let document_url = url("https://example.test/app/bypassed.html");
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        let client_id = service.register_reserved_client_with_storage_key_bypassing_service_worker(
            document_url.clone(),
            storage_key.clone(),
            ServiceWorkerClientFrameType::TopLevel,
            None,
        );
        assert!(service.matching_controller_for_client(client_id).is_none());

        assert!(
            service.update_client_document_with_storage_key_and_completion_sender(
                client_id,
                document_url,
                storage_key,
                ServiceWorkerClientFrameType::TopLevel,
                Some(crate::native_bridge::WindowDocumentOwner::for_test(1)),
                test_completion_sender(),
            )
        );
        assert!(service.matching_controller_for_client(client_id).is_none());

        service.finish_worker_clients_claim_requested(registration_id, version_id);
        assert!(service.matching_controller_for_client(client_id).is_some());
    }

    #[test]
    fn window_client_completion_target_rotates_with_exact_document_owner() {
        let service = new_service_worker_runtime_service();
        let document_url = url("https://client-epoch.test/page.html");
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
        let client_id = service.register_client_with_storage_key(
            document_url.clone(),
            storage_key.clone(),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(10)),
            test_completion_sender(),
        );
        let initial_target = service
            .inner
            .state
            .lock()
            .live_clients
            .get(&client_id)
            .and_then(ServiceWorkerClient::window_completion_target)
            .expect("initial window client target");

        assert!(service.update_client_document_with_storage_key(
            client_id,
            document_url,
            storage_key,
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(11)),
        ));
        let replacement_target = service
            .inner
            .state
            .lock()
            .live_clients
            .get(&client_id)
            .and_then(ServiceWorkerClient::window_completion_target)
            .expect("replacement window client target");

        assert_eq!(initial_target.client_id, replacement_target.client_id);
        assert_eq!(
            initial_target.document_owner,
            crate::native_bridge::WindowDocumentOwner::for_test(10)
        );
        assert_eq!(
            replacement_target.document_owner,
            crate::native_bridge::WindowDocumentOwner::for_test(11)
        );
        assert_ne!(initial_target, replacement_target);
    }

    #[test]
    fn clients_claim_notifies_only_newly_controlled_live_window_clients() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let app_page = url("https://example.test/app/page.html");
        let outside_page = url("https://example.test/outside.html");
        let mut app_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut outside_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let app_client_id = service.register_client_with_storage_key(
            app_page.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&app_page),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(7)),
            app_queue.sender(),
        );
        service.register_client_with_storage_key(
            outside_page.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&outside_page),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(11)),
            outside_queue.sender(),
        );
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        service.finish_worker_clients_claim_requested(registration_id, version_id);

        let completion = match app_queue.pop_internal() {
            Some(crate::page_task_queue::RendererServiceWorkerInternalTask::ControllerChange(
                completion,
            )) => completion,
            other => panic!("expected controllerchange completion, got {other:?}"),
        };
        assert_eq!(completion.target.client_id, app_client_id);
        assert_eq!(
            completion.target.document_owner,
            crate::native_bridge::WindowDocumentOwner::for_test(7)
        );
        assert!(!app_queue.has_ready_task());
        assert!(!outside_queue.has_ready_task());

        service.finish_worker_clients_claim_requested(registration_id, version_id);

        assert!(!app_queue.has_ready_task());
        assert!(!outside_queue.has_ready_task());
    }

    #[test]
    fn clients_claim_notifies_newly_controlled_live_worker_clients() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let app_worker = url("https://example.test/app/worker.js");
        let outside_worker = url("https://example.test/outside/worker.js");
        let (worker_tx, mut worker_rx) = tokio::sync::mpsc::unbounded_channel();
        service.register_worker_client_with_storage_key(
            app_worker.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&app_worker),
            ServiceWorkerClientType::DedicatedWorker,
            true,
            worker_tx,
        );
        let (outside_tx, mut outside_rx) = tokio::sync::mpsc::unbounded_channel();
        service.register_worker_client_with_storage_key(
            outside_worker.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&outside_worker),
            ServiceWorkerClientType::DedicatedWorker,
            true,
            outside_tx,
        );
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        service.finish_worker_clients_claim_requested(registration_id, version_id);

        assert!(matches!(
            worker_rx.try_recv(),
            Ok(crate::worker::WorkerMessage::ServiceWorkerControllerChange)
        ));
        assert!(matches!(
            worker_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            outside_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        service.finish_worker_clients_claim_requested(registration_id, version_id);

        assert!(matches!(
            worker_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            outside_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn client_query_matches_live_window_clients_for_active_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let controlled_page = url("https://example.test/app/controlled.html");
        let uncontrolled_page = url("https://example.test/app/uncontrolled.html");
        let not_ready_page = url("https://example.test/app/not-ready.html");
        let discarded_page = url("https://example.test/app/discarded.html");
        let out_of_scope_page = url("https://example.test/other.html");
        let cross_origin_page = url("https://other.test/app/page.html");
        let controlled_client_id = register_client_for_test(&service, controlled_page.clone());
        let uncontrolled_client_id = register_client_for_test(&service, uncontrolled_page.clone());
        let not_ready_client_id = register_client_for_test(&service, not_ready_page);
        let discarded_client_id = register_client_for_test(&service, discarded_page);
        let out_of_scope_client_id = register_client_for_test(&service, out_of_scope_page.clone());
        let cross_origin_client_id = register_client_for_test(&service, cross_origin_page);
        {
            let mut state = service.inner.state.lock();
            state
                .live_clients
                .get_mut(&not_ready_client_id)
                .expect("not-ready client should exist")
                .execution_ready = false;
            state
                .live_clients
                .get_mut(&discarded_client_id)
                .expect("discarded client should exist")
                .discarded_or_frozen = true;
        }
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let registration = state.registrations.get_mut(&registration_id).unwrap();
            registration
                .controlled_client_ids
                .insert(controlled_client_id);
        }

        let controlled_only = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 10,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert_eq!(controlled_only.request_id, 10);
        assert_eq!(controlled_only.clients.len(), 1);
        assert_eq!(controlled_only.clients[0].id, controlled_client_id);
        assert_eq!(
            controlled_only.clients[0].exposed_id,
            service_worker_exposed_client_id(controlled_client_id)
        );
        assert_ne!(
            controlled_only.clients[0].exposed_id,
            controlled_client_id.as_u64().to_string()
        );
        assert!(controlled_only.clients[0].controlled);
        assert_eq!(controlled_only.clients[0].url, controlled_page);
        assert_eq!(
            controlled_only.clients[0].client_type,
            ServiceWorkerClientType::Window
        );
        assert_eq!(
            controlled_only.clients[0].frame_type,
            ServiceWorkerClientFrameType::TopLevel
        );
        assert_eq!(
            controlled_only.clients[0].visibility_state,
            ServiceWorkerClientVisibilityState::Visible
        );

        let all_scope_windows = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 11,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::All,
                },
            },
        });
        assert_eq!(
            all_scope_windows
                .clients
                .iter()
                .map(|client| client.id)
                .collect::<Vec<_>>(),
            vec![controlled_client_id, uncontrolled_client_id]
        );
        assert_eq!(all_scope_windows.clients[1].url, uncontrolled_page);
        assert!(!all_scope_windows.clients[1].controlled);

        let worker_clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 12,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::Worker,
                },
            },
        });
        assert!(worker_clients.clients.is_empty());

        let shared_worker_clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 13,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::SharedWorker,
                },
            },
        });
        assert!(shared_worker_clients.clients.is_empty());

        let found_uncontrolled = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 14,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(uncontrolled_client_id),
            },
        });
        assert_eq!(found_uncontrolled.clients.len(), 1);
        assert_eq!(found_uncontrolled.clients[0].id, uncontrolled_client_id);
        assert!(!found_uncontrolled.clients[0].controlled);

        let hidden_not_ready = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 19,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(not_ready_client_id),
            },
        });
        assert!(hidden_not_ready.clients.is_empty());

        let hidden_discarded = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 20,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(discarded_client_id),
            },
        });
        assert!(hidden_discarded.clients.is_empty());

        let hidden_legacy_internal_id = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 18,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: uncontrolled_client_id.as_u64().to_string(),
            },
        });
        assert!(hidden_legacy_internal_id.clients.is_empty());

        let found_out_of_scope_same_origin = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 16,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(out_of_scope_client_id),
            },
        });
        assert_eq!(found_out_of_scope_same_origin.clients.len(), 1);
        assert_eq!(
            found_out_of_scope_same_origin.clients[0].id,
            out_of_scope_client_id
        );
        assert_eq!(
            found_out_of_scope_same_origin.clients[0].url,
            out_of_scope_page
        );
        assert!(!found_out_of_scope_same_origin.clients[0].controlled);

        let hidden_cross_origin = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 17,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(cross_origin_client_id),
            },
        });
        assert!(hidden_cross_origin.clients.is_empty());

        {
            let mut state = service.inner.state.lock();
            state.versions.get_mut(&version_id).unwrap().lifecycle_state =
                ServiceWorkerVersionLifecycleState::Installed;
        }
        let inactive_result = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 15,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(controlled_client_id),
            },
        });
        assert!(inactive_result.clients.is_empty());
    }

    #[test]
    fn client_query_filters_existing_live_clients_by_type() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let window_client_id =
            register_client_for_test(&service, url("https://example.test/app/window.html"));
        let dedicated_worker_client_id = insert_live_client_record_for_test(
            &service,
            url("https://example.test/app/dedicated-worker.js"),
            ServiceWorkerClientType::DedicatedWorker,
        );
        let shared_worker_client_id = insert_live_client_record_for_test(
            &service,
            url("https://example.test/app/shared-worker.js"),
            ServiceWorkerClientType::SharedWorker,
        );
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let registration = state.registrations.get_mut(&registration_id).unwrap();
            registration.controlled_client_ids.insert(window_client_id);
            registration
                .controlled_client_ids
                .insert(dedicated_worker_client_id);
        }

        let worker_clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 21,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::Worker,
                },
            },
        });
        assert_eq!(
            worker_clients
                .clients
                .iter()
                .map(|client| client.id)
                .collect::<Vec<_>>(),
            vec![dedicated_worker_client_id]
        );
        assert_eq!(
            worker_clients.clients[0].client_type,
            ServiceWorkerClientType::DedicatedWorker
        );
        assert_eq!(
            worker_clients.clients[0].frame_type,
            ServiceWorkerClientFrameType::None
        );
        assert!(worker_clients.clients[0].controlled);

        let shared_worker_controlled_only = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 22,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::SharedWorker,
                },
            },
        });
        assert!(shared_worker_controlled_only.clients.is_empty());

        let shared_worker_clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 23,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::SharedWorker,
                },
            },
        });
        assert_eq!(shared_worker_clients.clients.len(), 1);
        assert_eq!(shared_worker_clients.clients[0].id, shared_worker_client_id);
        assert_eq!(
            shared_worker_clients.clients[0].client_type,
            ServiceWorkerClientType::SharedWorker
        );
        assert!(!shared_worker_clients.clients[0].controlled);

        let all_clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 24,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::All,
                },
            },
        });
        assert_eq!(
            all_clients
                .clients
                .iter()
                .map(|client| client.id)
                .collect::<Vec<_>>(),
            vec![
                window_client_id,
                dedicated_worker_client_id,
                shared_worker_client_id
            ]
        );

        let window_clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 25,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert_eq!(
            window_clients
                .clients
                .iter()
                .map(|client| client.id)
                .collect::<Vec<_>>(),
            vec![window_client_id]
        );
    }

    #[test]
    fn client_query_preserves_window_client_frame_type() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let top_level_client_id = register_window_client_for_test(
            &service,
            url("https://example.test/app/top.html"),
            ServiceWorkerClientFrameType::TopLevel,
        );
        let nested_client_id = register_window_client_for_test(
            &service,
            url("https://example.test/app/frame.html"),
            ServiceWorkerClientFrameType::Nested,
        );
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let registration = state.registrations.get_mut(&registration_id).unwrap();
            registration
                .controlled_client_ids
                .insert(top_level_client_id);
            registration.controlled_client_ids.insert(nested_client_id);
        }

        let clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 26,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert_eq!(
            clients
                .clients
                .iter()
                .map(|client| (client.id, client.frame_type))
                .collect::<Vec<_>>(),
            vec![
                (top_level_client_id, ServiceWorkerClientFrameType::TopLevel),
                (nested_client_id, ServiceWorkerClientFrameType::Nested),
            ]
        );

        let nested = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 27,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: service_worker_exposed_client_id(nested_client_id),
            },
        });
        assert_eq!(nested.clients.len(), 1);
        assert_eq!(
            nested.clients[0].frame_type,
            ServiceWorkerClientFrameType::Nested
        );
    }

    #[test]
    fn client_query_requires_exact_active_controller_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker.js");
        let waiting_script_url = url("https://example.test/app/worker-updated.js");
        let client_id =
            register_client_for_test(&service, url("https://example.test/app/page.html"));
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            active_script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let registration = state.registrations.get_mut(&registration_id).unwrap();
            registration.script_url = waiting_script_url.clone();
            registration.waiting_version_id = Some(waiting_version_id);
            registration.controlled_client_ids.insert(client_id);
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installed,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        let waiting_worker_clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 26,
            registration_id,
            version_id: waiting_version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert!(waiting_worker_clients.clients.is_empty());

        let active_worker_clients = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 27,
            registration_id,
            version_id: active_version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: false,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert_eq!(
            active_worker_clients
                .clients
                .iter()
                .map(|client| client.id)
                .collect::<Vec<_>>(),
            vec![client_id]
        );
    }

    #[test]
    fn client_document_update_tracks_creation_url_and_regenerates_cross_origin_exposed_id() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let initial_creation_url = url("https://example.test/app/page.html#initial");
        let initial_current_url = url("https://example.test/app/page.html");
        let client_id = register_client_for_test(&service, initial_creation_url.clone());
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        let initial = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 30,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert_eq!(initial.clients.len(), 1);
        assert_eq!(initial.clients[0].url, initial_creation_url);
        let initial_exposed_id = initial.clients[0].exposed_id.clone();
        {
            let state = service.inner.state.lock();
            let client = state.live_clients.get(&client_id).unwrap();
            assert_eq!(client.creation_url, initial_creation_url);
            assert_eq!(client.document_url, initial_current_url);
            assert_eq!(client.exposed_id, initial_exposed_id);
        }

        let same_origin_creation_url = url("https://example.test/app/next.html#same-origin");
        let same_origin_current_url = url("https://example.test/app/next.html");
        assert!(service.update_client_document_with_storage_key(
            client_id,
            same_origin_creation_url.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(
                &same_origin_creation_url,
            ),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(2)),
        ));
        let same_origin_result = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 31,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: initial_exposed_id.clone(),
            },
        });
        assert_eq!(same_origin_result.clients.len(), 1);
        assert_eq!(same_origin_result.clients[0].url, same_origin_creation_url);
        assert_eq!(same_origin_result.clients[0].exposed_id, initial_exposed_id);
        assert!(same_origin_result.clients[0].controlled);
        {
            let state = service.inner.state.lock();
            let client = state.live_clients.get(&client_id).unwrap();
            assert_eq!(client.creation_url, same_origin_creation_url);
            assert_eq!(client.document_url, same_origin_current_url);
            assert_eq!(client.exposed_id, initial_exposed_id);
        }

        let cross_origin_creation_url = url("https://other.test/app/page.html#cross-origin");
        let cross_origin_current_url = url("https://other.test/app/page.html");
        assert!(service.update_client_document_with_storage_key(
            client_id,
            cross_origin_creation_url.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(
                &cross_origin_creation_url,
            ),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(3)),
        ));
        let hidden_old_exposed_id = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 32,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::Get {
                exposed_client_id: initial_exposed_id.clone(),
            },
        });
        assert!(hidden_old_exposed_id.clients.is_empty());
        {
            let state = service.inner.state.lock();
            let client = state.live_clients.get(&client_id).unwrap();
            assert_eq!(client.creation_url, cross_origin_creation_url);
            assert_eq!(client.document_url, cross_origin_current_url);
            assert_ne!(client.exposed_id, initial_exposed_id);
            let registration = state.registrations.get(&registration_id).unwrap();
            assert!(!registration.controlled_client_ids.contains(&client_id));
        }
    }

    #[test]
    fn client_navigate_result_exposes_same_origin_window_client_only() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );

        let controlled_client_id =
            register_client_for_test(&service, url("https://example.test/app/next.html"));
        let out_of_scope_same_origin_client_id =
            register_client_for_test(&service, url("https://example.test/elsewhere.html"));
        let cross_origin_client_id =
            register_client_for_test(&service, url("https://other.test/app/next.html"));
        let about_blank_client_id = register_client_for_test(&service, url("about:blank"));

        let controlled_result = service
            .client_navigate_result_for_current_window_client(version_id, controlled_client_id)
            .expect("controlled same-origin client should produce a result")
            .expect("controlled same-origin client should be exposed");
        assert_eq!(controlled_result.id, controlled_client_id);
        assert_eq!(
            controlled_result.url,
            url("https://example.test/app/next.html")
        );
        assert!(controlled_result.controlled);
        assert_eq!(
            controlled_result.client_type,
            ServiceWorkerClientType::Window
        );

        let out_of_scope_same_origin_result = service
            .client_navigate_result_for_current_window_client(
                version_id,
                out_of_scope_same_origin_client_id,
            )
            .expect("same-origin client should produce a result")
            .expect("same-origin client should be exposed even if no longer controlled");
        assert_eq!(
            out_of_scope_same_origin_result.id,
            out_of_scope_same_origin_client_id
        );
        assert_eq!(
            out_of_scope_same_origin_result.url,
            url("https://example.test/elsewhere.html")
        );
        assert!(!out_of_scope_same_origin_result.controlled);

        let cross_origin_result = service
            .client_navigate_result_for_current_window_client(version_id, cross_origin_client_id)
            .expect("cross-origin client should resolve as a successful null result");
        assert_eq!(cross_origin_result, None);

        let about_blank_result = service
            .client_navigate_result_for_current_window_client(version_id, about_blank_client_id)
            .expect("about:blank client should resolve as a successful null result");
        assert_eq!(about_blank_result, None);
    }

    #[test]
    fn client_focus_result_marks_current_window_client_focused() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        let first_client_id =
            register_client_for_test(&service, url("https://example.test/app/first.html"));
        let second_client_id =
            register_client_for_test(&service, url("https://example.test/app/second.html"));

        let focused = service
            .client_focus_result_for_current_window_client(version_id, second_client_id)
            .expect("focus should produce a window client snapshot");
        assert_eq!(focused.id, second_client_id);
        assert!(focused.focused);

        let all_scope_windows = service.query_clients(&ServiceWorkerClientQuery {
            request_id: 20,
            registration_id,
            version_id,
            kind: ServiceWorkerClientQueryKind::MatchAll {
                options: ServiceWorkerClientQueryOptions {
                    include_uncontrolled: true,
                    client_type: ServiceWorkerClientQueryType::Window,
                },
            },
        });
        assert_eq!(
            all_scope_windows
                .clients
                .iter()
                .map(|client| client.id)
                .collect::<Vec<_>>(),
            vec![second_client_id, first_client_id]
        );
        let first = all_scope_windows
            .clients
            .iter()
            .find(|client| client.id == first_client_id)
            .expect("first client should remain live");
        let second = all_scope_windows
            .clients
            .iter()
            .find(|client| client.id == second_client_id)
            .expect("second client should remain live");
        assert!(!first.focused);
        assert!(second.focused);
    }

    #[test]
    fn client_focus_result_reports_not_found_for_missing_or_cross_origin_client() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        let cross_origin_client_id =
            register_client_for_test(&service, url("https://other.test/app/page.html"));

        let missing = service
            .client_focus_result_for_current_window_client(
                version_id,
                ServiceWorkerClientId::from_u64_for_test(999),
            )
            .expect_err("missing focus target should reject");
        assert_eq!(missing, ServiceWorkerClientFocusError::not_found());

        let cross_origin = service
            .client_focus_result_for_current_window_client(version_id, cross_origin_client_id)
            .expect_err("cross-origin focus target should be hidden as not found");
        assert_eq!(cross_origin, ServiceWorkerClientFocusError::not_found());
    }

    #[test]
    fn client_focus_result_reports_inactive_for_discarded_window_client() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        let focused_client_id =
            register_client_for_test(&service, url("https://example.test/app/focused.html"));
        let discarded_client_id =
            register_client_for_test(&service, url("https://example.test/app/discarded.html"));
        {
            let mut state = service.inner.state.lock();
            state
                .live_clients
                .get_mut(&focused_client_id)
                .expect("focused client should exist")
                .focused = true;
            state
                .live_clients
                .get_mut(&discarded_client_id)
                .expect("discarded client should exist")
                .discarded_or_frozen = true;
        }

        let inactive = service
            .client_focus_result_for_current_window_client(version_id, discarded_client_id)
            .expect_err("discarded focus target should reject as inactive");
        assert_eq!(inactive, ServiceWorkerClientFocusError::inactive());

        let state = service.inner.state.lock();
        assert!(
            state
                .live_clients
                .get(&focused_client_id)
                .expect("focused client should remain live")
                .focused
        );
        assert!(
            !state
                .live_clients
                .get(&discarded_client_id)
                .expect("discarded client should remain live")
                .focused
        );
    }

    #[test]
    fn client_focus_request_rejects_discarded_window_before_page_owner() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let document_url = url("https://example.test/app/page.html");
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let client_id = service.register_client(document_url, 7, completion_queue.sender());
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .registrations
                .get_mut(&registration_id)
                .expect("registration should exist")
                .controlled_client_ids
                .insert(client_id);
            state
                .live_clients
                .get_mut(&client_id)
                .expect("client should exist")
                .discarded_or_frozen = true;
        }

        let run = exact_version_run(&service, version_id);
        service.finish_client_focus_requested(
            ServiceWorkerClientFocus {
                request_id: 40,
                source_version_id: version_id,
                target_client_id: client_id,
            },
            run,
        );

        assert!(!completion_queue.has_ready_task());
        let state = service.inner.state.lock();
        assert!(
            !state
                .live_clients
                .get(&client_id)
                .expect("client should remain live")
                .focused
        );
    }

    #[test]
    fn clients_open_window_uses_ready_active_window_host() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [],
        );

        let mut not_ready_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut discarded_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut ready_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        let not_ready_url = url("https://example.test/app/not-ready.html");
        let discarded_url = url("https://example.test/app/discarded.html");
        let ready_url = url("https://example.test/app/ready.html");
        let not_ready_client_id = service.register_client_with_storage_key(
            not_ready_url.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&not_ready_url),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(21)),
            not_ready_queue.sender(),
        );
        let discarded_client_id = service.register_client_with_storage_key(
            discarded_url.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&discarded_url),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(22)),
            discarded_queue.sender(),
        );
        let ready_client_id = service.register_client_with_storage_key(
            ready_url.clone(),
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&ready_url),
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(23)),
            ready_queue.sender(),
        );
        {
            let mut state = service.inner.state.lock();
            let registration = state
                .registrations
                .get_mut(&registration_id)
                .expect("registration should exist");
            registration.controlled_client_ids.extend([
                not_ready_client_id,
                discarded_client_id,
                ready_client_id,
            ]);
            state
                .live_clients
                .get_mut(&not_ready_client_id)
                .expect("not-ready client should exist")
                .execution_ready = false;
            state
                .live_clients
                .get_mut(&discarded_client_id)
                .expect("discarded client should exist")
                .discarded_or_frozen = true;
        }

        let run = exact_version_run(&service, version_id);
        service.finish_clients_open_window_requested(
            ServiceWorkerClientsOpenWindow {
                request_id: 51,
                source_version_id: version_id,
                url: url("https://example.test/app/opened.html"),
            },
            run.clone(),
        );

        assert!(!not_ready_queue.has_ready_task());
        assert!(!discarded_queue.has_ready_task());
        let Some(
            crate::page_task_queue::RendererServiceWorkerInternalTask::ClientsOpenWindowRequest(
                completion,
            ),
        ) = ready_queue.pop_internal()
        else {
            panic!("ready active window should receive openWindow request");
        };
        assert_eq!(completion.request_id, 51);
        assert_eq!(completion.host.client_id, ready_client_id);
        assert_eq!(
            completion.host.document_owner,
            crate::native_bridge::WindowDocumentOwner::for_test(23)
        );
        assert_eq!(completion.source_version_id, version_id);
        assert_eq!(completion.source_run, run);
        assert_eq!(completion.url, url("https://example.test/app/opened.html"));
    }

    #[test]
    fn unregister_client_removes_only_matching_client_id_for_same_url() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let state = insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [],
        );
        let page = url("https://example.test/app/page.html");
        let first_client_id = register_client_for_test(&service, page.clone());
        let second_client_id = register_client_for_test(&service, page);

        service.finish_worker_clients_claim_requested(registration_id, version_id);
        assert_eq!(service.diagnostics_snapshot().controlled_client_count, 2);
        assert_eq!(
            service.matching_controller_for_client(first_client_id),
            Some(state.clone())
        );
        assert_eq!(
            service.matching_controller_for_client(second_client_id),
            Some(state.clone())
        );

        service.unregister_client(first_client_id);

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.live_client_count, 1);
        assert_eq!(diagnostics.controlled_client_count, 1);
        assert_eq!(
            service.matching_controller_for_client(first_client_id),
            None
        );
        assert_eq!(
            service.matching_controller_for_client(second_client_id),
            Some(state)
        );
    }

    #[test]
    fn unregister_last_controlled_client_queues_waiting_activation() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let client_id = ServiceWorkerClientId::from_u64_for_test(1);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let waiting_script_url = url("https://example.test/app/worker-v2.js");
        let waiting_run = RendererServiceWorkerRunIdentity::fresh();
        let waiting_host = new_loading_test_host(waiting_version_id, &waiting_run);
        {
            let mut state = service.inner.state.lock();
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    creation_url: url("https://example.test/app/page.html"),
                    document_url: url("https://example.test/app/page.html"),
                    client_type: ServiceWorkerClientType::Window,
                    frame_type: ServiceWorkerClientFrameType::TopLevel,
                    visibility_state: ServiceWorkerClientVisibilityState::Visible,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    secure_context: true,
                    execution_ready: true,
                    discarded_or_frozen: false,
                    document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
                    endpoint: ServiceWorkerClientEndpoint::Page(test_completion_sender()),
                    focused: false,
                },
            );
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: waiting_script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: Some(waiting_version_id),
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installed,
                    running_state: ServiceWorkerVersionRunningState::Starting {
                        host: waiting_host,
                    },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: waiting_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.unregister_client(client_id);

        assert_eq!(service.pending_service_lane_event_count(), 0);
        let state = service.inner.state.lock();
        assert!(!state.live_clients.contains_key(&client_id));
        let registration = state.registrations.get(&registration_id).unwrap();
        assert!(registration.controlled_client_ids.is_empty());
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
        let waiting = state.versions.get(&waiting_version_id).unwrap();
        assert_eq!(
            waiting.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activating
        );
        assert_eq!(waiting.in_flight_event_count, 1);
        assert_eq!(waiting.pending_start_events.len(), 1);
        let ServiceWorkerPendingStartEvent::Lifecycle(event) =
            waiting.pending_start_events.front().unwrap()
        else {
            panic!("expected queued activate event");
        };
        assert_eq!(event.kind, ServiceWorkerLifecycleEventKind::Activate);
        assert_eq!(
            event.owner,
            test_run_owner(waiting_version_id, &waiting_run)
        );
    }

    #[test]
    fn pending_unregistration_keeps_existing_controlled_fetch_until_client_closes() {
        let service = new_service_worker_runtime_service();
        let document_url = url("https://example.test/app/page.html");
        let state = insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);

        assert!(service.mark_registration_unregistered(state.scope_url()));
        assert!(!service.mark_registration_unregistered(state.scope_url()));
        assert_eq!(
            service.matching_registration_for_client(&document_url),
            None
        );
        assert_eq!(
            service.matching_controller_for_fetch(
                &document_url,
                &url("https://example.test/app/api.json")
            ),
            Some(state.clone())
        );
        assert_eq!(
            service
                .matching_controller_for_fetch(&document_url, &url("https://other.test/api.json")),
            Some(state.clone())
        );
        assert_eq!(
            service.matching_controller_for_document(&document_url),
            Some(state.clone())
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.pending_unregistration_count, 1);
        assert_eq!(diagnostics.controlled_client_count, 1);
        assert_eq!(
            diagnostics.registrations[0].pending_clear_phase,
            Some("waiting-for-controllees")
        );

        service.unregister_client(client_id);

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 0);
        assert_eq!(diagnostics.version_count, 0);
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        assert_eq!(
            service.matching_controller_for_fetch(&document_url, &url("https://other.test/api")),
            None
        );
    }

    #[test]
    fn pending_unregistration_without_controllees_deletes_immediately() {
        let service = new_service_worker_runtime_service();
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/app/worker.js"),
            scope_url.clone(),
            [],
        );

        assert!(service.mark_registration_unregistered(&scope_url));

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 0);
        assert_eq!(diagnostics.version_count, 0);
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        assert!(!service.mark_registration_unregistered(&scope_url));
    }

    #[test]
    fn pending_unregistration_prunes_sync_and_periodic_sync_records() {
        let service = new_service_worker_runtime_service();
        let first_registration_id = ServiceWorkerRegistrationId(1);
        let second_registration_id = ServiceWorkerRegistrationId(2);
        let first_scope_url = url("https://example.test/app/");
        let second_scope_url = url("https://example.test/other/");
        insert_registered_version(
            &service,
            first_registration_id,
            ServiceWorkerVersionId(1),
            url("https://example.test/app/worker.js"),
            first_scope_url.clone(),
            [],
        );
        insert_registered_version(
            &service,
            second_registration_id,
            ServiceWorkerVersionId(2),
            url("https://example.test/other/worker.js"),
            second_scope_url,
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state.sync_registrations.insert(
                (first_registration_id, "first-sync".to_owned()),
                ServiceWorkerSyncRegistrationRecord {
                    failed_attempts: 1,
                    ..Default::default()
                },
            );
            state.sync_registrations.insert(
                (second_registration_id, "second-sync".to_owned()),
                ServiceWorkerSyncRegistrationRecord {
                    failed_attempts: 0,
                    ..Default::default()
                },
            );
            state.periodic_sync_registrations.insert(
                (first_registration_id, "first-periodic".to_owned()),
                ServiceWorkerPeriodicSyncRegistrationRecord::new(10),
            );
            state.periodic_sync_registrations.insert(
                (second_registration_id, "second-periodic".to_owned()),
                ServiceWorkerPeriodicSyncRegistrationRecord::new(20),
            );
        }

        assert!(service.mark_registration_unregistered(&first_scope_url));

        let state = service.inner.state.lock();
        assert!(
            state
                .sync_registrations
                .keys()
                .all(|(registration_id, _)| *registration_id != first_registration_id)
        );
        assert!(
            state
                .periodic_sync_registrations
                .keys()
                .all(|(registration_id, _)| *registration_id != first_registration_id)
        );
        assert!(
            state
                .sync_registrations
                .contains_key(&(second_registration_id, "second-sync".to_owned()))
        );
        assert!(
            state
                .periodic_sync_registrations
                .contains_key(&(second_registration_id, "second-periodic".to_owned()))
        );
    }

    #[test]
    fn pending_unregistration_deletion_clears_owner_state() {
        let service = new_service_worker_runtime_service();
        let deleted_registration_id = ServiceWorkerRegistrationId(1);
        let kept_registration_id = ServiceWorkerRegistrationId(2);
        let deleted_version_id = ServiceWorkerVersionId(1);
        let kept_version_id = ServiceWorkerVersionId(2);
        let deleted_event_id = ServiceWorkerEventId(31);
        let kept_event_id = ServiceWorkerEventId(32);
        let deleted_scope_url = url("https://example.test/app/");
        let kept_scope_url = url("https://example.test/other/");
        let request_url = url("https://example.test/app/data.txt");
        let mut deleted_fetch_queue = async_subresource_completion_queue();
        let mut kept_fetch_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            deleted_registration_id,
            deleted_version_id,
            url("https://example.test/app/worker.js"),
            deleted_scope_url.clone(),
            [],
        );
        insert_registered_version(
            &service,
            kept_registration_id,
            kept_version_id,
            url("https://example.test/other/worker.js"),
            kept_scope_url,
            [],
        );
        {
            let mut state = service.inner.state.lock();
            for (event_id, version_id, completion_tx) in [
                (
                    deleted_event_id,
                    deleted_version_id,
                    deleted_fetch_queue.sender(),
                ),
                (kept_event_id, kept_version_id, kept_fetch_queue.sender()),
            ] {
                let run = state
                    .versions
                    .get(&version_id)
                    .expect("inserted version")
                    .run
                    .clone();
                state.pending_fetch_jobs.insert(
                    event_id,
                    ServiceWorkerFetchJob {
                        internal_id: event_id.as_u64(),
                        owner: Some(ServiceWorkerRunOwner::new(version_id, run)),
                        request_url: request_url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        request_body_bytes: None,
                        cors_preflight_request_headers: Vec::new(),
                        client_id: ServiceWorkerClientId::from_u64_for_test(0),
                        resulting_client_id: None,
                        destination: ServiceWorkerRequestDestination::Empty,
                        is_reload: false,
                        metadata: Default::default(),
                        request_mode: moli_fetch::RequestMode::Cors,
                        credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                        priority: None,
                        redirect_chain: Vec::new(),
                        redirect_count: 0,
                        request_cookie_report: None,
                        network_context: AsyncSubresourceNetworkContext {
                            frame_id: None,
                            document_url: request_url.clone(),
                            resource_type: crate::types::SubresourceResourceType::Fetch,
                            policy_context: Default::default(),
                        },
                        completion_tx,
                        request_client: test_request_client(&service),
                        resource_task_runner: test_resource_task_runner(),
                        cancel_handle: moli_fetch::FetchCancelHandle::new(),
                        navigation_preload_cancel_handle: None,
                        streaming_body_source_id: None,
                        direct_completion_tx: None,
                    },
                );
            }
            for registration_id in [deleted_registration_id, kept_registration_id] {
                state.pending_ready_jobs.push(ServiceWorkerReadyJob {
                    request_id: registration_id.as_u64(),
                    document_owner: crate::native_bridge::WindowDocumentOwner::for_test(1),
                    completion_tx: test_completion_sender(),
                    registration_id,
                });
                state
                    .notification_records
                    .push(ServiceWorkerNotificationRecord {
                        id: registration_id.as_u64(),
                        registration_id,
                        title: format!("notification-{}", registration_id.as_u64()),
                        tag: "tag".to_owned(),
                        metadata: ServiceWorkerNotificationMetadata::default(),
                        actions: Vec::new(),
                        data: V8StructuredClonePayload::default(),
                    });
                state.push_subscriptions.insert(
                    registration_id,
                    service_worker_push_subscription_snapshot(registration_id, true),
                );
            }
            state.sync_registrations.insert(
                (deleted_registration_id, "deleted-sync".to_owned()),
                ServiceWorkerSyncRegistrationRecord::default(),
            );
            state.sync_registrations.insert(
                (kept_registration_id, "kept-sync".to_owned()),
                ServiceWorkerSyncRegistrationRecord::default(),
            );
            state.periodic_sync_registrations.insert(
                (deleted_registration_id, "deleted-periodic".to_owned()),
                ServiceWorkerPeriodicSyncRegistrationRecord::new(10),
            );
            state.periodic_sync_registrations.insert(
                (kept_registration_id, "kept-periodic".to_owned()),
                ServiceWorkerPeriodicSyncRegistrationRecord::new(20),
            );
        }

        assert!(service.mark_registration_unregistered(&deleted_scope_url));

        {
            let state = service.inner.state.lock();
            assert!(!state.registrations.contains_key(&deleted_registration_id));
            assert!(!state.versions.contains_key(&deleted_version_id));
            assert!(state.registrations.contains_key(&kept_registration_id));
            assert!(state.versions.contains_key(&kept_version_id));
            assert!(!state.pending_fetch_jobs.contains_key(&deleted_event_id));
            assert!(state.pending_fetch_jobs.contains_key(&kept_event_id));
            assert!(
                state
                    .pending_ready_jobs
                    .iter()
                    .all(|job| job.registration_id != deleted_registration_id)
            );
            assert!(
                state
                    .pending_ready_jobs
                    .iter()
                    .any(|job| job.registration_id == kept_registration_id)
            );
            assert!(
                state
                    .notification_records
                    .iter()
                    .all(|record| record.registration_id != deleted_registration_id)
            );
            assert!(
                state
                    .notification_records
                    .iter()
                    .any(|record| record.registration_id == kept_registration_id)
            );
            assert!(
                !state
                    .push_subscriptions
                    .contains_key(&deleted_registration_id)
            );
            assert!(state.push_subscriptions.contains_key(&kept_registration_id));
            assert!(
                state
                    .sync_registrations
                    .keys()
                    .all(|(registration_id, _)| *registration_id != deleted_registration_id)
            );
            assert!(
                state
                    .periodic_sync_registrations
                    .keys()
                    .all(|(registration_id, _)| *registration_id != deleted_registration_id)
            );
            assert!(
                state
                    .sync_registrations
                    .contains_key(&(kept_registration_id, "kept-sync".to_owned()))
            );
            assert!(
                state
                    .periodic_sync_registrations
                    .contains_key(&(kept_registration_id, "kept-periodic".to_owned()))
            );
        }

        let completion = pop_async_subresource_completion(&mut deleted_fetch_queue);
        assert_eq!(completion.internal_id, deleted_event_id.as_u64());
        assert_eq!(
            completion.result.err().as_deref(),
            Some(SERVICE_WORKER_REGISTRATION_DELETED_FETCH_ERROR)
        );
        assert!(!kept_fetch_queue.has_ready_completion());
    }

    #[tokio::test]
    async fn stale_notification_sync_periodic_and_push_owner_requests_reject_worker_promises() {
        ensure_v8_for_test();
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let mut handle = crate::worker::spawn_worker_with_options(
            crate::worker::WorkerSpawnOptions::new_with_request_client(
                r#"
self.addEventListener("message", event => {
  event.waitUntil((async () => {
    async function expectTypeError(label, promise) {
      let caught = null;
      try {
        await promise;
      } catch (error) {
        caught = {
          name: error && error.name,
          message: error && error.message
        };
      }
      if (!caught || caught.name !== "TypeError" ||
          !String(caught.message).includes("stale")) {
        throw new Error(label + " did not reject with stale TypeError: " + JSON.stringify(caught));
      }
    }

    await expectTypeError("sync.register", self.registration.sync.register("stale-sync"));
    await expectTypeError("sync.getTags", self.registration.sync.getTags());
    await expectTypeError("showNotification", self.registration.showNotification("stale"));
    await expectTypeError("getNotifications", self.registration.getNotifications());
    await expectTypeError("periodicSync.register",
      self.registration.periodicSync.register("stale-periodic", { minInterval: 60000 }));
    await expectTypeError("periodicSync.getTags", self.registration.periodicSync.getTags());
    await expectTypeError("periodicSync.unregister",
      self.registration.periodicSync.unregister("stale-periodic"));
    await expectTypeError("pushManager.getSubscription",
      self.registration.pushManager.getSubscription());
    await expectTypeError("pushManager.subscribe",
      self.registration.pushManager.subscribe({ userVisibleOnly: true }));

    const subscription = await self.registration.pushManager.subscribe({ userVisibleOnly: true });
    if (!subscription || typeof subscription.unsubscribe !== "function") {
      throw new Error("pushManager.subscribe did not return a PushSubscription");
    }
    await expectTypeError("PushSubscription.unsubscribe", subscription.unsubscribe());
  })());
});
"#
                .to_owned(),
                script_url.to_string(),
                service.request_client(),
            )
            .with_global_kind(crate::worker::WorkerGlobalKind::Service {
                registration_id,
                version_id,
                scope_url: scope_url.clone(),
            })
            .with_network_policy(WorkerNetworkPolicy {
                permission_overrides: vec![
                    crate::protocol_types::PermissionOverrideRegistration {
                        permission: serde_json::Value::String("background-sync".to_owned()),
                        setting: "granted".to_owned(),
                        origin: None,
                        embedded_origin: None,
                    },
                    crate::protocol_types::PermissionOverrideRegistration {
                        permission: serde_json::Value::String(
                            "periodic-background-sync".to_owned(),
                        ),
                        setting: "granted".to_owned(),
                        origin: None,
                        embedded_origin: None,
                    },
                    crate::protocol_types::PermissionOverrideRegistration {
                        permission: serde_json::Value::String("notifications".to_owned()),
                        setting: "granted".to_owned(),
                        origin: None,
                        embedded_origin: None,
                    },
                ],
                ..WorkerNetworkPolicy::default()
            }),
        );
        handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
            event_id: ServiceWorkerEventId::from_u64_for_worker(61),
            owner: test_run_owner(version_id, &run),
            source_client_id: None,
            source_client_url: None,
            source_client_snapshot: None,
            source_worker: None,
            source_origin: String::new(),
            payload: serialize_test_string("stale-requests"),
            window_interaction_allowed: false,
        });
        let first_message = tokio::time::timeout(Duration::from_secs(5), handle.recv())
            .await
            .expect("timed out waiting for first service worker owner request")
            .expect("service worker parent channel closed before first request");
        let mut parent_rx = handle
            .take_receiver()
            .expect("service worker should expose parent receiver");
        let host = new_running_test_host_with_handle(version_id, &run, handle);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host: host.clone() },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        let stale_run = RendererServiceWorkerRunIdentity::fresh();
        let mut stale_rejection_count = 0;
        let mut push_subscribe_count = 0;
        let mut first_message = Some(first_message);
        loop {
            let message = match first_message.take() {
                Some(message) => message,
                None => tokio::time::timeout(Duration::from_secs(5), parent_rx.recv())
                    .await
                    .expect("timed out waiting for stale owner request settlement")
                    .expect("service worker parent channel closed"),
            };
            match message {
                crate::worker::WorkerToParentMessage::ServiceWorkerSyncRegistration(request) => {
                    service.finish_sync_registration_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerSyncGetTags(request) => {
                    service.finish_sync_get_tags_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerGetNotifications(request) => {
                    service.finish_get_notifications_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerShowNotification(request) => {
                    service.finish_show_notification_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(
                    request,
                ) => {
                    service.finish_periodic_sync_registration_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(request) => {
                    service.finish_periodic_sync_get_tags_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(
                    request,
                ) => {
                    service.finish_periodic_sync_unregistration_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerPushGetSubscription(request) => {
                    service.finish_push_get_subscription_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerPushSubscribe(request) => {
                    push_subscribe_count += 1;
                    if push_subscribe_count == 1 {
                        service.finish_push_subscribe_requested(
                            request,
                            stale_run.clone(),
                            host.clone(),
                        );
                        stale_rejection_count += 1;
                    } else {
                        service.finish_push_subscribe_requested(request, run.clone(), host.clone());
                    }
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerPushUnsubscribe(request) => {
                    service.finish_push_unsubscribe_requested(
                        request,
                        stale_run.clone(),
                        host.clone(),
                    );
                    stale_rejection_count += 1;
                }
                crate::worker::WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                    assert_eq!(
                        completion.event_id,
                        ServiceWorkerEventId::from_u64_for_worker(61)
                    );
                    assert_eq!(completion.result, Ok(()));
                    assert_eq!(stale_rejection_count, 10);
                    assert_eq!(push_subscribe_count, 2);
                    break;
                }
                crate::worker::WorkerToParentMessage::Error { message, .. } => {
                    panic!("unexpected service worker error: {message}");
                }
                crate::worker::WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
                other => panic!("unexpected worker message: {other:?}"),
            }
        }
        host.terminate_without_join();
    }

    #[test]
    fn pending_unregistration_waits_for_active_fetch_to_complete_before_delete() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(31);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: true,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.pending_fetch_jobs.insert(
                event_id,
                ServiceWorkerFetchJob {
                    internal_id: 131,
                    owner: Some(test_run_owner(version_id, &run)),
                    request_url: request_url.clone(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    request_body_bytes: None,
                    cors_preflight_request_headers: Vec::new(),
                    client_id: ServiceWorkerClientId::from_u64_for_test(0),
                    resulting_client_id: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    is_reload: false,
                    metadata: Default::default(),
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    redirect_chain: Vec::new(),
                    redirect_count: 0,
                    request_cookie_report: None,
                    network_context: AsyncSubresourceNetworkContext {
                        frame_id: None,
                        document_url,
                        resource_type: crate::types::SubresourceResourceType::Fetch,
                        policy_context: Default::default(),
                    },
                    completion_tx: completion_queue.sender(),
                    request_client: test_request_client(&service),
                    resource_task_runner: test_resource_task_runner(),
                    cancel_handle: moli_fetch::FetchCancelHandle::new(),
                    navigation_preload_cancel_handle: None,
                    streaming_body_source_id: None,
                    direct_completion_tx: None,
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.pending_unregistration_count, 1);
        assert_eq!(
            diagnostics.registrations[0].pending_clear_phase,
            Some("waiting-for-events")
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            owner: test_run_owner(version_id, &run),
            result: ServiceWorkerFetchResult::Failure("done".to_owned()),
        });

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 0);
        assert_eq!(diagnostics.version_count, 0);
        assert_eq!(diagnostics.pending_unregistration_count, 0);

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 131);
        assert_eq!(completion.result.err().as_deref(), Some("done"));
    }

    #[test]
    fn register_aborts_pending_unregistration_and_starts_update() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let document_url = url("https://example.test/app/page.html");
        let scope_url = url("https://example.test/app/");
        let second_script_url = url("https://example.test/app/worker-v2.js");
        let browser_context_runtime = service.browser_context_runtime();
        let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let active_state = insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            url("https://example.test/app/worker-v1.js"),
            scope_url.clone(),
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);

        assert!(service.mark_registration_unregistered(&scope_url));
        assert_eq!(
            service.matching_registration_for_client(&document_url),
            None
        );
        service.start_registration(
            second_script_url.clone(),
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            22,
            1,
            second_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert_eq!(diagnostics.starting_version_count, 1);
        assert_eq!(
            service
                .matching_registration_for_client(&document_url)
                .expect("register should abort pending unregister and restore visibility")
                .installing()
                .expect("different script should start installing immediately")
                .script_url(),
            &second_script_url
        );
        assert_eq!(
            service.matching_controller_for_fetch(
                &document_url,
                &url("https://example.test/app/data.json")
            ),
            Some(active_state)
        );
        assert!(!second_queue.has_ready_task());

        service.unregister_client(client_id);
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        assert_eq!(diagnostics.starting_version_count, 1);

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn same_options_register_aborts_pending_unregistration_fast_path() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let document_url = url("https://example.test/app/page.html");
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker-v1.js");
        let browser_context_runtime = service.browser_context_runtime();
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url.clone()],
        );

        assert!(service.mark_registration_unregistered(&scope_url));
        service.start_registration(
            script_url,
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            33,
            1,
            completion_queue.sender(),
        );
        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 33);
        let snapshot = completion
            .result
            .expect("same-options register should resolve restored registration");
        assert_eq!(snapshot.registration_id(), registration_id);
        assert!(snapshot.installing().is_none());
        assert_eq!(
            snapshot
                .active()
                .expect("active version should remain visible")
                .version_id(),
            active_version_id
        );
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert_eq!(diagnostics.starting_version_count, 0);
        assert!(
            service
                .matching_registration_for_client(&document_url)
                .is_some()
        );
    }

    #[test]
    fn unregister_queues_behind_installing_registration_job() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker-v1.js");
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_starting_version_with_register_job(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            11,
            completion_queue.sender(),
        );

        assert!(service.mark_registration_unregistered(&scope_url));
        assert!(!service.mark_registration_unregistered(&scope_url));

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        assert_eq!(diagnostics.queued_unregistration_job_count, 1);
        assert_eq!(
            service
                .matching_registration_for_client(&url("https://example.test/app/page.html"))
                .expect("installing registration should remain visible before queued unregister")
                .installing()
                .expect("installing worker should remain current")
                .script_url(),
            &script_url
        );

        let run = exact_version_run(&service, version_id);
        service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());
        assert!(!completion_queue.has_ready_task());
        assert_eq!(
            service
                .diagnostics_snapshot()
                .queued_unregistration_job_count,
            1
        );

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 11);
        assert!(completion.result.is_ok());
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 0);
        assert_eq!(diagnostics.version_count, 0);
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        assert_eq!(diagnostics.queued_unregistration_job_count, 0);
        assert_eq!(
            service.matching_registration_for_client(&url("https://example.test/app/page.html")),
            None
        );
    }

    #[test]
    fn queued_unregister_coalesces_repeated_callbacks() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker-v1.js");
        let mut register_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut first_unregister_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_unregister_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_starting_version_with_register_job(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            11,
            register_queue.sender(),
        );

        assert_eq!(
            service.start_unregistration(&scope_url, 21, 1, first_unregister_queue.sender()),
            ServiceWorkerUnregisterStart::Queued
        );
        assert_eq!(
            service.start_unregistration(&scope_url, 22, 2, second_unregister_queue.sender()),
            ServiceWorkerUnregisterStart::Queued
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.queued_unregistration_job_count, 1);
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        {
            let state = service.inner.state.lock();
            let registration_key = ServiceWorkerRegistrationKey::for_scope_url(&scope_url);
            assert_eq!(
                state
                    .job_coordinator
                    .queued_unregistration_phases(&registration_key),
                vec![ServiceWorkerUnregisterJobPhase::Initial]
            );
        }

        let run = exact_version_run(&service, version_id);
        service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());
        assert!(!register_queue.has_ready_task());
        assert!(!first_unregister_queue.has_ready_task());
        assert!(!second_unregister_queue.has_ready_task());

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let register_completion = pop_register_completion(&mut register_queue);
        assert_eq!(register_completion.request_id, 11);
        assert!(register_completion.result.is_ok());
        let first_completion = pop_unregister_completion(&mut first_unregister_queue);
        assert_eq!(first_completion.request_id, 21);
        assert_eq!(
            first_completion.document_owner,
            crate::native_bridge::WindowDocumentOwner::for_test(1)
        );
        assert!(first_completion.result);
        let second_completion = pop_unregister_completion(&mut second_unregister_queue);
        assert_eq!(second_completion.request_id, 22);
        assert_eq!(
            second_completion.document_owner,
            crate::native_bridge::WindowDocumentOwner::for_test(2)
        );
        assert!(second_completion.result);

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 0);
        assert_eq!(diagnostics.version_count, 0);
        assert_eq!(diagnostics.queued_unregistration_job_count, 0);
    }

    #[test]
    fn queued_unregister_preserves_fifo_before_later_register() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let first_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let first_script_url = url("https://example.test/app/worker-v1.js");
        let second_script_url = url("https://example.test/app/worker-v2.js");
        let document_url = url("https://example.test/app/page.html");
        let browser_context_runtime = service.browser_context_runtime();
        let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_starting_version_with_register_job(
            &service,
            registration_id,
            first_version_id,
            first_script_url.clone(),
            scope_url.clone(),
            11,
            first_queue.sender(),
        );

        assert!(service.mark_registration_unregistered(&scope_url));
        service.start_registration(
            second_script_url.clone(),
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            22,
            1,
            second_queue.sender(),
        );
        {
            let mut state = service.inner.state.lock();
            state.pending_ready_jobs.push(ServiceWorkerReadyJob {
                request_id: 33,
                document_owner: crate::native_bridge::WindowDocumentOwner::for_test(1),
                completion_tx: test_completion_sender(),
                registration_id,
            });
            state
                .notification_records
                .push(ServiceWorkerNotificationRecord {
                    id: 44,
                    registration_id,
                    title: "old notification".to_owned(),
                    tag: "old".to_owned(),
                    metadata: ServiceWorkerNotificationMetadata::default(),
                    actions: Vec::new(),
                    data: V8StructuredClonePayload::default(),
                });
            state.sync_registrations.insert(
                (registration_id, "old-sync".to_owned()),
                ServiceWorkerSyncRegistrationRecord::default(),
            );
            state.periodic_sync_registrations.insert(
                (registration_id, "old-periodic".to_owned()),
                ServiceWorkerPeriodicSyncRegistrationRecord::new(10),
            );
            state.push_subscriptions.insert(
                registration_id,
                service_worker_push_subscription_snapshot(registration_id, true),
            );
        }

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.queued_unregistration_job_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 1);
        assert_eq!(diagnostics.pending_unregistration_count, 0);

        let first_run = exact_version_run(&service, first_version_id);
        service.finish_worker_start_completed(
            first_version_id,
            first_run.clone(),
            first_script_url.to_string(),
        );
        assert!(!first_queue.has_ready_task());
        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(first_version_id, &first_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let first_completion = pop_register_completion(&mut first_queue);
        assert_eq!(first_completion.request_id, 11);
        assert!(first_completion.result.is_ok());
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.pending_unregistration_count, 0);
        assert_eq!(diagnostics.queued_unregistration_job_count, 0);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert_eq!(diagnostics.starting_version_count, 1);
        assert_eq!(
            service
                .matching_registration_for_client(&document_url)
                .expect("later register should recreate visible registration")
                .installing()
                .expect("later register should start installing")
                .script_url(),
            &second_script_url
        );
        {
            let state = service.inner.state.lock();
            assert!(
                state
                    .pending_ready_jobs
                    .iter()
                    .all(|job| job.registration_id != registration_id)
            );
            assert!(
                state
                    .notification_records
                    .iter()
                    .all(|record| record.registration_id != registration_id)
            );
            assert!(
                state
                    .sync_registrations
                    .keys()
                    .all(|(record_registration_id, _)| *record_registration_id != registration_id)
            );
            assert!(
                state
                    .periodic_sync_registrations
                    .keys()
                    .all(|(record_registration_id, _)| *record_registration_id != registration_id)
            );
            assert!(!state.push_subscriptions.contains_key(&registration_id));
        }

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn registration_lookup_uses_client_url_and_omits_pending_unregistration() {
        let service = new_service_worker_runtime_service();
        insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(1),
            ServiceWorkerVersionId(1),
            url("https://example.test/app/worker.js"),
            url("https://example.test/app/"),
            [url("https://example.test/app/page.html")],
        );
        let app = snapshot_for_registration(&service, ServiceWorkerRegistrationId(1));
        insert_registered_version(
            &service,
            ServiceWorkerRegistrationId(2),
            ServiceWorkerVersionId(2),
            url("https://example.test/other/worker.js"),
            url("https://example.test/other/"),
            [url("https://example.test/other/page.html")],
        );
        let other = snapshot_for_registration(&service, ServiceWorkerRegistrationId(2));

        assert_eq!(
            service.matching_registration_for_client(&url("https://example.test/app/page.html")),
            Some(app.clone())
        );
        assert_eq!(
            service.matching_registration_for_client(&url("https://example.test/other/page.html")),
            Some(other.clone())
        );
        let registrations = service.all_registrations(&url("https://example.test/app/page.html"));
        assert_eq!(registrations.len(), 2);
        assert!(registrations.contains(&app));
        assert!(registrations.contains(&other));

        assert!(service.mark_registration_unregistered(other.scope_url()));
        assert_eq!(
            service.matching_registration_for_client(&url("https://example.test/other/page.html")),
            None
        );
        assert_eq!(
            service.all_registrations(&url("https://example.test/app/page.html")),
            vec![app]
        );
    }

    #[test]
    fn start_registration_queues_same_scope_job_while_installing() {
        let service = new_service_worker_runtime_service();
        let browser_context_runtime = service.browser_context_runtime();
        let document_url = url("https://example.test/app/page.html");
        let mut first_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        service.start_registration(
            url("https://example.test/app/worker-v1.js"),
            url("https://example.test/app/"),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime.clone(),
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            1,
            1,
            first_completion_queue.sender(),
        );
        service.start_registration(
            url("https://example.test/app/worker-v2.js"),
            url("https://example.test/app/"),
            document_url,
            WorkerScriptKind::Module,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            2,
            1,
            second_completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 1);
        assert!(!first_completion_queue.has_ready_task());
        assert!(!second_completion_queue.has_ready_task());
        assert_eq!(
            service
                .matching_registration_for_client(&url("https://example.test/app/client.html"))
                .expect("registration should exist")
                .installing()
                .expect("first registration version should keep installing")
                .script_url(),
            &url("https://example.test/app/worker-v1.js")
        );

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn start_registration_coalesces_same_installing_register_callbacks() {
        let service = new_service_worker_runtime_service();
        let browser_context_runtime = service.browser_context_runtime();
        let document_url = url("https://example.test/app/page.html");
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let mut first_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        service.start_registration(
            script_url.clone(),
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime.clone(),
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            1,
            1,
            first_completion_queue.sender(),
        );
        service.start_registration(
            script_url.clone(),
            scope_url,
            document_url,
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            2,
            1,
            second_completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert!(!first_completion_queue.has_ready_task());
        assert!(!second_completion_queue.has_ready_task());

        let version_id = ServiceWorkerVersionId(1);
        let run = exact_version_run(&service, version_id);
        service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());

        assert!(!first_completion_queue.has_ready_task());
        assert!(!second_completion_queue.has_ready_task());
        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        assert!(first_completion_queue.has_ready_task());
        assert!(second_completion_queue.has_ready_task());
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn queued_register_coalesces_with_last_identical_job() {
        let service = new_service_worker_runtime_service();
        let browser_context_runtime = service.browser_context_runtime();
        let document_url = url("https://example.test/app/page.html");
        let scope_url = url("https://example.test/app/");
        let first_script_url = url("https://example.test/app/worker-v1.js");
        let second_script_url = url("https://example.test/app/worker-v2.js");
        let mut first_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut third_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        service.start_registration(
            first_script_url.clone(),
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime.clone(),
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            1,
            1,
            first_completion_queue.sender(),
        );
        service.start_registration(
            second_script_url.clone(),
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime.clone(),
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            2,
            1,
            second_completion_queue.sender(),
        );
        service.start_registration(
            second_script_url.clone(),
            scope_url,
            document_url,
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            3,
            1,
            third_completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 1);

        let first_version_id = ServiceWorkerVersionId(1);
        let first_run = exact_version_run(&service, first_version_id);
        service.finish_worker_start_completed(
            first_version_id,
            first_run.clone(),
            first_script_url.to_string(),
        );
        assert!(!first_completion_queue.has_ready_task());
        assert!(!second_completion_queue.has_ready_task());
        assert!(!third_completion_queue.has_ready_task());

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(first_version_id, &first_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        assert!(first_completion_queue.has_ready_task());
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert_eq!(diagnostics.starting_version_count, 1);

        let second_version_id = ServiceWorkerVersionId(2);
        let second_run = exact_version_run(&service, second_version_id);
        service.finish_worker_start_completed(
            second_version_id,
            second_run.clone(),
            second_script_url.to_string(),
        );
        assert!(!second_completion_queue.has_ready_task());
        assert!(!third_completion_queue.has_ready_task());
        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(second_version_id, &second_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });
        assert!(second_completion_queue.has_ready_task());
        assert!(third_completion_queue.has_ready_task());

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn installing_register_coalesces_late_callback_until_install_completes() {
        let service = new_service_worker_runtime_service();
        let browser_context_runtime = service.browser_context_runtime();
        let document_url = url("https://example.test/app/page.html");
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let mut first_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        service.start_registration(
            script_url.clone(),
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime.clone(),
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            1,
            1,
            first_completion_queue.sender(),
        );
        let version_id = ServiceWorkerVersionId(1);
        let run = exact_version_run(&service, version_id);
        service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());
        assert!(!first_completion_queue.has_ready_task());

        service.start_registration(
            script_url,
            scope_url,
            document_url,
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            2,
            1,
            second_completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert!(!first_completion_queue.has_ready_task());
        assert!(!second_completion_queue.has_ready_task());

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        assert!(first_completion_queue.has_ready_task());
        assert!(second_completion_queue.has_ready_task());

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn register_job_waits_for_install_completion_when_install_event_is_not_dispatched() {
        let service = new_service_worker_runtime_service();
        let browser_context_runtime = service.browser_context_runtime();
        let document_url = url("https://example.test/app/page.html");
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        service.start_registration(
            script_url.clone(),
            scope_url,
            document_url,
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            1,
            1,
            completion_queue.sender(),
        );
        let version_id = ServiceWorkerVersionId(1);
        let run = exact_version_run(&service, version_id);
        service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());

        assert!(!completion_queue.has_ready_task());
        {
            let state = service.inner.state.lock();
            let registration = state
                .registrations
                .get(&ServiceWorkerRegistrationId(1))
                .expect("registration should exist");
            let pending_job = registration
                .pending_register_jobs
                .get(&ServiceWorkerVersionId(1))
                .expect("register job should wait for install completion");
            assert_eq!(pending_job.phase(), ServiceWorkerRegisterJobPhase::Update);
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 1);
        let snapshot = completion
            .result
            .expect("register should resolve after install store succeeds");
        assert!(snapshot.installing().is_none());
        assert!(
            snapshot.waiting().is_some(),
            "installed worker should be visible as waiting"
        );
        {
            let state = service.inner.state.lock();
            let registration = state
                .registrations
                .get(&ServiceWorkerRegistrationId(1))
                .expect("registration should exist");
            assert!(
                !registration
                    .pending_register_jobs
                    .contains_key(&ServiceWorkerVersionId(1))
            );
        }
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn install_completion_starts_next_queued_register_job() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let first_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        insert_starting_version_with_register_job(
            &service,
            registration_id,
            first_version_id,
            url("https://example.test/app/worker-v1.js"),
            scope_url.clone(),
            11,
            first_queue.sender(),
        );
        push_queued_register_job(
            &service,
            registration_id,
            url("https://example.test/app/worker-v2.js"),
            scope_url.clone(),
            22,
            second_queue.sender(),
        );

        let first_run = exact_version_run(&service, first_version_id);
        service.finish_worker_start_completed(
            first_version_id,
            first_run.clone(),
            "https://example.test/app/worker-v1.js".to_owned(),
        );
        assert!(!first_queue.has_ready_task());
        assert!(!second_queue.has_ready_task());

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(first_version_id, &first_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        assert!(first_queue.has_ready_task());
        assert!(!second_queue.has_ready_task());
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert_eq!(diagnostics.starting_version_count, 1);
        let snapshot = snapshot_for_registration(&service, registration_id);
        assert_eq!(
            snapshot
                .installing()
                .expect("queued version should start installing")
                .script_url(),
            &url("https://example.test/app/worker-v2.js")
        );

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn start_failure_starts_next_queued_register_job() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let first_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        insert_starting_version_with_register_job(
            &service,
            registration_id,
            first_version_id,
            url("https://example.test/app/worker-v1.js"),
            scope_url.clone(),
            11,
            first_queue.sender(),
        );
        push_queued_register_job(
            &service,
            registration_id,
            url("https://example.test/app/worker-v2.js"),
            scope_url,
            22,
            second_queue.sender(),
        );

        let first_run = exact_version_run(&service, first_version_id);
        service.finish_worker_start_failed(
            first_version_id,
            first_run,
            ServiceWorkerVersionStartFailure::ScriptLoad {
                message: "first worker load failed".to_owned(),
            },
        );

        assert!(first_queue.has_ready_task());
        let first_completion = pop_register_completion(&mut first_queue);
        assert_eq!(first_completion.request_id, 11);
        assert!(first_completion.result.is_err());

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert_eq!(diagnostics.starting_version_count, 1);
        let snapshot = snapshot_for_registration(&service, registration_id);
        assert_eq!(
            snapshot
                .installing()
                .expect("queued version should start after failure")
                .script_url(),
            &url("https://example.test/app/worker-v2.js")
        );

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn start_registration_preserves_existing_active_version_for_same_scope() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let active_state = insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            url("https://example.test/app/worker-v1.js"),
            url("https://example.test/app/"),
            [url("https://example.test/app/page.html")],
        );
        let browser_context_runtime = service.browser_context_runtime();
        let completion_tx = test_completion_sender();
        service.start_registration(
            url("https://example.test/app/worker-v2.js"),
            url("https://example.test/app/"),
            url("https://example.test/app/page.html"),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            1,
            1,
            completion_tx,
        );

        let snapshot = service
            .matching_registration_for_client(&url("https://example.test/app/page.html"))
            .expect("registration should exist");
        assert_eq!(snapshot.registration_id(), registration_id);
        assert_eq!(
            snapshot
                .active()
                .expect("existing active version should be preserved")
                .version_id(),
            active_version_id
        );
        assert_eq!(
            service.matching_controller_for_document(&url("https://example.test/app/page.html")),
            Some(active_state)
        );

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn same_options_active_register_resolves_without_new_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let active_state = insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url.clone()],
        );
        let browser_context_runtime = service.browser_context_runtime();
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        service.start_registration(
            script_url.clone(),
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            42,
            1,
            completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.starting_version_count, 0);
        assert_eq!(diagnostics.queued_register_job_count, 0);

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 42);
        let snapshot = completion
            .result
            .expect("same-options register should resolve existing registration");
        assert_eq!(snapshot.registration_id(), registration_id);
        assert!(snapshot.installing().is_none());
        assert!(snapshot.waiting().is_none());
        assert_eq!(
            snapshot
                .active()
                .expect("active version should be visible")
                .version_id(),
            active_version_id
        );
        assert_eq!(
            service.matching_controller_for_document(&document_url),
            Some(active_state)
        );

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn same_options_register_uses_waiting_newest_worker_for_fast_path() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let waiting_script_url = url("https://example.test/app/worker-v2.js");
        let document_url = url("https://example.test/app/page.html");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            active_script_url,
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let registration = state
                .registrations
                .get_mut(&registration_id)
                .expect("registration should exist");
            registration.script_url = waiting_script_url.clone();
            registration.waiting_version_id = Some(waiting_version_id);
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installed,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }
        let browser_context_runtime = service.browser_context_runtime();
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        service.start_registration(
            waiting_script_url.clone(),
            scope_url,
            document_url,
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            43,
            1,
            completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.starting_version_count, 0);
        assert_eq!(diagnostics.queued_register_job_count, 0);

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 43);
        let snapshot = completion
            .result
            .expect("same-options register should resolve existing registration");
        assert_eq!(snapshot.registration_id(), registration_id);
        assert_eq!(
            snapshot
                .active()
                .expect("active version should remain visible")
                .version_id(),
            active_version_id
        );
        assert_eq!(
            snapshot
                .waiting()
                .expect("waiting newest version should remain visible")
                .version_id(),
            waiting_version_id
        );

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn different_update_via_cache_register_starts_new_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url.clone()],
        );
        let browser_context_runtime = service.browser_context_runtime();
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        service.start_registration(
            script_url.clone(),
            scope_url,
            document_url,
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::All,
            44,
            1,
            completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.starting_version_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert!(!completion_queue.has_ready_task());
        {
            let state = service.inner.state.lock();
            let registration = state
                .registrations
                .get(&registration_id)
                .expect("registration should exist");
            assert_eq!(
                registration.update_via_cache,
                ServiceWorkerUpdateViaCache::All
            );
            assert_eq!(registration.active_version_id, Some(active_version_id));
            assert!(registration.installing_version_id.is_some());
        }

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn queued_register_does_not_coalesce_different_update_via_cache() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let first_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let first_script_url = url("https://example.test/app/worker-v1.js");
        let second_script_url = url("https://example.test/app/worker-v2.js");
        let document_url = url("https://example.test/app/page.html");
        let browser_context_runtime = service.browser_context_runtime();
        let first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut third_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_starting_version_with_register_job(
            &service,
            registration_id,
            first_version_id,
            first_script_url,
            scope_url.clone(),
            11,
            first_queue.sender(),
        );

        service.start_registration(
            second_script_url.clone(),
            scope_url.clone(),
            document_url.clone(),
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime.clone(),
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::All,
            22,
            1,
            second_queue.sender(),
        );
        service.start_registration(
            second_script_url,
            scope_url,
            document_url,
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::None,
            33,
            1,
            third_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.queued_register_job_count, 2);
        assert!(!second_queue.has_ready_task());
        assert!(!third_queue.has_ready_task());

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn worker_start_failed_prunes_empty_registration_and_destroys_installing_target() {
        let service = new_service_worker_runtime_service();
        let (script_url, scope_url, registration_id, version_id) =
            insert_starting_version(&service);
        let created_run = {
            let mut state = service.inner.state.lock();
            state.record_target_created(registration_id, version_id, script_url, scope_url);
            exact_created_target_run(&service.take_target_output_events_for_test(), version_id)
        };
        service.enqueue_worker_start_failed(
            test_run_owner(version_id, &created_run),
            ServiceWorkerVersionStartFailure::ScriptLoad {
                message: "service worker script load failed: network request client not available"
                    .to_owned(),
            },
        );

        let pre_drain_diagnostics = service.diagnostics_snapshot();
        assert_eq!(pre_drain_diagnostics.pending_service_lane_event_count, 1);
        assert_eq!(pre_drain_diagnostics.starting_version_count, 1);
        assert_eq!(pre_drain_diagnostics.stopped_version_count, 0);

        assert_eq!(service.drain_service_lane(), 1);

        let state = service.inner.state.lock();
        assert_eq!(state.registrations.len(), 0);
        assert_eq!(state.versions.len(), 0);
        drop(state);

        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                    version_id: destroyed_version_id,
                    active_run: Some(active_run),
                } if *destroyed_version_id == version_id.as_u64()
                    && active_run == &created_run
            )),
            "failed installing target should destroy its exact starting run: {target_events:?}"
        );
        assert!(
            !target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Stopped {
                    version_id: stopped_version_id,
                    ..
                } if *stopped_version_id == version_id.as_u64()
            )),
            "failed installing target should not be retained as stopped: {target_events:?}"
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 0);
        assert_eq!(diagnostics.version_count, 0);
        assert_eq!(diagnostics.installing_version_count, 0);
        assert_eq!(diagnostics.redundant_version_count, 0);
        assert_eq!(diagnostics.stopped_version_count, 0);
        assert_eq!(diagnostics.failed_start_count, 0);
        assert_eq!(diagnostics.pending_service_lane_event_count, 0);
    }

    #[test]
    fn install_completion_queues_service_worker_target_installed_version_update() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/sw.js");
        let scope_url = url("https://example.test/app/");
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_starting_version_with_register_job(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            11,
            completion_queue.sender(),
        );
        {
            let mut state = service.inner.state.lock();
            state.record_target_created(
                registration_id,
                version_id,
                script_url.clone(),
                scope_url.clone(),
            );
            service.take_target_output_events_for_test();
        }

        let run = exact_version_run(&service, version_id);
        service.finish_worker_start_completed(version_id, run.clone(), script_url.to_string());
        service.take_target_output_events_for_test();

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });
        let install_events = service.take_target_output_events_for_test();
        assert!(
            install_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated {
                    version_id: updated_version_id,
                    status: crate::runtime::RendererServiceWorkerVersionStatus::Installed,
                } if *updated_version_id == version_id.as_u64()
            )),
            "install completion should refresh target status to installed: {install_events:?}"
        );

        assert!(
            pop_register_completion(&mut completion_queue)
                .result
                .is_ok()
        );
    }

    #[test]
    fn stale_worker_start_completion_does_not_advance_version_state() {
        let service = new_service_worker_runtime_service();
        let (script_url, _, _, version_id) = insert_starting_version(&service);

        service.enqueue_worker_start_completed(
            ServiceWorkerRunOwner::fresh(version_id),
            "https://example.test/app/sw.js".to_owned(),
            test_script_resource(&script_url),
            ServiceWorkerFetchHandlerType::NotSkippable,
        );
        assert_eq!(service.pending_service_lane_event_count(), 1);
        assert_eq!(service.drain_service_lane(), 1);

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.running_version_count, 0);
        assert_eq!(diagnostics.starting_version_count, 1);
        assert_eq!(diagnostics.versions[0].final_script_url, None);
        assert_eq!(diagnostics.versions[0].main_script_status, None);
    }

    #[test]
    fn worker_start_completion_records_main_script_resource_metadata() {
        let service = new_service_worker_runtime_service();
        let (script_url, _, _, version_id) = insert_starting_version(&service);

        let run = exact_version_run(&service, version_id);
        service.finish_worker_start_completed_with_script_resource(
            version_id,
            run,
            script_url.to_string(),
            Some(test_script_resource(&script_url)),
            ServiceWorkerFetchHandlerType::NotSkippable,
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.running_version_count, 1);
        assert_eq!(
            diagnostics.versions[0].final_script_url.as_deref(),
            Some(script_url.as_str())
        );
        assert_eq!(diagnostics.versions[0].main_script_status, Some(200));
        assert_eq!(diagnostics.versions[0].main_script_body_len, Some(3));
        assert_eq!(
            diagnostics.versions[0].main_script_body_sha256.as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(
            diagnostics.versions[0].main_script_mime_type.as_deref(),
            Some("text/javascript")
        );
        assert!(
            diagnostics.registrations[0]
                .last_update_check_time_ms
                .is_some(),
            "installing version script load should initialize last update check time"
        );
    }

    #[test]
    fn imported_script_loaded_records_resource_metadata() {
        let service = new_service_worker_runtime_service();
        let (_, _, registration_id, version_id) = insert_starting_version(&service);
        let import_url = url("https://example.test/app/dep.js");

        let run = exact_version_run(&service, version_id);
        service.finish_imported_script_loaded(
            registration_id,
            version_id,
            RendererServiceWorkerRunIdentity::fresh(),
            test_worker_script_resource(&import_url),
        );
        assert_eq!(
            service.diagnostics_snapshot().versions[0].imported_script_count,
            0
        );

        service.finish_imported_script_loaded(
            registration_id,
            version_id,
            run,
            test_worker_script_resource(&import_url),
        );

        let diagnostics = service.diagnostics_snapshot();
        let version = &diagnostics.versions[0];
        assert_eq!(version.imported_script_count, 1);
        assert_eq!(version.imported_scripts.len(), 1);
        let imported = &version.imported_scripts[0];
        assert_eq!(imported.request_url, import_url.as_str());
        assert_eq!(imported.final_url, import_url.as_str());
        assert_eq!(imported.status, 200);
        assert_eq!(imported.body_len, 3);
        assert_eq!(
            imported.body_sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(imported.mime_type.as_deref(), Some("text/javascript"));
    }

    #[test]
    fn resource_store_restores_imported_resources_for_update_check() {
        let resource_store = new_shared_service_worker_resource_store();
        let first_service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let dep_url = url("https://example.test/app/dep.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &first_service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = first_service.inner.state.lock();
            state
                .registrations
                .get_mut(&registration_id)
                .expect("registration should exist")
                .last_update_check_time_ms = Some(5);
            let version = state
                .versions
                .get_mut(&version_id)
                .expect("version should exist");
            version.main_script_resource = Some(test_script_resource(&script_url));
            version
                .imported_script_resources
                .insert(dep_url.to_string(), test_script_resource(&dep_url));
        }
        assert!(
            first_service
                .store_registration_resources_for_test(registration_id, version_id)
                .expect("stored registration resources should persist")
        );
        assert_eq!(resource_store.lock().registration_count(), 1);

        let second_service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let queued_job = test_queued_register_job(&second_service, script_url.clone(), scope_url);
        let (restored_registration_id, update_check_params) = {
            let mut state = second_service.inner.state.lock();
            let restored_registration_id = second_service
                .restore_stored_registration_for_queued_job_locked(&mut state, &queued_job)
                .expect("stored registration should restore");
            let update_check_params = match second_service
                .start_main_script_update_check_locked(
                    &mut state,
                    restored_registration_id,
                    queued_job,
                )
                .expect("restored active version should start update check")
            {
                ServiceWorkerMainScriptUpdateCheckStart::Start(update_check) => {
                    let (_, update_check_params) = *update_check;
                    update_check_params
                }
                ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger => {
                    panic!("update check should not wait for debugger by default")
                }
            };
            (restored_registration_id, update_check_params)
        };

        assert_eq!(
            update_check_params.newest_main_body_sha256,
            test_script_resource(&script_url).body_sha256
        );
        assert_eq!(update_check_params.imported_scripts.len(), 1);
        let imported_script = &update_check_params.imported_scripts[0];
        assert_eq!(imported_script.request_url, dep_url);
        assert_eq!(imported_script.final_url, dep_url);
        assert_eq!(
            imported_script.body_sha256,
            test_script_resource(&dep_url).body_sha256
        );

        let diagnostics = second_service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.installing_version_count, 1);
        assert_eq!(diagnostics.starting_version_count, 1);
        assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
        assert_eq!(
            diagnostics.registrations[0].last_update_check_time_ms,
            Some(5)
        );
        assert_eq!(
            diagnostics.registrations[0].active_version_id,
            Some(ServiceWorkerVersionId(1))
        );
        assert_eq!(diagnostics.registrations[0].id, restored_registration_id);
    }

    #[test]
    fn devtools_pause_on_start_defers_initial_install_launch_after_target_creation() {
        let service = new_service_worker_runtime_service();
        service.set_pause_new_workers_on_start_for_devtools(true);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        let browser_context_runtime = service.browser_context_runtime();
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        service.start_registration(
            script_url,
            scope_url,
            document_url,
            WorkerScriptKind::Classic,
            test_request_client(&service),
            WorkerNetworkPolicy::default(),
            browser_context_runtime,
            None,
            None,
            None,
            ServiceWorkerUpdateViaCache::Imports,
            42,
            1,
            completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.installing_version_count, 1);
        assert_eq!(diagnostics.starting_version_count, 1);
        assert_eq!(diagnostics.running_host_count, 0);
        assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
        assert!(!completion_queue.has_ready_task());

        let version_id = {
            let state = service.inner.state.lock();
            assert_eq!(state.pending_devtools_launches.len(), 1);
            let (version_id, launch) = state
                .pending_devtools_launches
                .iter()
                .next()
                .expect("install launch should wait for debugger");
            assert_eq!(launch.params.run_owner.version_id(), *version_id);
            assert_eq!(
                launch.params.run_owner.run_identity(),
                &state
                    .versions
                    .get(version_id)
                    .expect("starting version")
                    .run
            );
            assert!(launch.preloaded_script.is_none());
            *version_id
        };
        let target_events = service.take_target_output_events_for_test();
        let created_run = exact_created_target_run(&target_events, version_id);
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. }
                    if info.version_id == version_id.as_u64()
            )),
            "deferred install launch should still expose the installing target: {target_events:?}"
        );

        service.terminate_all_for_context_shutdown();
        let shutdown_events = service.take_target_output_events_for_test();
        assert!(
            shutdown_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                    version_id: destroyed_version_id,
                    active_run: Some(active_run),
                } if *destroyed_version_id == version_id.as_u64()
                    && active_run == &created_run
            )),
            "context shutdown should destroy the pending launch's exact run: {shutdown_events:?}"
        );
    }

    #[test]
    fn devtools_pause_on_start_marks_non_released_launch_for_evaluation_pause() {
        let service = new_service_worker_runtime_service();
        service.set_pause_new_workers_on_start_for_devtools(true);
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            Vec::<Url>::new(),
        );
        {
            let mut state = service.inner.state.lock();
            let version = state
                .versions
                .get_mut(&version_id)
                .expect("inserted version");
            version.lifecycle_state = ServiceWorkerVersionLifecycleState::Installing;
            version.should_pause_on_start_for_devtools = true;
        }
        let mut launch =
            test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

        service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

        assert!(
            launch.params.pause_evaluation_until_debugger,
            "launches that did not consume a debugger release should pause top-level evaluation"
        );
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn devtools_pause_on_start_pauses_attached_stopped_worker_restart() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            Vec::<Url>::new(),
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&version_id)
                .expect("inserted version")
                .should_pause_on_start_for_devtools = true;
        }
        service.set_devtools_attached_for_version(version_id, true);
        let mut launch =
            test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

        service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

        assert!(
            launch.params.pause_evaluation_until_debugger,
            "Chromium pauses stopped Service Worker restart only when the retained target is attached"
        );
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn devtools_pause_on_start_does_not_pause_unattached_stopped_worker_restart() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            Vec::<Url>::new(),
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&version_id)
                .expect("inserted version")
                .should_pause_on_start_for_devtools = true;
        }
        let mut launch =
            test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

        service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

        assert!(
            !launch.params.pause_evaluation_until_debugger,
            "stopped Service Worker restart should not pause when DevTools retained the target but no session is attached"
        );
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn devtools_pause_on_start_does_not_retroactively_pause_existing_stopped_worker() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            Vec::<Url>::new(),
        );
        service.set_pause_new_workers_on_start_for_devtools(true);
        let mut launch =
            test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

        service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

        assert!(
            !launch.params.pause_evaluation_until_debugger,
            "a DevTools wait policy enabled after target creation must not pause an existing target"
        );
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn devtools_released_launch_does_not_double_pause_evaluation() {
        let service = new_service_worker_runtime_service();
        service.set_pause_new_workers_on_start_for_devtools(true);
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            Vec::<Url>::new(),
        );
        let mut launch =
            test_queued_launch(&service, registration_id, version_id, script_url, scope_url);

        service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, true);

        assert!(
            !launch.params.pause_evaluation_until_debugger,
            "pending launch released by Runtime.runIfWaitingForDebugger must not wait a second time"
        );
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn devtools_run_if_prereleases_starting_worker_evaluation_before_host_running() {
        let service = new_service_worker_runtime_service();
        service.set_pause_new_workers_on_start_for_devtools(true);
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            Vec::<Url>::new(),
        );
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.run = run.clone();
            version.should_pause_on_start_for_devtools = true;
            version.running_state = ServiceWorkerVersionRunningState::Starting {
                host: new_loading_test_host(version_id, &run),
            };
        }
        service.set_devtools_attached_for_version(version_id, true);

        assert!(
            service.devtools_run_if_waiting_for_debugger(version_id),
            "runIf should be remembered when the worker host is starting but not yet routable"
        );
        {
            let state = service.inner.state.lock();
            assert!(
                state
                    .pending_devtools_evaluation_releases
                    .contains(&version_id)
            );
        }

        let mut launch =
            test_queued_launch(&service, registration_id, version_id, script_url, scope_url);
        service.apply_devtools_evaluation_pause_to_launch_if_needed(&mut launch, false);

        assert!(
            !launch.params.pause_evaluation_until_debugger,
            "pre-released starting workers must not pause evaluation again"
        );
        {
            let state = service.inner.state.lock();
            assert!(
                !state
                    .pending_devtools_evaluation_releases
                    .contains(&version_id)
            );
        }
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn devtools_pause_on_start_defers_main_script_update_check_after_target_creation() {
        let service = new_service_worker_runtime_service_with_resource_store(
            new_shared_service_worker_resource_store(),
            test_worker_context_runtime(),
        );
        service.set_pause_new_workers_on_start_for_devtools(true);
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        let queued_job = test_queued_register_job(&service, script_url, scope_url);

        let start = {
            let mut state = service.inner.state.lock();
            service
                .start_main_script_update_check_locked(&mut state, registration_id, queued_job)
                .expect("active version should be eligible for update check")
        };

        assert!(matches!(
            start,
            ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger
        ));
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
        assert_eq!(diagnostics.installing_version_count, 1);
        assert_eq!(diagnostics.starting_version_count, 1);
        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. }
                    if info.version_id != active_version_id.as_u64()
            )),
            "deferred update check should still expose the precreated installing target: {target_events:?}"
        );
    }

    #[test]
    fn devtools_related_pause_on_start_defers_matching_update_check() {
        let service = new_service_worker_runtime_service_with_resource_store(
            new_shared_service_worker_resource_store(),
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        service.set_related_pause_on_start_policies_for_devtools(vec![(
            registration_id.as_u64(),
            active_version_id.as_u64(),
            script_url.to_string(),
            scope_url.to_string(),
        )]);
        let queued_job = test_queued_register_job(&service, script_url, scope_url);

        let start = {
            let mut state = service.inner.state.lock();
            service
                .start_main_script_update_check_locked(&mut state, registration_id, queued_job)
                .expect("active version should be eligible for update check")
        };

        assert!(matches!(
            start,
            ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger
        ));
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
        assert_eq!(diagnostics.installing_version_count, 1);
        let state = service.inner.state.lock();
        assert!(
            state.versions.iter().any(|(version_id, version)| {
                *version_id != active_version_id && version.should_pause_on_start_for_devtools
            }),
            "matching related policy should set the sticky version pause flag"
        );
    }

    #[test]
    fn forced_update_check_uses_validate_cache_and_skips_script_comparison() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let registration = state.registrations.get_mut(&registration_id).unwrap();
            registration.update_via_cache = ServiceWorkerUpdateViaCache::All;
            let version = state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist");
            version.main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut queued_job = test_queued_register_job(&service, script_url, scope_url);
        queued_job.update_via_cache = ServiceWorkerUpdateViaCache::All;
        queued_job.force_bypass_cache = true;
        queued_job.skip_script_comparison = true;
        queued_job.skip_waiting_after_install = true;

        let update_check_params = {
            let mut state = service.inner.state.lock();
            match service
                .start_main_script_update_check_locked(&mut state, registration_id, queued_job)
                .expect("force update should start update check")
            {
                ServiceWorkerMainScriptUpdateCheckStart::Start(update_check) => {
                    let (_, update_check_params) = *update_check;
                    update_check_params
                }
                ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger => {
                    panic!("force update should not wait for debugger by default")
                }
            }
        };

        assert_eq!(
            update_check_params.main_script.cache_mode,
            moli_fetch::RequestCacheMode::Validate
        );
        assert_eq!(
            update_check_params.imported_script_cache_mode,
            moli_fetch::RequestCacheMode::Validate
        );
        assert!(update_check_params.skip_script_comparison);
    }

    #[test]
    fn force_update_page_load_install_reports_devtools_warning() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut queued_job = test_queued_register_job(&service, script_url.clone(), scope_url);
        queued_job.force_bypass_cache = true;
        queued_job.skip_script_comparison = true;
        queued_job.skip_waiting_after_install = true;
        queued_job.force_update_page_load_waiter_ids = vec![1];

        let new_version_id = {
            let (force_update_tx, _force_update_rx) = tokio::sync::oneshot::channel();
            let mut state = service.inner.state.lock();
            state.insert_force_update_page_load_waiter(1, force_update_tx);
            match service
                .start_main_script_update_check_locked(&mut state, registration_id, queued_job)
                .expect("force update should start update check")
            {
                ServiceWorkerMainScriptUpdateCheckStart::Start(_) => {}
                ServiceWorkerMainScriptUpdateCheckStart::WaitForDebugger => {
                    panic!("force update should not wait for debugger by default")
                }
            }
            state
                .registrations
                .get(&registration_id)
                .expect("registration should exist")
                .installing_version_id
                .expect("update check should precreate an installing version")
        };

        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. }
                    if info.version_id == new_version_id.as_u64()
            )),
            "force-update update check should precreate a Service Worker target: {target_events:?}"
        );
        assert!(
            target_events.iter().all(|event| !matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Console { .. }
            )),
            "force-update warning should wait until the install launch starts: {target_events:?}"
        );

        service.finish_main_script_update_check_completed(
            registration_id,
            Ok(ServiceWorkerScriptUpdateCheckResult {
                main_script: test_loaded_script(&script_url, "self.skipWaiting();"),
                change: ServiceWorkerScriptUpdateCheckChange::ScriptComparisonSkipped,
            }),
        );

        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Console {
                    version_id,
                    message,
                    ..
                } if *version_id == new_version_id.as_u64()
                    && message.message == SERVICE_WORKER_FORCE_UPDATE_DEVTOOLS_CONSOLE_MESSAGE
                    && message.args.is_empty()
                    && message.stack.is_none()
            )),
            "force-update install should report Chromium DevTools warning: {target_events:?}"
        );
        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn resource_store_restores_registration_at_service_startup_for_client_observation() {
        let resource_store = new_shared_service_worker_resource_store();
        let first_service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        insert_registered_version(
            &first_service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = first_service.inner.state.lock();
            state
                .versions
                .get_mut(&version_id)
                .expect("version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        assert!(
            first_service
                .store_registration_resources_for_test(registration_id, version_id)
                .expect("stored registration resources should persist")
        );

        let second_service = new_service_worker_runtime_service_with_resource_store(
            resource_store,
            test_worker_context_runtime(),
        );
        assert_eq!(second_service.diagnostics_snapshot().registration_count, 1);

        let restored = second_service
            .matching_registration_for_client(&document_url)
            .expect("matching client query should see startup-restored registration");
        assert_eq!(restored.scope_url(), &scope_url);
        assert_eq!(second_service.diagnostics_snapshot().registration_count, 1);

        let mut ready_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        assert!(second_service.watch_ready_registration(
            document_url.clone(),
            5,
            1,
            ready_queue.sender(),
        ));
        assert!(ready_queue.has_ready_task());

        let client_id = second_service.register_client(document_url, 1, test_completion_sender());
        let control = second_service
            .matching_controller_for_client(client_id)
            .expect("new client should select restored active registration as controller");
        assert_eq!(control.scope_url(), &scope_url);
        assert_eq!(
            second_service
                .all_registrations(&url("https://example.test/app/page.html"))
                .len(),
            1
        );
    }

    #[test]
    fn resource_store_restores_no_fetch_handler_metadata_for_controlled_fetch() {
        let resource_store = new_shared_service_worker_resource_store();
        let first_service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        insert_registered_version(
            &first_service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = first_service.inner.state.lock();
            let version = state
                .versions
                .get_mut(&version_id)
                .expect("version should exist");
            version.main_script_resource = Some(test_script_resource(&script_url));
            version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::DoesNotExist;
            version.fetch_handler_type = ServiceWorkerFetchHandlerType::NoHandler;
        }
        assert!(
            first_service
                .store_registration_resources_for_test(registration_id, version_id)
                .expect("stored no-handler registration should persist")
        );

        let second_service = new_service_worker_runtime_service_with_resource_store(
            resource_store,
            test_worker_context_runtime(),
        );
        let client_id =
            second_service.register_client(document_url.clone(), 1, test_completion_sender());
        let restored_version_id = {
            let state = second_service.inner.state.lock();
            let restored_registration = state
                .registrations
                .values()
                .find(|registration| registration.scope_url == scope_url)
                .expect("stored registration should restore at startup");
            assert!(
                restored_registration
                    .controlled_client_ids
                    .contains(&client_id),
                "new client should select the restored active controller"
            );
            let restored_version_id = restored_registration
                .active_version_id
                .expect("restored registration should be active");
            let version = state
                .versions
                .get(&restored_version_id)
                .expect("restored version should exist");
            assert_eq!(
                version.fetch_handler_existence,
                ServiceWorkerFetchHandlerExistence::DoesNotExist
            );
            assert_eq!(
                version.fetch_handler_type,
                ServiceWorkerFetchHandlerType::NoHandler
            );
            restored_version_id
        };

        let mut completion_queue = async_subresource_completion_queue();
        let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
        assert!(
            second_service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
                internal_id: 89,
                request: ServiceWorkerFetchRequest {
                    client_id,
                    resulting_client_id: None,
                    url: request_url,
                    method: "GET".to_owned(),
                    headers: Vec::new(),
                    body: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    is_reload: false,
                    metadata: Default::default(),
                },
                request_body_text: None,
                cors_preflight_request_headers: Vec::new(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    document_url,
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&second_service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                direct_completion_tx: Some(direct_completion_tx),
            })
        );

        {
            let state = second_service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state
                .versions
                .get(&restored_version_id)
                .expect("restored version should still exist");
            assert_eq!(version.in_flight_event_count, 0);
            assert!(matches!(
                version.running_state,
                ServiceWorkerVersionRunningState::Stopped
            ));
        }

        expect_direct_fetch_fallback(&mut direct_completion_rx);
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn resource_store_lazily_restores_registration_added_after_startup() {
        let resource_store = new_shared_service_worker_resource_store();
        let observer_service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        assert_eq!(
            observer_service.diagnostics_snapshot().registration_count,
            0
        );
        {
            let state = observer_service.inner.state.lock();
            assert_eq!(state.stored_registration_cache_revision, Some(0));
            assert_eq!(state.stored_registration_cache.len(), 0);
        }

        let writer_service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        insert_registered_version(
            &writer_service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = writer_service.inner.state.lock();
            state
                .versions
                .get_mut(&version_id)
                .expect("version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        assert!(
            writer_service
                .store_registration_resources_for_test(registration_id, version_id)
                .expect("stored registration resources should persist")
        );
        let store_revision_after_write = resource_store.lock().revision();
        assert_eq!(store_revision_after_write, 1);
        assert_eq!(
            observer_service.diagnostics_snapshot().registration_count,
            0
        );
        {
            let state = observer_service.inner.state.lock();
            assert_eq!(state.stored_registration_cache_revision, Some(0));
            assert_eq!(state.stored_registration_cache.len(), 0);
        }

        let restored = observer_service
            .matching_registration_for_client(&document_url)
            .expect("matching client query should lazily restore later stored registration");
        assert_eq!(restored.scope_url(), &scope_url);
        assert_eq!(
            observer_service.diagnostics_snapshot().registration_count,
            1
        );
        {
            let state = observer_service.inner.state.lock();
            assert_eq!(
                state.stored_registration_cache_revision,
                Some(store_revision_after_write)
            );
            assert_eq!(state.stored_registration_cache.len(), 1);
        }
        assert!(
            observer_service
                .matching_registration_for_client(&document_url)
                .is_some()
        );
        {
            let state = observer_service.inner.state.lock();
            assert_eq!(
                state.stored_registration_cache_revision,
                Some(store_revision_after_write)
            );
            assert_eq!(state.stored_registration_cache.len(), 1);
        }
    }

    #[test]
    fn startup_restored_registrations_filter_client_observation_by_storage_key() {
        let resource_store = new_shared_service_worker_resource_store();
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        let script_url = url("https://example.test/app/worker.js");
        let wrong_script_url = url("https://example.test/app/wrong-worker.js");
        resource_store
            .lock()
            .store_registration(ServiceWorkerStoredRegistration {
                storage_key: "https://other-top-level.example".to_string(),
                scope_url: scope_url.clone(),
                script_url: wrong_script_url.clone(),
                script_kind: WorkerScriptKind::Classic,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                last_update_check_time_ms: Some(2),
                main_script_resource: test_script_resource(&wrong_script_url),
                imported_script_resources: std::collections::BTreeMap::new(),
            })
            .expect("wrong-key registration should store");
        resource_store
            .lock()
            .store_registration(ServiceWorkerStoredRegistration {
                storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&document_url),
                scope_url: scope_url.clone(),
                script_url: script_url.clone(),
                script_kind: WorkerScriptKind::Classic,
                update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
                fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                last_update_check_time_ms: Some(3),
                main_script_resource: test_script_resource(&script_url),
                imported_script_resources: std::collections::BTreeMap::new(),
            })
            .expect("matching-key registration should store");

        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store,
            test_worker_context_runtime(),
        );
        assert_eq!(service.diagnostics_snapshot().registration_count, 2);

        let registrations = service.all_registrations(&document_url);
        assert_eq!(registrations.len(), 1);
        assert_eq!(
            registrations[0]
                .active()
                .expect("matching-key registration should be active")
                .script_url(),
            &script_url
        );

        let matching = service
            .matching_registration_for_client(&document_url)
            .expect("matching registration should ignore different storage keys");
        assert_eq!(
            matching
                .active()
                .expect("matching registration should be active")
                .script_url(),
            &script_url
        );

        let client_id = service.register_client(document_url, 1, test_completion_sender());
        let control = service
            .matching_controller_for_client(client_id)
            .expect("controller selection should ignore different storage keys");
        assert_eq!(control.script_url(), &script_url);
    }

    #[test]
    fn client_observation_uses_explicit_partitioned_storage_key() {
        let resource_store = new_shared_service_worker_resource_store();
        let scope_url = url("https://cdn.example.test/app/");
        let document_url = url("https://cdn.example.test/app/page.html");
        let first_script_url = url("https://cdn.example.test/app/first-worker.js");
        let second_script_url = url("https://cdn.example.test/app/second-worker.js");
        let first_storage_key = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
            &document_url,
            moli_storage_key::site_for_url(&url("https://first-top.test/page.html")),
            None,
        )
        .serialized_storage_key();
        let second_storage_key = moli_storage_key::MoliStorageKey::from_url_and_top_level_site(
            &document_url,
            moli_storage_key::site_for_url(&url("https://second-top.test/page.html")),
            None,
        )
        .serialized_storage_key();
        assert_ne!(first_storage_key, second_storage_key);

        for (storage_key, script_url, check_time) in [
            (first_storage_key.clone(), first_script_url.clone(), 11),
            (second_storage_key.clone(), second_script_url.clone(), 12),
        ] {
            resource_store
                .lock()
                .store_registration(ServiceWorkerStoredRegistration {
                    storage_key,
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    script_kind: WorkerScriptKind::Classic,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    last_update_check_time_ms: Some(check_time),
                    main_script_resource: test_script_resource(&script_url),
                    imported_script_resources: std::collections::BTreeMap::new(),
                })
                .expect("partitioned registration should store");
        }

        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store,
            test_worker_context_runtime(),
        );
        let first_match = service
            .matching_registration_for_client_with_storage_key(&document_url, &first_storage_key)
            .expect("first storage key should match first registration");
        assert_eq!(
            first_match
                .active()
                .expect("first registration should be active")
                .script_url(),
            &first_script_url
        );
        let second_match = service
            .matching_registration_for_client_with_storage_key(&document_url, &second_storage_key)
            .expect("second storage key should match second registration");
        assert_eq!(
            second_match
                .active()
                .expect("second registration should be active")
                .script_url(),
            &second_script_url
        );

        assert_eq!(
            service
                .all_registrations_with_storage_key(&document_url, &first_storage_key)
                .len(),
            1
        );
        let client_id = service.register_client_with_storage_key(
            document_url,
            second_storage_key,
            ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(1)),
            test_completion_sender(),
        );
        let control = service
            .matching_controller_for_client(client_id)
            .expect("partitioned client should select matching-key controller");
        assert_eq!(control.script_url(), &second_script_url);
    }

    #[test]
    fn stored_registration_resources_use_serialized_storage_key() {
        let resource_store = new_shared_service_worker_resource_store();
        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&version_id)
                .expect("version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }

        assert!(
            service
                .store_registration_resources_for_test(registration_id, version_id)
                .expect("stored registration resources should persist")
        );
        let registrations = resource_store.lock().registrations();
        assert_eq!(registrations.len(), 1);
        assert_eq!(
            registrations[0].storage_key,
            "storage-key:v1;origin=https://example.test;top-level-site=https://example.test"
        );
        assert_ne!(registrations[0].storage_key, "https://example.test");
        assert_eq!(
            registrations[0].storage_key,
            ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url)
        );
    }

    #[test]
    fn resource_store_deletes_registration_on_unregistration_clear() {
        let resource_store = new_shared_service_worker_resource_store();
        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&version_id)
                .expect("version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        assert!(
            service
                .store_registration_resources_for_test(registration_id, version_id)
                .expect("stored registration resources should persist")
        );
        assert_eq!(resource_store.lock().registration_count(), 1);

        assert!(service.mark_registration_unregistered(&scope_url));

        assert_eq!(resource_store.lock().registration_count(), 0);
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 0);
    }

    #[test]
    fn resource_store_transient_failure_retries_install_store() {
        let temp_store = TempJsonStorePath::new("install-transient");
        let resource_store =
            crate::new_shared_json_service_worker_resource_store(temp_store.store_path())
                .expect("json resource store should open");
        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let (registration_id, version_id) = insert_running_installing_version(&service);
        {
            let mut state = service.inner.state.lock();
            let script_url = state
                .versions
                .get(&version_id)
                .expect("installing version should exist")
                .script_url
                .clone();
            state
                .versions
                .get_mut(&version_id)
                .expect("installing version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        resource_store.lock().fail_next_persist_attempts_for_test(1);
        let run = exact_version_run(&service, version_id);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        assert_eq!(resource_store.lock().registration_count(), 1);
        let state = service.inner.state.lock();
        let registration = state
            .registrations
            .get(&registration_id)
            .expect("registration should survive transient store failure");
        assert_eq!(registration.installing_version_id, None);
        assert_eq!(registration.waiting_version_id, Some(version_id));
        let version = state
            .versions
            .get(&version_id)
            .expect("version should survive transient store failure");
        assert_eq!(
            version.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Installed
        );
    }

    #[test]
    fn resource_store_failure_aborts_identical_update_check_register_job() {
        let failing_store = FailingJsonStorePath::new("update-check");
        let resource_store =
            crate::new_shared_json_service_worker_resource_store(failing_store.store_path())
                .expect("failing json resource store should open before first write");
        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store,
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_pending_main_script_update_check(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url,
            77,
            completion_queue.sender(),
        );

        service.finish_main_script_update_check_completed(
            registration_id,
            Ok(test_update_check_result(
                &script_url,
                "self.skipWaiting();",
                false,
            )),
        );

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 77);
        let error = completion
            .result
            .expect_err("store failure should reject update check register job");
        assert_eq!(
            error.kind,
            crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Abort
        );
        assert!(
            error
                .message
                .contains("failed to store Service Worker registration resources")
        );
        let diagnostics = service.diagnostics_snapshot();
        let update_check = diagnostics.registrations[0]
            .last_main_script_update_check
            .as_ref()
            .expect("store failure should record update-check diagnostics");
        assert_eq!(update_check.result, "store-failed");
        assert_eq!(update_check.failure_status, Some("abort"));
    }

    #[test]
    fn resource_store_failure_deletes_initial_install_registration() {
        let failing_store = FailingJsonStorePath::new("install");
        let resource_store =
            crate::new_shared_json_service_worker_resource_store(failing_store.store_path())
                .expect("failing json resource store should open before first write");
        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store.clone(),
            test_worker_context_runtime(),
        );
        let (script_url, _, registration_id, version_id) = insert_starting_version(&service);
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&version_id)
                .expect("installing version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        let run = exact_version_run(&service, version_id);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 0);
        assert_eq!(diagnostics.version_count, 0);
        assert_eq!(resource_store.lock().registration_count(), 0);
        let state = service.inner.state.lock();
        assert!(!state.registrations.contains_key(&registration_id));
        assert!(!state.versions.contains_key(&version_id));
    }

    #[test]
    fn resource_store_failure_starts_next_queued_register_job() {
        let failing_store = FailingJsonStorePath::new("install-queued");
        let resource_store =
            crate::new_shared_json_service_worker_resource_store(failing_store.store_path())
                .expect("failing json resource store should open before first write");
        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store,
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let first_version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let first_script_url = url("https://example.test/app/worker-v1.js");
        let second_script_url = url("https://example.test/app/worker-v2.js");
        let mut first_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut second_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();

        insert_starting_version_with_register_job(
            &service,
            registration_id,
            first_version_id,
            first_script_url.clone(),
            scope_url.clone(),
            11,
            first_queue.sender(),
        );
        push_queued_register_job(
            &service,
            registration_id,
            second_script_url.clone(),
            scope_url,
            22,
            second_queue.sender(),
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&first_version_id)
                .expect("first installing version should exist")
                .main_script_resource = Some(test_script_resource(&first_script_url));
        }
        let first_run = exact_version_run(&service, first_version_id);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(first_version_id, &first_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let first_completion = pop_register_completion(&mut first_queue);
        assert_eq!(first_completion.request_id, 11);
        let error = first_completion
            .result
            .expect_err("store failure should reject the first register job");
        assert_eq!(
            error.kind,
            crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Abort
        );
        assert!(!second_queue.has_ready_task());

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 0);
        assert_eq!(diagnostics.starting_version_count, 1);
        let snapshot = snapshot_for_registration(&service, registration_id);
        assert_eq!(
            snapshot
                .installing()
                .expect("queued version should start after failed store")
                .script_url(),
            &second_script_url
        );

        service.terminate_all_for_context_shutdown();
    }

    #[test]
    fn resource_store_failure_preserves_existing_active_registration() {
        let failing_store = FailingJsonStorePath::new("install-update");
        let resource_store =
            crate::new_shared_json_service_worker_resource_store(failing_store.store_path())
                .expect("failing json resource store should open before first write");
        let service = new_service_worker_runtime_service_with_resource_store(
            resource_store,
            test_worker_context_runtime(),
        );
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let installing_version_id = ServiceWorkerVersionId(2);
        let old_script_url = url("https://example.test/app/worker-v1.js");
        let new_script_url = url("https://example.test/app/worker-v2.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        let active_state = insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            old_script_url.clone(),
            scope_url.clone(),
            [document_url.clone()],
        );
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_starting_version_with_register_job(
            &service,
            registration_id,
            installing_version_id,
            new_script_url.clone(),
            scope_url,
            42,
            completion_queue.sender(),
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&installing_version_id)
                .expect("installing update version should exist")
                .main_script_resource = Some(test_script_resource(&new_script_url));
        }
        let installing_run = exact_version_run(&service, installing_version_id);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(installing_version_id, &installing_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 42);
        assert!(
            completion
                .result
                .expect_err("store failure should reject installing update")
                .message
                .contains("failed to store Service Worker registration resources")
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.installing_version_count, 0);
        assert_eq!(diagnostics.redundant_version_count, 0);
        assert_eq!(
            diagnostics.registrations[0].active_version_id,
            Some(active_version_id)
        );
        assert_eq!(diagnostics.registrations[0].waiting_version_id, None);
        assert_eq!(
            service.matching_controller_for_document(&document_url),
            Some(active_state)
        );
        let state = service.inner.state.lock();
        assert!(state.versions.contains_key(&active_version_id));
        assert!(!state.versions.contains_key(&installing_version_id));
    }

    #[test]
    fn identical_main_script_update_check_destroys_precreated_installing_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url.clone()],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .registrations
                .get_mut(&registration_id)
                .expect("registration should exist")
                .last_update_check_time_ms = Some(1);
            state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let new_version_id = insert_pending_main_script_update_check(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url,
            42,
            completion_queue.sender(),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.installing_version_count, 1);
        assert_eq!(diagnostics.starting_version_count, 1);
        assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
        let target_events = service.take_target_output_events_for_test();
        let created_run = exact_created_target_run(&target_events, new_version_id);
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Created { info, .. }
                    if info.version_id == new_version_id.as_u64()
                        && info.registration_id == registration_id.as_u64()
                        && info.script_url == script_url.as_str()
            )),
            "pending update check should create a Service Worker target: {target_events:?}"
        );
        assert!(!completion_queue.has_ready_task());

        service.finish_main_script_update_check_completed(
            registration_id,
            Ok(test_update_check_result(&script_url, "abc", false)),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.installing_version_count, 0);
        assert_eq!(diagnostics.starting_version_count, 0);
        assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                    version_id,
                    active_run: Some(active_run),
                } if *version_id == new_version_id.as_u64()
                    && active_run == &created_run
            )),
            "identical update check should destroy the precreated target's exact run: {target_events:?}"
        );
        let update_check = diagnostics.registrations[0]
            .last_main_script_update_check
            .as_ref()
            .expect("identical update check should be diagnosed");
        assert_eq!(update_check.script_url, script_url.as_str());
        assert_eq!(update_check.newest_version_id, active_version_id);
        assert_eq!(update_check.result, "identical");
        assert_eq!(update_check.failure_status, None);
        assert_eq!(update_check.message, None);
        assert_eq!(update_check.imported_script_url, None);
        assert!(
            diagnostics.registrations[0]
                .last_update_check_time_ms
                .is_some_and(|value| value > 1),
            "successful identical update check should bump last update check time"
        );

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 42);
        let snapshot = completion
            .result
            .expect("identical update check should resolve existing registration");
        assert!(snapshot.installing().is_none());
        assert_eq!(
            snapshot
                .active()
                .expect("active version should remain")
                .version_id(),
            active_version_id
        );
    }

    #[test]
    fn imported_script_update_check_change_creates_installing_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .registrations
                .get_mut(&registration_id)
                .expect("registration should exist")
                .last_update_check_time_ms = Some(1);
            state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let new_version_id = insert_pending_main_script_update_check(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url,
            42,
            completion_queue.sender(),
        );
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.installing_version_count, 1);
        assert_eq!(diagnostics.starting_version_count, 1);
        assert_eq!(
            diagnostics.registrations[0].installing_version_id,
            Some(new_version_id)
        );

        service.finish_main_script_update_check_completed(
            registration_id,
            Ok(test_update_check_result(&script_url, "abc", true)),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 2);
        assert_eq!(diagnostics.installing_version_count, 1);
        assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
        let update_check = diagnostics.registrations[0]
            .last_main_script_update_check
            .as_ref()
            .expect("imported script change should be diagnosed");
        assert_eq!(update_check.result, "imported-script-different");
        assert_eq!(
            update_check.imported_script_url.as_deref(),
            Some("https://example.test/app/dep.js")
        );
        assert!(
            diagnostics.registrations[0]
                .last_update_check_time_ms
                .is_some_and(|value| value > 1),
            "successful changed update check should bump last update check time"
        );
        assert!(!completion_queue.has_ready_task());
    }

    #[test]
    fn failed_main_script_update_check_records_diagnostics_and_rejects_register_job() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_pending_main_script_update_check(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url,
            42,
            completion_queue.sender(),
        );

        service.finish_main_script_update_check_completed(
            registration_id,
            Err(ServiceWorkerScriptUpdateCheckFailure::script_load(
                "script fetch failed".to_owned(),
            )),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.installing_version_count, 0);
        assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
        assert_eq!(
            diagnostics.registrations[0].active_version_id,
            Some(active_version_id)
        );
        let update_check = diagnostics.registrations[0]
            .last_main_script_update_check
            .as_ref()
            .expect("failed update check should be diagnosed");
        assert_eq!(update_check.result, "failed");
        assert_eq!(update_check.failure_status, Some("script-load-failed"));
        assert_eq!(update_check.message.as_deref(), Some("script fetch failed"));
        assert_eq!(diagnostics.registrations[0].last_update_check_time_ms, None);

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 42);
        let error = completion
            .result
            .expect_err("script load update check should reject");
        assert_eq!(
            error.kind,
            crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Type
        );
        assert_eq!(error.message, "script fetch failed");
    }

    #[test]
    fn stale_main_script_update_check_records_diagnostics_and_rejects_register_job() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url],
        );
        {
            let mut state = service.inner.state.lock();
            let resource = state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource
                .get_or_insert_with(|| test_script_resource(&script_url));
            resource.body_sha256 = "changed-after-check-started".to_owned();
        }
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_pending_main_script_update_check(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url,
            42,
            completion_queue.sender(),
        );

        service.finish_main_script_update_check_completed(
            registration_id,
            Ok(test_update_check_result(&script_url, "abc", false)),
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.installing_version_count, 0);
        assert_eq!(diagnostics.pending_main_script_update_check_count, 0);
        let update_check = diagnostics.registrations[0]
            .last_main_script_update_check
            .as_ref()
            .expect("stale update check should be diagnosed");
        assert_eq!(update_check.result, "stale");
        assert_eq!(update_check.failure_status, Some("stale"));
        assert_eq!(
            update_check.message.as_deref(),
            Some("service worker main script update check became stale")
        );
        assert_eq!(diagnostics.registrations[0].last_update_check_time_ms, None);

        let completion = pop_register_completion(&mut completion_queue);
        let error = completion
            .result
            .expect_err("stale update check should reject");
        assert_eq!(
            error.kind,
            crate::service_worker_runtime::ServiceWorkerRegistrationErrorKind::Abort
        );
        assert_eq!(
            error.message,
            "service worker main script update check became stale"
        );
    }

    #[test]
    fn identical_main_script_update_resolves_existing_registration_without_starting_worker() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let installing_version_id = ServiceWorkerVersionId(2);
        let script_url = url("https://example.test/app/worker.js");
        let scope_url = url("https://example.test/app/");
        let document_url = url("https://example.test/app/page.html");
        let active_state = insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url.clone()],
        );
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist")
                .main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_starting_version_with_register_job(
            &service,
            registration_id,
            installing_version_id,
            script_url.clone(),
            scope_url,
            42,
            completion_queue.sender(),
        );
        let installing_run = exact_version_run(&service, installing_version_id);

        let mut different_resource = test_script_resource(&script_url);
        different_resource.body_sha256 = "different".to_owned();
        assert!(!service.finish_worker_start_identical_script_update(
            installing_version_id,
            installing_run.clone(),
            &different_resource
        ));
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&installing_version_id)
                .expect("installing version should exist")
                .allow_identical_script_update = false;
        }
        assert!(!service.finish_worker_start_identical_script_update(
            installing_version_id,
            installing_run.clone(),
            &test_script_resource(&script_url)
        ));
        {
            let mut state = service.inner.state.lock();
            state
                .versions
                .get_mut(&installing_version_id)
                .expect("installing version should exist")
                .allow_identical_script_update = true;
        }

        assert!(service.finish_worker_start_identical_script_update(
            installing_version_id,
            installing_run,
            &test_script_resource(&script_url)
        ));

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.registration_count, 1);
        assert_eq!(diagnostics.version_count, 1);
        assert_eq!(diagnostics.starting_version_count, 0);
        assert_eq!(diagnostics.installing_version_count, 0);
        assert_eq!(diagnostics.activated_version_count, 1);

        let snapshot = snapshot_for_registration(&service, registration_id);
        assert!(snapshot.installing().is_none());
        assert!(snapshot.waiting().is_none());
        assert_eq!(
            snapshot
                .active()
                .expect("active version should remain")
                .version_id(),
            active_version_id
        );
        assert_eq!(
            service.matching_controller_for_document(&document_url),
            Some(active_state)
        );

        let completion = pop_register_completion(&mut completion_queue);
        assert_eq!(completion.request_id, 42);
        let completion_snapshot = completion
            .result
            .expect("identical main script update should resolve");
        assert!(completion_snapshot.installing().is_none());
        assert_eq!(
            completion_snapshot
                .active()
                .expect("completion should expose active version")
                .version_id(),
            active_version_id
        );
    }

    #[test]
    fn restart_start_failure_rejects_pending_fetch_job() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let client_id = register_client_for_test(&service, document_url.clone());
        let event_id = ServiceWorkerEventId(7);
        let mut completion_queue = async_subresource_completion_queue();
        let host = new_loading_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            state.pending_fetch_jobs.insert(
                event_id,
                ServiceWorkerFetchJob {
                    internal_id: 41,
                    owner: Some(test_run_owner(version_id, &run)),
                    request_url: request_url.clone(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    request_body_bytes: None,
                    cors_preflight_request_headers: Vec::new(),
                    client_id: ServiceWorkerClientId::from_u64_for_test(0),
                    resulting_client_id: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    is_reload: false,
                    metadata: Default::default(),
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    redirect_chain: Vec::new(),
                    redirect_count: 0,
                    request_cookie_report: None,
                    network_context: AsyncSubresourceNetworkContext {
                        frame_id: None,
                        document_url: document_url.clone(),
                        resource_type: crate::types::SubresourceResourceType::Fetch,
                        policy_context: Default::default(),
                    },
                    completion_tx: completion_queue.sender(),
                    request_client: test_request_client(&service),
                    resource_task_runner: test_resource_task_runner(),
                    cancel_handle: moli_fetch::FetchCancelHandle::new(),
                    navigation_preload_cancel_handle: None,
                    streaming_body_source_id: None,
                    direct_completion_tx: None,
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Starting { host },
                    pending_start_events: VecDeque::from([ServiceWorkerPendingStartEvent::Fetch(
                        ServiceWorkerFetchEvent {
                            event_id,
                            owner: test_run_owner(version_id, &run),
                            request: ServiceWorkerFetchRequest {
                                client_id,
                                resulting_client_id: None,
                                url: request_url,
                                method: "GET".to_owned(),
                                headers: Vec::new(),
                                body: None,
                                destination: ServiceWorkerRequestDestination::Empty,
                                request_mode: moli_fetch::RequestMode::Cors,
                                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                                redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                                priority: None,
                                is_reload: false,
                                metadata: Default::default(),
                            },
                            navigation_preload_sent: false,
                        },
                    )]),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_worker_start_failed(
            version_id,
            run,
            ServiceWorkerVersionStartFailure::ScriptLoad {
                message: "restart script load failed".to_owned(),
            },
        );

        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
            assert!(matches!(
                version.running_state,
                ServiceWorkerVersionRunningState::Stopped
            ));
            assert_eq!(
                version.last_start_error.as_deref(),
                Some("restart script load failed")
            );
        }

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 41);
        assert_eq!(
            completion.result.err().as_deref(),
            Some("restart script load failed")
        );
    }

    #[test]
    fn abort_controlled_fetch_clears_running_job_and_ignores_late_completion() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(17);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.run = run.clone();
            version.running_state = ServiceWorkerVersionRunningState::Running {
                host: new_running_test_host(version_id, &run),
            };
            version.in_flight_event_count = 1;
            state.pending_fetch_jobs.insert(
                event_id,
                test_fetch_job(
                    &service,
                    51,
                    version_id,
                    &run,
                    client_id,
                    document_url,
                    request_url,
                    completion_queue.sender(),
                    cancel_handle.clone(),
                ),
            );
        }

        assert!(service.abort_controlled_fetch(51));
        assert!(cancel_handle.is_cancelled());
        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
            assert!(version.pending_start_events.is_empty());
            assert!(version.pending_activation_fetch_events.is_empty());
        }
        assert!(!completion_queue.has_ready_completion());

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            owner: test_run_owner(version_id, &run),
            result: ServiceWorkerFetchResult::Fallback,
        });
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(
                version.in_flight_event_count, 0,
                "late completion after abort must not decrement unrelated event accounting"
            );
            assert!(state.pending_fetch_jobs.is_empty());
        }
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn navigation_preload_fallback_completion_cancels_unstarted_preload_only() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(60);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let fallback_cancel_handle = moli_fetch::FetchCancelHandle::new();
        let navigation_preload_cancel_handle = moli_fetch::FetchCancelHandle::new();
        insert_pending_navigation_preload_fetch_job(
            &service,
            event_id,
            301,
            version_id,
            &run,
            client_id,
            document_url,
            request_url,
            completion_queue.sender(),
            fallback_cancel_handle.clone(),
            navigation_preload_cancel_handle.clone(),
        );
        let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
        service
            .inner
            .state
            .lock()
            .pending_fetch_jobs
            .get_mut(&event_id)
            .expect("pending navigation preload fetch job")
            .direct_completion_tx = Some(direct_completion_tx);

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            owner: test_run_owner(version_id, &run),
            result: ServiceWorkerFetchResult::Fallback,
        });

        assert!(
            navigation_preload_cancel_handle.is_cancelled(),
            "fallback completion must cancel the parallel preload before it has response-started"
        );
        assert!(
            !fallback_cancel_handle.is_cancelled(),
            "preload cancellation must not abort the main fallback fetch handle"
        );
        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            assert_eq!(
                state
                    .versions
                    .get(&version_id)
                    .unwrap()
                    .in_flight_event_count,
                0
            );
        }
        expect_direct_fetch_fallback(&mut direct_completion_rx);
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn navigation_preload_completion_keeps_response_started_preload_alive() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(61);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let fetch_cancel_handle = moli_fetch::FetchCancelHandle::new();
        let navigation_preload_cancel_handle = moli_fetch::FetchCancelHandle::new();
        insert_pending_navigation_preload_fetch_job(
            &service,
            event_id,
            302,
            version_id,
            &run,
            client_id,
            document_url,
            request_url,
            completion_queue.sender(),
            fetch_cancel_handle,
            navigation_preload_cancel_handle.clone(),
        );

        assert!(
            service.mark_navigation_preload_response_started(
                event_id,
                &test_run_owner(version_id, &run),
            )
        );
        assert!(
            !navigation_preload_cancel_handle.is_cancelled(),
            "marking response-started should detach the runtime cancel handle, not cancel it"
        );

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            owner: test_run_owner(version_id, &run),
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                body: b"handled".to_vec(),
                final_url: None,
                response_type: "basic".to_owned(),
                redirected: false,
            }),
        });

        assert!(
            !navigation_preload_cancel_handle.is_cancelled(),
            "FetchEvent completion must not cancel a preload response already handed to the worker"
        );
        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 302);
        assert!(completion.result.is_ok());
    }

    #[test]
    fn abort_controlled_fetch_cancels_unstarted_navigation_preload() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(62);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let fetch_cancel_handle = moli_fetch::FetchCancelHandle::new();
        let navigation_preload_cancel_handle = moli_fetch::FetchCancelHandle::new();
        insert_pending_navigation_preload_fetch_job(
            &service,
            event_id,
            303,
            version_id,
            &run,
            client_id,
            document_url,
            request_url,
            completion_queue.sender(),
            fetch_cancel_handle.clone(),
            navigation_preload_cancel_handle.clone(),
        );

        assert!(service.abort_controlled_fetch(303));

        assert!(fetch_cancel_handle.is_cancelled());
        assert!(navigation_preload_cancel_handle.is_cancelled());
        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            assert_eq!(
                state
                    .versions
                    .get(&version_id)
                    .unwrap()
                    .in_flight_event_count,
                0
            );
        }
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn abort_controlled_fetch_records_aborted_network_diagnostic() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(20);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        {
            let mut state = service.inner.state.lock();
            state.record_target_created(registration_id, version_id, script_url, scope_url);
            service.take_target_output_events_for_test();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.run = run.clone();
            version.running_state = ServiceWorkerVersionRunningState::Running {
                host: new_running_test_host(version_id, &run),
            };
            version.in_flight_event_count = 1;
            state.pending_fetch_jobs.insert(
                event_id,
                test_fetch_job(
                    &service,
                    56,
                    version_id,
                    &run,
                    client_id,
                    document_url,
                    request_url,
                    completion_queue.sender(),
                    cancel_handle.clone(),
                ),
            );
        }

        assert!(service.abort_controlled_fetch(56));
        assert!(cancel_handle.is_cancelled());
        assert!(!completion_queue.has_ready_completion());

        let target_events = service.take_target_output_events_for_test();
        let Some(crate::runtime::RendererServiceWorkerTargetEvent::FetchDiagnostic {
            version_id: diagnostic_version_id,
            diagnostic,
            ..
        }) = target_events.iter().find(|event| {
            matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::FetchDiagnostic { .. }
            )
        })
        else {
            panic!("expected fetch diagnostic event after abort, got {target_events:?}");
        };
        assert_eq!(*diagnostic_version_id, version_id.as_u64());
        assert_eq!(diagnostic.internal_id, 56);
        assert_eq!(
            diagnostic.result,
            crate::runtime::RendererServiceWorkerFetchDiagnosticResult::Failure {
                message: crate::network_host::ABORTED_ERROR_TEXT.to_owned()
            }
        );
    }

    #[test]
    fn abort_controlled_fetch_cancels_running_stream_reader() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(18);
        let body_source_id = 77;
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        let (worker_tx, mut worker_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_parent_tx, parent_rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = crate::worker::WorkerHandle::new(
            worker_tx,
            parent_rx,
            std::thread::spawn(|| {}),
            Arc::new(parking_lot::Mutex::new(None)),
        );
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.run = run.clone();
            version.running_state = ServiceWorkerVersionRunningState::Running {
                host: new_running_test_host_with_handle(version_id, &run, handle),
            };
            version.in_flight_event_count = 1;
            let mut job = test_fetch_job(
                &service,
                55,
                version_id,
                &run,
                client_id,
                document_url,
                request_url,
                completion_queue.sender(),
                cancel_handle.clone(),
            );
            job.streaming_body_source_id = Some(body_source_id);
            state.pending_fetch_jobs.insert(event_id, job);
        }

        assert!(service.abort_controlled_fetch(55));
        assert!(cancel_handle.is_cancelled());
        match worker_rx.try_recv() {
            Ok(crate::worker::WorkerMessage::ServiceWorkerFetchRequestSignalAbort {
                event_id: actual_event_id,
                reason,
            }) => {
                assert_eq!(actual_event_id, event_id);
                assert!(reason.is_none());
            }
            other => panic!("expected request signal abort worker message, got {other:?}"),
        }
        match worker_rx.try_recv() {
            Ok(crate::worker::WorkerMessage::ServiceWorkerFetchStreamCancel {
                event_id: actual_event_id,
                body_source_id: actual_body_source_id,
            }) => {
                assert_eq!(actual_event_id, event_id);
                assert_eq!(actual_body_source_id, body_source_id);
            }
            other => panic!("expected stream cancel worker message, got {other:?}"),
        }
        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
        }
    }

    #[test]
    fn abort_controlled_fetch_drops_direct_worker_completion_and_ignores_late_completion() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(19);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/dedicated-worker.js");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.run = run.clone();
            version.running_state = ServiceWorkerVersionRunningState::Running {
                host: new_running_test_host(version_id, &run),
            };
            version.in_flight_event_count = 1;
            let mut job = test_fetch_job(
                &service,
                53,
                version_id,
                &run,
                client_id,
                document_url,
                request_url,
                completion_queue.sender(),
                cancel_handle.clone(),
            );
            job.direct_completion_tx = Some(direct_completion_tx);
            state.pending_fetch_jobs.insert(event_id, job);
        }

        assert!(service.abort_controlled_fetch(53));
        assert!(cancel_handle.is_cancelled());
        assert!(matches!(
            direct_completion_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
            assert!(version.pending_start_events.is_empty());
            assert!(version.pending_activation_fetch_events.is_empty());
        }
        assert!(!completion_queue.has_ready_completion());

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            owner: test_run_owner(version_id, &run),
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                status: 200,
                status_text: "OK".to_owned(),
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                body: b"late".to_vec(),
                final_url: None,
                response_type: "basic".to_owned(),
                redirected: false,
            }),
        });
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(
                version.in_flight_event_count, 0,
                "late direct completion after abort must not touch event accounting"
            );
            assert!(state.pending_fetch_jobs.is_empty());
        }
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn abort_controlled_fetch_removes_pending_start_event() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let event_id = ServiceWorkerEventId(18);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        {
            let mut state = service.inner.state.lock();
            state.pending_fetch_jobs.insert(
                event_id,
                test_fetch_job(
                    &service,
                    52,
                    version_id,
                    &run,
                    client_id,
                    document_url.clone(),
                    request_url.clone(),
                    completion_queue.sender(),
                    cancel_handle.clone(),
                ),
            );
            let version = state.versions.get_mut(&version_id).unwrap();
            version.run = run.clone();
            version.running_state = ServiceWorkerVersionRunningState::Starting {
                host: new_loading_test_host(version_id, &run),
            };
            version.pending_start_events = VecDeque::from([ServiceWorkerPendingStartEvent::Fetch(
                ServiceWorkerFetchEvent {
                    event_id,
                    owner: test_run_owner(version_id, &run),
                    request: test_fetch_request(client_id, request_url),
                    navigation_preload_sent: false,
                },
            )]);
            version.in_flight_event_count = 1;
        }

        assert!(service.abort_controlled_fetch(52));
        assert!(cancel_handle.is_cancelled());
        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
            assert!(version.pending_start_events.is_empty());
        }
        assert!(!completion_queue.has_ready_completion());

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            owner: test_run_owner(version_id, &run),
            result: ServiceWorkerFetchResult::Fallback,
        });
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
            assert!(state.pending_fetch_jobs.is_empty());
        }
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn no_fetch_handler_controlled_fetch_falls_back_without_dispatching_event() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let client_id = register_client_for_test(&service, document_url.clone());
        let mut completion_queue = async_subresource_completion_queue();
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
        assert!(
            service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
                internal_id: 88,
                request: ServiceWorkerFetchRequest {
                    client_id,
                    resulting_client_id: None,
                    url: request_url.clone(),
                    method: "GET".to_owned(),
                    headers: Vec::new(),
                    body: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    is_reload: false,
                    metadata: Default::default(),
                },
                request_body_text: None,
                cors_preflight_request_headers: Vec::new(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    document_url: document_url.clone(),
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                direct_completion_tx: Some(direct_completion_tx),
            })
        );

        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
            assert!(matches!(
                version.running_state,
                ServiceWorkerVersionRunningState::Running { .. }
            ));
            assert!(
                state
                    .registrations
                    .get(&registration_id)
                    .unwrap()
                    .controlled_client_ids
                    .contains(&client_id),
                "no-handler fallback must not clear the active controller"
            );
        }

        expect_direct_fetch_fallback(&mut direct_completion_rx);
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn controlled_fetch_waits_for_activating_active_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/");
        let mut completion_queue = async_subresource_completion_queue();
        let (direct_completion_tx, mut main_resource_completion_rx) =
            tokio::sync::oneshot::channel();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::DoesNotExist;
            version.fetch_handler_type = ServiceWorkerFetchHandlerType::NoHandler;
            version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
            version.running_state = ServiceWorkerVersionRunningState::Running { host };
            version.in_flight_event_count = 1;
            version.run = run.clone();
        }

        assert!(
            service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
                internal_id: 91,
                request: ServiceWorkerFetchRequest {
                    client_id,
                    resulting_client_id: None,
                    url: request_url,
                    method: "GET".to_owned(),
                    headers: Vec::new(),
                    body: None,
                    destination: ServiceWorkerRequestDestination::Document,
                    request_mode: moli_fetch::RequestMode::Navigate,
                    credentials_mode: moli_fetch::RequestCredentialsMode::Include,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    is_reload: false,
                    metadata: Default::default(),
                },
                request_body_text: None,
                cors_preflight_request_headers: Vec::new(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    document_url: document_url.clone(),
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                direct_completion_tx: Some(direct_completion_tx),
            })
        );

        {
            let state = service.inner.state.lock();
            assert_eq!(state.pending_fetch_jobs.len(), 1);
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
            assert!(version.pending_start_events.is_empty());
            assert_eq!(version.pending_activation_fetch_events.len(), 1);
            let event = version.pending_activation_fetch_events.front().unwrap();
            assert_eq!(event.owner, test_run_owner(version_id, &run));
            assert_eq!(
                event.request.destination,
                ServiceWorkerRequestDestination::Document
            );
        }
        assert!(matches!(
            main_resource_completion_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(77),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(
                version.lifecycle_state,
                ServiceWorkerVersionLifecycleState::Activated
            );
            assert_eq!(version.in_flight_event_count, 0);
            assert!(version.pending_activation_fetch_events.is_empty());
        }
        assert!(matches!(
            main_resource_completion_rx.try_recv(),
            Ok(ServiceWorkerDirectFetchResult::Fallback)
        ));
        assert!(!completion_queue.has_ready_completion());
    }

    #[tokio::test]
    async fn controlled_fetch_from_worker_client_dispatches_to_active_worker() {
        ensure_v8_for_test();
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let worker_script_url = url("https://example.test/app/dedicated-worker.js");
        let request_url = url("https://example.test/app/data.txt");
        let (bootstrap_tx, mut bootstrap_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::worker::WorkerBootstrapCompletion>();
        let mut handle = crate::worker::spawn_worker_with_options(
            crate::worker::WorkerSpawnOptions::new_with_request_client(
                r#"
                self.addEventListener("fetch", event => {
                    event.respondWith(new Response(JSON.stringify({
                        url: event.request.url,
                        clientId: event.clientId,
                        destination: event.request.destination,
                        mode: event.request.mode,
                        credentials: event.request.credentials,
                        redirect: event.request.redirect,
                        header: event.request.headers.get("x-worker-fetch")
                    }), {
                        status: 209,
                        statusText: "Worker Controlled",
                        headers: {
                            "content-type": "application/json",
                            "x-service-worker": "worker-client"
                        }
                    }));
                });
                "#
                .to_owned(),
                script_url.to_string(),
                service.request_client(),
            )
            .with_global_kind(crate::worker::WorkerGlobalKind::Service {
                registration_id,
                version_id,
                scope_url: scope_url.clone(),
            })
            .with_bootstrap_completion_sender(bootstrap_tx),
        );
        let bootstrap = tokio::time::timeout(Duration::from_secs(5), bootstrap_rx.recv())
            .await
            .expect("timed out waiting for service worker bootstrap")
            .expect("service worker bootstrap channel closed");
        bootstrap
            .result
            .expect("service worker bootstrap should succeed");
        let mut parent_rx = handle
            .take_receiver()
            .expect("service worker should expose parent receiver");
        let host = new_running_test_host_with_handle(version_id, &run, handle);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            std::iter::empty::<Url>(),
        );
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::Exists;
            version.fetch_handler_type = ServiceWorkerFetchHandlerType::NotSkippable;
            version.running_state = ServiceWorkerVersionRunningState::Running { host };
            version.run = run.clone();
        }
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&worker_script_url);
        let (worker_tx, _worker_rx) = tokio::sync::mpsc::unbounded_channel();
        let client_id = service.register_worker_client_with_storage_key(
            worker_script_url.clone(),
            storage_key,
            ServiceWorkerClientType::DedicatedWorker,
            true,
            worker_tx,
        );
        assert!(
            service
                .matching_controller_for_client_fetch(client_id, &request_url)
                .is_some(),
            "registered worker client should be controlled by the active registration"
        );

        let mut completion_queue = async_subresource_completion_queue();
        let (direct_completion_tx, direct_completion_rx) = tokio::sync::oneshot::channel();
        assert!(
            service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
                internal_id: 90,
                request: ServiceWorkerFetchRequest {
                    client_id,
                    resulting_client_id: None,
                    url: request_url.clone(),
                    method: "GET".to_owned(),
                    headers: vec![("x-worker-fetch".to_owned(), "dedicated".to_owned())],
                    body: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    is_reload: false,
                    metadata: Default::default(),
                },
                request_body_text: None,
                cors_preflight_request_headers: Vec::new(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    document_url: worker_script_url,
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                direct_completion_tx: Some(direct_completion_tx),
            }),
            "controlled worker client fetch should dispatch to the active worker"
        );

        let completion = loop {
            let message = tokio::time::timeout(Duration::from_secs(5), parent_rx.recv())
                .await
                .expect("timed out waiting for service worker fetch completion")
                .expect("service worker parent channel closed");
            match message {
                crate::worker::WorkerToParentMessage::ServiceWorkerFetchCompleted(completion) => {
                    break completion;
                }
                crate::worker::WorkerToParentMessage::Console(_) => {}
                other => panic!("unexpected service worker parent message: {other:?}"),
            }
        };
        service.finish_fetch_event_completed(completion);

        let direct_result = tokio::time::timeout(Duration::from_secs(5), direct_completion_rx)
            .await
            .expect("timed out waiting for direct service worker fetch completion")
            .expect("direct service worker fetch channel closed");
        let ServiceWorkerDirectFetchResult::Response(response) = direct_result else {
            panic!("expected direct service worker response, got {direct_result:?}");
        };
        assert_eq!(response.response.status, 209);
        assert_eq!(
            response
                .response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("x-service-worker"))
                .map(|(_, value)| value.as_str()),
            Some("worker-client")
        );
        assert_eq!(
            response.response.body_text(),
            format!(
                r#"{{"url":"https://example.test/app/data.txt","clientId":"client-{client_id:016x}","destination":"","mode":"cors","credentials":"same-origin","redirect":"follow","header":"dedicated"}}"#,
                client_id = client_id.as_u64()
            )
        );
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn empty_fetch_handler_controlled_fetch_falls_back_without_dispatching_event() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::Exists;
            version.fetch_handler_type = ServiceWorkerFetchHandlerType::EmptyFetchHandler;
            version.running_state = ServiceWorkerVersionRunningState::Running { host };
            version.run = run.clone();
        }

        let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
        assert!(
            service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
                internal_id: 89,
                request: ServiceWorkerFetchRequest {
                    client_id,
                    resulting_client_id: None,
                    url: request_url,
                    method: "GET".to_owned(),
                    headers: Vec::new(),
                    body: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    is_reload: false,
                    metadata: Default::default(),
                },
                request_body_text: None,
                cors_preflight_request_headers: Vec::new(),
                request_cookie_report: None,
                network_context: AsyncSubresourceNetworkContext {
                    frame_id: None,
                    document_url: document_url.clone(),
                    resource_type: crate::types::SubresourceResourceType::Fetch,
                    policy_context: Default::default(),
                },
                completion_tx: completion_queue.sender(),
                request_client: test_request_client(&service),
                resource_task_runner: test_resource_task_runner(),
                cancel_handle: moli_fetch::FetchCancelHandle::new(),
                direct_completion_tx: Some(direct_completion_tx),
            })
        );

        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
            assert!(matches!(
                version.running_state,
                ServiceWorkerVersionRunningState::Running { .. }
            ));
            assert!(
                state
                    .registrations
                    .get(&registration_id)
                    .unwrap()
                    .controlled_client_ids
                    .contains(&client_id),
                "empty-handler fallback must not clear the active controller"
            );
        }

        expect_direct_fetch_fallback(&mut direct_completion_rx);
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn empty_fetch_handler_pending_start_fetch_falls_back_after_worker_start() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let event_id = ServiceWorkerEventId(13);
        let mut completion_queue = async_subresource_completion_queue();
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url,
            [document_url.clone()],
        );
        let client_id = client_id_for_document(&service, &document_url);
        let host = new_loading_test_host(version_id, &run);
        let request = ServiceWorkerFetchRequest {
            client_id,
            resulting_client_id: None,
            url: request_url.clone(),
            method: "GET".to_owned(),
            headers: Vec::new(),
            body: None,
            destination: ServiceWorkerRequestDestination::Empty,
            request_mode: moli_fetch::RequestMode::Cors,
            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
            redirect_mode: moli_fetch::RequestRedirectMode::Follow,
            priority: None,
            is_reload: false,
            metadata: Default::default(),
        };
        let (direct_completion_tx, mut direct_completion_rx) = tokio::sync::oneshot::channel();
        {
            let mut state = service.inner.state.lock();
            state.pending_fetch_jobs.insert(
                event_id,
                ServiceWorkerFetchJob {
                    internal_id: 90,
                    owner: Some(test_run_owner(version_id, &run)),
                    request_url,
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    request_body_bytes: None,
                    cors_preflight_request_headers: Vec::new(),
                    client_id: ServiceWorkerClientId::from_u64_for_test(0),
                    resulting_client_id: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    is_reload: false,
                    metadata: Default::default(),
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    redirect_chain: Vec::new(),
                    redirect_count: 0,
                    request_cookie_report: None,
                    network_context: AsyncSubresourceNetworkContext {
                        frame_id: None,
                        document_url,
                        resource_type: crate::types::SubresourceResourceType::Fetch,
                        policy_context: Default::default(),
                    },
                    completion_tx: completion_queue.sender(),
                    request_client: test_request_client(&service),
                    resource_task_runner: test_resource_task_runner(),
                    cancel_handle: moli_fetch::FetchCancelHandle::new(),
                    navigation_preload_cancel_handle: None,
                    streaming_body_source_id: None,
                    direct_completion_tx: Some(direct_completion_tx),
                },
            );
            let version = state.versions.get_mut(&version_id).unwrap();
            version.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::Unknown;
            version.fetch_handler_type = ServiceWorkerFetchHandlerType::NoHandler;
            version.running_state = ServiceWorkerVersionRunningState::Starting { host };
            version.pending_start_events = VecDeque::from([ServiceWorkerPendingStartEvent::Fetch(
                ServiceWorkerFetchEvent {
                    event_id,
                    owner: test_run_owner(version_id, &run),
                    request,
                    navigation_preload_sent: false,
                },
            )]);
            version.in_flight_event_count = 1;
            version.run = run.clone();
        }

        service.finish_worker_start_completed_with_script_resource(
            version_id,
            run,
            script_url.to_string(),
            Some(test_script_resource(&script_url)),
            ServiceWorkerFetchHandlerType::EmptyFetchHandler,
        );

        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(
                version.fetch_handler_existence,
                ServiceWorkerFetchHandlerExistence::Exists
            );
            assert_eq!(
                version.fetch_handler_type,
                ServiceWorkerFetchHandlerType::EmptyFetchHandler
            );
            assert!(matches!(
                version.running_state,
                ServiceWorkerVersionRunningState::Running { .. }
            ));
            assert_eq!(version.in_flight_event_count, 1);
            assert!(state.pending_fetch_jobs.contains_key(&event_id));
        }
        assert_eq!(service.drain_service_lane(), 1);

        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 0);
            assert!(version.pending_start_events.is_empty());
        }

        expect_direct_fetch_fallback(&mut direct_completion_rx);
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn fetch_completion_schedules_idle_timeout_for_running_active_worker() {
        let service = new_service_worker_runtime_service();
        service.set_idle_delay_for_test(Duration::ZERO);
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let client_id = register_client_for_test(&service, document_url.clone());
        let event_id = ServiceWorkerEventId(11);
        let mut completion_queue = async_subresource_completion_queue();
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            state.pending_fetch_jobs.insert(
                event_id,
                ServiceWorkerFetchJob {
                    internal_id: 77,
                    owner: Some(test_run_owner(version_id, &run)),
                    request_url,
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    request_body_bytes: None,
                    cors_preflight_request_headers: Vec::new(),
                    client_id: ServiceWorkerClientId::from_u64_for_test(0),
                    resulting_client_id: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    is_reload: false,
                    metadata: Default::default(),
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    redirect_chain: Vec::new(),
                    redirect_count: 0,
                    request_cookie_report: None,
                    network_context: AsyncSubresourceNetworkContext {
                        frame_id: None,
                        document_url,
                        resource_type: crate::types::SubresourceResourceType::Fetch,
                        policy_context: Default::default(),
                    },
                    completion_tx: completion_queue.sender(),
                    request_client: test_request_client(&service),
                    resource_task_runner: test_resource_task_runner(),
                    cancel_handle: moli_fetch::FetchCancelHandle::new(),
                    navigation_preload_cancel_handle: None,
                    streaming_body_source_id: None,
                    direct_completion_tx: None,
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.record_target_created(
                registration_id,
                version_id,
                script_url.clone(),
                scope_url.clone(),
            );
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            owner: test_run_owner(version_id, &run),
            result: ServiceWorkerFetchResult::Failure("handled".to_owned()),
        });

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.in_flight_event_count, 0);
        assert_eq!(diagnostics.running_version_count, 1);
        assert_eq!(diagnostics.pending_service_lane_event_count, 1);

        assert_eq!(service.drain_service_lane(), 1);
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.running_version_count, 0);
        assert_eq!(diagnostics.running_host_count, 0);
        assert_eq!(diagnostics.stopped_version_count, 1);
        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Stopped {
                    version_id: stopped_version_id,
                    reason,
                    ..
                } if *stopped_version_id == version_id.as_u64()
                    && reason == "idle_timeout"
            )),
            "idle timeout should enqueue a stopped target lifecycle event: {target_events:?}"
        );
        assert!(
            !target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                    version_id: destroyed_version_id,
                    ..
                } if *destroyed_version_id == version_id.as_u64()
            )),
            "idle timeout must retain the service worker target: {target_events:?}"
        );

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 77);
        assert_eq!(completion.result.err().as_deref(), Some("handled"));
    }

    #[test]
    fn devtools_stop_worker_records_target_stopped() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.run = run.clone();
            version.running_state = ServiceWorkerVersionRunningState::Running { host };
            state.record_target_created(registration_id, version_id, script_url, scope_url);
            service.take_target_output_events_for_test();
        }

        assert_eq!(
            service.devtools_stop_worker_version(version_id),
            Ok(true),
            "DevTools stopWorker should accept a live version"
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.running_version_count, 0);
        assert_eq!(diagnostics.stopped_version_count, 1);
        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Stopped {
                    version_id: stopped_version_id,
                    reason,
                    ..
                } if *stopped_version_id == version_id.as_u64()
                    && reason == "devtools_stop"
            )),
            "DevTools stopWorker should enqueue a stopped target event: {target_events:?}"
        );
    }

    #[test]
    fn devtools_unregister_scope_uses_owner_unregistration_path() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            state.record_target_created(registration_id, version_id, script_url, scope_url.clone());
            service.take_target_output_events_for_test();
        }

        assert_eq!(
            service.devtools_unregister_scope(&scope_url),
            Ok(true),
            "DevTools unregister should accept an existing registration"
        );

        let state = service.inner.state.lock();
        assert!(!state.registrations.contains_key(&registration_id));
        assert!(!state.versions.contains_key(&version_id));
        drop(state);
        let target_events = service.take_target_output_events_for_test();
        assert!(
            target_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
                    version_id: destroyed_version_id,
                    active_run: None,
                } if *destroyed_version_id == version_id.as_u64()
            )),
            "DevTools unregister should destroy the runtime-owned target: {target_events:?}"
        );
    }

    #[test]
    fn devtools_skip_waiting_scope_uses_activation_progress() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let waiting_script_url = url("https://example.test/app/worker-v2.js");
        let waiting_run = RendererServiceWorkerRunIdentity::fresh();
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            active_script_url.clone(),
            scope_url.clone(),
            [],
        );
        let waiting_host = new_running_test_host(waiting_version_id, &waiting_run);
        {
            let mut state = service.inner.state.lock();
            let registration = state.registrations.get_mut(&registration_id).unwrap();
            registration.script_url = waiting_script_url.clone();
            registration.waiting_version_id = Some(waiting_version_id);
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installed,
                    running_state: ServiceWorkerVersionRunningState::Running { host: waiting_host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: waiting_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert_eq!(
            service.devtools_skip_waiting_for_scope(&scope_url),
            Ok(true),
            "DevTools skipWaiting should accept the waiting worker"
        );

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
        let waiting = state.versions.get(&waiting_version_id).unwrap();
        assert!(waiting.skip_waiting_requested);
        assert_eq!(
            waiting.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activating
        );
        assert_eq!(waiting.in_flight_event_count, 1);
    }

    #[test]
    fn devtools_update_registration_enters_existing_registration_job_queue() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let pending_script_url = url("https://example.test/app/pending-worker.js");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_pending_main_script_update_check(
            &service,
            registration_id,
            version_id,
            pending_script_url,
            scope_url.clone(),
            44,
            completion_queue.sender(),
        );

        assert_eq!(
            service.devtools_update_registration_for_scope(
                &scope_url,
                service.browser_context_runtime()
            ),
            Ok(true),
            "updateRegistration should enqueue through the runtime-owned job coordinator"
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 1);
        assert!(!completion_queue.has_ready_task());
    }

    #[test]
    fn devtools_force_update_on_page_load_enqueues_update_for_controlled_page_load() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let pending_script_url = url("https://example.test/app/pending-worker.js");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.main_script_resource = Some(test_script_resource(&script_url));
        }
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        insert_pending_main_script_update_check(
            &service,
            registration_id,
            version_id,
            pending_script_url,
            scope_url.clone(),
            44,
            completion_queue.sender(),
        );

        assert!(!service.force_update_on_page_load_for_devtools());
        assert!(
            !service
                .devtools_force_update_registration_for_page_load(
                    &scope_url,
                    service.browser_context_runtime()
                )
                .0
        );
        assert_eq!(service.diagnostics_snapshot().queued_register_job_count, 0);

        service.set_force_update_on_page_load_for_devtools(true);
        assert!(service.force_update_on_page_load_for_devtools());
        let (started, force_update_rx) = service.devtools_force_update_registration_for_page_load(
            &scope_url,
            service.browser_context_runtime(),
        );
        assert!(started);
        assert!(
            force_update_rx.is_some(),
            "force-update-on-page-load should expose a waiter"
        );

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.pending_main_script_update_check_count, 1);
        assert_eq!(diagnostics.queued_register_job_count, 1);
        {
            let state = service.inner.state.lock();
            let registration_key = ServiceWorkerRegistrationKey::for_scope_and_storage_key(
                &scope_url,
                ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
            );
            assert_eq!(
                state
                    .job_coordinator
                    .queued_register_job_options(&registration_key),
                vec![(true, true, true)],
                "force-update-on-page-load should queue a forced update job that skips waiting"
            );
        }
        assert!(!completion_queue.has_ready_task());

        service.set_force_update_on_page_load_for_devtools(false);
        assert!(!service.force_update_on_page_load_for_devtools());
    }

    #[test]
    fn devtools_functional_event_dispatch_uses_active_registration_owner() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let origin = url("https://example.test/");
        let other_origin = url("https://other.test/");
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            script_url,
            scope_url,
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.run = run.clone();
            version.running_state = ServiceWorkerVersionRunningState::Starting {
                host: new_loading_test_host(version_id, &run),
            };
        }

        assert_eq!(
            service.devtools_deliver_push_message(
                &other_origin,
                registration_id,
                Some(b"ignored".to_vec())
            ),
            Ok(false),
            "DevTools push dispatch should enforce the registration storage key"
        );
        assert_eq!(
            service.devtools_deliver_push_message(
                &origin,
                registration_id,
                Some(b"payload".to_vec())
            ),
            Ok(true)
        );
        assert_eq!(
            service.devtools_dispatch_sync_event(
                &origin,
                registration_id,
                "sync-tag".to_owned(),
                true
            ),
            Ok(true)
        );
        assert_eq!(
            service.devtools_dispatch_periodic_sync_event(
                &origin,
                registration_id,
                "periodic-tag".to_owned()
            ),
            Ok(true)
        );

        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 3);
        assert_eq!(version.pending_start_events.len(), 3);
        let ServiceWorkerPendingStartEvent::Push(push) = &version.pending_start_events[0] else {
            panic!("expected pending push event");
        };
        assert_eq!(push.owner, test_run_owner(version_id, &run));
        assert_eq!(push.data.as_deref(), Some(&b"payload"[..]));
        let ServiceWorkerPendingStartEvent::Sync(sync) = &version.pending_start_events[1] else {
            panic!("expected pending sync event");
        };
        assert_eq!(sync.registration_id, registration_id);
        assert_eq!(sync.owner, test_run_owner(version_id, &run));
        assert_eq!(sync.tag, "sync-tag");
        assert!(sync.last_chance);
        let ServiceWorkerPendingStartEvent::PeriodicSync(periodic) =
            &version.pending_start_events[2]
        else {
            panic!("expected pending periodic sync event");
        };
        assert_eq!(periodic.registration_id, registration_id);
        assert_eq!(periodic.owner, test_run_owner(version_id, &run));
        assert_eq!(periodic.tag, "periodic-tag");
    }

    #[test]
    fn active_fetch_completion_queues_waiting_activation_after_controllees_are_gone() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let active_run = RendererServiceWorkerRunIdentity::fresh();
        let waiting_run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let waiting_script_url = url("https://example.test/app/worker-v2.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let event_id = ServiceWorkerEventId(19);
        let waiting_host = new_loading_test_host(waiting_version_id, &waiting_run);
        let mut completion_queue = async_subresource_completion_queue();
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: waiting_script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: Some(waiting_version_id),
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.pending_fetch_jobs.insert(
                event_id,
                ServiceWorkerFetchJob {
                    internal_id: 91,
                    owner: Some(test_run_owner(active_version_id, &active_run)),
                    request_url: request_url.clone(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    request_body_bytes: None,
                    cors_preflight_request_headers: Vec::new(),
                    client_id: ServiceWorkerClientId::from_u64_for_test(0),
                    resulting_client_id: None,
                    destination: ServiceWorkerRequestDestination::Empty,
                    is_reload: false,
                    metadata: Default::default(),
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    priority: None,
                    redirect_chain: Vec::new(),
                    redirect_count: 0,
                    request_cookie_report: None,
                    network_context: AsyncSubresourceNetworkContext {
                        frame_id: None,
                        document_url,
                        resource_type: crate::types::SubresourceResourceType::Fetch,
                        policy_context: Default::default(),
                    },
                    completion_tx: completion_queue.sender(),
                    request_client: test_request_client(&service),
                    resource_task_runner: test_resource_task_runner(),
                    cancel_handle: moli_fetch::FetchCancelHandle::new(),
                    navigation_preload_cancel_handle: None,
                    streaming_body_source_id: None,
                    direct_completion_tx: None,
                },
            );
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: active_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installed,
                    running_state: ServiceWorkerVersionRunningState::Starting {
                        host: waiting_host,
                    },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: waiting_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id,
            owner: test_run_owner(active_version_id, &active_run),
            result: ServiceWorkerFetchResult::Failure("done".to_owned()),
        });

        let state = service.inner.state.lock();
        assert_eq!(
            state
                .versions
                .get(&active_version_id)
                .unwrap()
                .in_flight_event_count,
            0
        );
        let waiting = state.versions.get(&waiting_version_id).unwrap();
        assert_eq!(
            waiting.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activating
        );
        assert_eq!(waiting.in_flight_event_count, 1);
        assert_eq!(waiting.pending_start_events.len(), 1);
        drop(state);

        let completion = pop_async_subresource_completion(&mut completion_queue);
        assert_eq!(completion.internal_id, 91);
        assert_eq!(completion.result.err().as_deref(), Some("done"));
    }

    #[test]
    fn idle_timeout_is_ignored_after_new_event_invalidates_timeout_token() {
        let service = new_service_worker_runtime_service();
        service.set_idle_delay_for_test(Duration::ZERO);
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        let timeout = {
            let mut state = service.inner.state.lock();
            service
                .maybe_schedule_idle_timeout_locked(&mut state, version_id)
                .expect("active idle worker should schedule timeout")
        };
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            ServiceWorkerRuntimeService::begin_version_event_locked(version);
        }
        service.schedule_idle_timeout(timeout);
        assert_eq!(service.drain_service_lane(), 1);

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.running_version_count, 1);
        assert_eq!(diagnostics.stopped_version_count, 0);
        assert_eq!(diagnostics.in_flight_event_count, 1);
    }

    #[test]
    fn install_completion_moves_version_to_waiting_and_decrements_event_count() {
        let service = new_service_worker_runtime_service();
        let (registration_id, version_id) = insert_running_installing_version(&service);
        let run = exact_version_run(&service, version_id);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.installing_version_id, None);
        assert_eq!(registration.waiting_version_id, Some(version_id));
        assert_eq!(registration.active_version_id, None);
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(
            version.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Installed
        );
        assert_eq!(version.in_flight_event_count, 0);
    }

    #[test]
    fn install_rejection_deletes_initial_install_registration() {
        let service = new_service_worker_runtime_service();
        let (registration_id, version_id) = insert_running_installing_version(&service);
        let run = exact_version_run(&service, version_id);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Err("service worker waitUntil promise rejected".to_owned()),
        });

        let state = service.inner.state.lock();
        assert!(!state.registrations.contains_key(&registration_id));
        assert!(!state.versions.contains_key(&version_id));
    }

    #[test]
    fn activate_rejection_still_commits_active_version() {
        let service = new_service_worker_runtime_service();
        let (registration_id, version_id) = insert_running_installing_version(&service);
        let run = exact_version_run(&service, version_id);
        {
            let mut state = service.inner.state.lock();
            let registration = state.registrations.get_mut(&registration_id).unwrap();
            registration.installing_version_id = None;
            registration.waiting_version_id = Some(version_id);
            let version = state.versions.get_mut(&version_id).unwrap();
            version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
            version.in_flight_event_count = 1;
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Err("service worker waitUntil promise rejected".to_owned()),
        });

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.installing_version_id, None);
        assert_eq!(registration.waiting_version_id, None);
        assert_eq!(registration.active_version_id, Some(version_id));
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(
            version.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activated
        );
        assert_eq!(version.in_flight_event_count, 0);
        assert_eq!(
            version.last_start_error.as_deref(),
            Some("service worker waitUntil promise rejected")
        );
    }

    #[test]
    fn activate_completion_does_not_clear_newer_installing_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let previous_active_version_id = ServiceWorkerVersionId(1);
        let activating_version_id = ServiceWorkerVersionId(2);
        let installing_version_id = ServiceWorkerVersionId(3);
        let scope_url = url("https://example.test/app/");
        let previous_script_url = url("https://example.test/app/worker-v1.js");
        let activating_script_url = url("https://example.test/app/worker-v2.js");
        let installing_script_url = url("https://example.test/app/worker-v3.js");
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: installing_script_url.clone(),
                    installing_version_id: Some(installing_version_id),
                    waiting_version_id: Some(activating_version_id),
                    active_version_id: Some(previous_active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            for (version_id, script_url, lifecycle_state, in_flight_event_count) in [
                (
                    previous_active_version_id,
                    previous_script_url,
                    ServiceWorkerVersionLifecycleState::Activated,
                    0,
                ),
                (
                    activating_version_id,
                    activating_script_url,
                    ServiceWorkerVersionLifecycleState::Activating,
                    1,
                ),
                (
                    installing_version_id,
                    installing_script_url,
                    ServiceWorkerVersionLifecycleState::Installing,
                    0,
                ),
            ] {
                state.versions.insert(
                    version_id,
                    ServiceWorkerVersion {
                        id: version_id,
                        registration_id,
                        script_url: script_url.clone(),
                        final_script_url: Some(script_url.clone()),
                        main_script_resource: None,
                        imported_script_resources: Default::default(),
                        allow_identical_script_update: true,
                        should_pause_on_start_for_devtools: false,
                        script_kind: WorkerScriptKind::Classic,
                        fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                        fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                        launch_config: test_launch_config(&service, &script_url, &scope_url),
                        lifecycle_state,
                        running_state: ServiceWorkerVersionRunningState::Stopped,
                        pending_start_events: VecDeque::new(),
                        pending_activation_fetch_events: VecDeque::new(),
                        in_flight_event_count,
                        run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                        idle_timeout_token: None,
                        skip_waiting_requested: false,
                        clients_claim_requested: false,
                        last_start_error: None,
                    },
                );
            }
        }
        let activating_run = exact_version_run(&service, activating_version_id);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(activating_version_id, &activating_run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(
            registration.installing_version_id,
            Some(installing_version_id)
        );
        assert_eq!(registration.waiting_version_id, None);
        assert_eq!(registration.active_version_id, Some(activating_version_id));
        assert_eq!(
            state
                .versions
                .get(&previous_active_version_id)
                .unwrap()
                .lifecycle_state,
            ServiceWorkerVersionLifecycleState::Redundant
        );
        assert_eq!(
            state
                .versions
                .get(&installing_version_id)
                .unwrap()
                .lifecycle_state,
            ServiceWorkerVersionLifecycleState::Installing
        );
    }

    #[test]
    fn activate_completion_does_not_clear_newer_waiting_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let previous_active_version_id = ServiceWorkerVersionId(1);
        let activating_version_id = ServiceWorkerVersionId(2);
        let waiting_version_id = ServiceWorkerVersionId(3);
        let scope_url = url("https://example.test/app/");
        let previous_script_url = url("https://example.test/app/worker-v1.js");
        let activating_script_url = url("https://example.test/app/worker-v2.js");
        let waiting_script_url = url("https://example.test/app/worker-v3.js");
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: waiting_script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: Some(waiting_version_id),
                    active_version_id: Some(previous_active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            for (version_id, script_url, lifecycle_state, in_flight_event_count) in [
                (
                    previous_active_version_id,
                    previous_script_url,
                    ServiceWorkerVersionLifecycleState::Activated,
                    0,
                ),
                (
                    activating_version_id,
                    activating_script_url,
                    ServiceWorkerVersionLifecycleState::Activating,
                    1,
                ),
                (
                    waiting_version_id,
                    waiting_script_url,
                    ServiceWorkerVersionLifecycleState::Installed,
                    0,
                ),
            ] {
                state.versions.insert(
                    version_id,
                    ServiceWorkerVersion {
                        id: version_id,
                        registration_id,
                        script_url: script_url.clone(),
                        final_script_url: Some(script_url.clone()),
                        main_script_resource: None,
                        imported_script_resources: Default::default(),
                        allow_identical_script_update: true,
                        should_pause_on_start_for_devtools: false,
                        script_kind: WorkerScriptKind::Classic,
                        fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                        fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                        launch_config: test_launch_config(&service, &script_url, &scope_url),
                        lifecycle_state,
                        running_state: ServiceWorkerVersionRunningState::Stopped,
                        pending_start_events: VecDeque::new(),
                        pending_activation_fetch_events: VecDeque::new(),
                        in_flight_event_count,
                        run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                        idle_timeout_token: None,
                        skip_waiting_requested: false,
                        clients_claim_requested: false,
                        last_start_error: None,
                    },
                );
            }
        }
        let activating_run = exact_version_run(&service, activating_version_id);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(activating_version_id, &activating_run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.installing_version_id, None);
        assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
        assert_eq!(registration.active_version_id, Some(activating_version_id));
        assert_eq!(
            state
                .versions
                .get(&previous_active_version_id)
                .unwrap()
                .lifecycle_state,
            ServiceWorkerVersionLifecycleState::Redundant
        );
        assert_eq!(
            state
                .versions
                .get(&waiting_version_id)
                .unwrap()
                .lifecycle_state,
            ServiceWorkerVersionLifecycleState::Installed
        );
    }

    #[test]
    fn pending_ready_job_waits_for_target_registration_activation() {
        let service = new_service_worker_runtime_service();
        let root_registration_id = ServiceWorkerRegistrationId(1);
        let root_version_id = ServiceWorkerVersionId(1);
        let admin_registration_id = ServiceWorkerRegistrationId(2);
        let admin_version_id = ServiceWorkerVersionId(2);
        let root_scope_url = url("https://example.test/app/");
        let admin_scope_url = url("https://example.test/app/admin/");
        let document_url = url("https://example.test/app/admin/page.html");
        insert_inactive_registration(
            &service,
            root_registration_id,
            root_version_id,
            url("https://example.test/app/root-sw.js"),
            root_scope_url,
        );
        insert_inactive_registration(
            &service,
            admin_registration_id,
            admin_version_id,
            url("https://example.test/app/admin/sw.js"),
            admin_scope_url.clone(),
        );

        let mut ready_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        assert!(service.watch_ready_registration(document_url, 91, 1, ready_queue.sender(),));
        assert!(!ready_queue.has_ready_task());

        {
            let mut state = service.inner.state.lock();
            state
                .registrations
                .get_mut(&root_registration_id)
                .unwrap()
                .installing_version_id = None;
            state
                .registrations
                .get_mut(&root_registration_id)
                .unwrap()
                .waiting_version_id = Some(root_version_id);
            let root_version = state.versions.get_mut(&root_version_id).unwrap();
            root_version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
            root_version.in_flight_event_count = 1;
        }
        let root_run = exact_version_run(&service, root_version_id);
        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(root_version_id, &root_run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });
        assert!(
            !ready_queue.has_ready_task(),
            "broader scope activation must not resolve ready job for narrower scope"
        );

        {
            let mut state = service.inner.state.lock();
            state
                .registrations
                .get_mut(&admin_registration_id)
                .unwrap()
                .installing_version_id = None;
            state
                .registrations
                .get_mut(&admin_registration_id)
                .unwrap()
                .waiting_version_id = Some(admin_version_id);
            let admin_version = state.versions.get_mut(&admin_version_id).unwrap();
            admin_version.lifecycle_state = ServiceWorkerVersionLifecycleState::Activating;
            admin_version.in_flight_event_count = 1;
        }
        let admin_run = exact_version_run(&service, admin_version_id);
        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(3),
            owner: test_run_owner(admin_version_id, &admin_run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        let completion = match ready_queue.pop_internal() {
            Some(crate::page_task_queue::RendererServiceWorkerInternalTask::Ready(completion)) => {
                completion
            }
            other => panic!("expected ready completion, got {other:?}"),
        };
        assert_eq!(completion.request_id, 91);
        assert_eq!(completion.registration.scope_url(), &admin_scope_url);
    }

    #[test]
    fn update_install_waits_when_existing_active_version_has_not_been_skipped() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let installing_version_id = ServiceWorkerVersionId(2);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let installing_script_url = url("https://example.test/app/worker-v2.js");
        let installing_run = RendererServiceWorkerRunIdentity::fresh();
        let host = new_running_test_host(installing_version_id, &installing_run);
        let (force_update_tx, mut force_update_rx) = tokio::sync::oneshot::channel();
        {
            let mut state = service.inner.state.lock();
            let client_id = ServiceWorkerClientId::from_u64_for_test(1);
            state.insert_force_update_page_load_waiter(1, force_update_tx);
            state.bind_force_update_page_load_waiters(installing_version_id, vec![1]);
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    creation_url: scope_url.clone(),
                    document_url: scope_url.clone(),
                    client_type: ServiceWorkerClientType::Window,
                    frame_type: ServiceWorkerClientFrameType::TopLevel,
                    visibility_state: ServiceWorkerClientVisibilityState::Visible,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    secure_context: true,
                    execution_ready: true,
                    discarded_or_frozen: false,
                    document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
                    endpoint: ServiceWorkerClientEndpoint::Page(test_completion_sender()),
                    focused: false,
                },
            );
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: installing_script_url.clone(),
                    installing_version_id: Some(installing_version_id),
                    waiting_version_id: None,
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                installing_version_id,
                ServiceWorkerVersion {
                    id: installing_version_id,
                    registration_id,
                    script_url: installing_script_url.clone(),
                    final_script_url: Some(installing_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &installing_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: installing_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(installing_version_id, &installing_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        {
            let state = service.inner.state.lock();
            let registration = state.registrations.get(&registration_id).unwrap();
            assert_eq!(registration.active_version_id, Some(active_version_id));
            assert_eq!(registration.waiting_version_id, Some(installing_version_id));
            let installed = state.versions.get(&installing_version_id).unwrap();
            assert_eq!(
                installed.lifecycle_state,
                ServiceWorkerVersionLifecycleState::Installed
            );
            assert_eq!(installed.in_flight_event_count, 0);
            let active = state.versions.get(&active_version_id).unwrap();
            assert_eq!(
                active.lifecycle_state,
                ServiceWorkerVersionLifecycleState::Activated
            );
        }

        service.finish_worker_skip_waiting_requested(registration_id, installing_version_id);

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert_eq!(registration.waiting_version_id, Some(installing_version_id));
        let activating = state.versions.get(&installing_version_id).unwrap();
        assert_eq!(
            activating.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activating
        );
        assert!(activating.skip_waiting_requested);
        assert_eq!(activating.in_flight_event_count, 1);
        assert_eq!(
            force_update_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty),
            "force-update page load must keep waiting until activation settles"
        );
        drop(state);

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(installing_version_id, &installing_run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(installing_version_id));
        assert_eq!(
            state
                .versions
                .get(&active_version_id)
                .unwrap()
                .lifecycle_state,
            ServiceWorkerVersionLifecycleState::Redundant
        );
        assert_eq!(force_update_rx.try_recv(), Ok(()));
    }

    #[test]
    fn force_update_install_skips_waiting_even_with_active_controllee() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let installing_version_id = ServiceWorkerVersionId(2);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let installing_script_url = url("https://example.test/app/worker-v2.js");
        let installing_run = RendererServiceWorkerRunIdentity::fresh();
        let host = new_running_test_host(installing_version_id, &installing_run);
        {
            let mut state = service.inner.state.lock();
            let client_id = ServiceWorkerClientId::from_u64_for_test(1);
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    creation_url: url("https://example.test/app/page.html"),
                    document_url: url("https://example.test/app/page.html"),
                    client_type: ServiceWorkerClientType::Window,
                    frame_type: ServiceWorkerClientFrameType::TopLevel,
                    visibility_state: ServiceWorkerClientVisibilityState::Visible,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    secure_context: true,
                    execution_ready: true,
                    discarded_or_frozen: false,
                    document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
                    endpoint: ServiceWorkerClientEndpoint::Page(test_completion_sender()),
                    focused: false,
                },
            );
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: installing_script_url.clone(),
                    installing_version_id: Some(installing_version_id),
                    waiting_version_id: None,
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::from([(
                        installing_version_id,
                        ServiceWorkerPendingRegisterJob::new_with_options(Vec::new(), true),
                    )]),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                installing_version_id,
                ServiceWorkerVersion {
                    id: installing_version_id,
                    registration_id,
                    script_url: installing_script_url.clone(),
                    final_script_url: Some(installing_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &installing_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: installing_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(installing_version_id, &installing_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert_eq!(registration.waiting_version_id, Some(installing_version_id));
        let activating = state.versions.get(&installing_version_id).unwrap();
        assert_eq!(
            activating.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activating
        );
        assert!(activating.skip_waiting_requested);
        assert_eq!(activating.in_flight_event_count, 1);
    }

    #[test]
    fn update_install_activates_when_existing_active_has_no_controllees() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let installing_version_id = ServiceWorkerVersionId(2);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let installing_script_url = url("https://example.test/app/worker-v2.js");
        let installing_run = RendererServiceWorkerRunIdentity::fresh();
        let host = new_running_test_host(installing_version_id, &installing_run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: installing_script_url.clone(),
                    installing_version_id: Some(installing_version_id),
                    waiting_version_id: None,
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                installing_version_id,
                ServiceWorkerVersion {
                    id: installing_version_id,
                    registration_id,
                    script_url: installing_script_url.clone(),
                    final_script_url: Some(installing_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &installing_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installing,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: installing_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(1),
            owner: test_run_owner(installing_version_id, &installing_run),
            kind: ServiceWorkerLifecycleEventKind::Install,
            result: Ok(()),
        });

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert_eq!(registration.waiting_version_id, Some(installing_version_id));
        let installing = state.versions.get(&installing_version_id).unwrap();
        assert_eq!(
            installing.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activating
        );
        assert_eq!(installing.in_flight_event_count, 1);
        drop(state);
        assert_eq!(service.pending_service_lane_event_count(), 1);
    }

    #[test]
    fn stopped_waiting_activation_queues_lifecycle_event_and_starts_worker() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let waiting_script_url = url("https://example.test/app/worker-v2.js");
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: waiting_script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: Some(waiting_version_id),
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Installed,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: true,
                    clients_claim_requested: false,
                    last_start_error: Some("previous idle stop".to_owned()),
                },
            );
            state.record_target_created(
                registration_id,
                waiting_version_id,
                waiting_script_url.clone(),
                scope_url.clone(),
            );
            service.take_target_output_events_for_test();
        }

        let previous_waiting_run = exact_version_run(&service, waiting_version_id);
        let start = {
            let mut state = service.inner.state.lock();
            service
                .try_activate_waiting_version_locked(
                    &mut state,
                    registration_id,
                    waiting_version_id,
                )
                .expect("stopped waiting version should be startable")
        };
        let activation_start_events = service.take_target_output_events_for_test();
        assert!(
            activation_start_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated {
                    version_id: updated_version_id,
                    status: crate::runtime::RendererServiceWorkerVersionStatus::Activating,
                } if *updated_version_id == waiting_version_id.as_u64()
            )),
            "activation start should refresh target status to activating: {activation_start_events:?}"
        );
        let ServiceWorkerLifecycleStart::Start(launch) = start else {
            panic!("expected stopped waiting activation to start worker");
        };
        assert_eq!(launch.params.registration_id, registration_id);
        assert_eq!(launch.params.run_owner.version_id(), waiting_version_id);
        let restarted_run = launch.params.run_owner.cloned_run_identity();
        assert_ne!(restarted_run, previous_waiting_run);
        assert_eq!(launch.params.script_url, waiting_script_url);
        assert_eq!(launch.params.scope_url, scope_url);
        assert_eq!(launch.host.version_id(), waiting_version_id);
        assert_eq!(launch.host.run_identity(), restarted_run);

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
        let waiting = state.versions.get(&waiting_version_id).unwrap();
        assert_eq!(
            waiting.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activating
        );
        assert_eq!(waiting.run, restarted_run);
        assert_eq!(waiting.in_flight_event_count, 1);
        assert_eq!(waiting.last_start_error, None);
        assert!(matches!(
            waiting.running_state,
            ServiceWorkerVersionRunningState::Starting { .. }
        ));
        assert_eq!(waiting.pending_start_events.len(), 1);
        let ServiceWorkerPendingStartEvent::Lifecycle(event) =
            waiting.pending_start_events.front().unwrap()
        else {
            panic!("expected pending activate lifecycle event");
        };
        assert_eq!(
            event.owner,
            test_run_owner(waiting_version_id, &restarted_run)
        );
        assert_eq!(event.kind, ServiceWorkerLifecycleEventKind::Activate);
    }

    #[test]
    fn activate_completion_queues_service_worker_target_activated_version_update() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(2);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/worker.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: Some(version_id),
                    active_version_id: None,
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activating,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.record_target_created(
                registration_id,
                version_id,
                script_url.clone(),
                scope_url.clone(),
            );
            service.take_target_output_events_for_test();
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(7),
            owner: test_run_owner(version_id, &run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        let activation_events = service.take_target_output_events_for_test();
        assert!(
            activation_events.iter().any(|event| matches!(
                event,
                crate::runtime::RendererServiceWorkerTargetEvent::VersionUpdated {
                    version_id: updated_version_id,
                    status: crate::runtime::RendererServiceWorkerVersionStatus::Activated,
                } if *updated_version_id == version_id.as_u64()
            )),
            "activate completion should refresh target status to activated: {activation_events:?}"
        );
    }

    #[test]
    fn stale_start_completion_does_not_dispatch_pending_lifecycle_activation() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let waiting_script_url = url("https://example.test/app/worker-v2.js");
        let event_id = ServiceWorkerEventId(7);
        let host = new_loading_test_host(waiting_version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: waiting_script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: Some(waiting_version_id),
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activating,
                    running_state: ServiceWorkerVersionRunningState::Starting { host },
                    pending_start_events: VecDeque::from([
                        ServiceWorkerPendingStartEvent::Lifecycle(ServiceWorkerLifecycleEvent {
                            event_id,
                            owner: test_run_owner(waiting_version_id, &run),
                            kind: ServiceWorkerLifecycleEventKind::Activate,
                        }),
                    ]),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: true,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        let stale_run = RendererServiceWorkerRunIdentity::fresh();
        assert_ne!(stale_run, run);
        service.finish_worker_start_completed(
            waiting_version_id,
            stale_run,
            waiting_script_url.to_string(),
        );

        assert_eq!(service.pending_service_lane_event_count(), 0);
        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(active_version_id));
        assert_eq!(registration.waiting_version_id, Some(waiting_version_id));
        let waiting = state.versions.get(&waiting_version_id).unwrap();
        assert_eq!(
            waiting.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activating
        );
        assert_eq!(waiting.in_flight_event_count, 1);
        assert_eq!(waiting.pending_start_events.len(), 1);
        assert!(matches!(
            waiting.running_state,
            ServiceWorkerVersionRunningState::Starting { .. }
        ));
    }

    #[test]
    fn skip_waiting_update_activation_replaces_previous_active_version() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let waiting_script_url = url("https://example.test/app/worker-v2.js");
        let waiting_run = RendererServiceWorkerRunIdentity::fresh();
        let mut client_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let client_id = ServiceWorkerClientId::from_u64_for_test(1);
        {
            let mut state = service.inner.state.lock();
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    creation_url: url("https://example.test/app/page.html"),
                    document_url: url("https://example.test/app/page.html"),
                    client_type: ServiceWorkerClientType::Window,
                    frame_type: ServiceWorkerClientFrameType::TopLevel,
                    visibility_state: ServiceWorkerClientVisibilityState::Visible,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    secure_context: true,
                    execution_ready: true,
                    discarded_or_frozen: false,
                    document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
                    endpoint: ServiceWorkerClientEndpoint::Page(client_queue.sender()),
                    focused: false,
                },
            );
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: waiting_script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: Some(waiting_version_id),
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activating,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: waiting_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: true,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(waiting_version_id, &waiting_run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        let state = service.inner.state.lock();
        let registration = state.registrations.get(&registration_id).unwrap();
        assert_eq!(registration.active_version_id, Some(waiting_version_id));
        assert_eq!(registration.waiting_version_id, None);
        assert_eq!(
            state
                .versions
                .get(&waiting_version_id)
                .unwrap()
                .lifecycle_state,
            ServiceWorkerVersionLifecycleState::Activated
        );
        assert_eq!(
            state
                .versions
                .get(&active_version_id)
                .unwrap()
                .lifecycle_state,
            ServiceWorkerVersionLifecycleState::Redundant
        );
        drop(state);
        let completion = match client_queue.pop_internal() {
            Some(crate::page_task_queue::RendererServiceWorkerInternalTask::ControllerChange(
                completion,
            )) => completion,
            other => panic!("expected replacement controllerchange completion, got {other:?}"),
        };
        assert_eq!(completion.target.client_id, client_id);
        assert!(!client_queue.has_ready_task());
    }

    #[test]
    fn activating_new_worker_fails_replaced_active_fetch_and_pending_start_events() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let active_run = RendererServiceWorkerRunIdentity::fresh();
        let waiting_run = RendererServiceWorkerRunIdentity::fresh();
        let dispatched_event_id = ServiceWorkerEventId(41);
        let pending_event_id = ServiceWorkerEventId(42);
        let activation_wait_event_id = ServiceWorkerEventId(43);
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/worker-v1.js");
        let waiting_script_url = url("https://example.test/app/worker-v2.js");
        let document_url = url("https://example.test/app/page.html");
        let request_url = url("https://example.test/app/data.txt");
        let client_id = ServiceWorkerClientId::from_u64_for_test(1);
        let active_host = new_running_test_host(active_version_id, &active_run);
        let mut completion_queue = async_subresource_completion_queue();
        {
            let mut state = service.inner.state.lock();
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    creation_url: document_url.clone(),
                    document_url: document_url.clone(),
                    client_type: ServiceWorkerClientType::Window,
                    frame_type: ServiceWorkerClientFrameType::TopLevel,
                    visibility_state: ServiceWorkerClientVisibilityState::Visible,
                    storage_key: ServiceWorkerRegistrationKey::first_party_storage_key_for_url(
                        &document_url,
                    ),
                    secure_context: true,
                    execution_ready: true,
                    discarded_or_frozen: false,
                    document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(0)),
                    endpoint: ServiceWorkerClientEndpoint::Page(test_completion_sender()),
                    focused: false,
                },
            );
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: waiting_script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: Some(waiting_version_id),
                    active_version_id: Some(active_version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            for (event_id, internal_id) in [
                (dispatched_event_id, 201),
                (pending_event_id, 202),
                (activation_wait_event_id, 203),
            ] {
                state.pending_fetch_jobs.insert(
                    event_id,
                    ServiceWorkerFetchJob {
                        internal_id,
                        owner: Some(test_run_owner(active_version_id, &active_run)),
                        request_url: request_url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        request_body_bytes: None,
                        cors_preflight_request_headers: Vec::new(),
                        client_id: ServiceWorkerClientId::from_u64_for_test(0),
                        resulting_client_id: None,
                        destination: ServiceWorkerRequestDestination::Empty,
                        is_reload: false,
                        metadata: Default::default(),
                        request_mode: moli_fetch::RequestMode::Cors,
                        credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                        priority: None,
                        redirect_chain: Vec::new(),
                        redirect_count: 0,
                        request_cookie_report: None,
                        network_context: AsyncSubresourceNetworkContext {
                            frame_id: None,
                            document_url: document_url.clone(),
                            resource_type: crate::types::SubresourceResourceType::Fetch,
                            policy_context: Default::default(),
                        },
                        completion_tx: completion_queue.sender(),
                        request_client: test_request_client(&service),
                        resource_task_runner: test_resource_task_runner(),
                        cancel_handle: moli_fetch::FetchCancelHandle::new(),
                        navigation_preload_cancel_handle: None,
                        streaming_body_source_id: None,
                        direct_completion_tx: None,
                    },
                );
            }
            state.versions.insert(
                active_version_id,
                ServiceWorkerVersion {
                    id: active_version_id,
                    registration_id,
                    script_url: active_script_url.clone(),
                    final_script_url: Some(active_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &active_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host: active_host },
                    pending_start_events: VecDeque::from([ServiceWorkerPendingStartEvent::Fetch(
                        ServiceWorkerFetchEvent {
                            event_id: pending_event_id,
                            owner: test_run_owner(active_version_id, &active_run),
                            request: ServiceWorkerFetchRequest {
                                client_id,
                                resulting_client_id: None,
                                url: request_url.clone(),
                                method: "GET".to_owned(),
                                headers: Vec::new(),
                                body: None,
                                destination: ServiceWorkerRequestDestination::Empty,
                                request_mode: moli_fetch::RequestMode::Cors,
                                credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                                redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                                priority: None,
                                is_reload: false,
                                metadata: Default::default(),
                            },
                            navigation_preload_sent: false,
                        },
                    )]),
                    pending_activation_fetch_events: VecDeque::from([ServiceWorkerFetchEvent {
                        event_id: activation_wait_event_id,
                        owner: test_run_owner(active_version_id, &active_run),
                        request: ServiceWorkerFetchRequest {
                            client_id,
                            resulting_client_id: None,
                            url: request_url,
                            method: "GET".to_owned(),
                            headers: Vec::new(),
                            body: None,
                            destination: ServiceWorkerRequestDestination::Document,
                            request_mode: moli_fetch::RequestMode::Navigate,
                            credentials_mode: moli_fetch::RequestCredentialsMode::Include,
                            redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                            priority: None,
                            is_reload: false,
                            metadata: Default::default(),
                        },
                        navigation_preload_sent: false,
                    }]),
                    in_flight_event_count: 2,
                    run: active_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activating,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: waiting_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: true,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(waiting_version_id, &waiting_run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        {
            let state = service.inner.state.lock();
            assert!(state.pending_fetch_jobs.is_empty());
            let previous_active = state.versions.get(&active_version_id).unwrap();
            assert_eq!(
                previous_active.lifecycle_state,
                ServiceWorkerVersionLifecycleState::Redundant
            );
            assert!(matches!(
                previous_active.running_state,
                ServiceWorkerVersionRunningState::Stopped
            ));
            assert!(previous_active.pending_start_events.is_empty());
            assert!(previous_active.pending_activation_fetch_events.is_empty());
            assert_eq!(previous_active.in_flight_event_count, 0);
            let waiting = state.versions.get(&waiting_version_id).unwrap();
            assert_eq!(
                waiting.lifecycle_state,
                ServiceWorkerVersionLifecycleState::Activated
            );
            assert_eq!(waiting.in_flight_event_count, 0);
        }

        let mut results = Vec::new();
        for _ in 0..3 {
            let completion = pop_async_subresource_completion(&mut completion_queue);
            results.push((
                completion.internal_id,
                completion.result.err().unwrap_or_default(),
            ));
        }
        results.sort_by_key(|(internal_id, _)| *internal_id);
        assert_eq!(
            results,
            vec![
                (
                    201,
                    "service worker was replaced by a newer active worker".to_owned(),
                ),
                (
                    202,
                    "service worker was replaced by a newer active worker".to_owned(),
                ),
                (
                    203,
                    "service worker was replaced by a newer active worker".to_owned(),
                ),
            ]
        );
        assert!(!completion_queue.has_ready_completion());
    }

    #[test]
    fn activating_new_worker_destroys_replaced_active_target() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let active_version_id = ServiceWorkerVersionId(1);
        let waiting_version_id = ServiceWorkerVersionId(2);
        let active_run = RendererServiceWorkerRunIdentity::fresh();
        let waiting_run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let active_script_url = url("https://example.test/app/sw-v1.js");
        let waiting_script_url = url("https://example.test/app/sw-v2.js");
        insert_registered_version(
            &service,
            registration_id,
            active_version_id,
            active_script_url.clone(),
            scope_url.clone(),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let active_host = new_running_test_host(active_version_id, &active_run);
            let active = state
                .versions
                .get_mut(&active_version_id)
                .expect("active version should exist");
            active.run = active_run.clone();
            active.running_state = ServiceWorkerVersionRunningState::Running { host: active_host };
            state.record_target_created(
                registration_id,
                active_version_id,
                active_script_url,
                scope_url.clone(),
            );
            service.take_target_output_events_for_test();
            state
                .registrations
                .get_mut(&registration_id)
                .expect("registration should exist")
                .waiting_version_id = Some(waiting_version_id);
            state.versions.insert(
                waiting_version_id,
                ServiceWorkerVersion {
                    id: waiting_version_id,
                    registration_id,
                    script_url: waiting_script_url.clone(),
                    final_script_url: Some(waiting_script_url.clone()),
                    main_script_resource: Some(test_script_resource(&waiting_script_url)),
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &waiting_script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activating,
                    running_state: ServiceWorkerVersionRunningState::Stopped,
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: waiting_run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: true,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_lifecycle_event_completed(ServiceWorkerLifecycleCompletion {
            event_id: ServiceWorkerEventId(2),
            owner: test_run_owner(waiting_version_id, &waiting_run),
            kind: ServiceWorkerLifecycleEventKind::Activate,
            result: Ok(()),
        });

        let target_events = service.take_target_output_events_for_test();
        assert_eq!(target_events.len(), 2);
        let crate::runtime::RendererServiceWorkerTargetEvent::Stopped {
            version_id,
            run: _,
            reason,
        } = &target_events[0]
        else {
            panic!("active replacement must first retire its exact run: {target_events:?}");
        };
        assert_eq!(*version_id, active_version_id.as_u64());
        assert_eq!(reason, "replaced_by_newer_active_worker");
        let crate::runtime::RendererServiceWorkerTargetEvent::Destroyed {
            version_id,
            active_run,
        } = &target_events[1]
        else {
            panic!("active replacement must then destroy its version: {target_events:?}");
        };
        assert_eq!(*version_id, active_version_id.as_u64());
        assert!(
            active_run.is_none(),
            "the preceding stop must leave no live run on version destruction"
        );
        let state = service.inner.state.lock();
        assert_eq!(
            state
                .versions
                .get(&active_version_id)
                .expect("replaced version should remain diagnosable")
                .lifecycle_state,
            ServiceWorkerVersionLifecycleState::Redundant
        );
        assert!(
            !state
                .service_worker_target_infos
                .contains_key(&active_version_id),
            "doomed active version should no longer be exposed as a Service Worker target"
        );
    }

    #[test]
    fn stopped_active_message_queues_event_and_starts_worker() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/sw.js"),
            url("https://example.test/app/"),
            [],
        );
        let client_id =
            register_client_for_test(&service, url("https://example.test/app/page.html"));
        {
            let mut state = service.inner.state.lock();
            state.live_clients.get_mut(&client_id).unwrap().focused = true;
        }
        let previous_run = exact_version_run(&service, version_id);

        assert!(service.dispatch_message_to_version(
            version_id,
            client_id,
            Some("https://example.test".to_owned()),
            V8StructuredClonePayload::default()
        ));

        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert!(matches!(
            version.running_state,
            ServiceWorkerVersionRunningState::Starting { .. }
        ));
        assert_ne!(version.run, previous_run);
        let restarted_run = version.run.clone();
        assert_eq!(version.in_flight_event_count, 1);
        assert_eq!(version.pending_start_events.len(), 1);
        let ServiceWorkerPendingStartEvent::Message(event) =
            version.pending_start_events.front().unwrap()
        else {
            panic!("expected pending message event");
        };
        assert_eq!(event.owner, test_run_owner(version_id, &restarted_run));
        assert_eq!(event.source_client_id, Some(client_id));
        assert_eq!(
            event.source_client_snapshot,
            Some(ServiceWorkerClientSnapshot {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
                url: url("https://example.test/app/page.html"),
                client_type: ServiceWorkerClientType::Window,
                frame_type: ServiceWorkerClientFrameType::TopLevel,
                visibility_state: ServiceWorkerClientVisibilityState::Visible,
                controlled: true,
                focused: true,
            })
        );
    }

    #[test]
    fn message_to_redundant_or_missing_version_is_dropped_silently() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/sw.js"),
            url("https://example.test/app/"),
            [],
        );
        {
            let mut state = service.inner.state.lock();
            let version = state.versions.get_mut(&version_id).unwrap();
            version.lifecycle_state = ServiceWorkerVersionLifecycleState::Redundant;
        }
        let client_id =
            register_client_for_test(&service, url("https://example.test/app/page.html"));

        assert!(!service.dispatch_message_to_version(
            ServiceWorkerVersionId(404),
            client_id,
            Some("https://example.test".to_owned()),
            V8StructuredClonePayload::default()
        ));
        assert!(!service.dispatch_message_to_version(
            version_id,
            client_id,
            Some("https://example.test".to_owned()),
            V8StructuredClonePayload::default()
        ));

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.in_flight_event_count, 0);
        assert_eq!(diagnostics.pending_service_lane_event_count, 0);
    }

    #[test]
    fn message_completion_releases_event_accounting_and_can_idle_stop() {
        let service = new_service_worker_runtime_service();
        service.set_idle_delay_for_test(Duration::ZERO);
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 1,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        service.finish_message_event_completed(ServiceWorkerMessageCompletion {
            event_id: ServiceWorkerEventId(7),
            owner: test_run_owner(version_id, &run),
            result: Ok(()),
        });

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.in_flight_event_count, 0);
        assert_eq!(diagnostics.running_version_count, 1);
        assert_eq!(diagnostics.pending_service_lane_event_count, 1);
        assert_eq!(service.drain_service_lane(), 1);
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.running_version_count, 0);
        assert_eq!(diagnostics.stopped_version_count, 1);
    }

    #[test]
    fn show_notification_record_enables_scope_click_dispatch() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(!service.dispatch_notification_click_for_scope(
            &scope_url,
            "hello".to_owned(),
            "open".to_owned(),
        ));

        assert!(service.show_notification_for_scope(
            &scope_url,
            "hello".to_owned(),
            String::new(),
            ServiceWorkerNotificationMetadata::default(),
            Vec::new(),
            V8StructuredClonePayload::default(),
        ));
        {
            let state = service.inner.state.lock();
            assert_eq!(state.notification_records.len(), 1);
            assert_eq!(
                state.notification_records[0].registration_id,
                registration_id
            );
            assert_eq!(state.notification_records[0].title, "hello");
        }

        assert!(service.dispatch_notification_click_for_scope(
            &scope_url,
            "hello".to_owned(),
            "open".to_owned(),
        ));
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
    }

    #[test]
    fn push_dispatch_for_active_scope_uses_service_lane_completion() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(service.dispatch_push_for_scope(&scope_url, Some(b"payload".to_vec())));
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
        }

        assert_eq!(service.drain_service_lane(), 1);
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert_eq!(
            version.last_start_error.as_deref(),
            Some("service worker push dispatch failed: worker is not running")
        );
    }

    #[test]
    fn push_subscription_store_tracks_active_scope_subscription() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(service.push_subscription_for_scope(&scope_url).is_none());
        let subscription = service
            .subscribe_push_for_scope(&scope_url, true)
            .expect("active registration should accept push subscription");
        assert_eq!(
            subscription.endpoint,
            "https://moli.invalid/service-worker/push/1"
        );
        assert!(subscription.user_visible_only);
        assert_eq!(
            service.push_subscription_for_scope(&scope_url),
            Some(subscription)
        );
        assert!(service.unsubscribe_push_for_scope(&scope_url));
        assert!(service.push_subscription_for_scope(&scope_url).is_none());
        assert!(!service.unsubscribe_push_for_scope(&scope_url));
        assert!(
            service
                .subscribe_push_for_scope(&url("https://example.test/other/"), true)
                .is_none()
        );
    }

    #[test]
    fn sync_register_for_active_scope_retains_tag_until_success_or_last_chance_failure() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(service.register_sync_for_scope(&scope_url, "sync-tag".to_owned()));
        assert_eq!(service.sync_tags_for_scope(&scope_url), vec!["sync-tag"]);
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
        }

        assert_eq!(service.drain_service_lane(), 1);
        assert_eq!(
            service.sync_tags_for_scope(&scope_url),
            vec!["sync-tag"],
            "a failed first sync attempt should keep the registration for retry"
        );
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(
            version.in_flight_event_count, 1,
            "first failure should immediately schedule a lastChance retry"
        );
        assert_eq!(
            version.last_start_error.as_deref(),
            Some("service worker sync dispatch failed: worker is not running")
        );
        assert_eq!(
            state
                .sync_registrations
                .get(&(registration_id, "sync-tag".to_owned()))
                .map(|record| record.failed_attempts),
            Some(1)
        );
        drop(state);
        assert_eq!(service.pending_service_lane_event_count(), 1);

        assert_eq!(service.drain_service_lane(), 1);
        assert!(service.sync_tags_for_scope(&scope_url).is_empty());
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert_eq!(
            version.last_start_error.as_deref(),
            Some("service worker sync dispatch failed: worker is not running")
        );
    }

    #[test]
    fn sync_registration_request_deduplicates_active_tag_and_refires_after_finish() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let sync_key = (registration_id, "sync-tag".to_owned());
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host: host.clone() },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        for request_id in [1, 2] {
            service.finish_sync_registration_requested(
                ServiceWorkerSyncRegistration {
                    request_id,
                    registration_id,
                    version_id,
                    tag: "sync-tag".to_owned(),
                },
                run.clone(),
                host.clone(),
            );
        }
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
            let record = state.sync_registrations.get(&sync_key).unwrap();
            assert_eq!(record.failed_attempts, 0);
            assert!(matches!(
                record.dispatch_state,
                ServiceWorkerTagDispatchState::Active {
                    refire_after_finish: true,
                    ..
                }
            ));
        }
        assert_eq!(
            service.pending_service_lane_event_count(),
            1,
            "duplicate register must not dispatch a second active sync event"
        );

        assert_eq!(service.drain_service_lane(), 1);
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
            let record = state.sync_registrations.get(&sync_key).unwrap();
            assert_eq!(record.failed_attempts, 0);
            assert!(matches!(
                record.dispatch_state,
                ServiceWorkerTagDispatchState::Active {
                    refire_after_finish: false,
                    ..
                }
            ));
        }
        assert_eq!(
            service.pending_service_lane_event_count(),
            1,
            "refire should schedule one normal follow-up sync event"
        );

        assert_eq!(service.drain_service_lane(), 1);
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
            let record = state.sync_registrations.get(&sync_key).unwrap();
            assert_eq!(record.failed_attempts, 1);
            assert!(matches!(
                record.dispatch_state,
                ServiceWorkerTagDispatchState::Active {
                    refire_after_finish: false,
                    ..
                }
            ));
        }
        assert_eq!(
            service.pending_service_lane_event_count(),
            1,
            "the refired sync failure should still get one lastChance retry"
        );

        assert_eq!(service.drain_service_lane(), 1);
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert!(!state.sync_registrations.contains_key(&sync_key));
    }

    #[test]
    fn sync_retry_marks_failed_registration_as_last_chance_event() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Starting { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.sync_registrations.insert(
                (registration_id, "sync-tag".to_owned()),
                ServiceWorkerSyncRegistrationRecord {
                    failed_attempts: 1,
                    ..Default::default()
                },
            );
        }

        assert!(service.retry_sync_for_scope(&scope_url, "sync-tag"));
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
        let Some(ServiceWorkerPendingStartEvent::Sync(event)) = version.pending_start_events.back()
        else {
            panic!("expected queued sync retry event");
        };
        assert_eq!(event.tag, "sync-tag");
        assert!(event.last_chance);
        assert_eq!(event.owner, test_run_owner(version_id, &run));
    }

    #[test]
    fn periodic_sync_register_get_tags_and_unregister_use_owner_store() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let scope_url = url("https://example.test/app/");
        insert_registered_version(
            &service,
            registration_id,
            version_id,
            url("https://example.test/app/sw.js"),
            scope_url.clone(),
            [],
        );

        assert!(service.register_periodic_sync_for_scope(
            &scope_url,
            "daily".to_owned(),
            86_400_000,
        ));
        assert!(service.register_periodic_sync_for_scope(
            &scope_url,
            "hourly".to_owned(),
            3_600_000,
        ));
        assert_eq!(
            service.periodic_sync_tags_for_scope(&scope_url),
            vec!["daily", "hourly"]
        );

        assert!(service.register_periodic_sync_for_scope(
            &scope_url,
            "daily".to_owned(),
            172_800_000,
        ));
        {
            let state = service.inner.state.lock();
            assert_eq!(
                state
                    .periodic_sync_registrations
                    .get(&(registration_id, "daily".to_owned()))
                    .map(|record| record.min_interval_ms),
                Some(172_800_000)
            );
        }

        assert!(service.unregister_periodic_sync_for_scope(&scope_url, "daily"));
        assert_eq!(
            service.periodic_sync_tags_for_scope(&scope_url),
            vec!["hourly"]
        );

        let inactive_scope = url("https://example.test/inactive/");
        insert_inactive_registration(
            &service,
            ServiceWorkerRegistrationId(2),
            ServiceWorkerVersionId(2),
            url("https://example.test/inactive/sw.js"),
            inactive_scope.clone(),
        );
        assert!(!service.register_periodic_sync_for_scope(
            &inactive_scope,
            "inactive".to_owned(),
            1,
        ));
        assert!(
            service
                .periodic_sync_tags_for_scope(&inactive_scope)
                .is_empty()
        );
    }

    #[test]
    fn periodic_sync_dispatch_for_registered_tag_queues_functional_event() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Starting { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(service.register_periodic_sync_for_scope(
            &scope_url,
            "daily".to_owned(),
            86_400_000,
        ));
        assert!(!service.dispatch_periodic_sync_for_scope(&scope_url, "missing"));
        assert!(service.dispatch_periodic_sync_for_scope(&scope_url, "daily"));
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
            let Some(ServiceWorkerPendingStartEvent::PeriodicSync(event)) =
                version.pending_start_events.back()
            else {
                panic!("expected queued periodic sync event");
            };
            assert_eq!(event.registration_id, registration_id);
            assert_eq!(event.owner, test_run_owner(version_id, &run));
            assert_eq!(event.tag, "daily");
        }

        service.finish_worker_start_failed(
            version_id,
            run,
            ServiceWorkerVersionStartFailure::ScriptLoad {
                message: "worker start failed".to_owned(),
            },
        );
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        assert_eq!(
            version.last_start_error.as_deref(),
            Some("periodicsync `daily` failed: worker start failed")
        );
        assert!(
            state
                .periodic_sync_registrations
                .contains_key(&(registration_id, "daily".to_owned())),
            "functional dispatch should not remove the periodic sync registration"
        );
    }

    #[test]
    fn periodic_sync_dispatch_deduplicates_active_tag_and_refires_after_finish() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let periodic_key = (registration_id, "daily".to_owned());
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(service.register_periodic_sync_for_scope(
            &scope_url,
            "daily".to_owned(),
            86_400_000,
        ));
        assert!(service.dispatch_periodic_sync_for_scope(&scope_url, "daily"));
        assert!(service.dispatch_periodic_sync_for_scope(&scope_url, "daily"));
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
            let record = state
                .periodic_sync_registrations
                .get(&periodic_key)
                .unwrap();
            assert!(matches!(
                record.dispatch_state,
                ServiceWorkerTagDispatchState::Active {
                    refire_after_finish: true,
                    ..
                }
            ));
        }
        assert_eq!(
            service.pending_service_lane_event_count(),
            1,
            "duplicate periodic sync dispatch must not enqueue a concurrent event"
        );

        assert_eq!(service.drain_service_lane(), 1);
        {
            let state = service.inner.state.lock();
            let version = state.versions.get(&version_id).unwrap();
            assert_eq!(version.in_flight_event_count, 1);
            let record = state
                .periodic_sync_registrations
                .get(&periodic_key)
                .unwrap();
            assert_eq!(record.min_interval_ms, 86_400_000);
            assert!(matches!(
                record.dispatch_state,
                ServiceWorkerTagDispatchState::Active {
                    refire_after_finish: false,
                    ..
                }
            ));
        }
        assert_eq!(
            service.pending_service_lane_event_count(),
            1,
            "refire should schedule one follow-up periodic sync event"
        );

        assert_eq!(service.drain_service_lane(), 1);
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
        let record = state
            .periodic_sync_registrations
            .get(&periodic_key)
            .unwrap();
        assert!(matches!(
            record.dispatch_state,
            ServiceWorkerTagDispatchState::Idle
        ));
    }

    #[test]
    fn notification_store_filters_replaces_tagged_records_and_closes() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(service.show_notification_for_scope(
            &scope_url,
            "first".to_owned(),
            "same".to_owned(),
            ServiceWorkerNotificationMetadata {
                body: "old body".to_owned(),
                timestamp: Some(111),
                ..ServiceWorkerNotificationMetadata::default()
            },
            Vec::new(),
            V8StructuredClonePayload::default(),
        ));
        assert!(service.show_notification_for_scope(
            &scope_url,
            "second".to_owned(),
            "same".to_owned(),
            ServiceWorkerNotificationMetadata {
                dir: "rtl".to_owned(),
                lang: "fr".to_owned(),
                body: "new body".to_owned(),
                icon: "/icon.png".to_owned(),
                image: "/image.png".to_owned(),
                badge: "/badge.png".to_owned(),
                vibrate: vec![10, 20],
                timestamp: Some(222),
                renotify: true,
                silent: Some(true),
                require_interaction: true,
            },
            vec![ServiceWorkerNotificationAction {
                action: "reply".to_owned(),
                title: "Reply".to_owned(),
                icon: "/reply.png".to_owned(),
                navigate: None,
            }],
            V8StructuredClonePayload::default(),
        ));
        assert!(service.show_notification_for_scope(
            &scope_url,
            "loose".to_owned(),
            String::new(),
            ServiceWorkerNotificationMetadata::default(),
            Vec::new(),
            V8StructuredClonePayload::default(),
        ));

        let all = service.notifications_for_scope(&scope_url, None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].title, "second");
        assert_eq!(all[0].tag, "same");
        assert_eq!(all[0].actions.len(), 1);
        assert_eq!(all[0].actions[0].action, "reply");
        assert_eq!(all[0].actions[0].title, "Reply");
        assert_eq!(all[0].actions[0].icon, "/reply.png");
        assert_eq!(all[0].metadata.dir, "rtl");
        assert_eq!(all[0].metadata.lang, "fr");
        assert_eq!(all[0].metadata.body, "new body");
        assert_eq!(all[0].metadata.icon, "/icon.png");
        assert_eq!(all[0].metadata.image, "/image.png");
        assert_eq!(all[0].metadata.badge, "/badge.png");
        assert_eq!(all[0].metadata.vibrate, vec![10, 20]);
        assert_eq!(all[0].metadata.timestamp, Some(222));
        assert!(all[0].metadata.renotify);
        assert_eq!(all[0].metadata.silent, Some(true));
        assert!(all[0].metadata.require_interaction);
        assert_eq!(all[1].title, "loose");

        let tagged = service.notifications_for_scope(&scope_url, Some("same"));
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].title, "second");
        assert!(service.close_notification(registration_id, tagged[0].id));

        let tagged_after_close = service.notifications_for_scope(&scope_url, Some("same"));
        assert!(tagged_after_close.is_empty());
        let all_after_close = service.notifications_for_scope(&scope_url, None);
        assert_eq!(all_after_close.len(), 1);
        assert_eq!(all_after_close[0].title, "loose");
    }

    #[test]
    fn notification_close_dispatch_removes_record_and_starts_event() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(service.show_notification_for_scope(
            &scope_url,
            "closing".to_owned(),
            "tag".to_owned(),
            ServiceWorkerNotificationMetadata::default(),
            Vec::new(),
            V8StructuredClonePayload::default(),
        ));
        assert_eq!(
            service
                .notifications_for_scope(&scope_url, Some("tag"))
                .len(),
            1
        );
        assert!(service.dispatch_notification_close_for_scope(&scope_url, "closing".to_owned(),));

        assert!(
            service
                .notifications_for_scope(&scope_url, Some("tag"))
                .is_empty()
        );
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 1);
    }

    #[test]
    fn notification_action_navigate_routes_to_page_owner_without_click_event() {
        let service = new_service_worker_runtime_service();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let action_url = url("https://example.test/app/reply.html");
        let host = new_running_test_host(version_id, &run);
        let mut completion_queue =
            crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let client_id = ServiceWorkerClientId(9);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::from([client_id]),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    creation_url: url("https://example.test/app/page.html"),
                    document_url: url("https://example.test/app/page.html"),
                    client_type: ServiceWorkerClientType::Window,
                    frame_type: ServiceWorkerClientFrameType::TopLevel,
                    visibility_state: ServiceWorkerClientVisibilityState::Visible,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    secure_context: true,
                    execution_ready: true,
                    discarded_or_frozen: false,
                    document_owner: Some(crate::native_bridge::WindowDocumentOwner::for_test(7)),
                    endpoint: ServiceWorkerClientEndpoint::Page(completion_queue.sender()),
                    focused: false,
                },
            );
        }

        assert!(service.show_notification_for_scope(
            &scope_url,
            "hello".to_owned(),
            String::new(),
            ServiceWorkerNotificationMetadata::default(),
            vec![ServiceWorkerNotificationAction {
                action: "reply".to_owned(),
                title: "Reply".to_owned(),
                icon: String::new(),
                navigate: Some(action_url.clone()),
            }],
            V8StructuredClonePayload::default(),
        ));
        assert!(service.dispatch_notification_click_for_scope(
            &scope_url,
            "hello".to_owned(),
            "reply".to_owned(),
        ));

        let Some(
            crate::page_task_queue::RendererServiceWorkerInternalTask::NotificationActionNavigateRequest(
                completion,
            ),
        ) = completion_queue.pop_internal()
        else {
            panic!("expected notification action navigate request");
        };
        assert_eq!(completion.host.client_id, client_id);
        assert_eq!(
            completion.host.document_owner,
            crate::native_bridge::WindowDocumentOwner::for_test(7)
        );
        assert_eq!(completion.url, action_url);
        let state = service.inner.state.lock();
        let version = state.versions.get(&version_id).unwrap();
        assert_eq!(version.in_flight_event_count, 0);
    }

    #[test]
    fn notification_click_dispatch_enters_active_version_accounting() {
        let service = new_service_worker_runtime_service();
        service.set_idle_delay_for_test(Duration::ZERO);
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(1);
        let run = RendererServiceWorkerRunIdentity::fresh();
        let scope_url = url("https://example.test/app/");
        let script_url = url("https://example.test/app/sw.js");
        let host = new_running_test_host(version_id, &run);
        {
            let mut state = service.inner.state.lock();
            state.registrations.insert(
                registration_id,
                ServiceWorkerRegistration {
                    id: registration_id,
                    storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(
                        &scope_url,
                    ),
                    scope_url: scope_url.clone(),
                    script_url: script_url.clone(),
                    installing_version_id: None,
                    waiting_version_id: None,
                    active_version_id: Some(version_id),
                    pending_unregistration: false,
                    update_via_cache: ServiceWorkerUpdateViaCache::Imports,
                    navigation_preload_state: ServiceWorkerNavigationPreloadState::default(),
                    last_update_check_time_ms: None,
                    pending_register_jobs: HashMap::new(),
                    controlled_client_ids: HashSet::new(),
                },
            );
            state.versions.insert(
                version_id,
                ServiceWorkerVersion {
                    id: version_id,
                    registration_id,
                    script_url: script_url.clone(),
                    final_script_url: Some(script_url.clone()),
                    main_script_resource: None,
                    imported_script_resources: Default::default(),
                    allow_identical_script_update: true,
                    should_pause_on_start_for_devtools: false,
                    script_kind: WorkerScriptKind::Classic,
                    fetch_handler_existence: ServiceWorkerFetchHandlerExistence::Unknown,
                    fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
                    launch_config: test_launch_config(&service, &script_url, &scope_url),
                    lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
                    running_state: ServiceWorkerVersionRunningState::Running { host },
                    pending_start_events: VecDeque::new(),
                    pending_activation_fetch_events: VecDeque::new(),
                    in_flight_event_count: 0,
                    run: run.clone(),
                    idle_timeout_token: None,
                    skip_waiting_requested: false,
                    clients_claim_requested: false,
                    last_start_error: None,
                },
            );
        }

        assert!(service.dispatch_notification_click_event(
            registration_id,
            1,
            "hello".to_owned(),
            String::new(),
            ServiceWorkerNotificationMetadata::default(),
            Vec::new(),
            "open".to_owned(),
            V8StructuredClonePayload::default(),
        ));

        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.in_flight_event_count, 1);
        assert_eq!(diagnostics.pending_service_lane_event_count, 1);

        assert_eq!(service.drain_service_lane(), 1);
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.in_flight_event_count, 0);
        assert_eq!(diagnostics.running_version_count, 1);
        assert_eq!(diagnostics.pending_service_lane_event_count, 1);

        assert_eq!(service.drain_service_lane(), 1);
        let diagnostics = service.diagnostics_snapshot();
        assert_eq!(diagnostics.running_version_count, 0);
        assert_eq!(diagnostics.stopped_version_count, 1);
    }
}
