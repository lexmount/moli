use crate::conn::{
    BrowserContext, CdpConnection, ServiceWorkerAutoAttachRelatedOwner,
    ServiceWorkerAutoAttachRelatedOwnerSession,
};

impl CdpConnection {
    pub(crate) fn set_service_worker_auto_attach_related_owner(
        &mut self,
        owner_session_id: Option<&str>,
        browser_context_id: &str,
        registration_id: u64,
        base_version_id: u64,
        script_url: String,
        scope_url: String,
        allow_service_worker_targets: bool,
        wait_for_debugger_on_start: bool,
    ) {
        let owner_key = owner_session_id.map(str::to_owned);
        self.service_worker_auto_attach_related_owners
            .retain(|owner| {
                !(owner.owner_session_id == owner_key
                    && owner.browser_context_id == browser_context_id
                    && owner.registration_id == registration_id
                    && owner.script_url == script_url
                    && owner.scope_url == scope_url)
            });
        self.service_worker_auto_attach_related_owners
            .push(ServiceWorkerAutoAttachRelatedOwner {
                owner_session_id: owner_key,
                browser_context_id: browser_context_id.to_owned(),
                registration_id,
                base_version_id,
                script_url,
                scope_url,
                allow_service_worker_targets,
                wait_for_debugger_on_start,
            });
        self.sync_service_worker_related_pause_on_start_for_devtools();
    }

    pub(crate) fn replace_service_worker_auto_attach_related_owner(
        &mut self,
        owner_session_id: Option<&str>,
        browser_context_id: &str,
        registration_id: u64,
        base_version_id: u64,
        script_url: String,
        scope_url: String,
        allow_service_worker_targets: bool,
        wait_for_debugger_on_start: bool,
    ) {
        self.clear_auto_attach_owner(owner_session_id);
        self.set_service_worker_auto_attach_related_owner(
            owner_session_id,
            browser_context_id,
            registration_id,
            base_version_id,
            script_url,
            scope_url,
            allow_service_worker_targets,
            wait_for_debugger_on_start,
        );
    }

    pub(crate) fn clear_service_worker_auto_attach_related_owner(
        &mut self,
        owner_session_id: Option<&str>,
    ) {
        let owner_key = owner_session_id.map(str::to_owned);
        self.service_worker_auto_attach_related_owners
            .retain(|owner| owner.owner_session_id != owner_key);
        self.sync_service_worker_related_pause_on_start_for_devtools();
    }

    pub(crate) fn service_worker_auto_attach_related_owner_sessions_for_target(
        &self,
        browser_context_id: &str,
        registration_id: u64,
        version_id: u64,
        script_url: &str,
        scope_url: &str,
    ) -> Vec<ServiceWorkerAutoAttachRelatedOwnerSession> {
        let mut owners = Vec::new();
        for owner in &self.service_worker_auto_attach_related_owners {
            if !owner.allow_service_worker_targets
                || owner.browser_context_id != browser_context_id
                || owner.registration_id != registration_id
                || version_id <= owner.base_version_id
                || owner.script_url != script_url
                || owner.scope_url != scope_url
            {
                continue;
            }
            if !owners
                .iter()
                .any(|existing: &ServiceWorkerAutoAttachRelatedOwnerSession| {
                    existing.owner_session_id == owner.owner_session_id
                })
            {
                owners.push(ServiceWorkerAutoAttachRelatedOwnerSession {
                    owner_session_id: owner.owner_session_id.clone(),
                    wait_for_debugger_on_start: owner.wait_for_debugger_on_start,
                });
            }
        }
        owners
    }

    fn sync_service_worker_related_pause_on_start_for_devtools(&self) {
        for browser_context in self.browser_contexts() {
            let policies = self
                .service_worker_auto_attach_related_owners
                .iter()
                .filter(|owner| {
                    owner.browser_context_id == browser_context.id
                        && owner.allow_service_worker_targets
                        && owner.wait_for_debugger_on_start
                })
                .map(|owner| {
                    (
                        owner.registration_id,
                        owner.base_version_id,
                        owner.script_url.clone(),
                        owner.scope_url.clone(),
                    )
                })
                .collect::<Vec<_>>();
            browser_context
                .renderer_runtime()
                .set_service_worker_related_pause_on_start_policies_for_devtools(policies);
        }
    }

    pub(crate) fn set_service_worker_pause_on_start_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> bool {
        let key = session_id.map(str::to_owned);
        if enabled {
            self.service_worker_pause_on_start_owner_sessions
                .insert(key);
        } else {
            self.service_worker_pause_on_start_owner_sessions
                .remove(&key);
        }
        let pause = self.service_worker_pause_on_start_for_devtools();
        let runtimes = self
            .browser_contexts()
            .map(BrowserContext::renderer_runtime)
            .collect::<Vec<_>>();
        for runtime in runtimes {
            runtime.set_service_worker_pause_on_start_for_devtools(pause);
        }
        pause
    }

    pub(crate) fn service_worker_pause_on_start_for_devtools(&self) -> bool {
        !self.service_worker_pause_on_start_owner_sessions.is_empty()
    }

    pub(crate) fn set_dedicated_worker_pause_on_start_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> bool {
        let key = session_id.map(str::to_owned);
        if enabled {
            self.dedicated_worker_pause_on_start_owner_sessions
                .insert(key);
        } else {
            self.dedicated_worker_pause_on_start_owner_sessions
                .remove(&key);
        }
        self.dedicated_worker_pause_on_start_for_devtools()
    }

    pub(crate) fn dedicated_worker_pause_on_start_for_devtools(&self) -> bool {
        !self
            .dedicated_worker_pause_on_start_owner_sessions
            .is_empty()
    }

    #[cfg(test)]
    pub(crate) fn service_worker_pause_on_start_owner_count(&self) -> usize {
        self.service_worker_pause_on_start_owner_sessions.len()
    }
}
