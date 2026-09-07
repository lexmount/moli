use super::*;
use crate::conn::{BackgroundProtocolEvent, CdpTargetHostLifecycleDelta, TargetClosureCleanupPlan};

impl CdpConnection {
    pub fn subscribe_browser_events(
        &self,
    ) -> Result<
        (
            moli_core::browser::BrowserSnapshot,
            moli_core::browser::BrowserEventReceiver,
        ),
        String,
    > {
        self.browser.subscribe()
    }

    /// Retire DevTools state after a Browser-authored disposal. There is no
    /// physical cleanup here: the Context is already absent from its owner.
    pub async fn project_disposed_browser_context(
        &mut self,
        context: moli_core::browser::BrowserContextId,
    ) -> Vec<BackgroundProtocolEvent> {
        let removed = if self
            .browser_context
            .as_ref()
            .is_some_and(|projection| projection.browser_context_id() == context)
        {
            self.browser_context.take()
        } else {
            self.inactive_browser_contexts
                .iter()
                .position(|projection| projection.browser_context_id() == context)
                .map(|index| self.inactive_browser_contexts.swap_remove(index))
        };
        let Some(mut removed) = removed else {
            return Vec::new();
        };
        // Selecting a remaining DevTools projection must not reconfigure a
        // Browser Context: this is an observation, not another transaction.
        if self.browser_context.is_none() {
            self.browser_context = self.inactive_browser_contexts.pop();
        }
        let infos = removed.retired_devtools_target_infos();
        let mut events = Vec::new();
        Self::retire_browser_context_inspector_calls(&mut removed, &mut events);
        for target in removed.page_targets.iter() {
            self.record_collected_network_data_artifacts(
                target.runtime_slot.collected_network_data_artifacts(),
            );
        }
        for info in infos {
            let Some(target_id) = info.target_id.as_ref().map(|id| id.as_str().to_owned()) else {
                continue;
            };
            let destroyed = self
                .agent_hosts
                .project_page_tab_target_infos_for_destruction(info.clone());
            events.extend(self.target_destroyed_automation_events(info));
            let sessions = self.attached_sessions_for_target(&target_id);
            events.extend(
                self.dispose_target_closure_sessions_event_plan_async(
                    TargetClosureCleanupPlan::new(
                        target_id.clone(),
                        Some("Render process gone."),
                        sessions,
                    ),
                    None,
                )
                .await
                .into_background_events(),
            );
            let tab = self.take_closed_top_level_target_sessions_cleanup_plan(
                &target_id,
                Some("Render process gone."),
            );
            // Removing the page/tab pair already publishes both directory
            // removals. Workers have no paired tab and retire separately.
            if let Some(tab) = tab {
                events.extend(
                    self.dispose_target_closure_sessions_event_plan_async(tab, None)
                        .await
                        .into_background_events(),
                );
            } else {
                self.notify_target_host_lifecycle(CdpTargetHostLifecycleDelta::Destroyed {
                    target_id: target_id.clone(),
                });
            }
            for info in destroyed {
                events.extend(self.exact_target_destroyed_events_for_all_discovery_owners(info));
            }
            if target_id == self.default_target_id() {
                self.mark_default_browser_target_closed();
            }
        }
        removed.retire_page_projections();
        events
    }

    /// Explicitly end the private runtime of a WebDriver session. Disconnecting
    /// a frontend, or merely dropping its DevTools projection, must not do this.
    pub fn end_webdriver_session(self) -> Result<(), String> {
        let contexts = self
            .browser_contexts()
            .map(BrowserContext::browser_context_id)
            .collect::<Vec<_>>();
        let browser = self.browser.clone();
        drop(self);
        for context in contexts {
            browser.remove_context(context)?;
        }
        Ok(())
    }

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
            self.navigation_runtime_config.clone(),
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
