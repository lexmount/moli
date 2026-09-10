use std::{
    collections::{HashMap, HashSet},
    sync::{
        Weak,
        atomic::{AtomicBool, AtomicU64},
    },
};

use parking_lot::Mutex;
use url::Url;

use crate::{
    network::ResourceRequestClient,
    page_task_queue::{RendererPageServiceWorkerTaskSender, RendererResourceCompletionSender},
    runtime::{
        RendererRuntimeInspectorMessage, RendererServiceWorkerConsoleMessage,
        RendererServiceWorkerExceptionMessage, RendererServiceWorkerFetchDiagnostic,
        RendererServiceWorkerLifecycle, RendererServiceWorkerObservation,
        RendererServiceWorkerRunIdentity, RendererServiceWorkerTargetInfo,
        RendererServiceWorkerVersionStatus, RendererWorkerContextRuntime,
    },
    types::{
        AsyncSubresourceNetworkContext, ServiceWorkerClientMessageCompletion,
        ServiceWorkerControllerChangeCompletion, ServiceWorkerLifecycleClientEvent,
        ServiceWorkerLifecycleNotification, ServiceWorkerReadyCompletion,
        ServiceWorkerWindowClientTarget,
    },
    worker::WorkerMessage,
    worker_owner_wake::WorkerOwnerWakeRoutes,
};

use super::{
    clients::{
        ServiceWorkerClientFrameType, ServiceWorkerClientType, ServiceWorkerClientVisibilityState,
    },
    diagnostics::ServiceWorkerMainScriptUpdateCheckDiagnostics,
    errors::ServiceWorkerRegistrationError,
    events::{
        ServiceWorkerDirectFetchResult, ServiceWorkerFetchRequestMetadata,
        ServiceWorkerLifecycleEvent, ServiceWorkerMessageEvent, ServiceWorkerNotificationEvent,
        ServiceWorkerPeriodicSyncEvent, ServiceWorkerPushEvent,
        ServiceWorkerPushSubscriptionSnapshot, ServiceWorkerRequestDestination,
        ServiceWorkerSyncEvent,
    },
    functional_events::{
        ServiceWorkerNotificationRecord, ServiceWorkerPeriodicSyncRegistrationRecord,
        ServiceWorkerSyncRegistrationRecord,
    },
    host::SharedRendererServiceWorkerHost,
    ids::{
        ServiceWorkerClientId, ServiceWorkerEventId, ServiceWorkerRegistrationId,
        ServiceWorkerVersionId,
    },
    jobs::{
        ServiceWorkerJobCoordinator, ServiceWorkerLaunchParams,
        ServiceWorkerPendingMainScriptUpdateCheck, ServiceWorkerQueuedUnregisterJob,
        ServiceWorkerRegisterJob, ServiceWorkerRegistrationKey,
    },
    owner_wake::ServiceWorkerRuntimeOwnerWake,
    registration::ServiceWorkerRegistration,
    resource_store::{ServiceWorkerStoredRegistration, SharedServiceWorkerResourceStore},
    run_owner::ServiceWorkerRunOwner,
    script_loading::{LoadedServiceWorkerScript, ServiceWorkerScriptUpdateCheckParams},
    service_lane::ServiceWorkerServiceLane,
    snapshots::ServiceWorkerRegistrationSnapshot,
    target_output_streams::ServiceWorkerTargetOutputStreams,
    version::{
        ServiceWorkerIdleTimeout, ServiceWorkerVersion, ServiceWorkerVersionLifecycleState,
        ServiceWorkerVersionRunningState,
    },
};

#[derive(Clone, Debug, Default)]
pub(super) struct WeakServiceWorkerRuntimeService {
    pub(super) inner: Weak<ServiceWorkerRuntimeInner>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ServiceWorkerDevToolsRelatedPauseOnStartPolicy {
    pub(super) registration_id: ServiceWorkerRegistrationId,
    pub(super) base_version_id: ServiceWorkerVersionId,
    pub(super) script_url: Url,
    pub(super) scope_url: Url,
}

impl ServiceWorkerDevToolsRelatedPauseOnStartPolicy {
    pub(super) fn matches(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        script_url: &Url,
        scope_url: &Url,
    ) -> bool {
        self.registration_id == registration_id
            && version_id.as_u64() > self.base_version_id.as_u64()
            && self.script_url == *script_url
            && self.scope_url == *scope_url
    }
}

fn target_status_for_lifecycle_state(
    state: ServiceWorkerVersionLifecycleState,
) -> RendererServiceWorkerVersionStatus {
    match state {
        ServiceWorkerVersionLifecycleState::Installing => {
            RendererServiceWorkerVersionStatus::Installing
        }
        ServiceWorkerVersionLifecycleState::Installed => {
            RendererServiceWorkerVersionStatus::Installed
        }
        ServiceWorkerVersionLifecycleState::Activating => {
            RendererServiceWorkerVersionStatus::Activating
        }
        ServiceWorkerVersionLifecycleState::Activated => {
            RendererServiceWorkerVersionStatus::Activated
        }
        ServiceWorkerVersionLifecycleState::Redundant => {
            RendererServiceWorkerVersionStatus::Redundant
        }
    }
}

pub(super) struct ServiceWorkerRuntimeInner {
    pub(super) next_registration_id: AtomicU64,
    pub(super) next_version_id: AtomicU64,
    pub(super) client_id_allocator: super::ids::ServiceWorkerClientIdAllocator,
    pub(super) next_event_id: AtomicU64,
    pub(super) next_force_update_page_load_waiter_id: AtomicU64,
    pub(super) next_notification_id: AtomicU64,
    pub(super) idle_delay_ms: AtomicU64,
    pub(super) force_update_on_page_load: AtomicBool,
    pub(super) pause_new_workers_on_start_for_devtools: AtomicBool,
    pub(super) state: Mutex<ServiceWorkerRuntimeState>,
    pub(super) service_lane: ServiceWorkerServiceLane,
    pub(super) owner_wake: Mutex<WorkerOwnerWakeRoutes<ServiceWorkerRuntimeOwnerWake>>,
    pub(super) resource_store: SharedServiceWorkerResourceStore,
    pub(super) restored_worker_context_runtime: RendererWorkerContextRuntime,
    pub(super) browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
    #[cfg(test)]
    pub(super) target_output_test_rx:
        Mutex<Option<crate::runtime::RendererOutputTransportReceiver>>,
}

impl ServiceWorkerRuntimeInner {
    pub(super) fn new(
        default_idle_delay_ms: u64,
        resource_store: SharedServiceWorkerResourceStore,
        restored_worker_context_runtime: RendererWorkerContextRuntime,
        browser_resource_runtime: crate::network::BrowserResourceRuntimeBinding,
        client_id_allocator: super::ids::ServiceWorkerClientIdAllocator,
        worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
        output_transport: crate::runtime::RendererOutputTransportSenderSlot,
    ) -> Self {
        Self {
            next_registration_id: AtomicU64::new(1),
            next_version_id: AtomicU64::new(1),
            client_id_allocator,
            next_event_id: AtomicU64::new(1),
            next_force_update_page_load_waiter_id: AtomicU64::new(1),
            next_notification_id: AtomicU64::new(1),
            idle_delay_ms: AtomicU64::new(default_idle_delay_ms),
            force_update_on_page_load: AtomicBool::new(false),
            pause_new_workers_on_start_for_devtools: AtomicBool::new(false),
            state: Mutex::new(ServiceWorkerRuntimeState::new(
                worker_lifecycle,
                output_transport,
            )),
            service_lane: ServiceWorkerServiceLane::default(),
            owner_wake: Mutex::default(),
            resource_store,
            restored_worker_context_runtime,
            browser_resource_runtime,
            #[cfg(test)]
            target_output_test_rx: Mutex::new(None),
        }
    }
}

pub(super) struct ServiceWorkerRuntimeState {
    pub(super) registrations: HashMap<ServiceWorkerRegistrationId, ServiceWorkerRegistration>,
    pub(super) versions: HashMap<ServiceWorkerVersionId, ServiceWorkerVersion>,
    pub(super) pending_ready_jobs: Vec<ServiceWorkerReadyJob>,
    pub(super) pending_fetch_jobs: HashMap<ServiceWorkerEventId, ServiceWorkerFetchJob>,
    pub(super) lifecycle_watchers: Vec<ServiceWorkerLifecycleWatcher>,
    pub(super) live_clients: HashMap<ServiceWorkerClientId, ServiceWorkerClient>,
    pub(super) notification_records: Vec<ServiceWorkerNotificationRecord>,
    pub(super) sync_registrations:
        HashMap<(ServiceWorkerRegistrationId, String), ServiceWorkerSyncRegistrationRecord>,
    pub(super) periodic_sync_registrations:
        HashMap<(ServiceWorkerRegistrationId, String), ServiceWorkerPeriodicSyncRegistrationRecord>,
    pub(super) push_subscriptions:
        HashMap<ServiceWorkerRegistrationId, ServiceWorkerPushSubscriptionSnapshot>,
    pub(super) job_coordinator: ServiceWorkerJobCoordinator,
    pub(super) pending_main_script_update_checks:
        HashMap<ServiceWorkerRegistrationId, ServiceWorkerPendingMainScriptUpdateCheck>,
    pub(super) pending_force_update_page_load_waiters:
        HashMap<u64, tokio::sync::oneshot::Sender<()>>,
    pub(super) force_update_page_load_waiter_versions: HashMap<ServiceWorkerVersionId, Vec<u64>>,
    pub(super) pending_devtools_launches:
        HashMap<ServiceWorkerVersionId, ServiceWorkerQueuedLaunch>,
    pub(super) pending_devtools_evaluation_releases: HashSet<ServiceWorkerVersionId>,
    pub(super) devtools_attached_versions: HashSet<ServiceWorkerVersionId>,
    pub(super) main_script_update_check_diagnostics:
        HashMap<ServiceWorkerRegistrationId, ServiceWorkerMainScriptUpdateCheckDiagnostics>,
    target_output_streams: ServiceWorkerTargetOutputStreams,
    pub(super) devtools_related_pause_on_start_policies:
        Vec<ServiceWorkerDevToolsRelatedPauseOnStartPolicy>,
    pub(super) stored_registration_cache_revision: Option<u64>,
    pub(super) stored_registration_cache:
        HashMap<ServiceWorkerRegistrationKey, ServiceWorkerStoredRegistration>,
}

impl ServiceWorkerRuntimeState {
    fn new(
        worker_lifecycle: crate::runtime::RendererWorkerLifecycleReporter,
        output_transport: crate::runtime::RendererOutputTransportSenderSlot,
    ) -> Self {
        Self {
            registrations: HashMap::new(),
            versions: HashMap::new(),
            pending_ready_jobs: Vec::new(),
            pending_fetch_jobs: HashMap::new(),
            lifecycle_watchers: Vec::new(),
            live_clients: HashMap::new(),
            notification_records: Vec::new(),
            sync_registrations: HashMap::new(),
            periodic_sync_registrations: HashMap::new(),
            push_subscriptions: HashMap::new(),
            job_coordinator: ServiceWorkerJobCoordinator::default(),
            pending_main_script_update_checks: HashMap::new(),
            pending_force_update_page_load_waiters: HashMap::new(),
            force_update_page_load_waiter_versions: HashMap::new(),
            pending_devtools_launches: HashMap::new(),
            pending_devtools_evaluation_releases: HashSet::new(),
            devtools_attached_versions: HashSet::new(),
            main_script_update_check_diagnostics: HashMap::new(),
            target_output_streams: ServiceWorkerTargetOutputStreams::new(
                worker_lifecycle,
                output_transport,
            ),
            devtools_related_pause_on_start_policies: Vec::new(),
            stored_registration_cache_revision: None,
            stored_registration_cache: HashMap::new(),
        }
    }

    #[cfg(test)]
    fn new_with_target_output_for_test() -> (Self, crate::runtime::RendererOutputTransportReceiver)
    {
        let (sender, receiver) = crate::runtime::renderer_output_transport_channel();
        let output_transport = crate::runtime::RendererOutputTransportSenderSlot::default();
        output_transport.set(sender);
        (
            Self::new(
                crate::runtime::RendererWorkerLifecycleReporter::new(
                    crate::runtime::RendererBrowserContextRuntimeId::new_for_testing(0),
                ),
                output_transport,
            ),
            receiver,
        )
    }

    pub(super) fn bind_target_output_transport(
        &self,
        transport: crate::runtime::RendererOutputTransportSender,
    ) {
        self.target_output_streams.bind_transport(transport);
    }

    fn live_target_run(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> Option<RendererServiceWorkerRunIdentity> {
        let version = self.versions.get(&version_id)?;
        let host = match &version.running_state {
            ServiceWorkerVersionRunningState::Starting { host }
            | ServiceWorkerVersionRunningState::Running { host } => host,
            ServiceWorkerVersionRunningState::Stopped => return None,
        };
        (host.version_id() == version_id && host.run_identity() == version.run)
            .then(|| host.run_identity())
    }

    pub(super) fn observes_live_target_run(
        &self,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
    ) -> bool {
        self.live_target_run(version_id).as_ref() == Some(run)
    }

    pub(super) fn target_output_journal(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> Option<crate::runtime::RendererTurnOutputJournal> {
        self.target_output_streams.journal(version_id)
    }

    pub(super) fn record_target_created(
        &mut self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        script_url: Url,
        scope_url: Url,
    ) {
        if self.target_output_journal(version_id).is_some() {
            return;
        }
        let info = RendererServiceWorkerTargetInfo {
            registration_id: registration_id.as_u64(),
            version_id: version_id.as_u64(),
            script_url: script_url.to_string(),
            scope_url: scope_url.to_string(),
            status: self.target_status_for_version(version_id),
        };
        self.target_output_streams.publish_created(
            version_id,
            RendererServiceWorkerLifecycle::Created {
                info,
                active_run: self.live_target_run(version_id),
            },
        );
    }

    /// Called under the runtime state lock immediately after installing a host.
    /// Console and inspector output cannot introduce a run on their own.
    pub(super) fn record_target_starting(&self, version_id: ServiceWorkerVersionId) {
        if self.target_output_journal(version_id).is_none() {
            return;
        }
        let Some(run) = self.live_target_run(version_id) else {
            return;
        };
        self.target_output_streams.publish(
            version_id,
            RendererServiceWorkerLifecycle::Starting {
                version_id: version_id.as_u64(),
                run,
            },
        );
    }

    pub(super) fn record_target_execution_ready(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
    ) {
        if self.target_output_journal(version_id).is_none() {
            return;
        }
        let Some(version) = self.versions.get(&version_id) else {
            return;
        };
        let ServiceWorkerVersionRunningState::Starting { host } = &version.running_state else {
            return;
        };
        if host.run_identity() != run || host.inspection_handle().is_none() {
            return;
        }
        self.target_output_streams.publish(
            version_id,
            RendererServiceWorkerLifecycle::ExecutionReady {
                version_id: version_id.as_u64(),
                run,
            },
        );
    }

    pub(super) fn record_target_started(
        &mut self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
    ) -> bool {
        if self.target_output_journal(version_id).is_none()
            || !self.observes_live_target_run(version_id, &run)
        {
            return false;
        }
        self.target_output_streams.publish(
            version_id,
            RendererServiceWorkerLifecycle::Started {
                version_id: version_id.as_u64(),
                run,
            },
        );
        true
    }

    pub(super) fn record_target_stopped(
        &mut self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        reason: impl Into<String>,
    ) -> bool {
        if self.target_output_journal(version_id).is_none()
            || !self
                .versions
                .get(&version_id)
                .is_some_and(|version| version.run == run)
        {
            return false;
        }
        self.target_output_streams.publish(
            version_id,
            RendererServiceWorkerLifecycle::Stopped {
                version_id: version_id.as_u64(),
                run,
                reason: reason.into(),
            },
        );
        true
    }

    pub(super) fn record_target_destroyed(&mut self, version_id: ServiceWorkerVersionId) -> bool {
        self.record_target_destroyed_with_run(version_id, self.live_target_run(version_id))
    }

    pub(super) fn record_target_destroyed_with_run(
        &mut self,
        version_id: ServiceWorkerVersionId,
        active_run: Option<RendererServiceWorkerRunIdentity>,
    ) -> bool {
        self.devtools_attached_versions.remove(&version_id);
        if self.target_output_journal(version_id).is_none() {
            return false;
        }
        self.target_output_streams.publish_destroyed(
            version_id,
            RendererServiceWorkerLifecycle::Destroyed {
                version_id: version_id.as_u64(),
                active_run,
            },
        );
        true
    }

    pub(super) fn record_target_version_updated(
        &mut self,
        version_id: ServiceWorkerVersionId,
    ) -> bool {
        if self.target_output_journal(version_id).is_none() {
            return false;
        }
        self.target_output_streams.publish(
            version_id,
            RendererServiceWorkerLifecycle::VersionUpdated {
                version_id: version_id.as_u64(),
                status: self.target_status_for_version(version_id),
            },
        );
        true
    }

    fn target_status_for_version(
        &self,
        version_id: ServiceWorkerVersionId,
    ) -> RendererServiceWorkerVersionStatus {
        self.versions
            .get(&version_id)
            .map(|version| target_status_for_lifecycle_state(version.lifecycle_state))
            .unwrap_or(RendererServiceWorkerVersionStatus::New)
    }

    pub(super) fn record_target_console_message(
        &mut self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        message: RendererServiceWorkerConsoleMessage,
    ) -> bool {
        if self.target_output_journal(version_id).is_none()
            || !self.observes_live_target_run(version_id, &run)
        {
            return false;
        }
        self.target_output_streams.publish_observation(
            version_id,
            RendererServiceWorkerObservation::Console {
                version_id: version_id.as_u64(),
                run,
                message,
            },
        );
        true
    }

    pub(super) fn record_target_exception_message(
        &mut self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        message: RendererServiceWorkerExceptionMessage,
    ) -> bool {
        if self.target_output_journal(version_id).is_none()
            || !self.observes_live_target_run(version_id, &run)
        {
            return false;
        }
        self.target_output_streams.publish_observation(
            version_id,
            RendererServiceWorkerObservation::Exception {
                version_id: version_id.as_u64(),
                run,
                message,
            },
        );
        true
    }

    pub(super) fn record_target_fetch_diagnostic(
        &mut self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        diagnostic: RendererServiceWorkerFetchDiagnostic,
    ) -> bool {
        if self.target_output_journal(version_id).is_none()
            || !self.observes_live_target_run(version_id, &run)
        {
            return false;
        }
        self.target_output_streams.publish_observation(
            version_id,
            RendererServiceWorkerObservation::FetchDiagnostic {
                version_id: version_id.as_u64(),
                run,
                diagnostic,
            },
        );
        true
    }

    pub(super) fn record_target_runtime_inspector_messages(
        &mut self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        inspector_session_id: Option<String>,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> bool {
        if self.target_output_journal(version_id).is_none()
            || !self.observes_live_target_run(version_id, &run)
        {
            return false;
        }
        if messages.is_empty() {
            return false;
        }
        self.target_output_streams.publish_observation(
            version_id,
            RendererServiceWorkerObservation::RuntimeInspectorMessages {
                version_id: version_id.as_u64(),
                run,
                inspector_session_id,
                messages,
            },
        );
        true
    }

    pub(super) fn insert_force_update_page_load_waiter(
        &mut self,
        waiter_id: u64,
        sender: tokio::sync::oneshot::Sender<()>,
    ) {
        self.pending_force_update_page_load_waiters
            .insert(waiter_id, sender);
    }

    pub(super) fn bind_force_update_page_load_waiters(
        &mut self,
        version_id: ServiceWorkerVersionId,
        waiter_ids: Vec<u64>,
    ) {
        if waiter_ids.is_empty() {
            return;
        }
        self.force_update_page_load_waiter_versions
            .entry(version_id)
            .or_default()
            .extend(waiter_ids);
    }

    pub(super) fn take_force_update_page_load_waiters(
        &mut self,
        waiter_ids: Vec<u64>,
    ) -> Vec<tokio::sync::oneshot::Sender<()>> {
        waiter_ids
            .into_iter()
            .filter_map(|waiter_id| {
                self.pending_force_update_page_load_waiters
                    .remove(&waiter_id)
            })
            .collect()
    }

    pub(super) fn take_force_update_page_load_waiters_for_version(
        &mut self,
        version_id: ServiceWorkerVersionId,
    ) -> Vec<tokio::sync::oneshot::Sender<()>> {
        let waiter_ids = self
            .force_update_page_load_waiter_versions
            .remove(&version_id)
            .unwrap_or_default();
        self.take_force_update_page_load_waiters(waiter_ids)
    }

    pub(super) fn take_all_force_update_page_load_waiters(
        &mut self,
    ) -> Vec<tokio::sync::oneshot::Sender<()>> {
        self.force_update_page_load_waiter_versions.clear();
        self.pending_force_update_page_load_waiters
            .drain()
            .map(|(_, sender)| sender)
            .collect()
    }
}

pub(super) enum LifecycleProgress {
    Dispatch((SharedRendererServiceWorkerHost, ServiceWorkerLifecycleEvent)),
    TerminateHost(SharedRendererServiceWorkerHost),
    ScheduleIdleTimeout(ServiceWorkerIdleTimeout),
    ReadyCompleted(Box<(ServiceWorkerReadyJob, ServiceWorkerRegistrationSnapshot)>),
    RegisterCompleted(
        Box<(
            Vec<ServiceWorkerRegisterJob>,
            ServiceWorkerRegistrationSnapshot,
        )>,
    ),
    RegisterFailed(
        (
            Vec<ServiceWorkerRegisterJob>,
            ServiceWorkerRegistrationError,
        ),
    ),
    ForceUpdatePageLoadCompleted(Vec<tokio::sync::oneshot::Sender<()>>),
    UnregisterCompleted(ServiceWorkerQueuedUnregisterJob),
    FetchFailed(Box<(ServiceWorkerFetchJob, String)>),
    StartWorker(Box<ServiceWorkerQueuedLaunch>),
    StartMainScriptUpdateCheck(
        Box<(
            ServiceWorkerRegistrationId,
            ServiceWorkerScriptUpdateCheckParams,
        )>,
    ),
    NotifyLifecycle(Box<ServiceWorkerLifecycleNotificationDelivery>),
    NotifyControllerChange(ServiceWorkerControllerChangeDelivery),
}

pub(super) struct ServiceWorkerQueuedLaunch {
    pub(super) params: ServiceWorkerLaunchParams,
    pub(super) host: SharedRendererServiceWorkerHost,
    pub(super) lifecycle_notifications: Vec<ServiceWorkerLifecycleNotificationDelivery>,
    pub(super) preloaded_script: Option<LoadedServiceWorkerScript>,
}

pub(super) enum ServiceWorkerLifecycleStart {
    Dispatch((SharedRendererServiceWorkerHost, ServiceWorkerLifecycleEvent)),
    Start(Box<ServiceWorkerQueuedLaunch>),
    Queued,
}

pub(super) enum ServiceWorkerMessageStart {
    Dispatch(Box<(SharedRendererServiceWorkerHost, ServiceWorkerMessageEvent)>),
    Start(Box<ServiceWorkerQueuedLaunch>),
    Queued,
    Dropped,
}

pub(super) enum ServiceWorkerNotificationStart {
    Dispatch(
        Box<(
            SharedRendererServiceWorkerHost,
            ServiceWorkerNotificationEvent,
        )>,
    ),
    Start(Box<ServiceWorkerQueuedLaunch>),
    Queued,
    Dropped,
}

pub(super) enum ServiceWorkerPushStart {
    Dispatch(Box<(SharedRendererServiceWorkerHost, ServiceWorkerPushEvent)>),
    Start(Box<ServiceWorkerQueuedLaunch>),
    Queued,
    Dropped,
}

pub(super) enum ServiceWorkerSyncStart {
    Dispatch(Box<(SharedRendererServiceWorkerHost, ServiceWorkerSyncEvent)>),
    Start(Box<ServiceWorkerQueuedLaunch>),
    Queued,
    Dropped,
}

pub(super) enum ServiceWorkerPeriodicSyncStart {
    Dispatch(
        Box<(
            SharedRendererServiceWorkerHost,
            ServiceWorkerPeriodicSyncEvent,
        )>,
    ),
    Start(Box<ServiceWorkerQueuedLaunch>),
    Queued,
    Dropped,
}

pub(super) struct ServiceWorkerFetchJob {
    pub(super) internal_id: u64,
    pub(super) owner: Option<ServiceWorkerRunOwner>,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) request_body_bytes: Option<Vec<u8>>,
    pub(super) cors_preflight_request_headers: Vec<(String, String)>,
    pub(super) client_id: ServiceWorkerClientId,
    pub(super) resulting_client_id: Option<ServiceWorkerClientId>,
    pub(super) destination: ServiceWorkerRequestDestination,
    pub(super) is_reload: bool,
    pub(super) metadata: ServiceWorkerFetchRequestMetadata,
    pub(super) request_mode: moli_fetch::RequestMode,
    pub(super) credentials_mode: moli_fetch::RequestCredentialsMode,
    pub(super) redirect_mode: moli_fetch::RequestRedirectMode,
    pub(super) priority: Option<moli_fetch::FetchPriorityHint>,
    pub(super) redirect_chain: Vec<moli_fetch::RedirectInfo>,
    pub(super) redirect_count: usize,
    pub(super) request_cookie_report: Option<moli_cookie_jar::StoredCookieQueryReport>,
    pub(super) network_context: AsyncSubresourceNetworkContext,
    pub(super) completion_tx: RendererResourceCompletionSender,
    pub(super) request_client: ResourceRequestClient,
    pub(super) resource_task_runner: crate::network::RendererResourceTaskRunner,
    pub(super) cancel_handle: moli_fetch::FetchCancelHandle,
    pub(super) navigation_preload_cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    pub(super) streaming_body_source_id: Option<crate::types::NetworkBodySourceId>,
    pub(super) direct_completion_tx:
        Option<tokio::sync::oneshot::Sender<ServiceWorkerDirectFetchResult>>,
}

impl ServiceWorkerFetchJob {
    pub(super) fn bind_to_owner(&mut self, owner: ServiceWorkerRunOwner) {
        self.owner = Some(owner);
    }

    pub(super) fn owner(&self) -> &ServiceWorkerRunOwner {
        self.owner
            .as_ref()
            .expect("pending ServiceWorker fetch jobs must have an exact run owner")
    }

    pub(super) fn version_id(&self) -> ServiceWorkerVersionId {
        self.owner().version_id()
    }

    pub(super) fn is_bound_to_owner(&self, owner: &ServiceWorkerRunOwner) -> bool {
        self.owner.as_ref() == Some(owner)
    }

    pub(super) fn run_identity(&self) -> &RendererServiceWorkerRunIdentity {
        self.owner().run_identity()
    }

    pub(super) fn cancel_pending_navigation_preload(&mut self) {
        if let Some(cancel_handle) = self.navigation_preload_cancel_handle.take() {
            cancel_handle.cancel();
        }
    }

    pub(super) fn clear_pending_navigation_preload_cancel_handle(&mut self) {
        self.navigation_preload_cancel_handle = None;
    }
}

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerReadyJob {
    pub(super) request_id: u64,
    pub(super) document_owner: crate::window_document_identity::WindowDocumentOwner,
    pub(super) completion_tx: RendererPageServiceWorkerTaskSender,
    pub(super) registration_id: ServiceWorkerRegistrationId,
}

impl ServiceWorkerReadyJob {
    pub(super) fn send(self, registration: ServiceWorkerRegistrationSnapshot) {
        let _ = self
            .completion_tx
            .send_service_worker_ready(ServiceWorkerReadyCompletion {
                request_id: self.request_id,
                document_owner: self.document_owner,
                registration,
            });
    }
}

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerLifecycleWatcher {
    pub(super) scope_url: Url,
    pub(super) storage_key: String,
    pub(super) document_owner: crate::window_document_identity::WindowDocumentOwner,
    pub(super) completion_tx: RendererPageServiceWorkerTaskSender,
}

pub(super) struct ServiceWorkerLifecycleNotificationDelivery {
    pub(super) watcher: ServiceWorkerLifecycleWatcher,
    pub(super) registration: ServiceWorkerRegistrationSnapshot,
    pub(super) events: Vec<ServiceWorkerLifecycleClientEvent>,
}

impl ServiceWorkerLifecycleNotificationDelivery {
    pub(super) fn send(self) {
        let _ = self.watcher.completion_tx.send_service_worker_lifecycle(
            ServiceWorkerLifecycleNotification {
                document_owner: self.watcher.document_owner,
                storage_key: self.watcher.storage_key,
                registration: self.registration,
                events: self.events,
            },
        );
    }
}

pub(super) struct ServiceWorkerControllerChangeDelivery {
    pub(super) target: Option<ServiceWorkerWindowClientTarget>,
    pub(super) endpoint: ServiceWorkerClientEndpoint,
}

impl ServiceWorkerControllerChangeDelivery {
    pub(super) fn send(self) {
        let _ = self.endpoint.send_controller_change(self.target);
    }
}

#[derive(Clone, Debug)]
pub(super) enum ServiceWorkerClientEndpoint {
    /// A Window client reserved before its PageVm owns a concrete scheduler
    /// route. It cannot receive Page callbacks until promotion installs the
    /// exact root-Document sender.
    ReservedPage {
        bypass_service_worker: bool,
    },
    Page(RendererPageServiceWorkerTaskSender),
    PendingWorker,
    Worker(tokio::sync::mpsc::UnboundedSender<WorkerMessage>),
}

impl ServiceWorkerClientEndpoint {
    pub(super) fn page_task_sender(&self) -> Option<RendererPageServiceWorkerTaskSender> {
        match self {
            Self::Page(sender) => Some(sender.clone()),
            Self::ReservedPage { .. } | Self::PendingWorker => None,
            Self::Worker(_) => None,
        }
    }

    pub(super) fn send_client_message(
        &self,
        target: Option<ServiceWorkerWindowClientTarget>,
        source_version_id: ServiceWorkerVersionId,
        source_script_url: Url,
        source_state: &'static str,
        payload: crate::structured_clone::V8StructuredClonePayload,
    ) -> bool {
        match self {
            Self::Page(sender) => {
                let Some(target) = target else {
                    return false;
                };
                sender
                    .send_service_worker_client_message(ServiceWorkerClientMessageCompletion {
                        target,
                        source_version_id,
                        source_script_url,
                        source_state,
                        payload,
                    })
                    .is_ok()
            }
            Self::ReservedPage { .. } | Self::PendingWorker => false,
            Self::Worker(sender) => sender.send(WorkerMessage::Post(payload)).is_ok(),
        }
    }

    pub(super) fn send_controller_change(
        &self,
        target: Option<ServiceWorkerWindowClientTarget>,
    ) -> bool {
        match self {
            Self::Page(sender) => {
                let Some(target) = target else {
                    return false;
                };
                sender
                    .send_service_worker_controller_change(
                        ServiceWorkerControllerChangeCompletion { target },
                    )
                    .is_ok()
            }
            Self::ReservedPage { .. } | Self::PendingWorker => false,
            Self::Worker(sender) => sender
                .send(WorkerMessage::ServiceWorkerControllerChange)
                .is_ok(),
        }
    }
}

#[derive(Debug)]
pub(super) struct ServiceWorkerClient {
    pub(super) id: ServiceWorkerClientId,
    pub(super) exposed_id: String,
    pub(super) creation_url: Url,
    pub(super) document_url: Url,
    pub(super) client_type: ServiceWorkerClientType,
    pub(super) frame_type: ServiceWorkerClientFrameType,
    pub(super) visibility_state: ServiceWorkerClientVisibilityState,
    pub(super) storage_key: String,
    pub(super) secure_context: bool,
    pub(super) execution_ready: bool,
    pub(super) discarded_or_frozen: bool,
    pub(super) document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
    pub(super) endpoint: ServiceWorkerClientEndpoint,
    pub(super) focused: bool,
}

impl ServiceWorkerClient {
    pub(super) fn window_completion_target(&self) -> Option<ServiceWorkerWindowClientTarget> {
        if self.client_type != ServiceWorkerClientType::Window {
            return None;
        }
        Some(ServiceWorkerWindowClientTarget {
            client_id: self.id,
            document_owner: self.document_owner?,
        })
    }
}

#[cfg(test)]
mod target_run_identity_tests {
    use super::{ServiceWorkerRegistrationId, ServiceWorkerRuntimeState, ServiceWorkerVersionId};
    use crate::runtime::{RendererServiceWorkerConsoleMessage, RendererServiceWorkerRunIdentity};
    use crate::service_worker_runtime::target_output_streams::{
        ServiceWorkerOutputForTest, drain_service_worker_target_events_for_test,
    };
    use url::Url;

    #[test]
    fn version_facts_do_not_manufacture_a_worker_run() {
        let (mut state, mut target_output_rx) =
            ServiceWorkerRuntimeState::new_with_target_output_for_test();
        let registration_id = ServiceWorkerRegistrationId(1);
        let version_id = ServiceWorkerVersionId(7);
        state.record_target_created(
            registration_id,
            version_id,
            Url::parse("https://example.test/service-worker.js").unwrap(),
            Url::parse("https://example.test/").unwrap(),
        );
        drain_service_worker_target_events_for_test(&mut target_output_rx);

        assert!(state.record_target_version_updated(version_id));
        assert!(state.record_target_destroyed(version_id));

        let events = drain_service_worker_target_events_for_test(&mut target_output_rx);
        assert!(matches!(
            events.as_slice(),
            [
                ServiceWorkerOutputForTest::Lifecycle(
                    crate::runtime::RendererServiceWorkerLifecycle::VersionUpdated { .. }
                ),
                ServiceWorkerOutputForTest::Lifecycle(
                    crate::runtime::RendererServiceWorkerLifecycle::Destroyed {
                        active_run: None,
                        ..
                    }
                )
            ]
        ));
    }

    #[test]
    fn observations_cannot_create_a_host_for_a_stopped_version() {
        let (mut state, mut target_output_rx) =
            ServiceWorkerRuntimeState::new_with_target_output_for_test();
        let version = ServiceWorkerVersionId(7);
        state.record_target_created(
            ServiceWorkerRegistrationId(1),
            version,
            Url::parse("https://example.test/service-worker.js").unwrap(),
            Url::parse("https://example.test/").unwrap(),
        );
        drain_service_worker_target_events_for_test(&mut target_output_rx);
        let run = RendererServiceWorkerRunIdentity::fresh();
        assert!(!state.record_target_console_message(
            version,
            run.clone(),
            RendererServiceWorkerConsoleMessage {
                message: "no physical host".into(),
                args: Vec::new(),
                stack: None,
            },
        ));
        state.record_target_execution_ready(version, run.clone());
        assert!(!state.record_target_started(version, run));
        state.record_target_starting(version);
        assert!(drain_service_worker_target_events_for_test(&mut target_output_rx).is_empty());
    }
}
