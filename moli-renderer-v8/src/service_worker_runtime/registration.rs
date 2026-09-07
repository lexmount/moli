use std::collections::{HashMap, HashSet};

use url::Url;

use super::{
    ids::{ServiceWorkerClientId, ServiceWorkerRegistrationId, ServiceWorkerVersionId},
    jobs::{ServiceWorkerPendingRegisterJob, ServiceWorkerRegistrationKey},
};

pub(crate) const DEFAULT_NAVIGATION_PRELOAD_HEADER_VALUE: &str = "true";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, crate::webidl::WebIdlEnum)]
#[webidl(
    name = "ServiceWorkerUpdateViaCache",
    parse_with = Self::parse_webidl_token
)]
pub(crate) enum ServiceWorkerUpdateViaCache {
    #[default]
    Imports,
    All,
    None,
}

impl ServiceWorkerUpdateViaCache {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Imports => "imports",
            Self::All => "all",
            Self::None => "none",
        }
    }

    pub(crate) fn parse_webidl_token(value: &str) -> Option<Self> {
        match value {
            "imports" => Some(Self::Imports),
            "all" => Some(Self::All),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

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
    pub(super) storage_key: String,
    pub(super) scope_url: Url,
    pub(super) script_url: Url,
    pub(super) installing_version_id: Option<ServiceWorkerVersionId>,
    pub(super) waiting_version_id: Option<ServiceWorkerVersionId>,
    pub(super) active_version_id: Option<ServiceWorkerVersionId>,
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
