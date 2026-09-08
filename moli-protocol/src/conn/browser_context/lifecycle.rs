use super::*;
use crate::conn::{BackgroundProtocolEvent, CdpTargetHostLifecycleDelta, TargetClosureCleanupPlan};

/// Which lifecycle notifications are already owned by the initiating executor.
#[derive(Clone, Copy)]
pub(crate) enum PageCloseNotifications {
    BrowserEvent,
    PageCommand,
    ContextDisposal,
}

impl CdpConnection {
    pub fn project_created_browser_context(&mut self, id: moli_core::browser::BrowserContextId) {
        if self.browser_context_by_browser_id(id).is_some() {
            return;
        }
        let Ok(handle) = self.browser.context_handle(id) else {
            return;
        };
        if let Some(sender) = self.scheduler_hooks.renderer_publication_sender()
            && handle.set_renderer_output_transport_sender(sender).is_err()
        {
            return;
        }
        let wire_id = loop {
            let id = self.gen_bc_id();
            if !self.has_browser_context_id(&id) {
                break id;
            }
        };
        // Adoption is observation, not Context creation or policy installation.
        // In particular it must not bind the lazy default target to this Context.
        self.inactive_browser_contexts
            .push(BrowserContext::from_browser_handle(wire_id, handle));
    }

    pub async fn project_created_web_contents(
        &mut self,
        handle: moli_core::browser::WebContentsHandle,
    ) -> Vec<BackgroundProtocolEvent> {
        self.project_created_web_contents_inner(handle, false).await
    }

    pub(crate) async fn wait_for_renderer_popup(
        &self,
        opening: std::sync::Arc<moli_core::page::RendererPopupOpening>,
    ) -> Option<moli_core::browser::BrowserPopupAdmission> {
        self.browser.wait_for_renderer_popup(opening).await
    }

    pub(crate) async fn project_observed_popup(
        &mut self,
        handle: moli_core::browser::WebContentsHandle,
    ) -> Vec<BackgroundProtocolEvent> {
        self.pending_popup_projections.remove(&handle);
        self.project_created_web_contents_inner(handle, true).await
    }

    /// A retired/unobserved renderer stream cannot strand a committed Window.
    /// These entries retain only native handles, never an output or its storage.
    pub(crate) async fn project_unobserved_popups(&mut self) -> Vec<BackgroundProtocolEvent> {
        let mut events = Vec::new();
        for handle in self
            .pending_popup_projections
            .iter()
            .copied()
            .collect::<Vec<_>>()
        {
            let pending = self
                .browser
                .web_contents_snapshot(handle)
                .ok()
                .and_then(|snapshot| snapshot.popup)
                .and_then(|popup| popup.pending_renderer_opening())
                .is_some();
            if !pending {
                self.pending_popup_projections.remove(&handle);
                events.extend(self.project_created_web_contents_inner(handle, false).await);
            }
        }
        events
    }

    async fn project_created_web_contents_inner(
        &mut self,
        handle: moli_core::browser::WebContentsHandle,
        observed: bool,
    ) -> Vec<BackgroundProtocolEvent> {
        if self
            .browser_context_by_browser_id(handle.context())
            .is_some_and(|context| {
                context
                    .page_targets
                    .get_for_web_contents(handle.id())
                    .is_some()
            })
        {
            return Vec::new();
        }
        let Ok(snapshot) = self.browser.web_contents_snapshot(handle) else {
            self.pending_popup_projections.remove(&handle);
            return Vec::new();
        };
        if !observed
            && snapshot
                .popup
                .as_ref()
                .and_then(|popup| popup.pending_renderer_opening())
                .is_some()
        {
            self.pending_popup_projections.insert(handle);
            return Vec::new();
        }
        self.project_created_browser_context(handle.context());
        let mut events = Vec::new();
        if let Some(opener) = snapshot.popup.as_ref().and_then(|popup| popup.opener)
            && opener != handle
            && self.browser.web_contents_snapshot(opener).is_ok()
        {
            events.extend(Box::pin(self.project_created_web_contents_inner(opener, false)).await);
            if self
                .browser_context_by_browser_id(opener.context())
                .and_then(|context| context.target_id_for_web_contents(opener.id()))
                .is_none()
            {
                self.pending_popup_projections.insert(handle);
                return events;
            }
        }
        let target_id = self.gen_target_id();
        let Some(context) = self.browser_context_by_browser_id_mut(handle.context()) else {
            return Vec::new();
        };
        if !context.adopt_web_contents(&snapshot, target_id.clone()) {
            return Vec::new();
        }
        self.pending_popup_projections.remove(&handle);
        events.extend(if snapshot.popup.is_some() {
            crate::domains::target::project_browser_popup_target(self, &target_id, &snapshot).await
        } else {
            crate::domains::target::project_browser_created_target(
                self,
                &target_id,
                snapshot.document.is_some(),
            )
            .await
        });
        if let Some(document) = snapshot.document {
            events.extend(self.project_browser_document_commit(document).await);
        }
        if let Some(selection) = self
            .browser
            .context_handle(handle.context())
            .ok()
            .and_then(|context| context.selected_web_contents_snapshot())
        {
            events.extend(self.project_browser_selection(
                selection.web_contents,
                None,
                selection.sequence,
            ));
        }
        events
    }

    pub async fn project_browser_snapshot(
        &mut self,
        snapshot: moli_core::browser::BrowserSnapshot,
    ) -> Vec<BackgroundProtocolEvent> {
        let mut events = Vec::new();
        let disposed = self
            .browser_contexts()
            .map(BrowserContext::browser_context_id)
            .filter(|id| !snapshot.contexts.contains(id) && !self.browser.contains_context(*id))
            .collect::<Vec<_>>();
        for context in disposed {
            events.extend(self.project_disposed_browser_context(context).await);
        }
        for handle in self.projected_web_contents() {
            if !snapshot.web_contents.contains(&handle) {
                let activated = snapshot
                    .selected_web_contents
                    .iter()
                    .copied()
                    .find(|selected| selected.context() == handle.context());
                events.extend(
                    self.project_closed_web_contents(handle, activated, snapshot.sequence)
                        .await,
                );
            }
        }
        for context in snapshot.contexts {
            self.project_created_browser_context(context);
        }
        for handle in snapshot.web_contents {
            events.extend(self.project_created_web_contents(handle).await);
        }
        for document in snapshot.documents {
            events.extend(self.project_browser_document_commit(document).await);
        }
        // Missing attempts matter too: after stream loss they release native
        // holds whose terminal occurrence was evicted. Committed Document
        // fences above and command-response holds remain independently owned.
        for contents in self.projected_web_contents() {
            events.extend(self.project_browser_navigation(contents).await);
            events.extend(self.project_browser_navigation_responses(contents).await);
            events.extend(self.project_browser_navigation_decision(contents).await);
        }
        for selected in snapshot.selected_web_contents {
            events.extend(self.project_browser_selection(selected, None, snapshot.sequence));
        }
        for download in snapshot.downloads {
            events.extend(self.project_created_browser_download(download));
        }
        events.extend(self.project_retired_context_downloads());
        events
    }

    /// Physical identities retained by this observer, including unselected pages.
    pub fn projected_web_contents(&self) -> Vec<moli_core::browser::WebContentsHandle> {
        self.browser_contexts()
            .flat_map(|context| {
                context.page_targets.iter().map(|target| {
                    moli_core::browser::WebContentsHandle::new(
                        context.browser_context_id(),
                        target.web_contents_id(),
                    )
                })
            })
            .collect()
    }

    pub async fn project_closed_web_contents(
        &mut self,
        handle: moli_core::browser::WebContentsHandle,
        activated: Option<moli_core::browser::WebContentsHandle>,
        sequence: moli_core::browser::BrowserSequence,
    ) -> Vec<BackgroundProtocolEvent> {
        self.retire_closed_web_contents(
            handle,
            activated,
            sequence,
            PageCloseNotifications::BrowserEvent,
        )
        .await
    }

    pub(in crate::conn) async fn retire_closed_web_contents(
        &mut self,
        handle: moli_core::browser::WebContentsHandle,
        activated: Option<moli_core::browser::WebContentsHandle>,
        sequence: moli_core::browser::BrowserSequence,
        notifications: PageCloseNotifications,
    ) -> Vec<BackgroundProtocolEvent> {
        let Some(context) = self.browser_context_by_browser_id_mut(handle.context()) else {
            return Vec::new();
        };
        let Some(mut target) = context.take_closed_web_contents_projection(handle) else {
            return Vec::new();
        };
        let info = context.retired_page_target_info(&target);
        let mut events = Vec::new();
        Self::retire_page_pending_calls(&context.id, &mut target, &mut events, "Target closed");
        let mut sessions = target
            .session_id()
            .map(str::to_owned)
            .into_iter()
            .chain(
                target
                    .devtools_sessions
                    .attached_session_ids()
                    .map(str::to_owned),
            )
            .chain(self.attached_sessions_for_target(target.target_id()))
            .collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        sessions.retain(|session| seen.insert(session.clone()));
        if !matches!(notifications, PageCloseNotifications::ContextDisposal) {
            events.extend(sessions.iter().map(|session| {
                BackgroundProtocolEvent::inspector_detached(Some(session), "Render process gone.")
            }));
        }
        self.record_collected_network_data_artifacts(
            target.runtime_slot.collected_network_data_artifacts(),
        );
        target.runtime_slot.retire_for_target_close();
        events.extend(
            self.project_retired_target(
                info,
                sessions,
                matches!(notifications, PageCloseNotifications::BrowserEvent),
            )
            .await,
        );
        if let Some(activated) = activated {
            events.extend(self.project_browser_selection(activated, Some(handle), sequence));
        }
        events
    }

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
            return self.project_retired_context_downloads();
        };
        // Selecting a remaining DevTools projection must not reconfigure a
        // Browser Context: this is an observation, not another transaction.
        if self.browser_context.is_none() {
            self.browser_context = self.inactive_browser_contexts.pop();
        }
        let infos = removed.retired_devtools_target_infos();
        let mut events = Vec::new();
        Self::retire_browser_context_pending_calls(&mut removed, &mut events);
        for target in removed.page_targets.iter() {
            self.record_collected_network_data_artifacts(
                target.runtime_slot.collected_network_data_artifacts(),
            );
        }
        for info in infos {
            let sessions = info
                .target_id
                .as_ref()
                .map(|id| self.attached_sessions_for_target(id.as_str()))
                .unwrap_or_default();
            events.extend(self.project_retired_target(info, sessions, true).await);
        }
        removed.retire_page_projections();
        events.extend(self.project_retired_context_downloads());
        events
    }

    async fn project_retired_target(
        &mut self,
        info: crate::devtools_runtime::DevToolsTargetInfo,
        sessions: Vec<String>,
        emit_automation: bool,
    ) -> Vec<BackgroundProtocolEvent> {
        let Some(target_id) = info.target_id.as_ref().map(|id| id.as_str().to_owned()) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        let destroyed = self
            .agent_hosts
            .project_page_tab_target_infos_for_destruction(info.clone());
        for mut info in destroyed.iter().filter(|info| info.attached).cloned() {
            info.attached = false;
            events.extend(self.exact_target_info_changed_events_for_all_observer_owners(info));
        }
        if emit_automation {
            events.extend(self.target_destroyed_automation_events(info));
        }
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
        events
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
