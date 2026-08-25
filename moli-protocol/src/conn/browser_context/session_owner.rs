use super::*;
use crate::conn::{ServiceWorkerTargetState, SharedWorkerTargetState};
use crate::devtools_runtime::DevToolsTargetInfo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CdpSessionRoute {
    Browser,
    TabTarget {
        browser_context_id: String,
        tab_target_id: String,
    },
    ActiveTarget {
        browser_context_id: String,
        target_id: Option<String>,
    },
    AuxiliaryTarget {
        browser_context_id: String,
        target_id: String,
    },
    BackgroundTarget {
        browser_context_id: String,
        target_id: String,
    },
    SharedWorkerTarget {
        browser_context_id: String,
        target_id: String,
    },
    DedicatedWorkerTarget {
        browser_context_id: String,
        target_id: String,
    },
    ServiceWorkerTarget {
        browser_context_id: String,
        target_id: String,
    },
}

impl CdpSessionRoute {
    pub(crate) fn browser_context_id(&self) -> Option<&str> {
        match self {
            Self::Browser => None,
            Self::TabTarget {
                browser_context_id, ..
            }
            | Self::ActiveTarget {
                browser_context_id, ..
            }
            | Self::AuxiliaryTarget {
                browser_context_id, ..
            }
            | Self::BackgroundTarget {
                browser_context_id, ..
            }
            | Self::SharedWorkerTarget {
                browser_context_id, ..
            }
            | Self::DedicatedWorkerTarget {
                browser_context_id, ..
            }
            | Self::ServiceWorkerTarget {
                browser_context_id, ..
            } => Some(browser_context_id),
        }
    }
}

pub(super) enum TargetSessionOwner {
    ActiveTarget {
        browser_context_id: String,
        is_auxiliary_target_session: bool,
    },
    BackgroundTarget {
        browser_context_id: String,
        target_id: String,
        is_auxiliary_target_session: bool,
    },
    NoLoadedBrowserContext,
}

impl CdpConnection {
    pub(crate) fn session_route(&self, session_id: Option<&str>) -> Option<CdpSessionRoute> {
        let session_id = session_id?;
        if self.browser_session_ids.contains(session_id) {
            return Some(CdpSessionRoute::Browser);
        }
        if let Some(tab_target_id) = self.tab_target_id_for_session_id(session_id)
            && let Some(browser_context_id) =
                self.browser_context_id_for_tab_target_id(tab_target_id)
        {
            return Some(CdpSessionRoute::TabTarget {
                browser_context_id,
                tab_target_id: tab_target_id.to_owned(),
            });
        }
        self.browser_contexts()
            .find_map(|bc| browser_context_session_route(bc, session_id))
    }

    pub(crate) fn target_session_route_for_target_id(
        &self,
        target_id: &str,
    ) -> Option<CdpSessionRoute> {
        if self.page_target_id_for_tab_target_id(target_id).is_some()
            && let Some(browser_context_id) = self.browser_context_id_for_tab_target_id(target_id)
        {
            return Some(CdpSessionRoute::TabTarget {
                browser_context_id,
                tab_target_id: target_id.to_owned(),
            });
        }
        self.browser_contexts().find_map(|browser_context| {
            if browser_context.active_target_id() == Some(target_id) {
                return Some(CdpSessionRoute::ActiveTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: None,
                });
            }
            if browser_context.background_target(target_id).is_some() {
                return Some(CdpSessionRoute::BackgroundTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                });
            }
            if browser_context.has_shared_worker_target(target_id) {
                return Some(CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                });
            }
            if browser_context.has_dedicated_worker_target(target_id) {
                return Some(CdpSessionRoute::DedicatedWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                });
            }
            browser_context
                .has_service_worker_target(target_id)
                .then(|| CdpSessionRoute::ServiceWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                })
        })
    }

    pub fn worker_target_id_for_session(&self, session_id: Option<&str>) -> Option<String> {
        match self.session_route(session_id)? {
            CdpSessionRoute::SharedWorkerTarget { target_id, .. }
            | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
            | CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => Some(target_id),
            _ => None,
        }
    }

    pub(crate) fn target_session_route_for_child_frame_id(
        &self,
        frame_id: &str,
    ) -> Option<CdpSessionRoute> {
        self.browser_contexts().find_map(|browser_context| {
            if browser_context
                .active_target
                .owner_state
                .has_attached_child_frame_id(frame_id)
            {
                return Some(CdpSessionRoute::ActiveTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: None,
                });
            }
            browser_context
                .background_targets
                .iter()
                .find_map(|target| {
                    browser_context
                        .parked_target_owner_state(target.target_id())
                        .is_some_and(|owner_state| {
                            owner_state.has_attached_child_frame_id(frame_id)
                        })
                        .then(|| CdpSessionRoute::BackgroundTarget {
                            browser_context_id: browser_context.id.clone(),
                            target_id: target.target_id().to_owned(),
                        })
                })
        })
    }

    pub(crate) fn has_attached_child_frame_id(&self, frame_id: &str) -> bool {
        self.browser_contexts()
            .any(|browser_context| browser_context.has_attached_child_frame_id(frame_id))
    }

    #[cfg(test)]
    pub(crate) fn has_background_target_session(&self, session_id: Option<&str>) -> bool {
        self.background_target_route(session_id).is_some()
    }

    #[cfg(test)]
    pub(crate) fn background_target_id_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.background_target_route(session_id)
            .map(|(_, target_id)| target_id)
    }

    fn background_target_route(&self, session_id: Option<&str>) -> Option<(String, String)> {
        match self.session_route(session_id)? {
            CdpSessionRoute::BackgroundTarget {
                browser_context_id,
                target_id,
            } => Some((browser_context_id, target_id)),
            CdpSessionRoute::AuxiliaryTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id(&browser_context_id)?
                .background_target(&target_id)
                .is_some()
                .then_some((browser_context_id, target_id)),
            CdpSessionRoute::Browser
            | CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::ActiveTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => None,
        }
    }

    pub(crate) fn is_browser_session_id(&self, session_id: Option<&str>) -> bool {
        let Some(session_id) = session_id else {
            return false;
        };
        self.browser_session_ids.contains(session_id)
    }

    #[cfg(test)]
    pub(crate) fn register_browser_session(&mut self, session_id: String) {
        self.browser_session_ids.insert(session_id);
    }

    pub(crate) fn remove_browser_session(&mut self, session_id: &str) -> bool {
        self.browser_session_ids.remove(session_id)
    }

    fn clear_browser_session_owner_state(&mut self, session_id: &str) -> bool {
        if !self.remove_browser_session(session_id) {
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
    ) -> Option<crate::conn::target::TargetEventPlan> {
        if !self.clear_browser_session_owner_state(session_id) {
            return None;
        }
        let rollback_plan = self.rollback_attached_session_without_event(session_id);
        Some(rollback_plan)
    }

    pub(crate) fn detach_browser_session_owner_event_plan(
        &mut self,
        session_id: &str,
    ) -> Option<crate::conn::target::TargetEventPlan> {
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

    fn sync_auto_attach_flags_from_owners(&mut self) {
        self.auto_attach = !self.auto_attach_owner_sessions.is_empty();
        self.auto_attach_wait_for_debugger_on_start = self
            .auto_attach_owner_sessions
            .values()
            .any(|policy| policy.wait_for_debugger_on_start);
    }

    pub(crate) fn has_auto_attach_owner(&self, session_id: Option<&str>) -> bool {
        self.auto_attach_owner_sessions
            .contains_key(&session_id.map(str::to_owned))
    }

    pub(crate) fn auto_attach_owner_count(&self) -> usize {
        self.auto_attach_owner_sessions.len()
    }

    pub(crate) fn auto_attach_owner_sessions_for_target_type(
        &self,
        target_type: &str,
    ) -> Vec<Option<String>> {
        if self.auto_attach_owner_sessions.is_empty() && self.auto_attach {
            return super::super::CdpTargetFilter::default_auto_attach()
                .matches(target_type)
                .then_some(None)
                .into_iter()
                .collect();
        }
        self.auto_attach_owner_sessions
            .iter()
            .filter(|(_, policy)| policy.target_filter.matches(target_type))
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>()
    }

    pub(crate) fn auto_attach_owner_allows_target_type(
        &self,
        session_id: Option<&str>,
        target_type: &str,
    ) -> bool {
        self.auto_attach_owner_sessions
            .get(&session_id.map(str::to_owned))
            .is_some_and(|policy| policy.target_filter.matches(target_type))
            || (self.auto_attach_owner_sessions.is_empty()
                && self.auto_attach
                && super::super::CdpTargetFilter::default_auto_attach().matches(target_type))
    }

    pub(crate) fn auto_attach_owner_waits_for_debugger_on_start(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        if self.auto_attach_owner_sessions.is_empty() && self.auto_attach {
            return self.auto_attach_wait_for_debugger_on_start;
        }
        self.auto_attach_owner_sessions
            .get(&session_id.map(str::to_owned))
            .is_some_and(|policy| policy.wait_for_debugger_on_start)
    }

    pub(crate) fn set_auto_attach_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
        wait_for_debugger_on_start: bool,
        target_filter: super::super::CdpTargetFilter,
    ) {
        self.clear_service_worker_auto_attach_related_owner(session_id);
        let key = session_id.map(str::to_owned);
        if enabled {
            self.target_control.ensure_owner(session_id);
            self.auto_attach_owner_sessions.insert(
                key,
                super::super::AutoAttachOwnerPolicy {
                    wait_for_debugger_on_start,
                    target_filter,
                },
            );
        } else {
            self.auto_attach_owner_sessions.remove(&key);
        }
        self.sync_auto_attach_flags_from_owners();
    }

    pub(crate) fn clear_auto_attach_owner(&mut self, session_id: Option<&str>) {
        self.clear_service_worker_auto_attach_related_owner(session_id);
        let key = session_id.map(str::to_owned);
        self.auto_attach_owner_sessions.remove(&key);
        self.sync_auto_attach_flags_from_owners();
    }

    #[cfg(test)]
    pub(crate) fn register_auto_attached_session(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
    ) {
        let target_id = self.target_id_for_auto_attached_session(&session_id);
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
        prepared: crate::conn::PreparedTargetAttach,
    ) -> crate::conn::target::TargetEventPlan {
        self.commit_prepared_attach_event_plan_with_attached_state_delta(prepared, true)
    }

    pub(crate) fn commit_prepared_dedicated_worker_attach_event_plan(
        &mut self,
        prepared: crate::conn::PreparedTargetAttach,
    ) -> crate::conn::target::TargetEventPlan {
        self.commit_prepared_attach_event_plan_with_attached_state_delta(prepared, false)
    }

    fn commit_prepared_attach_event_plan_with_attached_state_delta(
        &mut self,
        prepared: crate::conn::PreparedTargetAttach,
        emit_attached_state_delta: bool,
    ) -> crate::conn::target::TargetEventPlan {
        let (target_id, target_info, sessions) = prepared.into_parts();
        let should_emit_attached_state_delta = emit_attached_state_delta && !sessions.is_empty();
        let attached_state_delta_plan = should_emit_attached_state_delta
            .then(|| self.exact_target_info_changed_event_plan_for_target_delta(&target_id));
        let mut plan = crate::conn::target::TargetEventPlan::default();
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
    ) -> crate::conn::TargetAttachSessionCommit {
        let session_id = session_id.into();
        let route = self.session_route(Some(&session_id));
        crate::conn::TargetAttachSessionCommit::auto_attached(
            session_id,
            owner_session_id,
            waiting_for_debugger,
        )
        .with_route(route)
    }

    pub(crate) fn prepare_direct_attach_session_commit(
        &self,
        session_id: impl Into<String>,
        owner_session_id: Option<String>,
        waiting_for_debugger: bool,
    ) -> crate::conn::TargetAttachSessionCommit {
        let session_id = session_id.into();
        let route = self.session_route(Some(&session_id));
        crate::conn::TargetAttachSessionCommit::direct(
            session_id,
            owner_session_id,
            waiting_for_debugger,
        )
        .with_route(route)
    }

    pub(crate) fn attach_tab_target_session_event_plan(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        tab_target_id: &str,
        auxiliary: bool,
    ) -> Result<crate::conn::target::TargetEventPlan, &'static str> {
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
            self.commit_prepared_attach_event_plan(crate::conn::PreparedTargetAttach::new(
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
    ) -> Result<crate::conn::target::TargetEventPlan, &'static str> {
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
            self.commit_prepared_attach_event_plan(crate::conn::PreparedTargetAttach::new(
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
    ) -> Result<crate::conn::target::TargetEventPlan, &'static str> {
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
            self.commit_prepared_attach_event_plan(crate::conn::PreparedTargetAttach::new(
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
    ) -> Result<crate::conn::target::TargetEventPlan, &'static str> {
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
        Ok(self.commit_prepared_dedicated_worker_attach_event_plan(
            crate::conn::PreparedTargetAttach::new(target_id, target_info, [prepared_session]),
        ))
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
    ) -> crate::conn::target::TargetEventPlan {
        self.browser_session_ids.insert(session_id.clone());
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
    ) -> crate::conn::target::TargetEventPlan {
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
    ) -> crate::conn::target::TargetEventPlan {
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
    ) -> crate::conn::target::TargetEventPlan {
        let attached_state_delta_plan = emit_attached_state_delta
            .then(|| self.exact_target_info_changed_event_plan_for_target_delta(target_id));
        let mut plan = self.target_control.detach_known_session_event_plan(
            target_id,
            session_id,
            reason,
            parent_session_id,
        );
        self.clear_detached_target_session_owner_state(session_id);
        if let Some(attached_state_delta_plan) = attached_state_delta_plan {
            plan.extend(attached_state_delta_plan);
        }
        plan
    }

    pub(crate) fn detach_target_closure_cleanup_event_plan(
        &mut self,
        cleanup_plan: crate::conn::TargetClosureCleanupPlan,
        parent_session_id: Option<&str>,
    ) -> crate::conn::target::TargetEventPlan {
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
        prepared: &crate::conn::TargetAttachSessionCommit,
    ) -> crate::conn::target::TargetEventPlan {
        self.rollback_attached_session_with_cleanup_without_event_async(
            crate::conn::TargetAttachRollbackPlan::from_prepared_attach_session(prepared),
        )
        .await
    }

    pub(crate) fn rollback_prepared_attach_session_sync_without_event(
        &mut self,
        prepared: &crate::conn::TargetAttachSessionCommit,
    ) -> crate::conn::target::TargetEventPlan {
        self.rollback_attached_session_with_cleanup_without_event_sync(
            crate::conn::TargetAttachRollbackPlan::from_prepared_attach_session(prepared),
        )
    }

    fn rollback_attached_session_with_cleanup_without_event_sync(
        &mut self,
        rollback_plan: crate::conn::TargetAttachRollbackPlan,
    ) -> crate::conn::target::TargetEventPlan {
        if let Some(cleanup_plan) = rollback_plan.cleanup_plan() {
            if matches!(
                cleanup_plan.action(),
                crate::conn::TargetBindingCleanupAction::ActiveTargetPrimaryAutoAttached
            ) {
                debug_assert!(
                    false,
                    "active target rollback requires async binding cleanup"
                );
            } else {
                self.execute_target_binding_cleanup_without_event_sync(cleanup_plan);
            }
        }
        self.rollback_attached_session_without_event(rollback_plan.session_id())
    }

    fn execute_target_binding_cleanup_without_event_sync(
        &mut self,
        cleanup_plan: &crate::conn::TargetBindingCleanupPlan,
    ) {
        match cleanup_plan.action() {
            crate::conn::TargetBindingCleanupAction::BackgroundTargetPrimaryAutoAttached {
                ..
            } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.clear_background_target_primary_auto_attached_session(
                        cleanup_plan.session_id(),
                    );
                }
            }
            crate::conn::TargetBindingCleanupAction::AuxiliaryTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.remove_auxiliary_session(cleanup_plan.session_id());
                }
            }
            crate::conn::TargetBindingCleanupAction::TabTarget { .. } => {
                self.remove_tab_session(cleanup_plan.session_id());
            }
            crate::conn::TargetBindingCleanupAction::SharedWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_shared_worker_target_session(cleanup_plan.session_id());
                }
            }
            crate::conn::TargetBindingCleanupAction::DedicatedWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_dedicated_worker_target_session(cleanup_plan.session_id());
                }
            }
            crate::conn::TargetBindingCleanupAction::ServiceWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_service_worker_target_session(cleanup_plan.session_id());
                }
            }
            crate::conn::TargetBindingCleanupAction::None
            | crate::conn::TargetBindingCleanupAction::ActiveTargetPrimaryAutoAttached => {}
        }
    }

    async fn rollback_attached_session_with_cleanup_without_event_async(
        &mut self,
        rollback_plan: crate::conn::TargetAttachRollbackPlan,
    ) -> crate::conn::target::TargetEventPlan {
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
    ) -> crate::conn::TargetAutoAttachedSessionDetachPlan {
        crate::conn::TargetAutoAttachedSessionDetachPlan::from_session_route(
            session_id,
            self.session_route(Some(session_id)),
        )
    }

    pub(crate) fn rollback_auto_attached_session_detach_plan_without_event(
        &mut self,
        detach_plan: &crate::conn::TargetAutoAttachedSessionDetachPlan,
    ) -> crate::conn::target::TargetEventPlan {
        self.rollback_attached_session_without_event(detach_plan.session_id())
    }

    pub(crate) async fn execute_target_binding_cleanup_without_event_async(
        &mut self,
        cleanup_plan: &crate::conn::TargetBindingCleanupPlan,
    ) {
        match cleanup_plan.action() {
            crate::conn::TargetBindingCleanupAction::ActiveTargetPrimaryAutoAttached => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc
                        .clear_active_target_primary_auto_attached_session_async()
                        .await;
                }
            }
            crate::conn::TargetBindingCleanupAction::BackgroundTargetPrimaryAutoAttached {
                ..
            } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.clear_background_target_primary_auto_attached_session(
                        cleanup_plan.session_id(),
                    );
                }
            }
            crate::conn::TargetBindingCleanupAction::AuxiliaryTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.remove_auxiliary_session(cleanup_plan.session_id());
                }
            }
            crate::conn::TargetBindingCleanupAction::TabTarget { .. } => {
                self.remove_tab_session(cleanup_plan.session_id());
            }
            crate::conn::TargetBindingCleanupAction::SharedWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_shared_worker_target_session(cleanup_plan.session_id());
                }
            }
            crate::conn::TargetBindingCleanupAction::DedicatedWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_dedicated_worker_target_session(cleanup_plan.session_id());
                }
            }
            crate::conn::TargetBindingCleanupAction::ServiceWorkerTarget { .. } => {
                if let Some(bc) = self.browser_context.as_mut() {
                    let _ = bc.detach_service_worker_target_session(cleanup_plan.session_id());
                }
            }
            crate::conn::TargetBindingCleanupAction::None => {}
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
        let cleanup_plan = crate::conn::TargetBindingCleanupPlan::from_route(session_id, &route);
        self.execute_target_binding_cleanup_without_event_async(&cleanup_plan)
            .await;
        true
    }

    pub(crate) async fn detach_session_with_binding_cleanup_event_plan_async(
        &mut self,
        cleanup_plan: crate::conn::TargetSessionDetachCleanupPlan,
    ) -> crate::conn::target::TargetEventPlan {
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
        cleanup_plan: crate::conn::TargetSessionDetachCleanupPlan,
    ) -> crate::conn::target::TargetEventPlan {
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
        cleanup_plan: crate::conn::TargetSessionDetachCleanupPlan,
    ) -> crate::conn::target::TargetEventPlan {
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
        cleanup_plan: crate::conn::TargetClosureCleanupPlan,
        parent_session_id: Option<&str>,
    ) -> crate::conn::target::TargetEventPlan {
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        let session_ids = cleanup_plan
            .session_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut plan = crate::conn::target::TargetEventPlan::default();
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
        cleanup_plan: crate::conn::TargetSessionDetachCleanupPlan,
    ) -> Result<crate::conn::target::TargetEventPlan, String> {
        let session_id = cleanup_plan.session_id().to_owned();
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        let parent_session_id = cleanup_plan.parent_session_id().map(str::to_owned);
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
        cleanup_plan: crate::conn::TargetSessionDetachCleanupPlan,
    ) -> Result<Option<crate::conn::target::TargetEventPlan>, String> {
        let session_id = cleanup_plan.session_id().to_owned();
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        let parent_session_id = cleanup_plan.parent_session_id().map(str::to_owned);
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
    ) -> Vec<crate::conn::TargetSessionDetachCleanupPlan> {
        self.browser_context
            .as_ref()
            .map(|bc| {
                bc.background_targets
                    .iter()
                    .filter_map(|target| {
                        Some(crate::conn::TargetSessionDetachCleanupPlan::new(
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
    ) -> crate::conn::target::TargetEventPlan {
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
        let mut plan = crate::conn::target::TargetEventPlan::default();
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
    ) -> crate::conn::target::TargetEventPlan {
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
        let mut plan = crate::conn::target::TargetEventPlan::default();
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
    ) -> crate::conn::target::TargetEventPlan {
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
        let mut plan = crate::conn::target::TargetEventPlan::default();
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

    #[cfg(test)]
    fn target_id_for_auto_attached_session(&self, session_id: &str) -> Option<String> {
        match self.session_route(Some(session_id))? {
            CdpSessionRoute::TabTarget {
                tab_target_id: target_id,
                ..
            }
            | CdpSessionRoute::BackgroundTarget { target_id, .. }
            | CdpSessionRoute::AuxiliaryTarget { target_id, .. }
            | CdpSessionRoute::SharedWorkerTarget { target_id, .. }
            | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
            | CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => Some(target_id),
            CdpSessionRoute::ActiveTarget {
                target_id: Some(target_id),
                ..
            } => Some(target_id),
            CdpSessionRoute::ActiveTarget {
                target_id: None, ..
            } => self
                .target_owner_identity_for_session(Some(session_id))
                .and_then(|(_, target_id)| target_id),
            CdpSessionRoute::Browser => None,
        }
    }

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
        self.service_worker_auto_attach_related_owners.push(
            super::super::ServiceWorkerAutoAttachRelatedOwner {
                owner_session_id: owner_key,
                browser_context_id: browser_context_id.to_owned(),
                registration_id,
                base_version_id,
                script_url,
                scope_url,
                allow_service_worker_targets,
                wait_for_debugger_on_start,
            },
        );
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
    ) -> Vec<super::super::ServiceWorkerAutoAttachRelatedOwnerSession> {
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
            if !owners.iter().any(
                |existing: &super::super::ServiceWorkerAutoAttachRelatedOwnerSession| {
                    existing.owner_session_id == owner.owner_session_id
                },
            ) {
                owners.push(super::super::ServiceWorkerAutoAttachRelatedOwnerSession {
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

    pub(crate) fn shared_worker_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&SharedWorkerTargetState> {
        let session_id = session_id?;
        match self.session_route(Some(session_id))? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id(&browser_context_id)?
                .shared_worker_target(&target_id),
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id(&browser_context_id)?
                .dedicated_worker_target(&target_id)
                .map(|target| &target.inner),
            _ => None,
        }
    }

    /// Captures the exact renderer worker and protocol attachment addressed by
    /// `session_id`.
    ///
    /// The attachment scope lives with the SharedWorker target's per-session
    /// state. Holding this weak identity across a publication-capture boundary does
    /// not keep a normally detached session alive and cannot be rebound by a
    /// later current-session lookup.
    pub(crate) fn shared_worker_protocol_attachment_identity_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<crate::conn::TargetSharedWorkerProtocolAttachmentIdentity> {
        let session_id = session_id?;
        let CdpSessionRoute::SharedWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id(&browser_context_id)?
            .shared_worker_target(&target_id)?
            .protocol_attachment_identity(&browser_context_id, session_id)
    }

    pub(crate) fn shared_worker_target_for_session_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<&mut SharedWorkerTargetState> {
        let session_id = session_id?;
        match self.session_route(Some(session_id))? {
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id_mut(&browser_context_id)?
                .shared_worker_target_mut(&target_id),
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id_mut(&browser_context_id)?
                .dedicated_worker_target_mut(&target_id)
                .map(|target| &mut target.inner),
            _ => None,
        }
    }

    pub(crate) fn dedicated_worker_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&crate::conn::DedicatedWorkerTargetState> {
        let session_id = session_id?;
        let CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id(&browser_context_id)?
            .dedicated_worker_target(&target_id)
    }

    pub(crate) fn dedicated_worker_target_for_session_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<&mut crate::conn::DedicatedWorkerTargetState> {
        let session_id = session_id?;
        let CdpSessionRoute::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id_mut(&browser_context_id)?
            .dedicated_worker_target_mut(&target_id)
    }

    pub(crate) fn service_worker_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<&ServiceWorkerTargetState> {
        let session_id = session_id?;
        let CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id(&browser_context_id)?
            .service_worker_target(&target_id)
    }

    pub(crate) fn service_worker_target_for_session_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<&mut ServiceWorkerTargetState> {
        let session_id = session_id?;
        let CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } = self.session_route(Some(session_id))?
        else {
            return None;
        };
        self.browser_context_by_id_mut(&browser_context_id)?
            .service_worker_target_mut(&target_id)
    }

    pub(crate) fn with_background_target_session<R>(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut BrowserContext, &str) -> R,
    ) -> Option<R> {
        let (browser_context_id, target_id) = self.background_target_route(session_id)?;
        let browser_context = self.browser_context_by_id_mut(&browser_context_id)?;
        Some(f(browser_context, &target_id))
    }

    pub(crate) fn mutate_background_target_page_session_state(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut ParkedPageSessionState),
    ) -> bool {
        self.with_background_target_session(session_id, |browser_context, target_id| {
            browser_context.mutate_parked_page_session_state(target_id, f);
        })
        .is_some()
    }

    pub(super) fn target_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetSessionOwner> {
        let route = match session_id {
            Some(session_id) => self.session_route(Some(session_id))?,
            None => match self.none_session_owner_route_override() {
                Some(route) => route,
                None => {
                    let Some(browser_context_id) = self
                        .browser_context
                        .as_ref()
                        .map(|browser_context| browser_context.id.clone())
                    else {
                        return Some(TargetSessionOwner::NoLoadedBrowserContext);
                    };
                    return Some(TargetSessionOwner::ActiveTarget {
                        browser_context_id,
                        is_auxiliary_target_session: false,
                    });
                }
            },
        };

        match route {
            CdpSessionRoute::Browser => self
                .browser_context
                .as_ref()
                .map(|browser_context| TargetSessionOwner::ActiveTarget {
                    browser_context_id: browser_context.id.clone(),
                    is_auxiliary_target_session: false,
                })
                .or(Some(TargetSessionOwner::NoLoadedBrowserContext)),
            CdpSessionRoute::ActiveTarget {
                browser_context_id, ..
            } => Some(TargetSessionOwner::ActiveTarget {
                browser_context_id,
                is_auxiliary_target_session: false,
            }),
            CdpSessionRoute::AuxiliaryTarget {
                browser_context_id,
                target_id,
            } => {
                let is_background_target = self
                    .browser_context_by_id(&browser_context_id)?
                    .background_target(&target_id)
                    .is_some();
                if is_background_target {
                    Some(TargetSessionOwner::BackgroundTarget {
                        browser_context_id,
                        target_id,
                        is_auxiliary_target_session: true,
                    })
                } else {
                    Some(TargetSessionOwner::ActiveTarget {
                        browser_context_id,
                        is_auxiliary_target_session: true,
                    })
                }
            }
            CdpSessionRoute::BackgroundTarget {
                browser_context_id,
                target_id,
            } => Some(TargetSessionOwner::BackgroundTarget {
                browser_context_id,
                target_id,
                is_auxiliary_target_session: false,
            }),
            CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => {
                Some(TargetSessionOwner::NoLoadedBrowserContext)
            }
        }
    }
}

fn browser_context_session_route(
    browser_context: &BrowserContext,
    session_id: &str,
) -> Option<CdpSessionRoute> {
    if browser_context.active_session_id() == Some(session_id) {
        return Some(CdpSessionRoute::ActiveTarget {
            browser_context_id: browser_context.id.clone(),
            target_id: browser_context.active_target_id().map(str::to_owned),
        });
    }

    if let Some(target_id) = browser_context.auxiliary_target_id_for_session(session_id) {
        return Some(CdpSessionRoute::AuxiliaryTarget {
            browser_context_id: browser_context.id.clone(),
            target_id: target_id.to_owned(),
        });
    }

    browser_context
        .background_targets
        .iter()
        .find(|target| target.is_session(session_id))
        .map(|target| CdpSessionRoute::BackgroundTarget {
            browser_context_id: browser_context.id.clone(),
            target_id: target.target_id().to_owned(),
        })
        .or_else(|| {
            browser_context
                .shared_worker_target_id_for_session(session_id)
                .map(|target_id| CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                })
                .or_else(|| {
                    browser_context
                        .dedicated_worker_target_id_for_session(session_id)
                        .map(|target_id| CdpSessionRoute::DedicatedWorkerTarget {
                            browser_context_id: browser_context.id.clone(),
                            target_id: target_id.to_owned(),
                        })
                        .or_else(|| {
                            browser_context
                                .service_worker_target_id_for_session(session_id)
                                .map(|target_id| CdpSessionRoute::ServiceWorkerTarget {
                                    browser_context_id: browser_context.id.clone(),
                                    target_id: target_id.to_owned(),
                                })
                        })
                })
        })
}
