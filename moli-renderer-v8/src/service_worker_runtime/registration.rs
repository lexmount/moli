use std::collections::{HashMap, HashSet};

pub(crate) use moli_webapi::worker_options::service_worker::ServiceWorkerUpdateViaCache;
use url::Url;

use super::{
    ids::{ServiceWorkerClientId, ServiceWorkerRegistrationId, ServiceWorkerVersionId},
    jobs::{ServiceWorkerPendingRegisterJob, ServiceWorkerRegistrationKey},
};

pub(crate) const DEFAULT_NAVIGATION_PRELOAD_HEADER_VALUE: &str = "true";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerNavigationPreloadState {
    pub(crate) enabled: bool,
    pub(crate) header_value: String,
}

impl Default for ServiceWorkerNavigationPreloadState {
    fn default() -> Self {
        Self {
            enabled: false,
            header_value: DEFAULT_NAVIGATION_PRELOAD_HEADER_VALUE.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerNavigationPreloadStateError {
    InvalidState,
    StorageFailure,
}

#[derive(Debug)]
pub(super) struct ServiceWorkerRegistration {
    pub(super) id: ServiceWorkerRegistrationId,
    /// Position assigned when this registration enters the registration map.
    pub(super) registration_order: u64,
    pub(super) storage_key: String,
    pub(super) scope_url: Url,
    pub(super) script_url: Url,
    pub(super) installing_version_id: Option<ServiceWorkerVersionId>,
    pub(super) waiting_version_id: Option<ServiceWorkerVersionId>,
    pub(super) active_version_id: Option<ServiceWorkerVersionId>,
    /// Removed from the registration map; existing clients/events may still use it.
    pub(super) pending_unregistration: bool,
    pub(super) update_via_cache: ServiceWorkerUpdateViaCache,
    pub(super) navigation_preload_state: ServiceWorkerNavigationPreloadState,
    pub(super) last_update_check_time_ms: Option<u64>,
    pub(super) pending_register_jobs:
        HashMap<ServiceWorkerVersionId, ServiceWorkerPendingRegisterJob>,
    pub(super) controlled_client_ids: HashSet<ServiceWorkerClientId>,
}

impl ServiceWorkerRegistration {
    pub(super) fn key(&self) -> ServiceWorkerRegistrationKey {
        ServiceWorkerRegistrationKey {
            scope_url: self.scope_url.clone(),
            storage_key: self.storage_key.clone(),
        }
    }

    pub(super) fn references_version(&self, version_id: ServiceWorkerVersionId) -> bool {
        self.installing_version_id == Some(version_id)
            || self.waiting_version_id == Some(version_id)
            || self.active_version_id == Some(version_id)
    }
}
