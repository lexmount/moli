use std::collections::HashSet;

use super::{
    ids::ServiceWorkerRegistrationId,
    registration::ServiceWorkerRegistration,
    state::{LifecycleProgress, ServiceWorkerRuntimeState},
};

pub(super) const SERVICE_WORKER_REGISTRATION_DELETED_FETCH_ERROR: &str =
    "service worker registration was deleted";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServiceWorkerPendingClearPhase {
    NotPending,
    WaitingForControllees,
    WaitingForEvents,
    ReadyToClear,
}

impl ServiceWorkerPendingClearPhase {
    pub(super) fn as_str(self) -> Option<&'static str> {
        match self {
            Self::NotPending => None,
            Self::WaitingForControllees => Some("waiting-for-controllees"),
            Self::WaitingForEvents => Some("waiting-for-events"),
            Self::ReadyToClear => Some("ready-to-clear"),
        }
    }
}

pub(super) fn registration_ready_to_delete_locked(
    state: &ServiceWorkerRuntimeState,
    registration: &ServiceWorkerRegistration,
) -> bool {
    pending_clear_phase_for_registration_locked(state, registration)
        == ServiceWorkerPendingClearPhase::ReadyToClear
}

pub(super) fn pending_clear_phase_for_registration_locked(
    state: &ServiceWorkerRuntimeState,
    registration: &ServiceWorkerRegistration,
) -> ServiceWorkerPendingClearPhase {
    if !registration.pending_unregistration {
        return ServiceWorkerPendingClearPhase::NotPending;
    }
    if !registration.controlled_client_ids.is_empty() {
        return ServiceWorkerPendingClearPhase::WaitingForControllees;
    }
    if state
        .versions
        .values()
        .filter(|version| version.registration_id == registration.id)
        .any(|version| {
            version.in_flight_event_count != 0
                || !version.pending_start_events.is_empty()
                || !version.pending_activation_fetch_events.is_empty()
        })
    {
        return ServiceWorkerPendingClearPhase::WaitingForEvents;
    }
    ServiceWorkerPendingClearPhase::ReadyToClear
}

pub(super) fn execute_pending_clear_locked(
    state: &mut ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
) -> Vec<LifecycleProgress> {
    if state.registrations.remove(&registration_id).is_none() {
        return Vec::new();
    }
    let mut progress = clear_registration_owner_state_locked(state, registration_id);
    progress.extend(clear_registration_versions_locked(state, registration_id));
    progress
}

fn clear_registration_owner_state_locked(
    state: &mut ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
) -> Vec<LifecycleProgress> {
    let registration_version_ids = state
        .versions
        .iter()
        .filter(|(_, version)| version.registration_id == registration_id)
        .map(|(version_id, _)| *version_id)
        .collect::<HashSet<_>>();
    let failed_fetch_event_ids = state
        .pending_fetch_jobs
        .iter()
        .filter(|(_, job)| registration_version_ids.contains(&job.version_id()))
        .map(|(event_id, _)| *event_id)
        .collect::<Vec<_>>();
    let progress = failed_fetch_event_ids
        .into_iter()
        .filter_map(|event_id| state.pending_fetch_jobs.remove(&event_id))
        .map(|job| {
            LifecycleProgress::FetchFailed(Box::new((
                job,
                SERVICE_WORKER_REGISTRATION_DELETED_FETCH_ERROR.to_owned(),
            )))
        })
        .collect::<Vec<_>>();
    state
        .pending_ready_jobs
        .retain(|job| job.registration_id != registration_id);
    state
        .notification_records
        .retain(|record| record.registration_id != registration_id);
    state.push_subscriptions.remove(&registration_id);
    state
        .sync_registrations
        .retain(|(record_registration_id, _), _| *record_registration_id != registration_id);
    state
        .periodic_sync_registrations
        .retain(|(record_registration_id, _), _| *record_registration_id != registration_id);
    progress
}

fn clear_registration_versions_locked(
    state: &mut ServiceWorkerRuntimeState,
    registration_id: ServiceWorkerRegistrationId,
) -> Vec<LifecycleProgress> {
    let version_ids = state
        .versions
        .iter()
        .filter(|(_, version)| version.registration_id == registration_id)
        .map(|(version_id, _)| *version_id)
        .collect::<Vec<_>>();
    version_ids
        .into_iter()
        .flat_map(|version_id| {
            let mut progress = vec![LifecycleProgress::ForceUpdatePageLoadCompleted(
                state.take_force_update_page_load_waiters_for_version(version_id),
            )];
            state.record_target_destroyed(version_id);
            if let Some(mut version) = state.versions.remove(&version_id)
                && let Some(host) = version.running_state.take_host_for_shutdown()
            {
                progress.push(LifecycleProgress::TerminateHost(host));
            }
            progress
        })
        .collect()
}
