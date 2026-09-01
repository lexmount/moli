use crate::conn::CdpConnection;
use crate::devtools_runtime::DevToolsTargetInfo;

use super::{
    CdpSessionRoute, PreparedTargetAttach, TargetAttachRollbackPlan, TargetAttachSessionCommit,
    TargetAutoAttachedSessionDetachPlan, TargetBindingCleanupAction, TargetBindingCleanupPlan,
    TargetClosureCleanupPlan, TargetEventPlan, TargetSessionDetachCleanupPlan,
};

impl CdpConnection {
    pub(crate) async fn clear_target_session_overrides_async(
        &mut self,
        session_id: &str,
    ) -> Result<(), String> {
        let browser_identity_changed = match self.browser_context.as_mut() {
            Some(browser_context) => {
                browser_context
                    .clear_target_session_overrides_async(session_id)
                    .await?
            }
            None => false,
        };
        if !browser_identity_changed {
            return Ok(());
        }

        let Some(pending) =
            self.start_rebuild_resource_runtime_for_session_owner(Some(session_id))?
        else {
            return Ok(());
        };
        let completion = pending
            .wait()
            .await
            .map_err(|error| format!("failed to restore detached session user agent: {error}"))?;
        self.finish_rebuild_resource_runtime_for_session_owner(Some(session_id), completion)
    }

    pub(crate) fn is_browser_session_id(&self, session_id: Option<&str>) -> bool {
        let Some(session_id) = session_id else {
            return false;
        };
        self.target_control.attached_session_route(session_id) == Some(&CdpSessionRoute::Browser)
    }

    #[cfg(test)]
    pub(crate) fn register_browser_session(&mut self, session_id: String) {
        self.target_control.commit_attached_session(
            session_id,
            None,
            "browser",
            Some(CdpSessionRoute::Browser),
            false,
            false,
        );
    }

    fn clear_browser_session_owner_state(&mut self, session_id: &str) -> bool {
        if !self.is_browser_session_id(Some(session_id)) {
            return false;
        }
        self.download_behavior
            .set_browser_events_enabled_for_session(Some(session_id), false);
        self.cancel_tracing_for_session_owner(Some(session_id));
        self.clear_auto_attach_owner(Some(session_id));
        self.clear_target_discovery_for_owner(Some(session_id));
        self.set_service_worker_pause_on_start_owner(Some(session_id), false);
        self.set_dedicated_worker_pause_on_start_owner(Some(session_id), false);
        true
    }

    pub(crate) fn detach_browser_session_owner_without_event(
        &mut self,
        session_id: &str,
    ) -> Option<TargetEventPlan> {
        if !self.clear_browser_session_owner_state(session_id) {
            return None;
        }
        let rollback_plan = self.rollback_attached_session_without_event(session_id);
        Some(rollback_plan)
    }

    pub(crate) fn detach_browser_session_owner_event_plan(
        &mut self,
        session_id: &str,
    ) -> Option<TargetEventPlan> {
        let owner_session_id = self
            .target_control
            .attached_session_owner_session_id(session_id)
            .map(str::to_owned);
        if !self.clear_browser_session_owner_state(session_id) {
            return None;
        }
        let plan = self.target_control.detach_attached_session_event_plan(
            session_id,
            None,
            owner_session_id.as_deref(),
        );
        self.clear_detached_target_session_owner_state(session_id);
        plan
    }

    pub(crate) fn release_root_target_frontend_owner_without_event(&mut self) {
        self.download_behavior
            .set_browser_events_enabled_for_session(None, false);
        self.cancel_tracing_for_session_owner(None);
        self.clear_auto_attach_owner(None);
        self.clear_target_discovery_for_owner(None);
        self.target_control.remove_owner(None);
    }

    pub(crate) fn release_primary_target_session_binding_without_event(
        &mut self,
        session_id: &str,
    ) -> bool {
        let released = self
            .browser_context
            .as_mut()
            .is_some_and(|browser_context| {
                browser_context
                    .release_primary_session_binding_preserving_frontend_state(session_id)
            });
        if released {
            self.rollback_attached_session_without_event(session_id);
        }
        released
    }

    #[cfg(test)]
    pub(crate) fn register_auto_attached_session(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
    ) {
        let target_id = self.non_browser_target_id_for_session(Some(&session_id));
        self.register_auto_attached_session_for_target(
            session_id,
            owner_session_id,
            target_id.as_deref(),
        );
    }

    #[cfg(test)]
    pub(crate) fn register_auto_attached_session_for_target(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: Option<&str>,
    ) {
        let route = self.session_route(Some(&session_id));
        self.target_control.commit_auto_attached_session_for_target(
            session_id,
            owner_session_id,
            target_id,
            route,
            false,
        );
    }

    pub(crate) fn commit_prepared_attach_event_plan(
        &mut self,
        prepared: PreparedTargetAttach,
    ) -> TargetEventPlan {
        self.commit_prepared_attach_event_plan_with_attached_state_delta(prepared, true)
    }

    pub(crate) fn commit_prepared_dedicated_worker_attach_event_plan(
        &mut self,
        prepared: PreparedTargetAttach,
    ) -> TargetEventPlan {
        self.commit_prepared_attach_event_plan_with_attached_state_delta(prepared, false)
    }

    fn commit_prepared_attach_event_plan_with_attached_state_delta(
        &mut self,
        prepared: PreparedTargetAttach,
        emit_attached_state_delta: bool,
    ) -> TargetEventPlan {
        let (target_id, target_info, sessions) = prepared.into_parts();
        let should_emit_attached_state_delta = emit_attached_state_delta && !sessions.is_empty();
        let attached_state_delta_plan = should_emit_attached_state_delta
            .then(|| self.exact_target_info_changed_event_plan_for_target_delta(&target_id));
        let mut plan = TargetEventPlan::default();
        for session in sessions {
            let (session_id, owner_session_id, route, auto_attached, waiting_for_debugger) =
                session.into_parts();
            if auto_attached {
                self.target_control
                    .ensure_owner(owner_session_id.as_deref());
            }
            plan.extend(self.target_control.commit_attached_session_event(
                session_id,
                owner_session_id.as_deref(),
                &target_id,
                route,
                auto_attached,
                waiting_for_debugger,
                target_info.clone(),
            ));
        }
        if let Some(attached_state_delta_plan) = attached_state_delta_plan {
            plan.extend(attached_state_delta_plan);
        }
        plan
    }

    pub(crate) fn prepare_auto_attach_session_commit(
        &self,
        session_id: impl Into<String>,
        owner_session_id: Option<String>,
        waiting_for_debugger: bool,
    ) -> TargetAttachSessionCommit {
        let session_id = session_id.into();
        let route = self.session_route(Some(&session_id));
        TargetAttachSessionCommit::auto_attached(session_id, owner_session_id, waiting_for_debugger)
            .with_route(route)
    }

    pub(crate) fn prepare_direct_attach_session_commit(
        &self,
        session_id: impl Into<String>,
        owner_session_id: Option<String>,
        waiting_for_debugger: bool,
    ) -> TargetAttachSessionCommit {
        let session_id = session_id.into();
        let route = self.session_route(Some(&session_id));
        TargetAttachSessionCommit::direct(session_id, owner_session_id, waiting_for_debugger)
            .with_route(route)
    }

    pub(crate) fn attach_tab_target_session_event_plan(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        tab_target_id: &str,
        auxiliary: bool,
    ) -> Result<TargetEventPlan, &'static str> {
        if !self.assign_session_to_tab_target(tab_target_id, session_id.clone(), auxiliary) {
            return Err("UnknownTargetId");
        }
        let prepared_session = self.prepare_direct_attach_session_commit(
            session_id,
            owner_session_id.map(str::to_owned),
            false,
        );
        let Some(target_info) = self.tab_target_info(tab_target_id) else {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("UnknownTargetId");
        };
        if prepared_session.route().is_none() {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("InvalidSessionId");
        }
        Ok(
            self.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                tab_target_id,
                target_info,
                [prepared_session],
            )),
        )
    }

    pub(crate) fn attach_shared_worker_target_session_event_plan(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: &str,
    ) -> Result<TargetEventPlan, &'static str> {
        let session_id_for_binding = session_id.clone();
        let target_info = {
            let Some(bc) = self.browser_context.as_mut() else {
                return Err("BrowserContextNotLoaded");
            };
            if !bc.assign_session_to_shared_worker_target(target_id, session_id_for_binding) {
                return Err("UnknownTargetId");
            }
            bc.devtools_target_info(target_id)
        };
        let prepared_session = self.prepare_direct_attach_session_commit(
            session_id,
            owner_session_id.map(str::to_owned),
            false,
        );
        let Some(target_info) = target_info else {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("UnknownTargetId");
        };
        if prepared_session.route().is_none() {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("InvalidSessionId");
        }
        Ok(
            self.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                target_id,
                target_info,
                [prepared_session],
            )),
        )
    }

    pub(crate) fn attach_service_worker_target_session_event_plan(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: &str,
    ) -> Result<TargetEventPlan, &'static str> {
        let session_id_for_binding = session_id.clone();
        let target_info = {
            let Some(bc) = self.browser_context.as_mut() else {
                return Err("BrowserContextNotLoaded");
            };
            if !bc.assign_session_to_service_worker_target(target_id, session_id_for_binding) {
                return Err("UnknownTargetId");
            }
            bc.devtools_target_info(target_id)
        };
        let prepared_session = self.prepare_direct_attach_session_commit(
            session_id,
            owner_session_id.map(str::to_owned),
            false,
        );
        let Some(target_info) = target_info else {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("UnknownTargetId");
        };
        if prepared_session.route().is_none() {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("InvalidSessionId");
        }
        Ok(
            self.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                target_id,
                target_info,
                [prepared_session],
            )),
        )
    }

    pub(crate) fn attach_dedicated_worker_target_session_event_plan(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: &str,
    ) -> Result<TargetEventPlan, &'static str> {
        let session_id_for_binding = session_id.clone();
        let target_info = {
            let Some(bc) = self.browser_context.as_mut() else {
                return Err("BrowserContextNotLoaded");
            };
            if !bc.assign_session_to_dedicated_worker_target(target_id, session_id_for_binding) {
                return Err("UnknownTargetId");
            }
            bc.devtools_target_info(target_id)
        };
        let prepared_session = self.prepare_direct_attach_session_commit(
            session_id,
            owner_session_id.map(str::to_owned),
            false,
        );
        let Some(target_info) = target_info else {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("UnknownTargetId");
        };
        if prepared_session.route().is_none() {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("InvalidSessionId");
        }
        Ok(
            self.commit_prepared_dedicated_worker_attach_event_plan(PreparedTargetAttach::new(
                target_id,
                target_info,
                [prepared_session],
            )),
        )
    }

    pub(crate) fn prepare_auto_attached_tab_session_binding(
        &mut self,
        tab_target_id: &str,
        session_id: String,
        owner_session_id: Option<&str>,
    ) -> bool {
        self.assign_session_to_tab_target(tab_target_id, session_id, owner_session_id.is_some())
    }

    pub(crate) fn prepare_auto_attached_page_session_binding(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        self.browser_context
            .as_mut()
            .is_some_and(|bc| bc.assign_auto_attached_session_to_target(target_id, session_id))
    }

    pub(crate) fn prepare_auto_attached_page_session_binding_in_browser_context(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        session_id: String,
    ) -> bool {
        self.browser_context_by_id_mut(browser_context_id)
            .is_some_and(|bc| bc.assign_auto_attached_session_to_target(target_id, session_id))
    }

    pub(crate) fn prepare_auto_attached_shared_worker_session_binding(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        self.browser_context
            .as_mut()
            .is_some_and(|bc| bc.assign_session_to_shared_worker_target(target_id, session_id))
    }

    pub(crate) fn prepare_auto_attached_dedicated_worker_session_binding(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        self.browser_context
            .as_mut()
            .is_some_and(|bc| bc.assign_session_to_dedicated_worker_target(target_id, session_id))
    }

    pub(crate) fn prepare_auto_attached_service_worker_session_binding(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        self.browser_context
            .as_mut()
            .is_some_and(|bc| bc.assign_session_to_service_worker_target(target_id, session_id))
    }

    pub(crate) fn prepare_auto_attached_shared_worker_session_binding_info_in_browser_context(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        session_id: String,
    ) -> Option<DevToolsTargetInfo> {
        let bc = self.browser_context_by_id_mut(browser_context_id)?;
        if !bc.assign_session_to_shared_worker_target(target_id, session_id) {
            return None;
        }
        bc.devtools_target_info(target_id)
    }

    pub(crate) fn prepare_auto_attached_dedicated_worker_session_binding_info_in_browser_context(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        session_id: String,
    ) -> Option<DevToolsTargetInfo> {
        let bc = self.browser_context_by_id_mut(browser_context_id)?;
        if !bc.assign_session_to_dedicated_worker_target(target_id, session_id) {
            return None;
        }
        bc.devtools_target_info(target_id)
    }

    pub(crate) fn prepare_auto_attached_service_worker_session_binding_info_in_browser_context(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        session_id: String,
    ) -> Option<DevToolsTargetInfo> {
        let bc = self.browser_context_by_id_mut(browser_context_id)?;
        if !bc.assign_session_to_service_worker_target(target_id, session_id) {
            return None;
        }
        bc.devtools_target_info(target_id)
    }

    pub(crate) fn prepare_auto_attached_service_worker_session_binding_info(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> Option<DevToolsTargetInfo> {
        let bc = self.browser_context.as_mut()?;
        if !bc.assign_session_to_service_worker_target(target_id, session_id) {
            return None;
        }
        bc.devtools_target_info(target_id)
    }

    pub(crate) fn commit_browser_attached_session_event_plan(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: &str,
        target_info: DevToolsTargetInfo,
    ) -> TargetEventPlan {
        self.target_control.commit_attached_session_event(
            session_id,
            owner_session_id,
            target_id,
            Some(CdpSessionRoute::Browser),
            false,
            false,
            target_info,
        )
    }

    pub(crate) fn rollback_attached_session_without_event(
        &mut self,
        session_id: &str,
    ) -> TargetEventPlan {
        let plan = self
            .target_control
            .rollback_attached_session_without_event(session_id);
        for session_id in plan.rolled_back_session_ids() {
            self.clear_detached_target_session_owner_state(session_id);
        }
        plan
    }

    pub(crate) fn detach_known_session_event_plan(
        &mut self,
        target_id: &str,
        session_id: &str,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        self.detach_known_session_event_plan_with_attached_state_delta(
            target_id,
            session_id,
            reason,
            parent_session_id,
            true,
        )
    }

    fn detach_known_session_event_plan_with_attached_state_delta(
        &mut self,
        target_id: &str,
        session_id: &str,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
        emit_attached_state_delta: bool,
    ) -> TargetEventPlan {
        let attached_state_delta_plan = emit_attached_state_delta
            .then(|| self.exact_target_info_changed_event_plan_for_target_delta(target_id));
        let mut plan = self.target_control.detach_known_session_event_plan(
            target_id,
            session_id,
            reason,
            parent_session_id,
        );
        let released_debugger_barrier = plan
            .detached_sessions()
            .iter()
            .any(|session| session.target_id() == target_id && session.was_waiting_for_debugger());
        self.clear_detached_target_session_owner_state(session_id);
        if let Some(attached_state_delta_plan) = attached_state_delta_plan {
            plan.extend(attached_state_delta_plan);
        }
        if released_debugger_barrier && !self.target_has_waiting_for_debugger_session(target_id) {
            crate::domains::target::schedule_initial_document_target_url_navigation_after_debugger_barrier_release_for_target(
                self,
                target_id,
            );
        }
        plan
    }

    pub(crate) fn detach_target_closure_cleanup_event_plan(
        &mut self,
        cleanup_plan: TargetClosureCleanupPlan,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let plan = self
            .target_control
            .detach_target_closure_cleanup_event_plan(cleanup_plan, parent_session_id);
        for session in plan.detached_sessions() {
            self.clear_detached_target_session_owner_state(session.session_id());
        }
        plan
    }

    pub(crate) async fn rollback_prepared_attach_session_without_event_async(
        &mut self,
        prepared: &TargetAttachSessionCommit,
    ) -> TargetEventPlan {
        self.rollback_attached_session_with_cleanup_without_event_async(
            TargetAttachRollbackPlan::from_prepared_attach_session(prepared),
        )
        .await
    }

    pub(crate) fn rollback_prepared_attach_session_sync_without_event(
        &mut self,
        prepared: &TargetAttachSessionCommit,
    ) -> TargetEventPlan {
        self.rollback_attached_session_with_cleanup_without_event_sync(
            TargetAttachRollbackPlan::from_prepared_attach_session(prepared),
        )
    }

    fn rollback_attached_session_with_cleanup_without_event_sync(
        &mut self,
        rollback_plan: TargetAttachRollbackPlan,
    ) -> TargetEventPlan {
        if let Some(cleanup_plan) = rollback_plan.cleanup_plan() {
            if matches!(
                cleanup_plan.action(),
                TargetBindingCleanupAction::PageTarget {
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                    ..
                }
            ) {
                debug_assert!(
                    false,
                    "primary Page target rollback requires async binding cleanup"
                );
            } else {
                self.execute_target_binding_cleanup_without_event_sync(cleanup_plan);
            }
        }
        self.rollback_attached_session_without_event(rollback_plan.session_id())
    }

    fn execute_target_binding_cleanup_without_event_sync(
        &mut self,
        cleanup_plan: &TargetBindingCleanupPlan,
    ) {
        match cleanup_plan.action() {
            TargetBindingCleanupAction::PageTarget {
                session_key: moli_page_types::DevToolsSessionKey::Attached(_),
                ..
            } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.remove_auxiliary_session(cleanup_plan.session_id());
                }
            }
            TargetBindingCleanupAction::TabTarget { .. } => {
                self.remove_tab_session(cleanup_plan.session_id());
            }
            TargetBindingCleanupAction::SharedWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_shared_worker_target_session(cleanup_plan.session_id());
                }
            }
            TargetBindingCleanupAction::DedicatedWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_dedicated_worker_target_session(cleanup_plan.session_id());
                }
            }
            TargetBindingCleanupAction::ServiceWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_service_worker_target_session(cleanup_plan.session_id());
                }
            }
            TargetBindingCleanupAction::None
            | TargetBindingCleanupAction::PageTarget {
                session_key: moli_page_types::DevToolsSessionKey::Primary,
                ..
            } => {}
        }
    }

    async fn rollback_attached_session_with_cleanup_without_event_async(
        &mut self,
        rollback_plan: TargetAttachRollbackPlan,
    ) -> TargetEventPlan {
        let Some(browser_context_id) = rollback_plan.browser_context_id().map(str::to_owned) else {
            return self.rollback_attached_session_without_event(rollback_plan.session_id());
        };
        if !self
            .activate_browser_context_by_id_async(&browser_context_id)
            .await
        {
            return self.rollback_attached_session_without_event(rollback_plan.session_id());
        }

        if let Some(cleanup_plan) = rollback_plan.cleanup_plan() {
            self.execute_target_binding_cleanup_without_event_async(cleanup_plan)
                .await;
        }
        self.rollback_attached_session_without_event(rollback_plan.session_id())
    }

    pub(crate) fn auto_attached_session_detach_plan(
        &self,
        session_id: &str,
    ) -> TargetAutoAttachedSessionDetachPlan {
        TargetAutoAttachedSessionDetachPlan::from_session_route(
            session_id,
            self.session_route(Some(session_id)),
        )
    }

    pub(crate) fn rollback_auto_attached_session_detach_plan_without_event(
        &mut self,
        detach_plan: &TargetAutoAttachedSessionDetachPlan,
    ) -> TargetEventPlan {
        self.rollback_attached_session_without_event(detach_plan.session_id())
    }

    pub(crate) async fn execute_target_binding_cleanup_without_event_async(
        &mut self,
        cleanup_plan: &TargetBindingCleanupPlan,
    ) {
        self.remove_document_start_scripts_for_detached_session_best_effort_async(
            cleanup_plan.session_id(),
        )
        .await;
        match cleanup_plan.action() {
            TargetBindingCleanupAction::PageTarget {
                target_id,
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    if bc.is_active_target(target_id) {
                        let _ = bc
                            .clear_active_target_primary_auto_attached_session_async()
                            .await;
                    } else {
                        let _ = bc.clear_background_target_primary_auto_attached_session(
                            cleanup_plan.session_id(),
                        );
                    }
                }
            }
            TargetBindingCleanupAction::PageTarget {
                session_key: moli_page_types::DevToolsSessionKey::Attached(_),
                ..
            } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.remove_auxiliary_session(cleanup_plan.session_id());
                }
            }
            TargetBindingCleanupAction::TabTarget { .. } => {
                self.remove_tab_session(cleanup_plan.session_id());
            }
            TargetBindingCleanupAction::SharedWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_shared_worker_target_session(cleanup_plan.session_id());
                }
            }
            TargetBindingCleanupAction::DedicatedWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_dedicated_worker_target_session(cleanup_plan.session_id());
                }
            }
            TargetBindingCleanupAction::ServiceWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_service_worker_target_session(cleanup_plan.session_id());
                }
            }
            TargetBindingCleanupAction::None => {}
        }
    }

    pub(crate) async fn execute_target_binding_cleanup_for_session_without_event_async(
        &mut self,
        session_id: &str,
    ) -> bool {
        let Some(route) = self.session_route(Some(session_id)) else {
            return false;
        };
        self.cancel_tracing_for_session_owner_async(Some(session_id))
            .await;
        let cleanup_plan = TargetBindingCleanupPlan::from_route(session_id, &route);
        self.execute_target_binding_cleanup_without_event_async(&cleanup_plan)
            .await;
        true
    }

    pub(crate) async fn detach_session_with_binding_cleanup_event_plan_async(
        &mut self,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    ) -> TargetEventPlan {
        let session_id = cleanup_plan.session_id().to_owned();
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        let parent_session_id = cleanup_plan
            .parent_session_id()
            .or_else(|| {
                self.target_control
                    .attached_session_owner_session_id(&session_id)
            })
            .map(str::to_owned);
        let _ = self
            .execute_target_binding_cleanup_for_session_without_event_async(&session_id)
            .await;
        self.detach_known_session_event_plan(
            &target_id,
            &session_id,
            reason.as_deref(),
            parent_session_id.as_deref(),
        )
    }

    pub(crate) async fn detach_dedicated_worker_session_with_binding_cleanup_event_plan_async(
        &mut self,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    ) -> TargetEventPlan {
        let session_id = cleanup_plan.session_id().to_owned();
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        let parent_session_id = cleanup_plan
            .parent_session_id()
            .or_else(|| {
                self.target_control
                    .attached_session_owner_session_id(&session_id)
            })
            .map(str::to_owned);
        let _ = self
            .execute_target_binding_cleanup_for_session_without_event_async(&session_id)
            .await;
        self.detach_known_session_event_plan_with_attached_state_delta(
            &target_id,
            &session_id,
            reason.as_deref(),
            parent_session_id.as_deref(),
            false,
        )
    }

    pub(crate) fn detach_dedicated_worker_session_after_target_removal_event_plan(
        &mut self,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    ) -> TargetEventPlan {
        let parent_session_id = cleanup_plan
            .parent_session_id()
            .or_else(|| {
                self.target_control
                    .attached_session_owner_session_id(cleanup_plan.session_id())
            })
            .map(str::to_owned);
        self.detach_known_session_event_plan_with_attached_state_delta(
            cleanup_plan.target_id(),
            cleanup_plan.session_id(),
            cleanup_plan.reason(),
            parent_session_id.as_deref(),
            false,
        )
    }

    pub(crate) async fn detach_target_sessions_with_binding_cleanup_event_plan_async(
        &mut self,
        cleanup_plan: TargetClosureCleanupPlan,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        let session_ids = cleanup_plan
            .session_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut plan = TargetEventPlan::default();
        for session_id in session_ids {
            if self
                .execute_target_binding_cleanup_for_session_without_event_async(&session_id)
                .await
            {
                plan.extend(self.detach_known_session_event_plan(
                    &target_id,
                    &session_id,
                    reason.as_deref(),
                    parent_session_id,
                ));
            }
        }
        plan
    }

    pub(crate) async fn detach_active_target_session_binding_event_plan_async(
        &mut self,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    ) -> Result<TargetEventPlan, String> {
        let session_id = cleanup_plan.session_id().to_owned();
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        let parent_session_id = cleanup_plan.parent_session_id().map(str::to_owned);
        self.remove_document_start_scripts_for_detached_session_best_effort_async(&session_id)
            .await;
        self.clear_active_target_session_binding_for_detach_async()
            .await?;
        Ok(self.detach_known_session_event_plan(
            &target_id,
            &session_id,
            reason.as_deref(),
            parent_session_id.as_deref(),
        ))
    }

    pub(crate) async fn detach_background_target_session_binding_event_plan_async(
        &mut self,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    ) -> Result<Option<TargetEventPlan>, String> {
        let session_id = cleanup_plan.session_id().to_owned();
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        let parent_session_id = cleanup_plan.parent_session_id().map(str::to_owned);
        self.remove_document_start_scripts_for_detached_session_best_effort_async(&session_id)
            .await;
        let Some(detached_target_id) = self
            .clear_background_target_session_binding_for_detach_async(&session_id)
            .await?
        else {
            return Ok(None);
        };
        debug_assert_eq!(detached_target_id, target_id);
        Ok(Some(self.detach_known_session_event_plan(
            &target_id,
            &session_id,
            reason.as_deref(),
            parent_session_id.as_deref(),
        )))
    }

    pub(crate) fn background_target_session_detach_cleanup_plans(
        &self,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> Vec<TargetSessionDetachCleanupPlan> {
        self.browser_context
            .as_ref()
            .map(|bc| {
                bc.background_targets()
                    .filter_map(|target| {
                        Some(TargetSessionDetachCleanupPlan::new(
                            target.target_id().to_owned(),
                            target.session_id()?.to_owned(),
                            reason,
                            parent_session_id,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) async fn clear_active_target_session_binding_for_detach_async(
        &mut self,
    ) -> Result<(), String> {
        let Some(bc) = self.browser_context.as_mut() else {
            return Ok(());
        };
        bc.clear_active_target_session_binding_and_scoped_state_async()
            .await
    }

    pub(crate) async fn clear_background_target_session_binding_for_detach_async(
        &mut self,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        let Some(bc) = self.browser_context.as_mut() else {
            return Ok(None);
        };
        bc.clear_background_target_session_binding_and_scoped_state_async(session_id)
            .await
    }

    pub(crate) async fn detach_all_shared_worker_target_sessions_event_plan_async(
        &mut self,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let target_ids = self
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.shared_worker_targets
                    .values()
                    .filter(|target| target.has_session())
                    .map(|target| target.target_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut plan = TargetEventPlan::default();
        for target_id in target_ids {
            let session_ids = self
                .browser_context
                .as_ref()
                .and_then(|bc| bc.shared_worker_target(&target_id))
                .map(|target| target.session_ids())
                .unwrap_or_default();
            for session_id in session_ids {
                if self
                    .execute_target_binding_cleanup_for_session_without_event_async(&session_id)
                    .await
                {
                    plan.extend(self.detach_known_session_event_plan(
                        &target_id,
                        &session_id,
                        reason,
                        parent_session_id,
                    ));
                }
            }
        }
        plan
    }

    pub(crate) async fn detach_all_service_worker_target_sessions_event_plan_async(
        &mut self,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let target_ids = self
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.service_worker_targets
                    .values()
                    .filter(|target| target.has_session())
                    .map(|target| target.target_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut plan = TargetEventPlan::default();
        for target_id in target_ids {
            let session_ids = self
                .browser_context
                .as_ref()
                .and_then(|bc| bc.service_worker_target(&target_id))
                .map(|target| target.session_ids())
                .unwrap_or_default();
            for session_id in session_ids {
                if self
                    .execute_target_binding_cleanup_for_session_without_event_async(&session_id)
                    .await
                {
                    plan.extend(self.detach_known_session_event_plan(
                        &target_id,
                        &session_id,
                        reason,
                        parent_session_id,
                    ));
                }
            }
        }
        plan
    }

    pub(crate) async fn detach_all_dedicated_worker_target_sessions_event_plan_async(
        &mut self,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let target_ids = self
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.dedicated_worker_targets
                    .values()
                    .filter(|target| target.has_session())
                    .map(|target| target.target_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut plan = TargetEventPlan::default();
        for target_id in target_ids {
            let session_ids = self
                .browser_context
                .as_ref()
                .and_then(|bc| bc.dedicated_worker_target(&target_id))
                .map(|target| target.session_ids())
                .unwrap_or_default();
            for session_id in session_ids {
                if self
                    .execute_target_binding_cleanup_for_session_without_event_async(&session_id)
                    .await
                {
                    plan.extend(self.detach_known_session_event_plan(
                        &target_id,
                        &session_id,
                        reason,
                        parent_session_id,
                    ));
                }
            }
        }
        plan
    }

    fn clear_detached_target_session_owner_state(&mut self, session_id: &str) {
        self.download_behavior
            .set_browser_events_enabled_for_session(Some(session_id), false);
        self.cancel_tracing_for_session_owner(Some(session_id));
        self.clear_auto_attach_owner(Some(session_id));
        self.set_service_worker_pause_on_start_owner(Some(session_id), false);
        self.target_control.remove_owner(Some(session_id));
    }

    pub(crate) fn attached_sessions_for_target(&self, target_id: &str) -> Vec<String> {
        self.target_control.attached_sessions_for_target(target_id)
    }

    pub(crate) fn target_has_waiting_for_debugger_session(&self, target_id: &str) -> bool {
        self.target_control
            .target_has_waiting_for_debugger_session(target_id)
    }

    pub(crate) fn release_waiting_for_debugger_session(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        session_id.is_some_and(|session_id| {
            self.target_control
                .release_waiting_for_debugger_session(session_id)
        })
    }

    pub(crate) fn auto_attached_sessions_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        self.target_control
            .auto_attached_sessions_for_owner(owner_session_id)
    }

    pub(crate) fn attached_session_cascade_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        self.target_control
            .attached_session_cascade_for_owner(owner_session_id)
    }

    pub(crate) fn attached_session_cascade_for_root_frontend(&self) -> Vec<String> {
        self.target_control
            .attached_session_cascade_for_root_frontend()
    }

    pub(crate) fn auto_attached_session_cascade_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        self.target_control
            .auto_attached_session_cascade_for_owner(owner_session_id)
    }
}
