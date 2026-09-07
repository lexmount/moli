use super::*;

impl CdpConnection {
    pub(crate) fn active_browser_context_id(&self) -> Option<moli_core::browser::BrowserContextId> {
        self.browser_context
            .as_ref()
            .map(BrowserContext::browser_context_id)
    }

    pub(crate) fn activate_browser_context_by_browser_id(
        &mut self,
        browser_context_id: moli_core::browser::BrowserContextId,
    ) -> bool {
        self.activate_matching_browser_context(|bc| bc.browser_context_id() == browser_context_id)
    }

    pub fn activate_browser_context_by_id(&mut self, browser_context_id: &str) -> bool {
        self.activate_matching_browser_context(|bc| bc.id == browser_context_id)
    }

    pub async fn activate_browser_context_by_id_async(&mut self, browser_context_id: &str) -> bool {
        self.activate_browser_context_by_id(browser_context_id)
    }

    pub fn activate_browser_context_for_session(&mut self, session_id: &str) -> bool {
        let Some(route) = self.session_route(Some(session_id)) else {
            return false;
        };
        match route.browser_context_id() {
            Some(browser_context_id) => self.activate_browser_context_by_id(browser_context_id),
            None => true,
        }
    }

    pub async fn activate_browser_context_for_session_async(&mut self, session_id: &str) -> bool {
        self.activate_browser_context_for_session(session_id)
    }

    pub fn activate_browser_context_for_target(&mut self, target_id: &str) -> bool {
        self.activate_matching_browser_context(|bc| {
            bc.is_active_target(target_id)
                || bc
                    .background_targets()
                    .any(|target| target.is_target(target_id))
                || bc.has_shared_worker_target(target_id)
                || bc.has_dedicated_worker_target(target_id)
                || bc.has_service_worker_target(target_id)
        })
    }

    pub async fn activate_browser_context_for_target_async(&mut self, target_id: &str) -> bool {
        self.activate_browser_context_for_target(target_id)
    }

    pub fn insert_browser_context(&mut self, mut browser_context: BrowserContext) {
        browser_context.apply_browser_cache_disabled(self.browser_global_overrides.cache_disabled);
        browser_context
            .set_service_worker_pause_on_start(self.service_worker_pause_on_start_for_devtools());
        browser_context.set_dedicated_worker_pause_on_start(
            self.dedicated_worker_pause_on_start_for_devtools(),
        );
        browser_context.bind_page_navigation_engines(
            self.standalone_navigation_engine.runtime_config(),
            self.scheduler_hooks.renderer_publication_sender(),
        );
        if self.browser_context.is_none() {
            self.browser_context = Some(browser_context);
            self.apply_active_engine_fetch_overrides();
        } else {
            self.inactive_browser_contexts.push(browser_context);
        }
    }

    pub async fn remove_browser_context_by_id_restoring_active_async(
        &mut self,
        browser_context_id: &str,
        restore_browser_context_id: Option<&str>,
    ) -> Option<BrowserContext> {
        let browser_context_id = self
            .browser_context_by_id(browser_context_id)
            .map(BrowserContext::browser_context_id)?;
        let restore_browser_context_id = restore_browser_context_id.and_then(|id| {
            self.browser_context_by_id(id)
                .map(BrowserContext::browser_context_id)
        });
        self.remove_browser_context_restoring_active(browser_context_id, restore_browser_context_id)
    }

    pub(crate) fn remove_browser_context_restoring_active(
        &mut self,
        browser_context_id: moli_core::browser::BrowserContextId,
        restore_browser_context_id: Option<moli_core::browser::BrowserContextId>,
    ) -> Option<BrowserContext> {
        if self
            .browser_context
            .as_ref()
            .is_some_and(|bc| bc.browser_context_id() == browser_context_id)
        {
            let removed = self.browser_context.take();
            if self.browser_context.is_none() && !self.inactive_browser_contexts.is_empty() {
                self.select_inactive_browser_context_as_active(0);
            }
            self.invalidate_resource_runtime();
            self.restore_preferred_browser_context(restore_browser_context_id, browser_context_id);
            self.apply_active_engine_fetch_overrides();
            return removed;
        }

        if let Some(index) = self
            .inactive_browser_contexts
            .iter()
            .position(|bc| bc.browser_context_id() == browser_context_id)
        {
            let removed = self.inactive_browser_contexts.swap_remove(index);
            self.restore_preferred_browser_context(restore_browser_context_id, browser_context_id);
            Some(removed)
        } else {
            None
        }
    }

    pub(crate) fn refresh_active_browser_context_loader(&mut self) {
        self.apply_active_engine_fetch_overrides();
        self.invalidate_resource_runtime();
    }

    fn select_inactive_browser_context_as_active(&mut self, index: usize) {
        self.browser_context = Some(self.inactive_browser_contexts.swap_remove(index));
    }

    fn activate_matching_browser_context<F>(&mut self, mut matches: F) -> bool
    where
        F: FnMut(&BrowserContext) -> bool,
    {
        if self
            .browser_context
            .as_ref()
            .map(&mut matches)
            .unwrap_or(false)
        {
            return true;
        }

        let Some(index) = self.inactive_browser_contexts.iter().position(matches) else {
            return false;
        };
        let matched = self.inactive_browser_contexts.swap_remove(index);
        if let Some(active) = self.browser_context.replace(matched) {
            self.inactive_browser_contexts.push(active);
        }
        self.apply_active_engine_fetch_overrides();
        self.invalidate_resource_runtime();
        true
    }

    fn restore_preferred_browser_context(
        &mut self,
        restore_browser_context_id: Option<moli_core::browser::BrowserContextId>,
        removed_browser_context_id: moli_core::browser::BrowserContextId,
    ) {
        let Some(restore_browser_context_id) = restore_browser_context_id else {
            return;
        };
        if restore_browser_context_id == removed_browser_context_id {
            return;
        }
        if self
            .browser_context
            .as_ref()
            .is_some_and(|bc| bc.browser_context_id() == restore_browser_context_id)
        {
            return;
        }
        let _ = self.activate_browser_context_by_browser_id(restore_browser_context_id);
    }
}
