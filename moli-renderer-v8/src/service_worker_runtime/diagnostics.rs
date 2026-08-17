use crate::worker::WorkerScriptKind;

use super::ids::{ServiceWorkerRegistrationId, ServiceWorkerVersionId};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ServiceWorkerRuntimeDiagnostics {
    pub(crate) registration_count: usize,
    pub(crate) version_count: usize,
    pub(crate) registrations: Vec<ServiceWorkerRegistrationDiagnostics>,
    pub(crate) versions: Vec<ServiceWorkerVersionDiagnostics>,
    pub(crate) installing_version_count: usize,
    pub(crate) activated_version_count: usize,
    pub(crate) redundant_version_count: usize,
    pub(crate) stopped_version_count: usize,
    pub(crate) starting_version_count: usize,
    pub(crate) running_version_count: usize,
    pub(crate) stopping_version_count: usize,
    pub(crate) running_host_count: usize,
    pub(crate) pending_unregistration_count: usize,
    pub(crate) in_flight_event_count: usize,
    pub(crate) failed_start_count: usize,
    pub(crate) live_client_count: usize,
    pub(crate) controlled_client_count: usize,
    pub(crate) queued_register_job_count: usize,
    pub(crate) queued_unregistration_job_count: usize,
    pub(crate) pending_main_script_update_check_count: usize,
    pub(crate) pending_service_lane_event_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerRegistrationDiagnostics {
    pub(crate) id: ServiceWorkerRegistrationId,
    pub(crate) scope_url: String,
    pub(crate) script_url: String,
    pub(crate) installing_version_id: Option<ServiceWorkerVersionId>,
    pub(crate) waiting_version_id: Option<ServiceWorkerVersionId>,
    pub(crate) active_version_id: Option<ServiceWorkerVersionId>,
    pub(crate) pending_unregistration: bool,
    pub(crate) pending_clear_phase: Option<&'static str>,
    pub(crate) pending_main_script_update_check: bool,
    pub(crate) queued_register_job_count: usize,
    pub(crate) queued_unregistration_job_count: usize,
    pub(crate) controlled_client_count: usize,
    pub(crate) last_update_check_time_ms: Option<u64>,
    pub(crate) last_main_script_update_check: Option<ServiceWorkerMainScriptUpdateCheckDiagnostics>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerMainScriptUpdateCheckDiagnostics {
    pub(crate) script_url: String,
    pub(crate) newest_version_id: ServiceWorkerVersionId,
    pub(crate) result: &'static str,
    pub(crate) failure_status: Option<&'static str>,
    pub(crate) message: Option<String>,
    pub(crate) imported_script_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerVersionDiagnostics {
    pub(crate) id: ServiceWorkerVersionId,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) script_url: String,
    pub(crate) final_script_url: Option<String>,
    pub(crate) main_script_status: Option<u16>,
    pub(crate) main_script_body_len: Option<usize>,
    pub(crate) main_script_body_sha256: Option<String>,
    pub(crate) main_script_mime_type: Option<String>,
    pub(crate) imported_script_count: usize,
    pub(crate) imported_scripts: Vec<ServiceWorkerScriptResourceDiagnostics>,
    pub(crate) script_kind: WorkerScriptKind,
    pub(crate) lifecycle_state: &'static str,
    pub(crate) running_state: &'static str,
    pub(crate) in_flight_event_count: usize,
    pub(crate) host_is_running: bool,
    pub(crate) last_start_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerScriptResourceDiagnostics {
    pub(crate) request_url: String,
    pub(crate) final_url: String,
    pub(crate) kind: &'static str,
    pub(crate) status: u16,
    pub(crate) body_len: usize,
    pub(crate) body_sha256: String,
    pub(crate) mime_type: Option<String>,
}
