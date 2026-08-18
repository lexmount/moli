use super::*;

impl ServiceWorkerRuntimeService {
    #[cfg(test)]
    pub(crate) fn watch_ready_registration(
        &self,
        document_url: Url,
        request_id: u64,
        document_owner_identity: u64,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> bool {
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
        self.watch_ready_registration_with_storage_key(
            document_url,
            storage_key,
            request_id,
            crate::window_document_identity::WindowDocumentOwner::for_test(document_owner_identity),
            completion_tx,
        )
    }

    pub(crate) fn watch_ready_registration_with_storage_key(
        &self,
        document_url: Url,
        storage_key: String,
        request_id: u64,
        document_owner: crate::window_document_identity::WindowDocumentOwner,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> bool {
        let current_document_url = service_worker_current_url_for_creation_url(&document_url);
        let mut state = self.inner.state.lock();
        self.restore_stored_registrations_for_document_url_locked(
            &mut state,
            &current_document_url,
            &storage_key,
        );
        let active_registration_id = state
            .registrations
            .values()
            .filter(|registration| !registration.pending_unregistration)
            .filter(|registration| registration.active_version_id.is_some())
            .filter(|registration| {
                service_worker_registration_matches_url(
                    registration,
                    &current_document_url,
                    &storage_key,
                )
            })
            .max_by_key(|registration| registration.scope_url.as_str().len())
            .map(|registration| registration.id);
        if let Some(registration_id) = active_registration_id
            && let Some(registration) = state.registrations.get(&registration_id)
        {
            let snapshot = service_worker_registration_snapshot(&state, registration);
            drop(state);
            let _ = completion_tx.send_service_worker_ready(ServiceWorkerReadyCompletion {
                request_id,
                document_owner,
                registration: snapshot,
            });
            return true;
        }
        let Some(registration_id) = state
            .registrations
            .values()
            .filter(|registration| !registration.pending_unregistration)
            .filter(|registration| {
                service_worker_registration_matches_url(
                    registration,
                    &current_document_url,
                    &storage_key,
                )
            })
            .max_by_key(|registration| registration.scope_url.as_str().len())
            .map(|registration| registration.id)
        else {
            return false;
        };
        if !state.registrations.contains_key(&registration_id) {
            return false;
        }
        let ready_job = ServiceWorkerReadyJob {
            request_id,
            document_owner,
            completion_tx,
            registration_id,
        };
        state.pending_ready_jobs.push(ready_job);
        true
    }

    pub(crate) fn watch_registration_lifecycle(
        &self,
        scope_url: Url,
        storage_key: String,
        document_owner: crate::window_document_identity::WindowDocumentOwner,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) {
        self.inner
            .state
            .lock()
            .lifecycle_watchers
            .push(ServiceWorkerLifecycleWatcher {
                scope_url,
                storage_key,
                document_owner,
                completion_tx,
            });
    }

    pub(crate) fn navigation_preload_state_for_scope(
        &self,
        scope_url: &Url,
    ) -> Option<ServiceWorkerNavigationPreloadState> {
        let state = self.inner.state.lock();
        state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url
                    && !registration.pending_unregistration
                    && registration.active_version_id.is_some()
            })
            .map(|registration| registration.navigation_preload_state.clone())
    }

    pub(crate) fn set_navigation_preload_enabled_for_scope(
        &self,
        scope_url: &Url,
        enabled: bool,
    ) -> Result<(), ServiceWorkerNavigationPreloadStateError> {
        self.update_navigation_preload_state_for_scope(scope_url, |state| {
            state.enabled = enabled;
        })
    }

    pub(crate) fn set_navigation_preload_header_value_for_scope(
        &self,
        scope_url: &Url,
        header_value: String,
    ) -> Result<(), ServiceWorkerNavigationPreloadStateError> {
        self.update_navigation_preload_state_for_scope(scope_url, |state| {
            state.header_value = header_value;
        })
    }

    fn update_navigation_preload_state_for_scope(
        &self,
        scope_url: &Url,
        update: impl FnOnce(&mut ServiceWorkerNavigationPreloadState),
    ) -> Result<(), ServiceWorkerNavigationPreloadStateError> {
        let mut state = self.inner.state.lock();
        let Some(registration_id) = state
            .registrations
            .values()
            .find(|registration| {
                registration.scope_url == *scope_url && !registration.pending_unregistration
            })
            .map(|registration| registration.id)
        else {
            return Err(ServiceWorkerNavigationPreloadStateError::InvalidState);
        };
        let (version_id, previous_state) = {
            let Some(registration) = state.registrations.get_mut(&registration_id) else {
                return Err(ServiceWorkerNavigationPreloadStateError::InvalidState);
            };
            let Some(version_id) = registration.active_version_id else {
                return Err(ServiceWorkerNavigationPreloadStateError::InvalidState);
            };
            let previous_state = registration.navigation_preload_state.clone();
            update(&mut registration.navigation_preload_state);
            (version_id, previous_state)
        };
        if self
            .store_registration_resources_locked(&state, registration_id, version_id)
            .is_ok()
        {
            return Ok(());
        }
        if let Some(registration) = state.registrations.get_mut(&registration_id) {
            registration.navigation_preload_state = previous_state;
        }
        Err(ServiceWorkerNavigationPreloadStateError::StorageFailure)
    }

    #[cfg(test)]
    pub(crate) fn matching_registration_for_client(
        &self,
        client_url: &Url,
    ) -> Option<ServiceWorkerRegistrationSnapshot> {
        let storage_key = ServiceWorkerRegistrationKey::first_party_storage_key_for_url(client_url);
        self.matching_registration_for_client_with_storage_key(client_url, &storage_key)
    }

    pub(crate) fn matching_registration_for_client_with_storage_key(
        &self,
        client_url: &Url,
        storage_key: &str,
    ) -> Option<ServiceWorkerRegistrationSnapshot> {
        let current_client_url = service_worker_current_url_for_creation_url(client_url);
        let mut state = self.inner.state.lock();
        self.restore_stored_registrations_for_document_url_locked(
            &mut state,
            &current_client_url,
            storage_key,
        );
        state
            .registrations
            .values()
            .filter(|registration| !registration.pending_unregistration)
            .filter(|registration| {
                service_worker_registration_matches_url(
                    registration,
                    &current_client_url,
                    storage_key,
                )
            })
            .max_by_key(|registration| registration.scope_url.as_str().len())
            .map(|registration| service_worker_registration_snapshot(&state, registration))
    }

    pub(crate) fn registration_snapshot_by_id(
        &self,
        registration_id: ServiceWorkerRegistrationId,
    ) -> Option<ServiceWorkerRegistrationSnapshot> {
        let state = self.inner.state.lock();
        state
            .registrations
            .get(&registration_id)
            .filter(|registration| !registration.pending_unregistration)
            .map(|registration| service_worker_registration_snapshot(&state, registration))
    }

    #[cfg(test)]
    pub(crate) fn all_registrations(
        &self,
        document_url: &Url,
    ) -> Vec<ServiceWorkerRegistrationSnapshot> {
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(document_url);
        self.all_registrations_with_storage_key(document_url, &storage_key)
    }

    pub(crate) fn all_registrations_with_storage_key(
        &self,
        document_url: &Url,
        storage_key: &str,
    ) -> Vec<ServiceWorkerRegistrationSnapshot> {
        let current_document_url = service_worker_current_url_for_creation_url(document_url);
        let mut state = self.inner.state.lock();
        self.restore_stored_registrations_for_document_url_locked(
            &mut state,
            &current_document_url,
            storage_key,
        );
        state
            .registrations
            .values()
            .filter(|registration| !registration.pending_unregistration)
            .filter(|registration| registration.storage_key == storage_key)
            .map(|registration| service_worker_registration_snapshot(&state, registration))
            .collect()
    }

    pub(crate) fn matching_controller_for_client(
        &self,
        client_id: ServiceWorkerClientId,
    ) -> Option<ServiceWorkerControlState> {
        let state = self.inner.state.lock();
        let client = state.live_clients.get(&client_id)?;
        state
            .registrations
            .values()
            .filter(|registration| registration.active_version_id.is_some())
            .filter(|registration| registration.controlled_client_ids.contains(&client_id))
            .filter(|registration| service_worker_registration_matches_client(registration, client))
            .max_by_key(|registration| registration.scope_url.as_str().len())
            .and_then(|registration| {
                service_worker_control_state_from_registration_active_version(&state, registration)
            })
    }

    pub(crate) fn matching_controller_for_client_fetch(
        &self,
        client_id: ServiceWorkerClientId,
        _request_url: &Url,
    ) -> Option<ServiceWorkerControlState> {
        let state = self.inner.state.lock();
        let client = state.live_clients.get(&client_id)?;
        let registration_id = state
            .registrations
            .values()
            .filter(|registration| registration.active_version_id.is_some())
            .filter(|registration| registration.controlled_client_ids.contains(&client_id))
            .filter(|registration| service_worker_registration_matches_client(registration, client))
            .max_by_key(|registration| registration.scope_url.as_str().len())
            .map(|registration| registration.id)?;
        let registration = state.registrations.get(&registration_id)?;
        registration.active_version_id?;
        let control_state =
            service_worker_control_state_from_registration_active_version(&state, registration)?;
        Some(control_state)
    }

    #[cfg(test)]
    pub(crate) fn matching_controller_for_document(
        &self,
        document_url: &Url,
    ) -> Option<ServiceWorkerControlState> {
        let current_document_url = service_worker_current_url_for_creation_url(document_url);
        let client_id = {
            let state = self.inner.state.lock();
            state
                .live_clients
                .values()
                .find(|client| client.document_url == current_document_url)
                .map(|client| client.id)
        }?;
        self.matching_controller_for_client(client_id)
    }

    #[cfg(test)]
    pub(crate) fn matching_controller_for_fetch(
        &self,
        document_url: &Url,
        request_url: &Url,
    ) -> Option<ServiceWorkerControlState> {
        let current_document_url = service_worker_current_url_for_creation_url(document_url);
        let client_id = {
            let state = self.inner.state.lock();
            state
                .live_clients
                .values()
                .find(|client| client.document_url == current_document_url)
                .map(|client| client.id)
        }?;
        self.matching_controller_for_client_fetch(client_id, request_url)
    }
}

fn service_worker_control_state_from_registration_active_version(
    state: &ServiceWorkerRuntimeState,
    registration: &ServiceWorkerRegistration,
) -> Option<ServiceWorkerControlState> {
    let active_version_id = registration.active_version_id?;
    let active_version = state.versions.get(&active_version_id)?;
    Some(ServiceWorkerControlState::new(
        registration.id,
        Some(active_version_id),
        active_version.script_url.clone(),
        registration.scope_url.clone(),
    ))
}

pub(super) fn active_registration_id_for_scope_locked(
    state: &ServiceWorkerRuntimeState,
    scope_url: &Url,
) -> Option<ServiceWorkerRegistrationId> {
    let registration = state.registrations.values().find(|registration| {
        registration.scope_url == *scope_url
            && !registration.pending_unregistration
            && registration.active_version_id.is_some()
    })?;
    let version = state.versions.get(&registration.active_version_id?)?;
    if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
        return None;
    }
    Some(registration.id)
}

pub(super) fn select_controller_for_new_client_locked(
    state: &mut ServiceWorkerRuntimeState,
    client_id: ServiceWorkerClientId,
    document_url: &Url,
    storage_key: &str,
) -> Option<ServiceWorkerVersionId> {
    let registration_id = state
        .registrations
        .values()
        .filter(|registration| !registration.pending_unregistration)
        .filter(|registration| registration.active_version_id.is_some())
        .filter(|registration| {
            service_worker_registration_matches_url(registration, document_url, storage_key)
        })
        .max_by_key(|registration| registration.scope_url.as_str().len())
        .map(|registration| registration.id);
    if let Some(registration_id) = registration_id
        && let Some(registration) = state.registrations.get_mut(&registration_id)
        && registration.controlled_client_ids.insert(client_id)
    {
        return registration.active_version_id;
    }
    None
}

pub(super) fn service_worker_registration_matches_url(
    registration: &ServiceWorkerRegistration,
    document_url: &Url,
    storage_key: &str,
) -> bool {
    service_worker_registration_storage_key_matches(registration, storage_key)
        && service_worker_scope_matches_url(&registration.scope_url, document_url)
}

pub(super) fn service_worker_registration_matches_client(
    registration: &ServiceWorkerRegistration,
    client: &ServiceWorkerClient,
) -> bool {
    service_worker_registration_matches_url(registration, &client.document_url, &client.storage_key)
}

pub(super) fn service_worker_registration_can_see_client(
    registration: &ServiceWorkerRegistration,
    client: &ServiceWorkerClient,
) -> bool {
    service_worker_registration_storage_key_matches(registration, &client.storage_key)
        && moli_url::same_origin(&registration.scope_url, &client.document_url)
}

fn service_worker_registration_storage_key_matches(
    registration: &ServiceWorkerRegistration,
    storage_key: &str,
) -> bool {
    registration.storage_key == storage_key
}
