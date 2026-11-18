use super::*;

impl ServiceWorkerRuntimeService {
    pub(super) fn finish_worker_skip_waiting_requested(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) {
        let progress = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get_mut(&version_id) else {
                return;
            };
            if version.registration_id != registration_id {
                return;
            }
            version.skip_waiting_requested = true;
            let Some(registration) = state.registrations.get(&registration_id) else {
                return;
            };
            if registration.waiting_version_id != Some(version_id) {
                return;
            }
            self.activation_progress_for_registration_if_ready_locked(&mut state, registration_id)
        };
        for progress in progress {
            self.run_lifecycle_progress(progress);
        }
    }

    #[cfg(test)]
    pub(super) fn finish_worker_clients_claim_requested(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) {
        self.claim_clients_for_version(registration_id, version_id, None);
    }

    pub(super) fn finish_worker_clients_claim_request(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        owner: ServiceWorkerRunOwner,
        request_id: u64,
    ) {
        let (version_id, run) = owner.into_parts();
        self.claim_clients_for_version(registration_id, version_id, Some(&run));
        // Current clients can observe their controller once the registry has
        // changed. During activation the service records the deferred claim
        // here so an activate event can await it without waiting on itself.
        self.finish_client_query(
            version_id,
            run,
            ServiceWorkerClientQueryResult {
                request_id,
                clients: Vec::new(),
            },
        );
    }

    fn claim_clients_for_version(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        run: Option<&RendererServiceWorkerRunIdentity>,
    ) {
        let progress = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&version_id) else {
                return;
            };
            if version.registration_id != registration_id
                || run.is_some_and(|run| &version.run != run)
            {
                return;
            }
            let Some(registration) = state.registrations.get(&registration_id) else {
                return;
            };
            if registration.active_version_id == Some(version_id) {
                let claim_result = claim_live_scope_clients_locked(&mut state, registration_id);
                let mut progress = claim_result
                    .controller_change_deliveries
                    .into_iter()
                    .map(LifecycleProgress::NotifyControllerChange)
                    .collect::<Vec<_>>();
                progress.extend(claim_result.changed_registration_ids.into_iter().flat_map(
                    |changed_registration_id| {
                        if state
                            .registrations
                            .get(&changed_registration_id)
                            .is_some_and(|registration| registration.pending_unregistration)
                        {
                            self.unregistration_progress_for_registration_if_ready_locked(
                                &mut state,
                                changed_registration_id,
                            )
                        } else {
                            self.activation_progress_for_registration_if_ready_locked(
                                &mut state,
                                changed_registration_id,
                            )
                        }
                    },
                ));
                progress
            } else {
                let should_defer_claim = registration.waiting_version_id == Some(version_id)
                    && version.lifecycle_state == ServiceWorkerVersionLifecycleState::Activating;
                if should_defer_claim && let Some(version) = state.versions.get_mut(&version_id) {
                    version.clients_claim_requested = true;
                }
                Vec::new()
            }
        };
        for progress in progress {
            self.run_lifecycle_progress(progress);
        }
    }
}
