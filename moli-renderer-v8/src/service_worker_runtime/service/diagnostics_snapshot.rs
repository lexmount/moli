use super::*;

impl ServiceWorkerRuntimeService {
    pub(crate) fn diagnostics_snapshot(&self) -> ServiceWorkerRuntimeDiagnostics {
        let state = self.inner.state.lock();
        let mut diagnostics = ServiceWorkerRuntimeDiagnostics {
            registration_count: state.registrations.len(),
            version_count: state.versions.len(),
            live_client_count: state.live_clients.len(),
            pending_service_lane_event_count: self.pending_service_lane_event_count(),
            ..ServiceWorkerRuntimeDiagnostics::default()
        };
        for registration in state.registrations.values() {
            let registration_key = registration.key();
            let queued_register_job_count = state
                .job_coordinator
                .queued_register_job_count(&registration_key);
            let queued_unregistration_job_count = state
                .job_coordinator
                .queued_unregistration_job_count(&registration_key);
            let pending_main_script_update_check = state
                .pending_main_script_update_checks
                .contains_key(&registration.id);
            let pending_clear_phase =
                pending_clear_phase_for_registration_locked(&state, registration).as_str();
            diagnostics.controlled_client_count += registration.controlled_client_ids.len();
            diagnostics.queued_register_job_count += queued_register_job_count;
            diagnostics.queued_unregistration_job_count += queued_unregistration_job_count;
            if pending_main_script_update_check {
                diagnostics.pending_main_script_update_check_count += 1;
            }
            diagnostics
                .registrations
                .push(ServiceWorkerRegistrationDiagnostics {
                    id: registration.id,
                    scope_url: registration.scope_url.to_string(),
                    script_url: registration.script_url.to_string(),
                    installing_version_id: registration.installing_version_id,
                    waiting_version_id: registration.waiting_version_id,
                    active_version_id: registration.active_version_id,
                    pending_unregistration: registration.pending_unregistration,
                    pending_clear_phase,
                    pending_main_script_update_check,
                    queued_register_job_count,
                    queued_unregistration_job_count,
                    controlled_client_count: registration.controlled_client_ids.len(),
                    last_update_check_time_ms: registration.last_update_check_time_ms,
                    last_main_script_update_check: state
                        .main_script_update_check_diagnostics
                        .get(&registration.id)
                        .cloned(),
                });
            if registration.pending_unregistration {
                diagnostics.pending_unregistration_count += 1;
            }
        }
        for version in state.versions.values() {
            match version.lifecycle_state {
                ServiceWorkerVersionLifecycleState::Installing => {
                    diagnostics.installing_version_count += 1;
                }
                ServiceWorkerVersionLifecycleState::Installed
                | ServiceWorkerVersionLifecycleState::Activating => {}
                ServiceWorkerVersionLifecycleState::Activated => {
                    diagnostics.activated_version_count += 1;
                }
                ServiceWorkerVersionLifecycleState::Redundant => {
                    diagnostics.redundant_version_count += 1;
                }
            }
            let (running_state, host_is_running) = version.running_state.diagnostics();
            match &version.running_state {
                ServiceWorkerVersionRunningState::Stopped => {
                    diagnostics.stopped_version_count += 1;
                }
                ServiceWorkerVersionRunningState::Starting { host } => {
                    diagnostics.starting_version_count += 1;
                    if host.has_running_worker() {
                        diagnostics.running_host_count += 1;
                    }
                }
                ServiceWorkerVersionRunningState::Running { host } => {
                    diagnostics.running_version_count += 1;
                    if host.has_running_worker() {
                        diagnostics.running_host_count += 1;
                    }
                }
            }
            if version.last_start_error.is_some() {
                diagnostics.failed_start_count += 1;
            }
            let imported_scripts = version
                .imported_script_resources
                .values()
                .map(|resource| ServiceWorkerScriptResourceDiagnostics {
                    request_url: resource.request_url.to_string(),
                    final_url: resource.final_url.to_string(),
                    kind: resource.kind.as_str(),
                    status: resource.status,
                    body_len: resource.body_len,
                    body_sha256: resource.body_sha256.clone(),
                    mime_type: resource.mime_type.clone(),
                })
                .collect::<Vec<_>>();
            diagnostics.versions.push(ServiceWorkerVersionDiagnostics {
                id: version.id,
                registration_id: version.registration_id,
                script_url: version.script_url.to_string(),
                final_script_url: version.final_script_url.as_ref().map(ToString::to_string),
                main_script_status: version
                    .main_script_resource
                    .as_ref()
                    .map(|resource| resource.status),
                main_script_body_len: version
                    .main_script_resource
                    .as_ref()
                    .map(|resource| resource.body_len),
                main_script_body_sha256: version
                    .main_script_resource
                    .as_ref()
                    .map(|resource| resource.body_sha256.clone()),
                main_script_mime_type: version
                    .main_script_resource
                    .as_ref()
                    .and_then(|resource| resource.mime_type.clone()),
                imported_script_count: imported_scripts.len(),
                imported_scripts,
                script_kind: version.script_kind,
                lifecycle_state: version.lifecycle_state.as_str(),
                running_state,
                in_flight_event_count: version.in_flight_event_count,
                host_is_running,
                last_start_error: version.last_start_error.clone(),
            });
            diagnostics.in_flight_event_count += version.in_flight_event_count;
        }
        diagnostics
    }
}
