use url::Url;

use super::{
    ids::{ServiceWorkerRegistrationId, ServiceWorkerVersionId},
    matching::service_worker_scope_matches_url,
    registration::{ServiceWorkerNavigationPreloadState, ServiceWorkerUpdateViaCache},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceWorkerControlState {
    registration_id: ServiceWorkerRegistrationId,
    active_version_id: Option<ServiceWorkerVersionId>,
    script_url: Url,
    scope_url: Url,
}

impl ServiceWorkerControlState {
    pub(super) fn new(
        registration_id: ServiceWorkerRegistrationId,
        active_version_id: Option<ServiceWorkerVersionId>,
        script_url: Url,
        scope_url: Url,
    ) -> Self {
        Self {
            registration_id,
            active_version_id,
            script_url,
            scope_url,
        }
    }

    pub(crate) fn script_url(&self) -> &Url {
        &self.script_url
    }

    pub(crate) fn scope_url(&self) -> &Url {
        &self.scope_url
    }

    pub(crate) fn has_active_version(&self) -> bool {
        self.active_version_id.is_some()
    }

    pub(crate) fn active_version_id(&self) -> Option<ServiceWorkerVersionId> {
        self.active_version_id
    }

    pub(crate) fn controls_document(&self, document_url: &Url) -> bool {
        self.has_active_version() && service_worker_scope_matches_url(&self.scope_url, document_url)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerVersionSnapshot {
    version_id: ServiceWorkerVersionId,
    script_url: Url,
    state: &'static str,
}

impl ServiceWorkerVersionSnapshot {
    pub(crate) fn new(
        version_id: ServiceWorkerVersionId,
        script_url: Url,
        state: &'static str,
    ) -> Self {
        Self {
            version_id,
            script_url,
            state,
        }
    }

    pub(crate) fn version_id(&self) -> ServiceWorkerVersionId {
        self.version_id
    }

    pub(crate) fn script_url(&self) -> &Url {
        &self.script_url
    }

    pub(crate) fn state(&self) -> &'static str {
        self.state
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerRegistrationSnapshot {
    registration_id: ServiceWorkerRegistrationId,
    scope_url: Url,
    update_via_cache: ServiceWorkerUpdateViaCache,
    navigation_preload_state: ServiceWorkerNavigationPreloadState,
    installing: Option<ServiceWorkerVersionSnapshot>,
    waiting: Option<ServiceWorkerVersionSnapshot>,
    active: Option<ServiceWorkerVersionSnapshot>,
}

impl ServiceWorkerRegistrationSnapshot {
    pub(super) fn new(
        registration_id: ServiceWorkerRegistrationId,
        scope_url: Url,
        update_via_cache: ServiceWorkerUpdateViaCache,
        navigation_preload_state: ServiceWorkerNavigationPreloadState,
        installing: Option<ServiceWorkerVersionSnapshot>,
        waiting: Option<ServiceWorkerVersionSnapshot>,
        active: Option<ServiceWorkerVersionSnapshot>,
    ) -> Self {
        Self {
            registration_id,
            scope_url,
            update_via_cache,
            navigation_preload_state,
            installing,
            waiting,
            active,
        }
    }

    #[cfg(test)]
    pub(crate) fn active_for_binding_test(scope_url: Url, script_url: Url) -> Self {
        Self::new(
            ServiceWorkerRegistrationId(1),
            scope_url,
            ServiceWorkerUpdateViaCache::Imports,
            ServiceWorkerNavigationPreloadState::default(),
            None,
            None,
            Some(ServiceWorkerVersionSnapshot::new(
                ServiceWorkerVersionId(1),
                script_url,
                "activated",
            )),
        )
    }

    pub(crate) fn registration_id(&self) -> ServiceWorkerRegistrationId {
        self.registration_id
    }

    pub(crate) fn scope_url(&self) -> &Url {
        &self.scope_url
    }

    pub(crate) fn update_via_cache(&self) -> ServiceWorkerUpdateViaCache {
        self.update_via_cache
    }

    #[cfg(test)]
    pub(crate) fn navigation_preload_state(&self) -> &ServiceWorkerNavigationPreloadState {
        &self.navigation_preload_state
    }

    pub(crate) fn installing(&self) -> Option<&ServiceWorkerVersionSnapshot> {
        self.installing.as_ref()
    }

    pub(crate) fn waiting(&self) -> Option<&ServiceWorkerVersionSnapshot> {
        self.waiting.as_ref()
    }

    pub(crate) fn active(&self) -> Option<&ServiceWorkerVersionSnapshot> {
        self.active.as_ref()
    }

    pub(crate) fn from_active_control_for_binding(state: ServiceWorkerControlState) -> Self {
        let active_version_id = state.active_version_id.unwrap_or(ServiceWorkerVersionId(0));
        Self::new(
            state.registration_id,
            state.scope_url,
            ServiceWorkerUpdateViaCache::Imports,
            ServiceWorkerNavigationPreloadState::default(),
            None,
            None,
            Some(ServiceWorkerVersionSnapshot::new(
                active_version_id,
                state.script_url,
                "activated",
            )),
        )
    }
}
