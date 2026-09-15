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

fn request_body_text(body: &Option<Vec<u8>>) -> Option<String> {
    body.as_ref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
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
        self.inner.owner_wake.lock().register(sender.clone());
        if self.pending_service_lane_event_count() > 0 {
            let _ = sender.send(ServiceWorkerRuntimeOwnerWake::ServiceLane);
        }
    }

    pub(super) fn signal_service_lane_wake(&self) -> bool {
        self.inner
            .owner_wake
            .lock()
            .broadcast(ServiceWorkerRuntimeOwnerWake::ServiceLane)
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
            request: {
                let origin = moli_url::WebOrigin::from_url(&document_url);
                let initiator = (document_url).clone();
                moli_fetch::Request::new_browser("GET", request_url, None, Vec::new(), origin)
                    .with_initiator_url(&initiator)
                    .with_request_mode(moli_fetch::RequestMode::Cors)
                    .with_credentials_mode(moli_fetch::RequestCredentialsMode::SameOrigin)
                    .with_redirect_mode(moli_fetch::RequestRedirectMode::Follow)
                    .with_fetch_priority_hint(None)
            },
            internal_id,
            owner: Some(test_run_owner(version_id, run)),
            cors_preflight_request_headers: Vec::new(),
            client_id,
            resulting_client_id: None,
            destination: ServiceWorkerRequestDestination::Empty,
            is_reload: false,
            metadata: Default::default(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: moli_url::WebOrigin::from_url(&document_url),
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
            headers: vec![("Content-Type".to_owned(), b"text/javascript".to_vec())],
            body_len: 3,
            body_sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                .to_owned(),
            response_time_ms: 7,
            mime_type: Some("text/javascript".to_owned()),
            classic_script: None,
        }
    }

    fn test_worker_script_resource(script_url: &Url) -> WorkerScriptResource {
        WorkerScriptResource {
            request_url: script_url.clone(),
            final_url: script_url.clone(),
            kind: crate::worker::WorkerScriptResourceKind::JavaScript,
            status: 200,
            headers: vec![("Content-Type".to_owned(), b"text/javascript".to_vec())],
            body_len: 3,
            body_sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                .to_owned(),
            response_time_ms: 7,
            mime_type: Some("text/javascript".to_owned()),
            classic_script: None,
        }
    }

    fn test_loaded_script(script_url: &Url, source: &str) -> LoadedServiceWorkerScript {
        LoadedServiceWorkerScript {
            updated_imports: Default::default(),
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
            update_registration_id: None,
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
            worker_context_runtime: service.browser_context_runtime().worker_context_runtime(),
            broadcast_channel_top_level_site: None,
            indexed_db_manager: None,
            storage_bucket_store: None,
            callbacks: vec![ServiceWorkerRegisterJob::Page {
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
            update_registration_id: None,
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
            worker_context_runtime: browser_context_runtime.worker_context_runtime(),
            broadcast_channel_top_level_site: None,
            indexed_db_manager: None,
            storage_bucket_store: None,
            callbacks: vec![ServiceWorkerRegisterJob::Page {
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
            ServiceWorkerPendingRegisterJob::new(vec![ServiceWorkerRegisterJob::Page {
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
                update_registration_id: None,
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
                worker_context_runtime: service.browser_context_runtime().worker_context_runtime(),
                broadcast_channel_top_level_site: None,
                indexed_db_manager: None,
                storage_bucket_store: None,
                callbacks: vec![ServiceWorkerRegisterJob::Page {
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

    mod extracted;
}
