use super::*;

impl ServiceWorkerRuntimeService {
    #[cfg(test)]
    pub(crate) fn register_client(
        &self,
        document_url: Url,
        document_owner_identity: u64,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerClientId {
        let storage_key =
            ServiceWorkerRegistrationKey::first_party_storage_key_for_url(&document_url);
        self.register_client_with_storage_key(
            document_url,
            storage_key,
            ServiceWorkerClientFrameType::TopLevel,
            Some(
                crate::window_document_identity::WindowDocumentOwner::for_test(
                    document_owner_identity,
                ),
            ),
            completion_tx,
        )
    }

    pub(crate) fn register_client_with_storage_key(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerClientId {
        let client_id = self.register_window_client_with_storage_key(
            document_url,
            storage_key,
            frame_type,
            document_owner,
            ServiceWorkerClientEndpoint::Page(completion_tx),
        );
        self.mark_client_execution_ready(client_id);
        client_id
    }

    pub(crate) fn register_reserved_client_with_storage_key(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
    ) -> ServiceWorkerClientId {
        self.register_reserved_client_with_storage_key_and_bypass(
            document_url,
            storage_key,
            frame_type,
            document_owner,
            false,
        )
    }

    pub(crate) fn register_reserved_client_with_storage_key_bypassing_service_worker(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
    ) -> ServiceWorkerClientId {
        self.register_reserved_client_with_storage_key_and_bypass(
            document_url,
            storage_key,
            frame_type,
            document_owner,
            true,
        )
    }

    fn register_reserved_client_with_storage_key_and_bypass(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        bypass_service_worker: bool,
    ) -> ServiceWorkerClientId {
        self.register_window_client_with_storage_key(
            document_url,
            storage_key,
            frame_type,
            document_owner,
            ServiceWorkerClientEndpoint::ReservedPage {
                bypass_service_worker,
            },
        )
    }

    pub(crate) fn register_worker_client_with_storage_key(
        &self,
        script_url: Url,
        storage_key: String,
        client_type: ServiceWorkerClientType,
        secure_context: bool,
        worker_tx: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerMessage>,
    ) -> ServiceWorkerClientId {
        debug_assert!(matches!(
            client_type,
            ServiceWorkerClientType::DedicatedWorker | ServiceWorkerClientType::SharedWorker
        ));
        let client_id =
            ServiceWorkerClientId(self.inner.next_client_id.fetch_add(1, Ordering::Relaxed));
        let current_script_url = service_worker_current_url_for_creation_url(&script_url);
        {
            let mut state = self.inner.state.lock();
            self.restore_stored_registrations_for_document_url_locked(
                &mut state,
                &current_script_url,
                &storage_key,
            );
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    exposed_id_generation: 0,
                    creation_url: script_url,
                    document_url: current_script_url.clone(),
                    client_type,
                    frame_type: ServiceWorkerClientFrameType::None,
                    visibility_state: ServiceWorkerClientVisibilityState::Hidden,
                    storage_key: storage_key.clone(),
                    secure_context,
                    execution_ready: true,
                    discarded_or_frozen: false,
                    document_owner: None,
                    endpoint: ServiceWorkerClientEndpoint::Worker(worker_tx),
                    focused: false,
                },
            );
            if secure_context
                && let Some(version_id) = select_controller_for_new_client_locked(
                    &mut state,
                    client_id,
                    &current_script_url,
                    &storage_key,
                )
            {
                state.record_target_version_updated(version_id);
            }
        }
        client_id
    }

    pub(crate) fn register_reserved_worker_client_with_storage_key(
        &self,
        script_url: Url,
        storage_key: String,
        client_type: ServiceWorkerClientType,
        secure_context: bool,
    ) -> ServiceWorkerClientId {
        debug_assert!(matches!(
            client_type,
            ServiceWorkerClientType::DedicatedWorker | ServiceWorkerClientType::SharedWorker
        ));
        let client_id =
            ServiceWorkerClientId(self.inner.next_client_id.fetch_add(1, Ordering::Relaxed));
        let current_script_url = service_worker_current_url_for_creation_url(&script_url);
        {
            let mut state = self.inner.state.lock();
            self.restore_stored_registrations_for_document_url_locked(
                &mut state,
                &current_script_url,
                &storage_key,
            );
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    exposed_id_generation: 0,
                    creation_url: script_url,
                    document_url: current_script_url.clone(),
                    client_type,
                    frame_type: ServiceWorkerClientFrameType::None,
                    visibility_state: ServiceWorkerClientVisibilityState::Hidden,
                    storage_key: storage_key.clone(),
                    secure_context,
                    execution_ready: false,
                    discarded_or_frozen: false,
                    document_owner: None,
                    endpoint: ServiceWorkerClientEndpoint::PendingWorker,
                    focused: false,
                },
            );
            if secure_context
                && let Some(version_id) = select_controller_for_new_client_locked(
                    &mut state,
                    client_id,
                    &current_script_url,
                    &storage_key,
                )
            {
                state.record_target_version_updated(version_id);
            }
        }
        client_id
    }

    pub(crate) fn register_reserved_worker_client_inheriting_controller_from_client(
        &self,
        script_url: Url,
        storage_key: String,
        client_type: ServiceWorkerClientType,
        secure_context: bool,
        parent_client_id: ServiceWorkerClientId,
    ) -> Option<ServiceWorkerClientId> {
        debug_assert!(matches!(
            client_type,
            ServiceWorkerClientType::DedicatedWorker | ServiceWorkerClientType::SharedWorker
        ));
        if script_url.scheme() != "blob" {
            return None;
        }
        let mut state = self.inner.state.lock();
        let parent_document_url = state
            .live_clients
            .get(&parent_client_id)
            .map(|client| client.document_url.clone())?;
        self.restore_stored_registrations_for_document_url_locked(
            &mut state,
            &parent_document_url,
            &storage_key,
        );
        let inherited_registration_id = secure_context
            .then(|| {
                state
                    .registrations
                    .values()
                    .filter(|registration| !registration.pending_unregistration)
                    .filter(|registration| registration.active_version_id.is_some())
                    .filter(|registration| {
                        registration
                            .controlled_client_ids
                            .contains(&parent_client_id)
                    })
                    .filter(|registration| {
                        service_worker_registration_matches_url(
                            registration,
                            &parent_document_url,
                            &storage_key,
                        )
                    })
                    .max_by_key(|registration| registration.scope_url.as_str().len())
                    .map(|registration| registration.id)
            })
            .flatten();
        let client_id =
            ServiceWorkerClientId(self.inner.next_client_id.fetch_add(1, Ordering::Relaxed));
        state.live_clients.insert(
            client_id,
            ServiceWorkerClient {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
                exposed_id_generation: 0,
                creation_url: script_url,
                document_url: parent_document_url,
                client_type,
                frame_type: ServiceWorkerClientFrameType::None,
                visibility_state: ServiceWorkerClientVisibilityState::Hidden,
                storage_key,
                secure_context,
                execution_ready: false,
                discarded_or_frozen: false,
                document_owner: None,
                endpoint: ServiceWorkerClientEndpoint::PendingWorker,
                focused: false,
            },
        );
        let controlled_version_id = if let Some(registration_id) = inherited_registration_id
            && let Some(registration) = state.registrations.get_mut(&registration_id)
            && registration.controlled_client_ids.insert(client_id)
        {
            registration.active_version_id
        } else {
            None
        };
        if let Some(version_id) = controlled_version_id {
            state.record_target_version_updated(version_id);
        }
        Some(client_id)
    }

    pub(crate) fn activate_reserved_worker_client(
        &self,
        client_id: ServiceWorkerClientId,
        worker_tx: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerMessage>,
    ) -> bool {
        let mut state = self.inner.state.lock();
        let Some(client) = state.live_clients.get_mut(&client_id) else {
            return false;
        };
        if !matches!(client.endpoint, ServiceWorkerClientEndpoint::PendingWorker) {
            return false;
        }
        client.execution_ready = true;
        client.endpoint = ServiceWorkerClientEndpoint::Worker(worker_tx);
        true
    }

    fn register_window_client_with_storage_key(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        endpoint: ServiceWorkerClientEndpoint,
    ) -> ServiceWorkerClientId {
        let client_id =
            ServiceWorkerClientId(self.inner.next_client_id.fetch_add(1, Ordering::Relaxed));
        let current_document_url = service_worker_current_url_for_creation_url(&document_url);
        let bypass_service_worker = matches!(
            &endpoint,
            ServiceWorkerClientEndpoint::ReservedPage {
                bypass_service_worker: true
            }
        );
        {
            let mut state = self.inner.state.lock();
            self.restore_stored_registrations_for_document_url_locked(
                &mut state,
                &current_document_url,
                &storage_key,
            );
            state.live_clients.insert(
                client_id,
                ServiceWorkerClient {
                    id: client_id,
                    exposed_id: service_worker_exposed_client_id(client_id),
                    exposed_id_generation: 0,
                    creation_url: document_url.clone(),
                    document_url: current_document_url.clone(),
                    client_type: ServiceWorkerClientType::Window,
                    frame_type,
                    visibility_state: ServiceWorkerClientVisibilityState::Visible,
                    storage_key: storage_key.clone(),
                    secure_context: true,
                    execution_ready: false,
                    discarded_or_frozen: false,
                    document_owner,
                    endpoint,
                    focused: false,
                },
            );
            if !bypass_service_worker
                && let Some(version_id) = select_controller_for_new_client_locked(
                    &mut state,
                    client_id,
                    &current_document_url,
                    &storage_key,
                )
            {
                state.record_target_version_updated(version_id);
            }
        }
        client_id
    }

    fn mark_client_execution_ready(&self, client_id: ServiceWorkerClientId) {
        if let Some(client) = self.inner.state.lock().live_clients.get_mut(&client_id) {
            client.execution_ready = true;
        }
    }

    pub(crate) fn unregister_client(&self, client_id: ServiceWorkerClientId) {
        let progress = {
            let mut state = self.inner.state.lock();
            state.live_clients.remove(&client_id);
            let mut registration_ids = Vec::new();
            for registration in state.registrations.values_mut() {
                if registration.controlled_client_ids.remove(&client_id) {
                    registration_ids.push(registration.id);
                }
            }
            record_target_version_updates_for_registration_ids_locked(
                &mut state,
                registration_ids.iter().copied(),
            );
            registration_ids
                .into_iter()
                .flat_map(|registration_id| {
                    if state
                        .registrations
                        .get(&registration_id)
                        .is_some_and(|registration| registration.pending_unregistration)
                    {
                        self.unregistration_progress_for_registration_if_ready_locked(
                            &mut state,
                            registration_id,
                        )
                    } else {
                        self.activation_progress_for_registration_if_ready_locked(
                            &mut state,
                            registration_id,
                        )
                    }
                })
                .collect::<Vec<_>>()
        };
        for progress in progress {
            self.run_lifecycle_progress(progress);
        }
    }

    pub(crate) fn update_client_document_with_storage_key(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
    ) -> bool {
        self.update_client_document_with_storage_key_internal(
            client_id,
            document_url,
            storage_key,
            frame_type,
            document_owner,
            None,
        )
    }

    pub(crate) fn update_client_document_with_storage_key_and_completion_sender(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> bool {
        self.update_client_document_with_storage_key_internal(
            client_id,
            document_url,
            storage_key,
            frame_type,
            document_owner,
            Some(completion_tx),
        )
    }

    fn update_client_document_with_storage_key_internal(
        &self,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: Option<RendererPageServiceWorkerTaskSender>,
    ) -> bool {
        let current_document_url = service_worker_current_url_for_creation_url(&document_url);
        let progress = {
            let mut state = self.inner.state.lock();
            if !state.live_clients.contains_key(&client_id) {
                return false;
            }
            let bypass_service_worker = state.live_clients.get(&client_id).is_some_and(|client| {
                matches!(
                    &client.endpoint,
                    ServiceWorkerClientEndpoint::ReservedPage {
                        bypass_service_worker: true
                    }
                )
            });
            let mut registration_ids = Vec::new();
            for registration in state.registrations.values_mut() {
                if registration.controlled_client_ids.remove(&client_id) {
                    registration_ids.push(registration.id);
                }
            }
            record_target_version_updates_for_registration_ids_locked(
                &mut state,
                registration_ids.iter().copied(),
            );
            if let Some(client) = state.live_clients.get_mut(&client_id) {
                if !moli_url::same_origin(&client.document_url, &current_document_url) {
                    client.exposed_id_generation = service_worker_next_exposed_client_id_generation(
                        client.exposed_id_generation,
                    );
                    client.exposed_id = service_worker_exposed_client_id_for_generation(
                        client.id,
                        client.exposed_id_generation,
                    );
                }
                client.creation_url = document_url.clone();
                client.document_url = current_document_url.clone();
                client.frame_type = frame_type;
                client.storage_key = storage_key.clone();
                client.execution_ready = true;
                client.document_owner = document_owner;
                if let Some(completion_tx) = completion_tx {
                    client.endpoint = ServiceWorkerClientEndpoint::Page(completion_tx);
                }
            }
            self.restore_stored_registrations_for_document_url_locked(
                &mut state,
                &current_document_url,
                &storage_key,
            );
            if !bypass_service_worker
                && let Some(version_id) = select_controller_for_new_client_locked(
                    &mut state,
                    client_id,
                    &current_document_url,
                    &storage_key,
                )
            {
                state.record_target_version_updated(version_id);
            }
            registration_ids
                .into_iter()
                .flat_map(|registration_id| {
                    if state
                        .registrations
                        .get(&registration_id)
                        .is_some_and(|registration| registration.pending_unregistration)
                    {
                        self.unregistration_progress_for_registration_if_ready_locked(
                            &mut state,
                            registration_id,
                        )
                    } else {
                        self.activation_progress_for_registration_if_ready_locked(
                            &mut state,
                            registration_id,
                        )
                    }
                })
                .collect::<Vec<_>>()
        };
        for progress in progress {
            self.run_lifecycle_progress(progress);
        }
        true
    }

    pub(crate) fn controlled_window_client_urls_for_version_for_devtools(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Vec<String> {
        let state = self.inner.state.lock();
        let Some(registration) =
            service_worker_query_registration_locked(&state, registration_id, version_id)
        else {
            return Vec::new();
        };
        let mut urls = registration
            .controlled_client_ids
            .iter()
            .filter_map(|client_id| state.live_clients.get(client_id))
            .filter(|client| client.client_type == ServiceWorkerClientType::Window)
            .filter(|client| service_worker_client_is_exposed_to_clients_api(client))
            .filter(|client| service_worker_registration_matches_client(registration, client))
            .map(|client| client.document_url.as_str().to_owned())
            .collect::<Vec<_>>();
        urls.sort();
        urls.dedup();
        urls
    }

    pub(crate) fn controlled_window_client_ids_for_version_for_devtools(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Vec<u64> {
        let state = self.inner.state.lock();
        let Some(registration) =
            service_worker_query_registration_locked(&state, registration_id, version_id)
        else {
            return Vec::new();
        };
        let mut client_ids = registration
            .controlled_client_ids
            .iter()
            .filter_map(|client_id| state.live_clients.get(client_id))
            .filter(|client| client.client_type == ServiceWorkerClientType::Window)
            .filter(|client| service_worker_client_is_exposed_to_clients_api(client))
            .filter(|client| service_worker_registration_matches_client(registration, client))
            .map(|client| client.id.as_u64())
            .collect::<Vec<_>>();
        client_ids.sort_unstable();
        client_ids.dedup();
        client_ids
    }

    pub(crate) fn query_clients(
        &self,
        query: &ServiceWorkerClientQuery,
    ) -> ServiceWorkerClientQueryResult {
        let state = self.inner.state.lock();
        let clients = match &query.kind {
            ServiceWorkerClientQueryKind::Get { exposed_client_id } => {
                service_worker_get_client_locked(
                    &state,
                    query.registration_id,
                    query.version_id,
                    exposed_client_id,
                )
                .into_iter()
                .collect()
            }
            ServiceWorkerClientQueryKind::MatchAll { options } => {
                service_worker_match_all_clients_locked(
                    &state,
                    query.registration_id,
                    query.version_id,
                    *options,
                )
            }
        };
        ServiceWorkerClientQueryResult {
            request_id: query.request_id,
            clients,
        }
    }
}

#[derive(Default)]
pub(super) struct ClaimLiveClientsResult {
    pub(super) changed_registration_ids: Vec<ServiceWorkerRegistrationId>,
    pub(super) controller_change_deliveries: Vec<ServiceWorkerControllerChangeDelivery>,
}

pub(super) fn controller_change_deliveries_for_controlled_clients_locked(
    state: &ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
) -> Vec<ServiceWorkerControllerChangeDelivery> {
    let Some(registration) = state.registrations.get(&registration_id) else {
        return Vec::new();
    };
    registration
        .controlled_client_ids
        .iter()
        .filter_map(|client_id| state.live_clients.get(client_id))
        .map(|client| ServiceWorkerControllerChangeDelivery {
            target: client.window_completion_target(),
            endpoint: client.endpoint.clone(),
        })
        .collect()
}

pub(super) fn claim_live_scope_clients_locked(
    state: &mut ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
) -> ClaimLiveClientsResult {
    if !state.registrations.contains_key(&registration_id) {
        return ClaimLiveClientsResult::default();
    }
    let claimed_client_ids = state
        .live_clients
        .values()
        .filter(|client| {
            service_worker_client_is_claimable_by_registration(state, client, registration_id)
        })
        .map(|client| client.id)
        .collect::<HashSet<_>>();
    if claimed_client_ids.is_empty() {
        return ClaimLiveClientsResult::default();
    }
    let previously_controlled_client_ids = state
        .registrations
        .get(&registration_id)
        .map(|registration| registration.controlled_client_ids.clone())
        .unwrap_or_default();
    let controller_change_deliveries = claimed_client_ids
        .iter()
        .filter(|client_id| !previously_controlled_client_ids.contains(client_id))
        .filter_map(|client_id| state.live_clients.get(client_id))
        .map(|client| ServiceWorkerControllerChangeDelivery {
            target: client.window_completion_target(),
            endpoint: client.endpoint.clone(),
        })
        .collect::<Vec<_>>();
    let mut changed_registration_ids = HashSet::new();
    for registration in state.registrations.values_mut() {
        if registration.id == registration_id {
            continue;
        }
        for client_id in &claimed_client_ids {
            if registration.controlled_client_ids.remove(client_id) {
                changed_registration_ids.insert(registration.id);
            }
        }
    }
    record_target_version_updates_for_registration_ids_locked(
        state,
        changed_registration_ids.iter().copied(),
    );
    let Some(registration) = state.registrations.get_mut(&registration_id) else {
        return ClaimLiveClientsResult {
            changed_registration_ids: changed_registration_ids.into_iter().collect(),
            controller_change_deliveries,
        };
    };
    let target_active_version_id = registration.active_version_id;
    let mut target_controlled_changed = false;
    for client_id in claimed_client_ids {
        target_controlled_changed =
            registration.controlled_client_ids.insert(client_id) || target_controlled_changed;
    }
    if target_controlled_changed && let Some(version_id) = target_active_version_id {
        state.record_target_version_updated(version_id);
    }
    ClaimLiveClientsResult {
        changed_registration_ids: changed_registration_ids.into_iter().collect(),
        controller_change_deliveries,
    }
}

fn record_target_version_updates_for_registration_ids_locked(
    state: &mut ServiceWorkerRuntimeState,
    registration_ids: impl IntoIterator<Item = ServiceWorkerRegistrationId>,
) {
    let version_ids = registration_ids
        .into_iter()
        .filter_map(|registration_id| {
            state
                .registrations
                .get(&registration_id)
                .and_then(|registration| registration.active_version_id)
        })
        .collect::<Vec<_>>();
    record_target_version_updates_locked(state, version_ids);
}

fn record_target_version_updates_locked(
    state: &mut ServiceWorkerRuntimeState,
    version_ids: impl IntoIterator<Item = ServiceWorkerVersionId>,
) {
    let mut seen_version_ids = HashSet::new();
    for version_id in version_ids {
        if seen_version_ids.insert(version_id) {
            state.record_target_version_updated(version_id);
        }
    }
}

fn service_worker_client_is_claimable_by_registration(
    state: &ServiceWorkerRuntimeState,
    client: &ServiceWorkerClient,
    registration_id: ServiceWorkerRegistrationId,
) -> bool {
    client.execution_ready
        && !client.discarded_or_frozen
        && client.secure_context
        && service_worker_matching_registration_id_for_client(state, client)
            == Some(registration_id)
}

fn service_worker_matching_registration_id_for_client(
    state: &ServiceWorkerRuntimeState,
    client: &ServiceWorkerClient,
) -> Option<ServiceWorkerRegistrationId> {
    state
        .registrations
        .values()
        .filter(|registration| !registration.pending_unregistration)
        .filter(|registration| registration.active_version_id.is_some())
        .filter(|registration| service_worker_registration_matches_client(registration, client))
        .max_by_key(|registration| registration.scope_url.as_str().len())
        .map(|registration| registration.id)
}

fn service_worker_get_client_locked(
    state: &ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
    version_id: ServiceWorkerVersionId,
    exposed_client_id: &str,
) -> Option<ServiceWorkerClientSnapshot> {
    let registration =
        service_worker_query_registration_locked(state, registration_id, version_id)?;
    let client = state
        .live_clients
        .values()
        .find(|client| client.exposed_id == exposed_client_id)?;
    if !service_worker_client_is_exposed_to_clients_api(client) {
        return None;
    }
    if !service_worker_registration_can_see_client(registration, client) {
        return None;
    }
    Some(service_worker_client_snapshot(registration, client))
}

fn service_worker_match_all_clients_locked(
    state: &ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
    version_id: ServiceWorkerVersionId,
    options: ServiceWorkerClientQueryOptions,
) -> Vec<ServiceWorkerClientSnapshot> {
    let Some(registration) =
        service_worker_query_registration_locked(state, registration_id, version_id)
    else {
        return Vec::new();
    };
    let mut clients = state
        .live_clients
        .values()
        .filter_map(|client| {
            service_worker_client_visible_to_query(
                registration,
                client.id,
                client,
                options.include_uncontrolled,
                options.client_type,
            )
        })
        .collect::<Vec<_>>();
    service_worker_sort_client_snapshots(&mut clients);
    clients
}

fn service_worker_query_registration_locked(
    state: &ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
    version_id: ServiceWorkerVersionId,
) -> Option<&ServiceWorkerRegistration> {
    let version = state.versions.get(&version_id)?;
    if version.registration_id != registration_id
        || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
    {
        return None;
    }
    let registration = state.registrations.get(&registration_id)?;
    if registration.pending_unregistration || registration.active_version_id != Some(version_id) {
        return None;
    }
    Some(registration)
}

fn service_worker_client_visible_to_query(
    registration: &ServiceWorkerRegistration,
    client_id: ServiceWorkerClientId,
    client: &ServiceWorkerClient,
    include_uncontrolled: bool,
    query_type: ServiceWorkerClientQueryType,
) -> Option<ServiceWorkerClientSnapshot> {
    if !service_worker_client_type_matches_query(client.client_type, query_type) {
        return None;
    }
    if !service_worker_registration_matches_client(registration, client) {
        return None;
    }
    if !service_worker_client_is_exposed_to_clients_api(client) {
        return None;
    }
    let controlled = registration.controlled_client_ids.contains(&client_id);
    if !include_uncontrolled && !controlled {
        return None;
    }
    Some(service_worker_client_snapshot_with_controlled(
        client, controlled,
    ))
}

fn service_worker_client_type_matches_query(
    client_type: ServiceWorkerClientType,
    query_type: ServiceWorkerClientQueryType,
) -> bool {
    match query_type {
        ServiceWorkerClientQueryType::All => true,
        ServiceWorkerClientQueryType::Window => client_type == ServiceWorkerClientType::Window,
        ServiceWorkerClientQueryType::Worker => {
            client_type == ServiceWorkerClientType::DedicatedWorker
        }
        ServiceWorkerClientQueryType::SharedWorker => {
            client_type == ServiceWorkerClientType::SharedWorker
        }
    }
}

pub(super) fn service_worker_client_snapshot(
    registration: &ServiceWorkerRegistration,
    client: &ServiceWorkerClient,
) -> ServiceWorkerClientSnapshot {
    service_worker_client_snapshot_with_controlled(
        client,
        registration.controlled_client_ids.contains(&client.id),
    )
}

pub(super) fn service_worker_client_snapshot_with_controlled(
    client: &ServiceWorkerClient,
    controlled: bool,
) -> ServiceWorkerClientSnapshot {
    ServiceWorkerClientSnapshot {
        id: client.id,
        exposed_id: client.exposed_id.clone(),
        url: client.creation_url.clone(),
        client_type: client.client_type,
        frame_type: client.frame_type,
        visibility_state: client.visibility_state,
        controlled,
        focused: client.focused,
    }
}

fn service_worker_client_is_exposed_to_clients_api(client: &ServiceWorkerClient) -> bool {
    client.execution_ready && !client.discarded_or_frozen
}

fn service_worker_sort_client_snapshots(clients: &mut [ServiceWorkerClientSnapshot]) {
    clients.sort_by(|left, right| {
        let left_is_window = left.client_type == ServiceWorkerClientType::Window;
        let right_is_window = right.client_type == ServiceWorkerClientType::Window;
        right_is_window
            .cmp(&left_is_window)
            .then_with(|| right.focused.cmp(&left.focused))
            .then_with(|| left.id.as_u64().cmp(&right.id.as_u64()))
    });
}
