use crate::conn::CdpConnection;
use crate::devtools_runtime::DevToolsTargetInfo;

use super::{
    CdpSessionRoute, PreparedTargetAttach, SessionDisposalPlan, SessionDisposalTarget,
    TargetAttachRollbackPlan, TargetAttachSessionCommit, TargetAutoAttachedSessionDetachPlan,
    TargetClosureCleanupPlan, TargetEventPlan, TargetSessionDetachCleanupPlan,
};

impl CdpConnection {
    pub(crate) async fn clear_devtools_network_session_policy_async(
        &mut self,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let Some(CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id,
            session_key,
        }) = self.session_route(Some(session_id))
        else {
            return Ok(());
        };

        let Some(browser_context) = self.browser_context_by_id_mut(&browser_context_id) else {
            return Ok(());
        };
        browser_context
            .clear_devtools_network_session_policy_async(&target_id, &session_key)
            .await
    }

    pub(crate) async fn clear_devtools_emulation_session_policy_async(
        &mut self,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let Some(CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id,
            session_key,
        }) = self.session_route(Some(session_id))
        else {
            return Ok(());
        };

        let browser_identity_changed = match self.browser_context_by_id_mut(&browser_context_id) {
            Some(browser_context) => {
                browser_context
                    .clear_devtools_emulation_session_policy_async(&target_id, &session_key)
                    .await?
            }
            None => false,
        };
        if !browser_identity_changed {
            return Ok(());
        }

        let Some(pending) = self
            .start_rebuild_resource_runtime_for_session_owner(Some(session_id))
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(());
        };
        let completion = pending.wait().await.map_err(|error| {
            anyhow::anyhow!("failed to restore detached session user agent: {error}")
        })?;
        self.finish_rebuild_resource_runtime_for_session_owner(Some(session_id), completion)
            .map_err(anyhow::Error::msg)
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

    pub(crate) fn commit_browser_session_disposal_without_event(
        &mut self,
        plan: &SessionDisposalPlan,
    ) -> anyhow::Result<TargetEventPlan> {
        anyhow::ensure!(
            matches!(plan.target(), SessionDisposalTarget::Browser)
                && self.is_browser_session_id(Some(plan.session_id())),
            "InvalidSessionId"
        );
        Ok(self.rollback_attached_session_without_event(plan.session_id()))
    }

    pub(crate) fn commit_browser_session_disposal_event_plan(
        &mut self,
        plan: &SessionDisposalPlan,
    ) -> anyhow::Result<TargetEventPlan> {
        anyhow::ensure!(
            matches!(plan.target(), SessionDisposalTarget::Browser)
                && self.is_browser_session_id(Some(plan.session_id())),
            "InvalidSessionId"
        );
        let owner_session_id = self
            .target_control
            .attached_session_owner_session_id(plan.session_id())
            .map(str::to_owned);
        let session_id = plan.session_id().to_owned();
        let event_plan = self
            .target_control
            .detach_attached_session_event_plan(
                plan.session_id(),
                None,
                owner_session_id.as_deref(),
            )
            .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
        self.remove_detached_session_control_owner(&session_id);
        Ok(event_plan)
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
        let Some(browser_context_id) = self
            .session_route(Some(session_id))
            .and_then(|route| route.browser_context_id().map(str::to_owned))
        else {
            return false;
        };
        let released = self
            .browser_context_by_id_mut(&browser_context_id)
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
        let route = self.bound_session_route_for_test(&session_id, None);
        let target_id = route.as_ref().and_then(|route| match route {
            CdpSessionRoute::TabTarget { tab_target_id, .. } => Some(tab_target_id.clone()),
            CdpSessionRoute::PageTarget { target_id, .. }
            | CdpSessionRoute::SharedWorkerTarget { target_id, .. }
            | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
            | CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => Some(target_id.clone()),
            CdpSessionRoute::Browser | CdpSessionRoute::BrowserContext { .. } => None,
        });
        self.target_control.commit_auto_attached_session_for_target(
            session_id,
            owner_session_id,
            target_id.as_deref(),
            route,
            false,
        );
    }

    #[cfg(test)]
    pub(crate) fn register_auto_attached_session_for_target(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: Option<&str>,
    ) {
        let route = self.bound_session_route_for_test(&session_id, target_id);
        self.target_control.commit_auto_attached_session_for_target(
            session_id,
            owner_session_id,
            target_id,
            route,
            false,
        );
    }

    #[cfg(test)]
    pub(crate) fn register_bound_session_for_test(&mut self, session_id: &str) {
        let route = self
            .bound_session_route_for_test(session_id, None)
            .unwrap_or_else(|| panic!("test session {session_id} must own a concrete route"));
        let target_id = match &route {
            CdpSessionRoute::TabTarget { tab_target_id, .. } => tab_target_id.as_str(),
            CdpSessionRoute::PageTarget { target_id, .. }
            | CdpSessionRoute::SharedWorkerTarget { target_id, .. }
            | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
            | CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => target_id.as_str(),
            CdpSessionRoute::Browser => "browser",
            CdpSessionRoute::BrowserContext { browser_context_id } => browser_context_id.as_str(),
        };
        self.target_control.commit_attached_session(
            session_id.to_owned(),
            None,
            target_id,
            Some(route.clone()),
            false,
            false,
        );
    }

    #[cfg(test)]
    pub(crate) fn bound_session_route_for_test(
        &self,
        session_id: &str,
        wanted_target_id: Option<&str>,
    ) -> Option<CdpSessionRoute> {
        if let Some(tab_target_id) = self.tab_target_id_for_session_id(session_id)
            && wanted_target_id.is_none_or(|target_id| target_id == tab_target_id)
        {
            return Some(CdpSessionRoute::TabTarget {
                browser_context_id: self.browser_context_id_for_tab_target_id(tab_target_id)?,
                tab_target_id: tab_target_id.to_owned(),
            });
        }
        self.browser_contexts().find_map(|browser_context| {
            if let Some((target, session_key)) =
                browser_context.page_targets.iter().find_map(|target| {
                    (wanted_target_id.is_none_or(|target_id| target_id == target.target_id()))
                        .then(|| {
                            target
                                .devtools_sessions
                                .key_for_wire_session_id(session_id)
                                .map(|session_key| (target, session_key))
                        })
                        .flatten()
                })
            {
                return Some(CdpSessionRoute::PageTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target.target_id().to_owned(),
                    session_key,
                });
            }
            browser_context
                .shared_worker_targets
                .values()
                .find(|target| {
                    target.is_session(session_id)
                        && wanted_target_id.is_none_or(|wanted| wanted == target.target_id)
                })
                .map(|target| CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target.target_id.clone(),
                })
                .or_else(|| {
                    browser_context
                        .dedicated_worker_targets
                        .values()
                        .find(|target| {
                            target.is_session(session_id)
                                && wanted_target_id.is_none_or(|wanted| wanted == target.target_id)
                        })
                        .map(|target| CdpSessionRoute::DedicatedWorkerTarget {
                            browser_context_id: browser_context.id.clone(),
                            target_id: target.target_id.clone(),
                        })
                })
                .or_else(|| {
                    browser_context
                        .service_worker_targets
                        .values()
                        .find(|target| {
                            target.is_session(session_id)
                                && wanted_target_id.is_none_or(|wanted| wanted == target.target_id)
                        })
                        .map(|target| CdpSessionRoute::ServiceWorkerTarget {
                            browser_context_id: browser_context.id.clone(),
                            target_id: target.target_id.clone(),
                        })
                })
        })
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
                Some(route),
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

    pub(crate) fn attach_tab_target_session_event_plan(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        tab_target_id: &str,
        is_attached_session: bool,
    ) -> Result<TargetEventPlan, &'static str> {
        let Some(browser_context_id) = self.browser_context_id_for_tab_target_id(tab_target_id)
        else {
            return Err("UnknownTargetId");
        };
        if !self.assign_session_to_tab_target(
            tab_target_id,
            session_id.clone(),
            is_attached_session,
        ) {
            return Err("UnknownTargetId");
        }
        let prepared_session = TargetAttachSessionCommit::direct(
            session_id,
            owner_session_id.map(str::to_owned),
            CdpSessionRoute::TabTarget {
                browser_context_id,
                tab_target_id: tab_target_id.to_owned(),
            },
            false,
        );
        let Some(target_info) = self.tab_target_info(tab_target_id) else {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("UnknownTargetId");
        };
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
        let (browser_context_id, target_info) = {
            let Some(bc) = self.browser_context.as_mut() else {
                return Err("BrowserContextNotLoaded");
            };
            if !bc.assign_session_to_shared_worker_target(target_id, session_id_for_binding) {
                return Err("UnknownTargetId");
            }
            (bc.id.clone(), bc.devtools_target_info(target_id))
        };
        let prepared_session = TargetAttachSessionCommit::direct(
            session_id,
            owner_session_id.map(str::to_owned),
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id: target_id.to_owned(),
            },
            false,
        );
        let Some(target_info) = target_info else {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("UnknownTargetId");
        };
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
        let (browser_context_id, target_info) = {
            let Some(bc) = self.browser_context.as_mut() else {
                return Err("BrowserContextNotLoaded");
            };
            if !bc.assign_session_to_service_worker_target(target_id, session_id_for_binding) {
                return Err("UnknownTargetId");
            }
            (bc.id.clone(), bc.devtools_target_info(target_id))
        };
        let prepared_session = TargetAttachSessionCommit::direct(
            session_id,
            owner_session_id.map(str::to_owned),
            CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id: target_id.to_owned(),
            },
            false,
        );
        let Some(target_info) = target_info else {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("UnknownTargetId");
        };
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
        let (browser_context_id, target_info) = {
            let Some(bc) = self.browser_context.as_mut() else {
                return Err("BrowserContextNotLoaded");
            };
            if !bc.assign_session_to_dedicated_worker_target(target_id, session_id_for_binding) {
                return Err("UnknownTargetId");
            }
            (bc.id.clone(), bc.devtools_target_info(target_id))
        };
        let prepared_session = TargetAttachSessionCommit::direct(
            session_id,
            owner_session_id.map(str::to_owned),
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id: target_id.to_owned(),
            },
            false,
        );
        let Some(target_info) = target_info else {
            self.rollback_prepared_attach_session_sync_without_event(&prepared_session);
            return Err("UnknownTargetId");
        };
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
    ) -> Option<CdpSessionRoute> {
        let browser_context_id = self.browser_context_id_for_tab_target_id(tab_target_id)?;
        self.assign_session_to_tab_target(tab_target_id, session_id, owner_session_id.is_some())
            .then(|| CdpSessionRoute::TabTarget {
                browser_context_id,
                tab_target_id: tab_target_id.to_owned(),
            })
    }

    pub(crate) fn prepare_auto_attached_page_session_binding(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> Option<CdpSessionRoute> {
        let browser_context = self.browser_context.as_mut()?;
        let browser_context_id = browser_context.id.clone();
        if !browser_context.assign_auto_attached_session_to_target(target_id, session_id.clone()) {
            return None;
        }
        let session_key = browser_context
            .page_target(target_id)?
            .devtools_sessions
            .key_for_wire_session_id(&session_id)?;
        Some(CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id: target_id.to_owned(),
            session_key,
        })
    }

    pub(crate) fn prepare_auto_attached_page_session_binding_in_browser_context(
        &mut self,
        browser_context_id: &str,
        target_id: &str,
        session_id: String,
    ) -> Option<CdpSessionRoute> {
        let browser_context = self.browser_context_by_id_mut(browser_context_id)?;
        if !browser_context.assign_auto_attached_session_to_target(target_id, session_id.clone()) {
            return None;
        }
        let session_key = browser_context
            .page_target(target_id)?
            .devtools_sessions
            .key_for_wire_session_id(&session_id)?;
        Some(CdpSessionRoute::PageTarget {
            browser_context_id: browser_context_id.to_owned(),
            target_id: target_id.to_owned(),
            session_key,
        })
    }

    pub(crate) fn prepare_auto_attached_shared_worker_session_binding(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> Option<CdpSessionRoute> {
        let browser_context = self.browser_context.as_mut()?;
        let browser_context_id = browser_context.id.clone();
        browser_context
            .assign_session_to_shared_worker_target(target_id, session_id)
            .then(|| CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id: target_id.to_owned(),
            })
    }

    pub(crate) fn prepare_auto_attached_dedicated_worker_session_binding(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> Option<CdpSessionRoute> {
        let browser_context = self.browser_context.as_mut()?;
        let browser_context_id = browser_context.id.clone();
        browser_context
            .assign_session_to_dedicated_worker_target(target_id, session_id)
            .then(|| CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id: target_id.to_owned(),
            })
    }

    pub(crate) fn prepare_auto_attached_service_worker_session_binding(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> Option<CdpSessionRoute> {
        let browser_context = self.browser_context.as_mut()?;
        let browser_context_id = browser_context.id.clone();
        browser_context
            .assign_session_to_service_worker_target(target_id, session_id)
            .then(|| CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id: target_id.to_owned(),
            })
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
            self.remove_detached_session_control_owner(session_id);
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
        self.remove_detached_session_control_owner(session_id);
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

    pub(crate) async fn dispose_target_closure_sessions_event_plan_async(
        &mut self,
        cleanup_plan: TargetClosureCleanupPlan,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let session_ids = cleanup_plan
            .session_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for session_id in session_ids {
            let Some(route) = self.session_route(Some(&session_id)) else {
                tracing::warn!(
                    session_id,
                    "closed target session no longer has an authoritative route"
                );
                continue;
            };
            let Some(disposal_plan) = SessionDisposalPlan::for_session_route(&session_id, &route)
            else {
                tracing::warn!(
                    session_id,
                    "closed target session does not support target disposal"
                );
                continue;
            };
            crate::domains::target::dispose_closed_session_domains_async(self, &disposal_plan)
                .await;
        }
        self.commit_target_closure_session_detachment_events(cleanup_plan, parent_session_id)
    }

    fn commit_target_closure_session_detachment_events(
        &mut self,
        cleanup_plan: TargetClosureCleanupPlan,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let plan = self
            .target_control
            .detach_target_closure_cleanup_event_plan(cleanup_plan, parent_session_id);
        for session in plan.detached_sessions() {
            self.remove_detached_session_control_owner(session.session_id());
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
                cleanup_plan.target(),
                SessionDisposalTarget::PageTarget {
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                    ..
                }
            ) {
                debug_assert!(
                    false,
                    "primary Page target rollback requires async binding cleanup"
                );
            } else {
                self.rollback_prepared_session_binding_sync(cleanup_plan);
            }
        }
        self.rollback_attached_session_without_event(rollback_plan.session_id())
    }

    /// Removes a prepared binding before any asynchronous domain work has
    /// started. Once a prepared session can own renderer resources, callers
    /// must use the asynchronous SessionDisposalPlan executor instead.
    fn rollback_prepared_session_binding_sync(&mut self, cleanup_plan: &SessionDisposalPlan) {
        match cleanup_plan.target() {
            SessionDisposalTarget::PageTarget {
                target_id,
                session_key: session_key @ moli_page_types::DevToolsSessionKey::Attached(_),
                ..
            } => {
                if let Some(bc) = self.session_disposal_browser_context_mut(cleanup_plan) {
                    let _ = bc.remove_page_session_binding(
                        target_id,
                        cleanup_plan.session_id(),
                        session_key,
                    );
                }
            }
            SessionDisposalTarget::Browser => {}
            SessionDisposalTarget::TabTarget { .. } => {
                self.remove_tab_session(cleanup_plan.session_id());
            }
            SessionDisposalTarget::SharedWorkerTarget { .. } => {
                if let Some(bc) = self.session_disposal_browser_context_mut(cleanup_plan) {
                    let _ = bc.detach_shared_worker_target_session(cleanup_plan.session_id());
                }
            }
            SessionDisposalTarget::DedicatedWorkerTarget { .. } => {
                if let Some(bc) = self.session_disposal_browser_context_mut(cleanup_plan) {
                    let _ = bc.detach_dedicated_worker_target_session(cleanup_plan.session_id());
                }
            }
            SessionDisposalTarget::ServiceWorkerTarget { .. } => {
                if let Some(bc) = self.session_disposal_browser_context_mut(cleanup_plan) {
                    let _ = bc.detach_service_worker_target_session(cleanup_plan.session_id());
                }
            }
            SessionDisposalTarget::PageTarget {
                session_key: moli_page_types::DevToolsSessionKey::Primary,
                ..
            } => {}
        }
    }

    async fn rollback_attached_session_with_cleanup_without_event_async(
        &mut self,
        rollback_plan: TargetAttachRollbackPlan,
    ) -> TargetEventPlan {
        if let Some(cleanup_plan) = rollback_plan.cleanup_plan()
            && let Err(error) =
                crate::domains::target::dispose_uncommitted_session_async(self, cleanup_plan).await
        {
            tracing::warn!(
                session_id = rollback_plan.session_id(),
                %error,
                "failed to clean prepared target binding during attach rollback"
            );
            // Keep both the domain binding and its control-plane route as
            // retry authority. Dropping only the latter would make any
            // renderer-owned state unreachable.
            return TargetEventPlan::default();
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

    pub(crate) fn commit_session_disposal(
        &mut self,
        cleanup_plan: &SessionDisposalPlan,
    ) -> anyhow::Result<()> {
        match cleanup_plan.target() {
            SessionDisposalTarget::Browser => anyhow::bail!("InvalidSessionId"),
            SessionDisposalTarget::PageTarget {
                browser_context_id,
                target_id,
                session_key,
            } => {
                let bc = self
                    .browser_context_by_id_mut(browser_context_id)
                    .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
                anyhow::ensure!(
                    bc.remove_page_session_binding(
                        target_id,
                        cleanup_plan.session_id(),
                        session_key,
                    ),
                    "InvalidSessionId"
                );
            }
            SessionDisposalTarget::TabTarget { tab_target_id, .. } => {
                let removed_target_id = self
                    .remove_tab_session(cleanup_plan.session_id())
                    .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
                anyhow::ensure!(removed_target_id == *tab_target_id, "UnknownTargetId");
            }
            SessionDisposalTarget::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let bc = self
                    .browser_context_by_id_mut(browser_context_id)
                    .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
                let removed_target_id = bc
                    .detach_shared_worker_target_session(cleanup_plan.session_id())
                    .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
                anyhow::ensure!(removed_target_id == *target_id, "UnknownTargetId");
            }
            SessionDisposalTarget::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let bc = self
                    .browser_context_by_id_mut(browser_context_id)
                    .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
                let removed_target_id = bc
                    .detach_dedicated_worker_target_session(cleanup_plan.session_id())
                    .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
                anyhow::ensure!(removed_target_id == *target_id, "UnknownTargetId");
            }
            SessionDisposalTarget::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let bc = self
                    .browser_context_by_id_mut(browser_context_id)
                    .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
                let removed_target_id = bc
                    .detach_service_worker_target_session(cleanup_plan.session_id())
                    .ok_or_else(|| anyhow::anyhow!("InvalidSessionId"))?;
                anyhow::ensure!(removed_target_id == *target_id, "UnknownTargetId");
            }
        }
        Ok(())
    }

    fn session_disposal_browser_context_mut(
        &mut self,
        cleanup_plan: &SessionDisposalPlan,
    ) -> Option<&mut crate::conn::BrowserContext> {
        let browser_context_id = cleanup_plan.browser_context_id()?;
        self.browser_context_by_id_mut(browser_context_id)
    }

    pub(crate) fn commit_target_session_detachment_event_plan(
        &mut self,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    ) -> TargetEventPlan {
        self.commit_target_session_detachment_event_plan_inner(cleanup_plan, true)
    }

    pub(crate) fn commit_target_session_detachment_after_prepared_state_delta_event_plan(
        &mut self,
        cleanup_plan: TargetSessionDetachCleanupPlan,
    ) -> TargetEventPlan {
        self.commit_target_session_detachment_event_plan_inner(cleanup_plan, false)
    }

    fn commit_target_session_detachment_event_plan_inner(
        &mut self,
        cleanup_plan: TargetSessionDetachCleanupPlan,
        emit_attached_state_delta: bool,
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
        if emit_attached_state_delta {
            self.detach_known_session_event_plan(
                &target_id,
                &session_id,
                reason.as_deref(),
                parent_session_id.as_deref(),
            )
        } else {
            self.detach_known_session_event_plan_with_attached_state_delta(
                &target_id,
                &session_id,
                reason.as_deref(),
                parent_session_id.as_deref(),
                false,
            )
        }
    }

    fn remove_detached_session_control_owner(&mut self, session_id: &str) {
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
