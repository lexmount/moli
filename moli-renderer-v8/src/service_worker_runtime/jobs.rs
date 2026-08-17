use std::collections::{HashMap, VecDeque};

use moli_storage_key::MoliStorageKey;
use url::Url;

use crate::{
    network::{BrowserResourceRuntimeBinding, ResourceRequestClient},
    page_task_queue::RendererPageServiceWorkerTaskSender,
    runtime::{
        RendererBrowserContextRuntime, RendererServiceWorkerRunIdentity,
        RendererWorkerContextRuntime,
    },
    types::{ServiceWorkerRegisterCompletion, ServiceWorkerUnregisterCompletion},
    worker::{WorkerNetworkPolicy, WorkerScriptKind},
};

use super::{
    errors::ServiceWorkerRegistrationError,
    ids::{ServiceWorkerRegistrationId, ServiceWorkerVersionId},
    registration::ServiceWorkerUpdateViaCache,
    script_loading::ServiceWorkerScriptUpdateCheckParams,
    snapshots::ServiceWorkerRegistrationSnapshot,
};

#[derive(Clone)]
pub(crate) struct ServiceWorkerLaunchParams {
    pub(super) registration_id: ServiceWorkerRegistrationId,
    pub(super) version_id: ServiceWorkerVersionId,
    pub(super) run: RendererServiceWorkerRunIdentity,
    pub(super) script_url: Url,
    pub(super) scope_url: Url,
    pub(super) storage_key: String,
    pub(super) document_url: Url,
    pub(super) script_kind: WorkerScriptKind,
    pub(super) request_client: ResourceRequestClient,
    pub(super) network_policy: WorkerNetworkPolicy,
    pub(super) worker_context_runtime: RendererWorkerContextRuntime,
    pub(super) broadcast_channel_top_level_site: Option<String>,
    pub(super) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub(super) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub(super) pause_evaluation_until_debugger: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerRegisterJob {
    pub(super) request_id: u64,
    pub(super) runtime_generation: u64,
    pub(super) completion_tx: RendererPageServiceWorkerTaskSender,
}

impl ServiceWorkerRegisterJob {
    pub(super) fn send(
        self,
        result: std::result::Result<
            ServiceWorkerRegistrationSnapshot,
            ServiceWorkerRegistrationError,
        >,
    ) {
        let _ = self
            .completion_tx
            .send_service_worker_register(ServiceWorkerRegisterCompletion {
                request_id: self.request_id,
                runtime_generation: self.runtime_generation,
                result,
            });
    }

    pub(super) fn send_all(
        jobs: Vec<Self>,
        result: std::result::Result<
            ServiceWorkerRegistrationSnapshot,
            ServiceWorkerRegistrationError,
        >,
    ) {
        for job in jobs {
            job.send(result.clone());
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServiceWorkerRegisterJobPhase {
    Initial,
    Start,
    Register,
    Update,
    Install,
    Store,
    Complete,
    Abort,
}

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerPendingRegisterJob {
    phase: ServiceWorkerRegisterJobPhase,
    skip_waiting_after_install: bool,
    callbacks: Vec<ServiceWorkerRegisterJob>,
    resolved_result: Option<
        std::result::Result<ServiceWorkerRegistrationSnapshot, ServiceWorkerRegistrationError>,
    >,
}

impl ServiceWorkerPendingRegisterJob {
    #[cfg(test)]
    pub(super) fn new(callbacks: Vec<ServiceWorkerRegisterJob>) -> Self {
        Self::new_with_options(callbacks, false)
    }

    pub(super) fn new_with_options(
        callbacks: Vec<ServiceWorkerRegisterJob>,
        skip_waiting_after_install: bool,
    ) -> Self {
        Self {
            phase: ServiceWorkerRegisterJobPhase::Initial,
            skip_waiting_after_install,
            callbacks,
            resolved_result: None,
        }
    }

    #[cfg(test)]
    pub(super) fn phase(&self) -> ServiceWorkerRegisterJobPhase {
        self.phase
    }

    pub(super) fn start_current_moli_job(&mut self) {
        self.phase = ServiceWorkerRegisterJobPhase::Start;
        self.phase = ServiceWorkerRegisterJobPhase::Register;
        self.phase = ServiceWorkerRegisterJobPhase::Update;
    }

    fn mark_install(&mut self) {
        self.phase = ServiceWorkerRegisterJobPhase::Install;
    }

    fn mark_store(&mut self) {
        self.phase = ServiceWorkerRegisterJobPhase::Store;
    }

    fn mark_complete(&mut self) {
        self.phase = ServiceWorkerRegisterJobPhase::Complete;
    }

    fn mark_abort(&mut self) {
        self.phase = ServiceWorkerRegisterJobPhase::Abort;
    }

    pub(super) fn skip_waiting_after_install(&self) -> bool {
        self.skip_waiting_after_install
    }

    pub(super) fn add_callbacks(
        &mut self,
        callbacks: Vec<ServiceWorkerRegisterJob>,
        skip_waiting_after_install: bool,
    ) -> Option<(
        Vec<ServiceWorkerRegisterJob>,
        std::result::Result<ServiceWorkerRegistrationSnapshot, ServiceWorkerRegistrationError>,
    )> {
        self.skip_waiting_after_install |= skip_waiting_after_install;
        if let Some(result) = &self.resolved_result {
            return Some((callbacks, result.clone()));
        }
        self.callbacks.extend(callbacks);
        None
    }

    fn resolve_promise(
        &mut self,
        snapshot: ServiceWorkerRegistrationSnapshot,
    ) -> Vec<ServiceWorkerRegisterJob> {
        self.resolved_result = Some(Ok(snapshot));
        std::mem::take(&mut self.callbacks)
    }

    fn reject_promise(
        &mut self,
        error: ServiceWorkerRegistrationError,
    ) -> Vec<ServiceWorkerRegisterJob> {
        if self.resolved_result.is_none() {
            self.resolved_result = Some(Err(error));
            return std::mem::take(&mut self.callbacks);
        }
        Vec::new()
    }

    pub(super) fn complete_install_started(
        &mut self,
        snapshot: ServiceWorkerRegistrationSnapshot,
    ) -> Vec<ServiceWorkerRegisterJob> {
        self.mark_install();
        self.resolve_promise(snapshot)
    }

    pub(super) fn complete_without_install(
        &mut self,
        snapshot: ServiceWorkerRegistrationSnapshot,
    ) -> Vec<ServiceWorkerRegisterJob> {
        self.mark_complete();
        self.resolve_promise(snapshot)
    }

    pub(super) fn abort_before_install(
        &mut self,
        error: ServiceWorkerRegistrationError,
    ) -> Vec<ServiceWorkerRegisterJob> {
        self.mark_abort();
        self.reject_promise(error)
    }

    pub(super) fn complete_install_failure(
        &mut self,
        error: ServiceWorkerRegistrationError,
    ) -> Vec<ServiceWorkerRegisterJob> {
        self.mark_complete();
        self.reject_promise(error)
    }

    pub(super) fn complete_install_success(
        &mut self,
        snapshot: ServiceWorkerRegistrationSnapshot,
    ) -> Vec<ServiceWorkerRegisterJob> {
        self.mark_store();
        self.mark_complete();
        self.resolve_promise(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;
    use crate::service_worker_runtime::snapshots::ServiceWorkerVersionSnapshot;

    fn registration_snapshot() -> ServiceWorkerRegistrationSnapshot {
        let script_url = Url::parse("https://example.test/app/sw.js").unwrap();
        ServiceWorkerRegistrationSnapshot::new(
            ServiceWorkerRegistrationId(1),
            Url::parse("https://example.test/app/").unwrap(),
            ServiceWorkerUpdateViaCache::Imports,
            crate::service_worker_runtime::ServiceWorkerNavigationPreloadState::default(),
            Some(ServiceWorkerVersionSnapshot::new(
                ServiceWorkerVersionId(1),
                script_url,
                "installing",
            )),
            None,
            None,
        )
    }

    #[test]
    fn pending_register_job_resolves_when_install_starts() {
        let queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let mut job = ServiceWorkerPendingRegisterJob::new(vec![ServiceWorkerRegisterJob {
            request_id: 1,
            runtime_generation: 1,
            completion_tx: queue.sender(),
        }]);

        job.start_current_moli_job();
        assert_eq!(job.phase(), ServiceWorkerRegisterJobPhase::Update);
        let callbacks = job.complete_install_started(registration_snapshot());
        assert_eq!(callbacks.len(), 1);
        assert_eq!(job.phase(), ServiceWorkerRegisterJobPhase::Install);

        let callbacks = job.complete_install_success(registration_snapshot());
        assert!(callbacks.is_empty());
        assert_eq!(job.phase(), ServiceWorkerRegisterJobPhase::Complete);
    }

    #[test]
    fn pending_register_job_abort_before_install_sets_abort_phase() {
        let mut job = ServiceWorkerPendingRegisterJob::new(Vec::new());

        job.start_current_moli_job();
        assert!(
            job.abort_before_install(ServiceWorkerRegistrationError::type_error(
                "failed to start"
            ))
            .is_empty()
        );

        assert_eq!(job.phase(), ServiceWorkerRegisterJobPhase::Abort);
    }
}

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerUnregisterJob {
    pub(super) request_id: u64,
    pub(super) runtime_generation: u64,
    pub(super) completion_tx: RendererPageServiceWorkerTaskSender,
}

impl ServiceWorkerUnregisterJob {
    pub(super) fn send(self, result: bool) {
        let _ =
            self.completion_tx
                .send_service_worker_unregister(ServiceWorkerUnregisterCompletion {
                    request_id: self.request_id,
                    runtime_generation: self.runtime_generation,
                    result,
                });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServiceWorkerUnregisterJobPhase {
    Initial,
    MarkPending,
    Resolve,
    Complete,
}

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerQueuedUnregisterJob {
    phase: ServiceWorkerUnregisterJobPhase,
    callbacks: Vec<ServiceWorkerUnregisterJob>,
}

impl ServiceWorkerQueuedUnregisterJob {
    pub(super) fn new(job: Option<ServiceWorkerUnregisterJob>) -> Self {
        Self {
            phase: ServiceWorkerUnregisterJobPhase::Initial,
            callbacks: job.into_iter().collect(),
        }
    }

    pub(super) fn append_callbacks_from(&mut self, other: Self) {
        debug_assert_eq!(self.phase, ServiceWorkerUnregisterJobPhase::Initial);
        debug_assert_eq!(other.phase, ServiceWorkerUnregisterJobPhase::Initial);
        self.callbacks.extend(other.callbacks);
    }

    pub(super) fn mark_pending(&mut self) {
        debug_assert_eq!(self.phase, ServiceWorkerUnregisterJobPhase::Initial);
        self.phase = ServiceWorkerUnregisterJobPhase::MarkPending;
    }

    #[cfg(test)]
    pub(super) fn phase(&self) -> ServiceWorkerUnregisterJobPhase {
        self.phase
    }

    pub(super) fn send_all(mut self, result: bool) -> ServiceWorkerUnregisterJobPhase {
        debug_assert_eq!(self.phase, ServiceWorkerUnregisterJobPhase::MarkPending);
        self.phase = ServiceWorkerUnregisterJobPhase::Resolve;
        for callback in self.callbacks {
            callback.send(result);
        }
        self.phase = ServiceWorkerUnregisterJobPhase::Complete;
        self.phase
    }
}

#[derive(Clone)]
pub(super) struct ServiceWorkerQueuedRegisterJob {
    pub(super) script_url: Url,
    pub(super) scope_url: Url,
    pub(super) document_url: Url,
    pub(super) storage_key: String,
    pub(super) script_kind: WorkerScriptKind,
    pub(super) update_via_cache: ServiceWorkerUpdateViaCache,
    pub(super) force_bypass_cache: bool,
    pub(super) skip_script_comparison: bool,
    pub(super) skip_waiting_after_install: bool,
    pub(super) force_update_page_load_waiter_ids: Vec<u64>,
    pub(super) request_client: ResourceRequestClient,
    pub(super) network_policy: WorkerNetworkPolicy,
    pub(super) browser_context_runtime: RendererBrowserContextRuntime,
    pub(super) broadcast_channel_top_level_site: Option<String>,
    pub(super) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub(super) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub(super) callbacks: Vec<ServiceWorkerRegisterJob>,
}

impl ServiceWorkerQueuedRegisterJob {
    pub(super) fn registration_key(&self) -> ServiceWorkerRegistrationKey {
        ServiceWorkerRegistrationKey::for_scope_and_storage_key(
            &self.scope_url,
            self.storage_key.clone(),
        )
    }

    pub(super) fn matches_registration_job(&self, other: &Self) -> bool {
        self.scope_url == other.scope_url
            && self.storage_key == other.storage_key
            && self.script_url == other.script_url
            && self.script_kind == other.script_kind
            && self.update_via_cache == other.update_via_cache
            && self.force_bypass_cache == other.force_bypass_cache
            && self.skip_script_comparison == other.skip_script_comparison
            && self.skip_waiting_after_install == other.skip_waiting_after_install
    }

    pub(super) fn append_callbacks_from(&mut self, other: Self) {
        self.callbacks.extend(other.callbacks);
        self.force_update_page_load_waiter_ids
            .extend(other.force_update_page_load_waiter_ids);
    }
}

pub(super) struct ServiceWorkerPendingMainScriptUpdateCheck {
    pub(super) queued_job: ServiceWorkerQueuedRegisterJob,
    pub(super) newest_version_id: ServiceWorkerVersionId,
    pub(super) newest_body_sha256: String,
    pub(super) new_version_id: ServiceWorkerVersionId,
    deferred_load_params: Option<ServiceWorkerScriptUpdateCheckParams>,
}

impl ServiceWorkerPendingMainScriptUpdateCheck {
    pub(super) fn new(
        queued_job: ServiceWorkerQueuedRegisterJob,
        newest_version_id: ServiceWorkerVersionId,
        newest_body_sha256: String,
        new_version_id: ServiceWorkerVersionId,
    ) -> Self {
        Self {
            queued_job,
            newest_version_id,
            newest_body_sha256,
            new_version_id,
            deferred_load_params: None,
        }
    }

    pub(super) fn matches_registration_job(&self, job: &ServiceWorkerQueuedRegisterJob) -> bool {
        self.queued_job.matches_registration_job(job)
    }

    pub(super) fn append_callbacks_from(&mut self, job: ServiceWorkerQueuedRegisterJob) {
        self.queued_job.append_callbacks_from(job);
    }

    pub(super) fn defer_until_debugger(
        &mut self,
        load_params: ServiceWorkerScriptUpdateCheckParams,
    ) {
        self.deferred_load_params = Some(load_params);
    }

    pub(super) fn take_deferred_load_params(
        &mut self,
    ) -> Option<ServiceWorkerScriptUpdateCheckParams> {
        self.deferred_load_params.take()
    }

    pub(super) fn abort(self) -> ServiceWorkerAbortedJob {
        ServiceWorkerAbortedJob::Register(self.queued_job.callbacks)
    }
}

pub(super) enum ServiceWorkerMainScriptUpdateCheckStart {
    Start(
        Box<(
            ServiceWorkerRegistrationId,
            ServiceWorkerScriptUpdateCheckParams,
        )>,
    ),
    WaitForDebugger,
}

impl std::fmt::Debug for ServiceWorkerQueuedRegisterJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceWorkerQueuedRegisterJob")
            .field("script_url", &self.script_url)
            .field("scope_url", &self.scope_url)
            .field("document_url", &self.document_url)
            .field("storage_key", &self.storage_key)
            .field("script_kind", &self.script_kind)
            .field("update_via_cache", &self.update_via_cache)
            .field("force_bypass_cache", &self.force_bypass_cache)
            .field("skip_script_comparison", &self.skip_script_comparison)
            .field(
                "skip_waiting_after_install",
                &self.skip_waiting_after_install,
            )
            .field(
                "force_update_page_load_waiter_count",
                &self.force_update_page_load_waiter_ids.len(),
            )
            .field("callback_count", &self.callbacks.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub(super) enum ServiceWorkerQueuedJob {
    Register(Box<ServiceWorkerQueuedRegisterJob>),
    Unregister(ServiceWorkerQueuedUnregisterJob),
}

pub(super) enum ServiceWorkerAbortedJob {
    Register(Vec<ServiceWorkerRegisterJob>),
    Unregister(Vec<ServiceWorkerUnregisterJob>),
}

impl ServiceWorkerQueuedJob {
    fn abort(self) -> ServiceWorkerAbortedJob {
        match self {
            Self::Register(job) => ServiceWorkerAbortedJob::Register(job.callbacks),
            Self::Unregister(job) => ServiceWorkerAbortedJob::Unregister(job.callbacks),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ServiceWorkerRegistrationKey {
    pub(super) scope_url: Url,
    pub(super) storage_key: String,
}

impl ServiceWorkerRegistrationKey {
    #[cfg(test)]
    pub(super) fn for_scope_url(scope_url: &Url) -> Self {
        Self {
            scope_url: scope_url.clone(),
            storage_key: Self::storage_key_for_scope_url(scope_url),
        }
    }

    pub(super) fn for_scope_and_storage_key(scope_url: &Url, storage_key: String) -> Self {
        Self {
            scope_url: scope_url.clone(),
            storage_key,
        }
    }

    pub(super) fn storage_key_for_scope_url(scope_url: &Url) -> String {
        Self::first_party_storage_key_for_url(scope_url)
    }

    pub(super) fn first_party_storage_key_for_url(url: &Url) -> String {
        MoliStorageKey::first_party_from_url(url, None).serialized_storage_key()
    }
}

#[derive(Default)]
pub(super) struct ServiceWorkerJobCoordinator {
    queues: HashMap<ServiceWorkerRegistrationKey, VecDeque<ServiceWorkerQueuedJob>>,
}

impl ServiceWorkerJobCoordinator {
    pub(super) fn enqueue_register(
        &mut self,
        key: ServiceWorkerRegistrationKey,
        job: ServiceWorkerQueuedRegisterJob,
    ) {
        let queue = self.queues.entry(key).or_default();
        if let Some(ServiceWorkerQueuedJob::Register(existing)) = queue.back_mut()
            && existing.matches_registration_job(&job)
        {
            existing.append_callbacks_from(job);
            return;
        }
        queue.push_back(ServiceWorkerQueuedJob::Register(Box::new(job)));
    }

    pub(super) fn enqueue_unregister(
        &mut self,
        key: ServiceWorkerRegistrationKey,
        job: Option<ServiceWorkerUnregisterJob>,
    ) {
        let queue = self.queues.entry(key).or_default();
        let job = ServiceWorkerQueuedUnregisterJob::new(job);
        if let Some(ServiceWorkerQueuedJob::Unregister(existing)) = queue.back_mut() {
            existing.append_callbacks_from(job);
            return;
        }
        queue.push_back(ServiceWorkerQueuedJob::Unregister(job));
    }

    pub(super) fn pop_next(
        &mut self,
        key: &ServiceWorkerRegistrationKey,
    ) -> Option<ServiceWorkerQueuedJob> {
        let queue = self.queues.get_mut(key)?;
        let job = queue.pop_front();
        if queue.is_empty() {
            self.queues.remove(key);
        }
        job
    }

    pub(super) fn abort_all(&mut self) -> Vec<ServiceWorkerAbortedJob> {
        self.queues
            .drain()
            .flat_map(|(_, queue)| queue.into_iter().map(ServiceWorkerQueuedJob::abort))
            .collect()
    }

    pub(super) fn has_jobs(&self, key: &ServiceWorkerRegistrationKey) -> bool {
        self.queues.get(key).is_some_and(|queue| !queue.is_empty())
    }

    pub(super) fn has_queued_unregistration(&self, key: &ServiceWorkerRegistrationKey) -> bool {
        self.queues.get(key).is_some_and(|queue| {
            queue
                .iter()
                .any(|job| matches!(job, ServiceWorkerQueuedJob::Unregister(_)))
        })
    }

    pub(super) fn queued_register_job_count(&self, key: &ServiceWorkerRegistrationKey) -> usize {
        self.queues
            .get(key)
            .map(|queue| {
                queue
                    .iter()
                    .filter(|job| matches!(job, ServiceWorkerQueuedJob::Register(_)))
                    .count()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn queued_register_job_options(
        &self,
        key: &ServiceWorkerRegistrationKey,
    ) -> Vec<(bool, bool, bool)> {
        self.queues
            .get(key)
            .map(|queue| {
                queue
                    .iter()
                    .filter_map(|job| match job {
                        ServiceWorkerQueuedJob::Register(job) => Some((
                            job.force_bypass_cache,
                            job.skip_script_comparison,
                            job.skip_waiting_after_install,
                        )),
                        ServiceWorkerQueuedJob::Unregister(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn queued_unregistration_job_count(
        &self,
        key: &ServiceWorkerRegistrationKey,
    ) -> usize {
        self.queues
            .get(key)
            .map(|queue| {
                queue
                    .iter()
                    .filter(|job| matches!(job, ServiceWorkerQueuedJob::Unregister(_)))
                    .count()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn queued_unregistration_phases(
        &self,
        key: &ServiceWorkerRegistrationKey,
    ) -> Vec<ServiceWorkerUnregisterJobPhase> {
        self.queues
            .get(key)
            .map(|queue| {
                queue
                    .iter()
                    .filter_map(|job| match job {
                        ServiceWorkerQueuedJob::Register(_) => None,
                        ServiceWorkerQueuedJob::Unregister(job) => Some(job.phase),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerUnregisterStart {
    Completed(bool),
    Queued,
}

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerVersionLaunchConfig {
    pub(super) document_url: Url,
    request_client: ServiceWorkerRequestClientSource,
    pub(super) network_policy: WorkerNetworkPolicy,
    pub(super) worker_context_runtime: RendererWorkerContextRuntime,
    pub(super) broadcast_channel_top_level_site: Option<String>,
    pub(super) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub(super) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
}

#[derive(Clone, Debug)]
enum ServiceWorkerRequestClientSource {
    /// A registration/update job captures the exact browser runtime and
    /// request policy that created this version.
    Captured(ResourceRequestClient),
    /// Persisted registrations have no serializable network object. Resolve
    /// their client from the browser-context authority at launch time instead
    /// of waiting for an ambient page fetch to patch the version.
    Restored(BrowserResourceRuntimeBinding),
}

impl ServiceWorkerRequestClientSource {
    fn materialize(&self) -> ResourceRequestClient {
        match self {
            Self::Captured(client) => client.clone(),
            Self::Restored(binding) => {
                ResourceRequestClient::from_browser_resource_runtime(binding.current())
            }
        }
    }
}

impl ServiceWorkerVersionLaunchConfig {
    pub(super) fn request_client(&self) -> ResourceRequestClient {
        self.request_client.materialize()
    }

    pub(super) fn from_queued_register_job(job: &ServiceWorkerQueuedRegisterJob) -> Self {
        Self {
            document_url: job.document_url.clone(),
            request_client: ServiceWorkerRequestClientSource::Captured(job.request_client.clone()),
            network_policy: job.network_policy.clone(),
            worker_context_runtime: job.browser_context_runtime.worker_context_runtime(),
            broadcast_channel_top_level_site: job.broadcast_channel_top_level_site.clone(),
            indexed_db_manager: job.indexed_db_manager.clone(),
            storage_bucket_store: job.storage_bucket_store.clone(),
        }
    }

    pub(super) fn restored(
        document_url: Url,
        worker_context_runtime: RendererWorkerContextRuntime,
        browser_resource_runtime: BrowserResourceRuntimeBinding,
    ) -> Self {
        Self {
            document_url,
            request_client: ServiceWorkerRequestClientSource::Restored(browser_resource_runtime),
            network_policy: WorkerNetworkPolicy::default(),
            worker_context_runtime,
            broadcast_channel_top_level_site: None,
            indexed_db_manager: None,
            storage_bucket_store: None,
        }
    }

    pub(super) fn to_launch_params(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        run: &RendererServiceWorkerRunIdentity,
        script_url: Url,
        scope_url: Url,
        storage_key: String,
        script_kind: WorkerScriptKind,
    ) -> ServiceWorkerLaunchParams {
        ServiceWorkerLaunchParams {
            registration_id,
            version_id,
            run: run.clone(),
            script_url,
            scope_url,
            storage_key,
            document_url: self.document_url.clone(),
            script_kind,
            request_client: self.request_client.materialize(),
            network_policy: self.network_policy.clone(),
            worker_context_runtime: self.worker_context_runtime.clone(),
            broadcast_channel_top_level_site: self.broadcast_channel_top_level_site.clone(),
            indexed_db_manager: self.indexed_db_manager.clone(),
            storage_bucket_store: self.storage_bucket_store.clone(),
            pause_evaluation_until_debugger: false,
        }
    }
}
