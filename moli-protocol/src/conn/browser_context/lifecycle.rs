use super::*;

impl CdpConnection {
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
                    .background_targets
                    .iter()
                    .any(|target| target.is_target(target_id))
                || bc.has_shared_worker_target(target_id)
                || bc.has_dedicated_worker_target(target_id)
                || bc.has_service_worker_target(target_id)
        })
    }

    pub async fn activate_browser_context_for_target_async(&mut self, target_id: &str) -> bool {
        self.activate_browser_context_for_target(target_id)
    }

    pub fn insert_browser_context(&mut self, browser_context: BrowserContext) {
        browser_context
            .renderer_runtime()
            .set_service_worker_pause_on_start_for_devtools(
                self.service_worker_pause_on_start_for_devtools(),
            );
        browser_context
            .renderer_runtime()
            .set_dedicated_worker_pause_on_start_for_devtools(
                self.dedicated_worker_pause_on_start_for_devtools(),
            );
        if self.browser_context.is_none() {
            let renderer_runtime = browser_context.renderer_runtime_owner_access();
            let next_engine = moli_core::runtime::NavigationEngine::
                new_with_runtime_config_and_browser_context_access(
                    self.engine.runtime_config(),
                    renderer_runtime,
                )
                .expect("newly inserted BrowserContext owner must be live");
            self.replace_navigation_engine(next_engine);
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
        if self
            .browser_context
            .as_ref()
            .is_some_and(|bc| bc.id == browser_context_id)
        {
            let removed = self.browser_context.take();
            if self.browser_context.is_none() && !self.inactive_browser_contexts.is_empty() {
                self.promote_inactive_browser_context_index_to_active(0);
            } else if self.browser_context.is_none() {
                // The active engine only carries weak access to the removed
                // context root. Move to a standalone root before returning
                // that BrowserContext to a caller that may drop it at once.
                let next_engine = moli_core::runtime::NavigationEngine::new_with_runtime_config(
                    self.engine.runtime_config(),
                );
                self.replace_navigation_engine(next_engine);
            }
            self.forget_retained_navigation_engines_for_browser_context(browser_context_id);
            self.invalidate_resource_runtime_async().await;
            self.restore_preferred_browser_context_async(
                restore_browser_context_id,
                browser_context_id,
            )
            .await;
            self.apply_active_engine_fetch_overrides();
            return removed;
        }

        if let Some(index) = self
            .inactive_browser_contexts
            .iter()
            .position(|bc| bc.id == browser_context_id)
        {
            let removed = self.inactive_browser_contexts.swap_remove(index);
            self.forget_retained_navigation_engines_for_browser_context(browser_context_id);
            self.restore_preferred_browser_context_async(
                restore_browser_context_id,
                browser_context_id,
            )
            .await;
            Some(removed)
        } else {
            None
        }
    }

    fn forget_retained_navigation_engines_for_browser_context(&mut self, browser_context_id: &str) {
        self.retained_background_navigation_engines
            .retain(|(retained_context_id, _), _| retained_context_id != browser_context_id);
    }

    pub(crate) async fn refresh_active_browser_context_loader_async(&mut self) {
        self.apply_active_engine_fetch_overrides();
        self.invalidate_resource_runtime_async().await;
    }

    fn navigation_engine_for_inactive_browser_context_index(
        &mut self,
        index: usize,
    ) -> moli_core::runtime::NavigationEngine {
        let next_browser_context_id = self.inactive_browser_contexts[index].id.clone();
        let next_renderer_runtime =
            self.inactive_browser_contexts[index].renderer_runtime_owner_access();
        let navigation_runtime_config = self.engine.runtime_config();
        let next_active_target_id = self.inactive_browser_contexts[index]
            .active_target_id()
            .map(str::to_owned);
        next_active_target_id
            .as_ref()
            .and_then(|target_id| {
                self.retained_background_navigation_engines
                    .remove(&(next_browser_context_id.clone(), target_id.clone()))
            })
            .unwrap_or_else(|| {
                moli_core::runtime::NavigationEngine::new_with_runtime_config_and_browser_context_access(
                    navigation_runtime_config,
                    next_renderer_runtime,
                )
                .expect("inactive BrowserContext owner must be live")
            })
    }

    fn promote_inactive_browser_context_index_to_active(&mut self, index: usize) {
        let next_engine = self.navigation_engine_for_inactive_browser_context_index(index);
        self.replace_navigation_engine(next_engine);
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
        let next_engine = self.navigation_engine_for_inactive_browser_context_index(index);
        let current_active_engine_key = self.browser_context.as_ref().and_then(|bc| {
            let active_target_id = bc.active_target_id()?;
            bc.has_loaded_page()
                .then(|| (bc.id.clone(), active_target_id.to_owned()))
        });
        if let Some((browser_context_id, target_id)) = current_active_engine_key {
            self.apply_scheduler_senders_to_navigation_engine(&next_engine);
            let active_engine = self
                .engine
                .replace(next_engine)
                .expect("active browser context must have a navigation engine");
            self.retain_background_navigation_engine(browser_context_id, target_id, active_engine)
                .expect("inactive BrowserContext must retain its exact active-target engine");
        } else {
            self.replace_navigation_engine(next_engine);
        }

        let matched = self.inactive_browser_contexts.swap_remove(index);
        if let Some(active) = self.browser_context.replace(matched) {
            self.inactive_browser_contexts.push(active);
        }
        self.apply_active_engine_fetch_overrides();
        self.invalidate_resource_runtime();
        true
    }

    async fn restore_preferred_browser_context_async(
        &mut self,
        restore_browser_context_id: Option<&str>,
        removed_browser_context_id: &str,
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
            .is_some_and(|bc| bc.id == restore_browser_context_id)
        {
            return;
        }
        let _ = self
            .activate_browser_context_by_id_async(restore_browser_context_id)
            .await;
    }
}
