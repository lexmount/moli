use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::service_worker_runtime::resource_store::ServiceWorkerStoredRegistration;

fn service_worker_update_check_now_ms() -> u64 {
    let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return 0;
    };
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

pub(super) fn bump_registration_last_update_check_time_locked(
    state: &mut ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
) {
    let now_ms = service_worker_update_check_now_ms();
    if let Some(registration) = state.registrations.get_mut(&registration_id) {
        let next_ms = match registration.last_update_check_time_ms {
            Some(previous_ms) if now_ms <= previous_ms => previous_ms.saturating_add(1),
            _ => now_ms,
        };
        registration.last_update_check_time_ms = Some(next_ms);
    }
}

impl ServiceWorkerRuntimeService {
    pub(super) fn restore_stored_registration_for_queued_job_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        queued_job: &ServiceWorkerQueuedRegisterJob,
    ) -> Option<ServiceWorkerRegistrationId> {
        let registration_key = queued_job.registration_key();
        self.sync_stored_registration_cache_locked(state);
        let stored = state
            .stored_registration_cache
            .get(&registration_key)
            .cloned()?;
        let launch_config = ServiceWorkerVersionLaunchConfig::from_queued_register_job(queued_job);
        self.restore_stored_registration_locked(state, stored, launch_config)
    }

    pub(super) fn restore_all_stored_registrations(&self) -> Vec<ServiceWorkerRegistrationId> {
        let mut state = self.inner.state.lock();
        self.restore_all_stored_registrations_locked(&mut state)
    }

    fn restore_all_stored_registrations_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
    ) -> Vec<ServiceWorkerRegistrationId> {
        self.sync_stored_registration_cache_locked(state);
        let stored_registrations = state
            .stored_registration_cache
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let worker_context_runtime = self.inner.restored_worker_context_runtime.clone();
        let browser_resource_runtime = self.inner.browser_resource_runtime.clone();
        stored_registrations
            .into_iter()
            .filter_map(|stored| {
                let launch_config = ServiceWorkerVersionLaunchConfig::restored(
                    stored.scope_url.clone(),
                    worker_context_runtime.clone(),
                    browser_resource_runtime.clone(),
                );
                self.restore_stored_registration_locked(state, stored, launch_config)
            })
            .collect()
    }

    pub(super) fn restore_stored_registrations_for_document_url_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        document_url: &Url,
        storage_key: &str,
    ) -> Vec<ServiceWorkerRegistrationId> {
        self.sync_stored_registration_cache_locked(state);
        let stored_registrations = state
            .stored_registration_cache
            .values()
            .filter(|registration| registration.storage_key == storage_key)
            .cloned()
            .collect::<Vec<_>>();
        let worker_context_runtime = self.inner.restored_worker_context_runtime.clone();
        let browser_resource_runtime = self.inner.browser_resource_runtime.clone();
        stored_registrations
            .into_iter()
            .filter(|stored| service_worker_scope_matches_url(&stored.scope_url, document_url))
            .filter_map(|stored| {
                let launch_config = ServiceWorkerVersionLaunchConfig::restored(
                    document_url.clone(),
                    worker_context_runtime.clone(),
                    browser_resource_runtime.clone(),
                );
                self.restore_stored_registration_locked(state, stored, launch_config)
            })
            .collect()
    }

    fn sync_stored_registration_cache_locked(&self, state: &mut ServiceWorkerRuntimeState) {
        let resource_store = self.inner.resource_store.lock();
        let revision = resource_store.revision();
        if state.stored_registration_cache_revision == Some(revision) {
            return;
        }
        let registrations = resource_store.registrations();
        drop(resource_store);

        state.stored_registration_cache.clear();
        for stored in registrations {
            state
                .stored_registration_cache
                .insert(stored_registration_key(&stored), stored);
        }
        state.stored_registration_cache_revision = Some(revision);
    }

    fn restore_stored_registration_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        stored: ServiceWorkerStoredRegistration,
        launch_config: ServiceWorkerVersionLaunchConfig,
    ) -> Option<ServiceWorkerRegistrationId> {
        let registration_key = ServiceWorkerRegistrationKey {
            scope_url: stored.scope_url.clone(),
            storage_key: stored.storage_key.clone(),
        };
        if let Some(registration) = state
            .registrations
            .values()
            .find(|registration| registration.key() == registration_key)
        {
            return Some(registration.id);
        }
        if !matches!(
            stored.lifecycle_state,
            ServiceWorkerVersionLifecycleState::Installed
                | ServiceWorkerVersionLifecycleState::Activated
        ) {
            return None;
        }
        let registration_id = self.next_unused_registration_id_locked(state);
        let version_id = self.next_unused_version_id_locked(state);
        let (waiting_version_id, active_version_id) = match stored.lifecycle_state {
            ServiceWorkerVersionLifecycleState::Installed => (Some(version_id), None),
            ServiceWorkerVersionLifecycleState::Activated => (None, Some(version_id)),
            ServiceWorkerVersionLifecycleState::Installing
            | ServiceWorkerVersionLifecycleState::Activating
            | ServiceWorkerVersionLifecycleState::Redundant => return None,
        };
        state.registrations.insert(
            registration_id,
            ServiceWorkerRegistration {
                id: registration_id,
                storage_key: stored.storage_key.clone(),
                scope_url: stored.scope_url.clone(),
                script_url: stored.script_url.clone(),
                installing_version_id: None,
                waiting_version_id,
                active_version_id,
                pending_unregistration: false,
                update_via_cache: stored.update_via_cache,
                navigation_preload_state: stored.navigation_preload_state,
                last_update_check_time_ms: stored.last_update_check_time_ms,
                pending_register_jobs: HashMap::new(),
                controlled_client_ids: HashSet::new(),
            },
        );
        state.versions.insert(
            version_id,
            ServiceWorkerVersion {
                id: version_id,
                registration_id,
                script_url: stored.script_url.clone(),
                final_script_url: Some(stored.main_script_resource.final_url.clone()),
                main_script_resource: Some(stored.main_script_resource),
                imported_script_resources: stored.imported_script_resources,
                allow_identical_script_update: true,
                should_pause_on_start_for_devtools: false,
                script_kind: stored.script_kind,
                fetch_handler_existence: stored.fetch_handler_existence,
                fetch_handler_type: stored.fetch_handler_type,
                launch_config,
                lifecycle_state: stored.lifecycle_state,
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
        state.record_target_created(
            registration_id,
            version_id,
            stored.script_url.clone(),
            stored.scope_url.clone(),
        );
        Some(registration_id)
    }

    fn next_unused_registration_id_locked(
        &self,
        state: &ServiceWorkerRuntimeState,
    ) -> ServiceWorkerRegistrationId {
        loop {
            let registration_id = ServiceWorkerRegistrationId(
                self.inner
                    .next_registration_id
                    .fetch_add(1, Ordering::Relaxed),
            );
            if !state.registrations.contains_key(&registration_id) {
                return registration_id;
            }
        }
    }

    fn next_unused_version_id_locked(
        &self,
        state: &ServiceWorkerRuntimeState,
    ) -> ServiceWorkerVersionId {
        loop {
            let version_id =
                ServiceWorkerVersionId(self.inner.next_version_id.fetch_add(1, Ordering::Relaxed));
            if !state.versions.contains_key(&version_id) {
                return version_id;
            }
        }
    }

    pub(super) fn store_registration_resources_locked(
        &self,
        state: &ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Result<bool, ServiceWorkerRegistrationError> {
        let Some(record) =
            stored_registration_from_state_locked(state, registration_id, version_id)
        else {
            return Ok(false);
        };
        self.inner
            .resource_store
            .lock()
            .store_registration(record)
            .map(|()| true)
            .map_err(registration_error_for_resource_store_failure)
    }

    pub(super) fn delete_registration_resources_for_key_locked(
        &self,
        key: &ServiceWorkerRegistrationKey,
    ) {
        let _ = self.inner.resource_store.lock().delete_registration(key);
    }
}

fn registration_error_for_resource_store_failure(
    error: anyhow::Error,
) -> ServiceWorkerRegistrationError {
    ServiceWorkerRegistrationError::abort(format!(
        "failed to store Service Worker registration resources: {error:#}"
    ))
}

fn stored_registration_from_state_locked(
    state: &ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
    version_id: ServiceWorkerVersionId,
) -> Option<ServiceWorkerStoredRegistration> {
    let registration = state.registrations.get(&registration_id)?;
    if registration.pending_unregistration {
        return None;
    }
    let version = state.versions.get(&version_id)?;
    if version.registration_id != registration_id {
        return None;
    }
    match version.lifecycle_state {
        ServiceWorkerVersionLifecycleState::Installed => {
            if registration.waiting_version_id != Some(version_id) {
                return None;
            }
        }
        ServiceWorkerVersionLifecycleState::Activated => {
            if registration.active_version_id != Some(version_id) {
                return None;
            }
        }
        ServiceWorkerVersionLifecycleState::Installing
        | ServiceWorkerVersionLifecycleState::Activating
        | ServiceWorkerVersionLifecycleState::Redundant => return None,
    }
    let main_script_resource = version.main_script_resource.clone()?;
    Some(ServiceWorkerStoredRegistration {
        storage_key: registration.storage_key.clone(),
        scope_url: registration.scope_url.clone(),
        script_url: version.script_url.clone(),
        script_kind: version.script_kind,
        update_via_cache: registration.update_via_cache,
        navigation_preload_state: registration.navigation_preload_state.clone(),
        lifecycle_state: version.lifecycle_state,
        fetch_handler_existence: version.fetch_handler_existence,
        fetch_handler_type: version.fetch_handler_type,
        last_update_check_time_ms: registration.last_update_check_time_ms,
        main_script_resource,
        imported_script_resources: version.imported_script_resources.clone(),
    })
}

fn stored_registration_key(
    registration: &ServiceWorkerStoredRegistration,
) -> ServiceWorkerRegistrationKey {
    ServiceWorkerRegistrationKey {
        scope_url: registration.scope_url.clone(),
        storage_key: registration.storage_key.clone(),
    }
}
