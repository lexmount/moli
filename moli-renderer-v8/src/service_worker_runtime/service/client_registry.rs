use super::registration_lookup::{
    attach_controller_to_client_locked, controller_registration_for_client_locked,
};
use super::*;

type WindowClientQueryTarget = (
    crate::types::ServiceWorkerWindowClientTarget,
    crate::page_task_queue::RendererPageServiceWorkerTaskSender,
);

impl ServiceWorkerRuntimeService {
    /// The current environment ID, including replacement documents which kept
    /// the runtime's internal client ID but received a new public Client.id.
    pub(crate) fn client_exposed_id(&self, client_id: ServiceWorkerClientId) -> Option<String> {
        self.inner
            .state
            .lock()
            .live_clients
            .get(&client_id)
            .map(|client| client.exposed_id.clone())
    }
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

    #[cfg(test)]
    pub(crate) fn register_client_with_storage_key(
        &self,
        document_url: Url,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> ServiceWorkerClientId {
        let client_id = self.inner.client_id_allocator.allocate();
        let inserted = self.register_window_client_with_storage_key(
            client_id,
            document_url.into(),
            storage_key,
            frame_type,
            document_owner,
            ServiceWorkerClientEndpoint::Page(completion_tx),
        );
        debug_assert!(inserted, "new ServiceWorker client id must be unused");
        self.mark_client_execution_ready(client_id);
        client_id
    }

    pub(crate) fn register_allocated_client_with_storage_key(
        &self,
        client_id: ServiceWorkerClientId,
        document: ServiceWorkerWindowClientDocument,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> bool {
        if !self.register_window_client_with_storage_key(
            client_id,
            document,
            storage_key,
            frame_type,
            document_owner,
            ServiceWorkerClientEndpoint::Page(completion_tx),
        ) {
            return false;
        }
        self.mark_client_execution_ready(client_id);
        true
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
        let client_id = self.inner.client_id_allocator.allocate();
        let inserted = self.register_window_client_with_storage_key(
            client_id,
            document_url.into(),
            storage_key,
            frame_type,
            document_owner,
            ServiceWorkerClientEndpoint::ReservedPage {
                bypass_service_worker,
            },
        );
        debug_assert!(
            inserted,
            "new reserved ServiceWorker client id must be unused"
        );
        client_id
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
        let client_id = self.inner.client_id_allocator.allocate();
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
        let client_id = self.inner.client_id_allocator.allocate();
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
        let client_id = self.inner.client_id_allocator.allocate();
        state.live_clients.insert(
            client_id,
            ServiceWorkerClient {
                id: client_id,
                exposed_id: service_worker_exposed_client_id(client_id),
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

    /// Match a main worker request's current URL without promoting its reserved
    /// environment or allocating a second client when a redirect changes scope.
    pub(super) fn retarget_reserved_worker_client(
        &self,
        client_id: ServiceWorkerClientId,
        script_url: Url,
    ) -> bool {
        let current_url = service_worker_current_url_for_creation_url(&script_url);
        let progress = {
            let mut state = self.inner.state.lock();
            let Some(client) = state.live_clients.get(&client_id) else {
                return false;
            };
            if !matches!(client.endpoint, ServiceWorkerClientEndpoint::PendingWorker) {
                return false;
            }
            let storage_key = client.storage_key.clone();
            let secure_context = client.secure_context;
            let mut removed = Vec::new();
            for registration in state.registrations.values_mut() {
                if registration.controlled_client_ids.remove(&client_id) {
                    removed.push(registration.id);
                }
            }
            record_target_version_updates_for_registration_ids_locked(
                &mut state,
                removed.iter().copied(),
            );
            let client = state
                .live_clients
                .get_mut(&client_id)
                .expect("reserved worker remains live under the state lock");
            client.creation_url = script_url;
            client.document_url = current_url.clone();
            self.restore_stored_registrations_for_document_url_locked(
                &mut state,
                &current_url,
                &storage_key,
            );
            if secure_context
                && let Some(version_id) = select_controller_for_new_client_locked(
                    &mut state,
                    client_id,
                    &current_url,
                    &storage_key,
                )
            {
                state.record_target_version_updated(version_id);
            }
            removed
                .into_iter()
                .flat_map(|id| {
                    if state
                        .registrations
                        .get(&id)
                        .is_some_and(|registration| registration.pending_unregistration)
                    {
                        self.unregistration_progress_for_registration_if_ready_locked(
                            &mut state, id,
                        )
                    } else {
                        self.activation_progress_for_registration_if_ready_locked(&mut state, id)
                    }
                })
                .collect::<Vec<_>>()
        };
        for progress in progress {
            self.run_lifecycle_progress(progress);
        }
        true
    }

    /// The response URL becomes the worker's creation URL. Its controller is
    /// still the one selected for the main request, including a SW response
    /// whose URL lies outside that registration's scope.
    pub(crate) fn update_reserved_worker_client_creation_url(
        &self,
        client_id: ServiceWorkerClientId,
        script_url: Url,
    ) -> bool {
        let mut state = self.inner.state.lock();
        let Some(client) = state.live_clients.get_mut(&client_id) else {
            return false;
        };
        if !matches!(client.endpoint, ServiceWorkerClientEndpoint::PendingWorker) {
            return false;
        }
        client.creation_url = script_url;
        true
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
        client_id: ServiceWorkerClientId,
        document: ServiceWorkerWindowClientDocument,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        endpoint: ServiceWorkerClientEndpoint,
    ) -> bool {
        let current_document_url = service_worker_current_url_for_creation_url(&document.url);
        let bypass_service_worker = matches!(
            &endpoint,
            ServiceWorkerClientEndpoint::ReservedPage {
                bypass_service_worker: true
            }
        );
        {
            let mut state = self.inner.state.lock();
            if state.live_clients.contains_key(&client_id) {
                return false;
            }
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
                    creation_url: document.url,
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
                && let Some(version_id) = select_window_client_controller_locked(
                    &mut state,
                    client_id,
                    &current_document_url,
                    &storage_key,
                    document.controller_source,
                )
            {
                state.record_target_version_updated(version_id);
            }
        }
        true
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
            document_url.into(),
            storage_key,
            frame_type,
            document_owner,
            None,
        )
    }

    pub(crate) fn update_client_document_with_storage_key_and_completion_sender(
        &self,
        client_id: ServiceWorkerClientId,
        document: ServiceWorkerWindowClientDocument,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: RendererPageServiceWorkerTaskSender,
    ) -> bool {
        self.update_client_document_with_storage_key_internal(
            client_id,
            document,
            storage_key,
            frame_type,
            document_owner,
            Some(completion_tx),
        )
    }

    fn update_client_document_with_storage_key_internal(
        &self,
        client_id: ServiceWorkerClientId,
        document: ServiceWorkerWindowClientDocument,
        storage_key: String,
        frame_type: ServiceWorkerClientFrameType,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        completion_tx: Option<RendererPageServiceWorkerTaskSender>,
    ) -> bool {
        let current_document_url = service_worker_current_url_for_creation_url(&document.url);
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
                if !moli_url::same_origin(&client.creation_url, &current_document_url) {
                    client.exposed_id = allocate_service_worker_exposed_client_id();
                }
                client.creation_url = document.url;
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
                && let Some(version_id) = select_window_client_controller_locked(
                    &mut state,
                    client_id,
                    &current_document_url,
                    &storage_key,
                    document.controller_source,
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
            .map(|client| service_worker_current_url_for_creation_url(&client.creation_url).into())
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

    #[cfg(test)]
    pub(crate) fn query_clients(
        &self,
        query: &ServiceWorkerClientQuery,
    ) -> ServiceWorkerClientQueryResult {
        let state = self.inner.state.lock();
        service_worker_query_clients_locked(&state, query)
    }

    pub(super) fn query_clients_with_window_targets(
        &self,
        query: &ServiceWorkerClientQuery,
        run: &RendererServiceWorkerRunIdentity,
    ) -> Option<(ServiceWorkerClientQueryResult, Vec<WindowClientQueryTarget>)> {
        let state = self.inner.state.lock();
        if state.versions.get(&query.version_id)?.run != *run {
            return None;
        }
        let mut result = service_worker_query_clients_locked(&state, query);
        let mut targets = Vec::new();
        result.clients.retain(|snapshot| {
            if snapshot.client_type != ServiceWorkerClientType::Window {
                return true;
            }
            let Some(client) = state.live_clients.get(&snapshot.id) else {
                return false;
            };
            let Some(target) = client.window_completion_target() else {
                return false;
            };
            let Some(sender) = client.endpoint.page_task_sender() else {
                return false;
            };
            targets.push((target, sender));
            true
        });
        Some((result, targets))
    }
}

fn service_worker_query_clients_locked(
    state: &ServiceWorkerRuntimeState,
    query: &ServiceWorkerClientQuery,
) -> ServiceWorkerClientQueryResult {
    let clients = match &query.kind {
        ServiceWorkerClientQueryKind::Get { exposed_client_id } => {
            service_worker_get_client_locked(
                state,
                query.registration_id,
                query.version_id,
                exposed_client_id,
            )
            .into_iter()
            .collect()
        }
        ServiceWorkerClientQueryKind::MatchAll { options } => {
            service_worker_match_all_clients_locked(
                state,
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

fn select_window_client_controller_locked(
    state: &mut ServiceWorkerRuntimeState,
    client_id: ServiceWorkerClientId,
    document_url: &Url,
    storage_key: &str,
    controller_source: Option<ServiceWorkerClientId>,
) -> Option<ServiceWorkerVersionId> {
    let Some(source_id) = controller_source else {
        return select_controller_for_new_client_locked(
            state,
            client_id,
            document_url,
            storage_key,
        );
    };
    let source = state.live_clients.get(&source_id)?;
    if source.storage_key != storage_key
        || moli_storage_key::serialized_storage_key_has_opaque_origin(storage_key)
        || !source.secure_context
        || !source.execution_ready
        || source.discarded_or_frozen
    {
        return None;
    }
    let registration_id = controller_registration_for_client_locked(state, source)?.id;
    // The local document keeps its creation URL for Client.url. Its
    // inherited controller is matched using the parent's effective URL.
    let scope_match_url = source.document_url.clone();
    state.live_clients.get_mut(&client_id)?.document_url = scope_match_url;
    attach_controller_to_client_locked(state, registration_id, client_id)
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
    Some(service_worker_client_snapshot_with_controlled(
        client,
        registration.active_version_id == Some(version_id)
            && registration.controlled_client_ids.contains(&client.id),
    ))
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
                version_id,
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
    if version.registration_id != registration_id {
        return None;
    }
    // Clients queries are also available during evaluation and installation,
    // and while a live worker's registration is being unregistered.
    state.registrations.get(&registration_id)
}

fn service_worker_client_visible_to_query(
    registration: &ServiceWorkerRegistration,
    version_id: ServiceWorkerVersionId,
    client_id: ServiceWorkerClientId,
    client: &ServiceWorkerClient,
    include_uncontrolled: bool,
    query_type: ServiceWorkerClientQueryType,
) -> Option<ServiceWorkerClientSnapshot> {
    if !service_worker_client_type_matches_query(client.client_type, query_type) {
        return None;
    }
    // The storage key includes the origin, including inherited origins whose
    // creation URLs (such as about:blank) do not identify that origin.
    if registration.storage_key != client.storage_key {
        return None;
    }
    if !service_worker_client_is_exposed_to_clients_api(client) || !client.secure_context {
        return None;
    }
    let controlled = registration.active_version_id == Some(version_id)
        && registration.controlled_client_ids.contains(&client_id);
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
