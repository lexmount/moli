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

    pub(super) fn finish_clients_claim_requested(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        owner: ServiceWorkerRunOwner,
        request_id: u64,
    ) {
        let (host, progress) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&owner.version_id()) else {
                return;
            };
            if version.run != *owner.run_identity() {
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Starting { host }
                | ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Stopped => return,
            };
            let progress =
                self.claim_clients_locked(&mut state, registration_id, owner.version_id());
            (host, progress)
        };
        let claimed = progress.is_some();
        for progress in progress.into_iter().flatten() {
            self.run_lifecycle_progress(progress);
        }
        host.dispatch_clients_claim_result(request_id, claimed);
    }

    #[cfg(test)]
    pub(super) fn finish_worker_clients_claim_requested(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) {
        let progress =
            self.claim_clients_locked(&mut self.inner.state.lock(), registration_id, version_id);
        for progress in progress.into_iter().flatten() {
            self.run_lifecycle_progress(progress);
        }
    }

    fn claim_clients_locked(
        &self,
        state: &mut ServiceWorkerRuntimeState,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) -> Option<Vec<LifecycleProgress>> {
        let version = state.versions.get(&version_id)?;
        let registration = state.registrations.get(&registration_id)?;
        if version.registration_id != registration_id
            || registration.active_version_id != Some(version_id)
            || !matches!(
                version.lifecycle_state,
                ServiceWorkerVersionLifecycleState::Activating
                    | ServiceWorkerVersionLifecycleState::Activated
            )
        {
            return None;
        }
        let claim_result = claim_live_scope_clients_locked(state, registration_id);
        let mut progress = claim_result
            .controller_change_deliveries
            .into_iter()
            .map(LifecycleProgress::NotifyControllerChange)
            .collect::<Vec<_>>();
        for changed_registration_id in claim_result.changed_registration_ids {
            if state
                .registrations
                .get(&changed_registration_id)
                .is_some_and(|registration| registration.pending_unregistration)
            {
                progress.extend(
                    self.unregistration_progress_for_registration_if_ready_locked(
                        state,
                        changed_registration_id,
                    ),
                );
            } else {
                progress.extend(self.activation_progress_for_registration_if_ready_locked(
                    state,
                    changed_registration_id,
                ));
            }
        }
        Some(progress)
    }
}
