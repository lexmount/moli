use crate::domains::target::session_disposal::SessionDisposalError;
use anyhow::Result;

use crate::automation::DevToolsTargetInfo;
#[cfg(test)]
use crate::conn::BrowserContext;
use crate::conn::CdpConnection;

use super::{
    CdpSessionRoute, PreparedTargetAttach, SessionDisposalPlan, SessionDisposalTarget,
    TargetAttachRollbackPlan, TargetAttachSessionCommit, TargetAutoAttachedSessionDetachPlan,
    TargetClosureCleanupPlan, TargetEventPlan, TargetSessionDetachCleanupPlan,
};

impl CdpConnection {
    pub(crate) fn session_binding_lifetime(
        &self,
        session_id: &str,
    ) -> Option<super::SessionDisposalWait> {
        self.target_control.session_binding_lifetime(session_id)
    }

    pub(crate) fn session_disposal_wait(
        &self,
        session_id: &str,
    ) -> Option<super::SessionDisposalWait> {
        self.target_control.session_disposal_wait(session_id)
    }

    pub(crate) fn mark_session_closing(&mut self, plan: &SessionDisposalPlan) {
        self.target_control.mark_session_closing(plan.session_id());
        let owner = crate::conn::CommandOwnerScope::for_session(plan.session_id());
        let _ = self.with_target_devtools_session_state_for_owner_mut(&owner, |state| {
            state.closing = true;
        });
    }

    pub fn session_is_closing(&self, session_id: &str) -> bool {
        self.target_control.session_is_closing(session_id)
            || self
                .session_route(Some(session_id))
                .as_ref()
                .and_then(|route| route.target_id())
                .is_some_and(|target| {
                    let page = self
                        .primary_page_target_id_for_tab_target_id(target)
                        .unwrap_or(target);
                    self.page_target_is_closing(page)
                })
    }

    /// Commits target-side session declarations built by low-level test
    /// fixtures into the same control-plane registry used by production
    /// attachment transactions.
    ///
    /// Older tests construct `BrowserContext` and worker targets directly,
    /// before either object is owned by a `CdpConnection`. The test harness
    /// calls this once at its fixture boundary. Runtime route lookup remains
    /// strict: it never searches target state when the committed registry has
    /// no entry.
    #[cfg(test)]
    pub(crate) fn commit_declared_session_fixtures_for_test(&mut self) {
        let mut declared = Vec::new();
        for browser_context in self.browser_contexts() {
            let browser_context_id = browser_context.id.clone();
            for target in browser_context.page_targets.iter() {
                let target_id = target.target_id().to_owned();
                if let Some(session_id) = target.devtools_sessions.primary_session_id() {
                    declared.push((
                        session_id.to_owned(),
                        target_id.clone(),
                        CdpSessionRoute::PageTarget {
                            browser_context_id: browser_context_id.clone(),
                            target_id: target_id.clone(),
                            session_key: moli_page_types::DevToolsSessionKey::Primary,
                        },
                    ));
                }
                for session_id in target.devtools_sessions.attached_session_ids() {
                    declared.push((
                        session_id.to_owned(),
                        target_id.clone(),
                        CdpSessionRoute::PageTarget {
                            browser_context_id: browser_context_id.clone(),
                            target_id: target_id.clone(),
                            session_key: moli_page_types::DevToolsSessionKey::Attached(
                                session_id.to_owned(),
                            ),
                        },
                    ));
                }
            }
            for target in browser_context.shared_worker_targets.values() {
                for session_id in target.session_ids() {
                    declared.push((
                        session_id,
                        target.target_id.clone(),
                        CdpSessionRoute::SharedWorkerTarget {
                            browser_context_id: browser_context_id.clone(),
                            target_id: target.target_id.clone(),
                        },
                    ));
                }
            }
            for target in browser_context.dedicated_worker_targets.values() {
                for session_id in target.inner.session_ids() {
                    declared.push((
                        session_id,
                        target.inner.target_id.clone(),
                        CdpSessionRoute::DedicatedWorkerTarget {
                            browser_context_id: browser_context_id.clone(),
                            target_id: target.inner.target_id.clone(),
                        },
                    ));
                }
            }
            for target in browser_context.service_worker_targets.values() {
                for session_id in target.session_ids() {
                    declared.push((
                        session_id,
                        target.target_id.clone(),
                        CdpSessionRoute::ServiceWorkerTarget {
                            browser_context_id: browser_context_id.clone(),
                            target_id: target.target_id.clone(),
                        },
                    ));
                }
            }
        }

        for (session_id, target_id, route) in declared {
            if self
                .target_control
                .attached_session_route(&session_id)
                .is_some()
            {
                continue;
            }
            let attachment =
                self.worker_protocol_attachment_identity_for_route(&route, &session_id);
            self.target_control.commit_attached_session(
                session_id.clone(),
                None,
                &target_id,
                route,
                false,
                false,
            );
            if let Some(attachment) = attachment {
                self.target_control
                    .bind_worker_attachment(&session_id, attachment);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn install_browser_context_fixture_for_test(
        &mut self,
        browser_context: BrowserContext,
    ) {
        self.browser_context = Some(browser_context);
        self.commit_declared_session_fixtures_for_test();
    }

    #[cfg(test)]
    pub(crate) fn push_inactive_browser_context_fixture_for_test(
        &mut self,
        browser_context: BrowserContext,
    ) {
        self.inactive_browser_contexts.push(browser_context);
        self.commit_declared_session_fixtures_for_test();
    }

    #[cfg(test)]
    pub(crate) async fn clear_devtools_network_session_policy_async(
        &mut self,
        session_id: &str,
    ) -> Result<()> {
        let owner = super::super::CommandOwnerScope::capture(self, Some(session_id));
        self.start_clear_devtools_network_session_policy(session_id)
            .wait()
            .await
            .finish(self.loaded_page_mut_for_owner(&owner))
            .into_result()
    }

    pub(crate) fn start_clear_devtools_network_session_policy(
        &mut self,
        session_id: &str,
    ) -> super::super::SessionPageUpdates {
        let Some(CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id,
            session_key,
        }) = self.session_route(Some(session_id))
        else {
            return Default::default();
        };
        self.browser_context_by_id_mut(&browser_context_id)
            .map(|context| {
                context.start_clear_devtools_network_session_policy(&target_id, &session_key)
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) async fn clear_devtools_emulation_session_policy_async(
        &mut self,
        session_id: &str,
    ) -> Result<()> {
        let owner = super::super::CommandOwnerScope::capture(self, Some(session_id));
        self.start_clear_devtools_emulation_session_policy(session_id)
            .wait()
            .await
            .finish(self.loaded_page_mut_for_owner(&owner))
            .into_result()
    }

    pub(crate) fn start_clear_devtools_emulation_session_policy(
        &mut self,
        session_id: &str,
    ) -> super::super::SessionPageUpdates {
        let Some(CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id,
            session_key,
        }) = self.session_route(Some(session_id))
        else {
            return Default::default();
        };
        let (identity_changed, mut updates) = self
            .browser_context_by_id_mut(&browser_context_id)
            .map(|context| {
                context.start_clear_devtools_emulation_session_policy(&target_id, &session_key)
            })
            .unwrap_or_default();
        if identity_changed {
            match self.start_rebuild_resource_runtime_for_session_owner(Some(session_id)) {
                Ok(Some(pending)) => {
                    if let Some(page) = self.loaded_page_mut_for_session_owner(Some(session_id)) {
                        updates.push(
                            page,
                            "failed to restore detached session user agent",
                            Ok(pending),
                        );
                    }
                }
                Err(error) => updates.record_error(anyhow::Error::msg(error)),
                Ok(None) => {}
            }
        }
        updates
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
            CdpSessionRoute::Browser,
            false,
            false,
        );
    }

    pub(crate) fn commit_browser_session_disposal_without_event(
        &mut self,
        plan: &SessionDisposalPlan,
    ) -> Result<TargetEventPlan> {
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
    ) -> Result<TargetEventPlan> {
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
    pub(crate) fn register_auto_attached_session_route_for_test(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        route: CdpSessionRoute,
    ) {
        let target_id = route.target_id().unwrap_or_else(|| {
            panic!("test auto-attached session route must identify a target: {route:?}")
        });
        let attachment = self.worker_protocol_attachment_identity_for_route(&route, &session_id);
        let binding_session = session_id.clone();
        self.target_control.commit_auto_attached_session_for_target(
            session_id,
            owner_session_id,
            target_id,
            route.clone(),
            false,
        );
        if let Some(attachment) = attachment {
            self.target_control
                .bind_worker_attachment(&binding_session, attachment);
        }
    }

    #[cfg(test)]
    pub(crate) fn mark_session_auto_attached_for_test(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
    ) {
        let route = self
            .session_route(Some(&session_id))
            .unwrap_or_else(|| panic!("test session {session_id} must have a committed route"));
        self.register_auto_attached_session_route_for_test(session_id, owner_session_id, route);
    }

    #[cfg(test)]
    pub(crate) fn register_session_route_for_test(
        &mut self,
        session_id: &str,
        route: CdpSessionRoute,
    ) {
        let target_id = route.target_id().unwrap_or("browser").to_owned();
        let attachment = self.worker_protocol_attachment_identity_for_route(&route, session_id);
        self.target_control.commit_attached_session(
            session_id.to_owned(),
            None,
            &target_id,
            route,
            false,
            false,
        );
        if let Some(attachment) = attachment {
            self.target_control
                .bind_worker_attachment(session_id, attachment);
        }
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
        let mut attached_state_delta_plan = None;
        let mut plan = TargetEventPlan::default();
        for session in sessions {
            let worker_attachment = session.worker_attachment().cloned();
            assert!(
                worker_attachment.is_some()
                    || !matches!(
                        session.route(),
                        CdpSessionRoute::SharedWorkerTarget { .. }
                            | CdpSessionRoute::DedicatedWorkerTarget { .. }
                            | CdpSessionRoute::ServiceWorkerTarget { .. }
                    ),
                "worker attach must freeze its original native attachment before waiting"
            );
            if worker_attachment
                .as_ref()
                .is_some_and(|attachment| !attachment.is_current())
            {
                continue;
            }
            if emit_attached_state_delta && attached_state_delta_plan.is_none() {
                attached_state_delta_plan =
                    Some(self.exact_target_info_changed_event_plan_for_target_delta(&target_id));
            }
            let (session_id, owner_session_id, route, auto_attached, waiting_for_debugger) =
                session.into_parts();
            let binding_session_id = session_id.clone();
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
            if let Some(attachment) = worker_attachment {
                self.target_control
                    .bind_worker_attachment(&binding_session_id, attachment);
            }
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
        let attachment = self
            .worker_protocol_attachment_identity_for_route(
                prepared_session.route(),
                prepared_session.session_id(),
            )
            .expect("synchronous worker attachment retains its original native scope");
        Ok(self.commit_prepared_attach_event_plan(
            PreparedTargetAttach::new(target_id, target_info, [prepared_session])
                .with_worker_attachment(attachment),
        ))
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
        let attachment = self
            .worker_protocol_attachment_identity_for_route(
                prepared_session.route(),
                prepared_session.session_id(),
            )
            .expect("synchronous worker attachment retains its original native scope");
        Ok(self.commit_prepared_attach_event_plan(
            PreparedTargetAttach::new(target_id, target_info, [prepared_session])
                .with_worker_attachment(attachment),
        ))
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
        let attachment = self
            .worker_protocol_attachment_identity_for_route(
                prepared_session.route(),
                prepared_session.session_id(),
            )
            .expect("synchronous worker attachment retains its original native scope");
        Ok(self.commit_prepared_dedicated_worker_attach_event_plan(
            PreparedTargetAttach::new(target_id, target_info, [prepared_session])
                .with_worker_attachment(attachment),
        ))
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
            CdpSessionRoute::Browser,
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
        crate::domains::target::session_disposal::dispose_closed_target_sessions_using(
            &mut crate::conn::ProtocolWorkflowDriver::inline(self),
            cleanup_plan,
            parent_session_id.map(str::to_owned),
        )
        .await
        .expect("inline closed-session disposal retains its protocol state")
    }

    pub(crate) fn commit_target_closure_session_detachment_events(
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
        match crate::domains::target::rollback_attachment(
            &mut crate::conn::ProtocolWorkflowDriver::inline(self),
            prepared.clone(),
        )
        .await
        {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(session_id = prepared.session_id(), %error,
                    "failed to clean prepared target binding during attach rollback");
                TargetEventPlan::default()
            }
        }
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

    pub(crate) fn auto_attached_session_detach_plan(
        &self,
        session_id: &str,
    ) -> TargetAutoAttachedSessionDetachPlan {
        let route = self.session_route(Some(session_id)).unwrap_or_else(|| {
            panic!("committed auto-attached session {session_id} must retain its route")
        });
        TargetAutoAttachedSessionDetachPlan::from_session_route(session_id, route)
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
    ) -> Result<()> {
        match cleanup_plan.target() {
            SessionDisposalTarget::Browser => anyhow::bail!(SessionDisposalError::InvalidSession),
            SessionDisposalTarget::PageTarget {
                browser_context_id,
                target_id,
                session_key,
            } => {
                let bc = self
                    .browser_context_by_id_mut(browser_context_id)
                    .ok_or(SessionDisposalError::InvalidSession)?;
                anyhow::ensure!(
                    bc.remove_page_session_binding(
                        target_id,
                        cleanup_plan.session_id(),
                        session_key,
                    ),
                    SessionDisposalError::InvalidSession
                );
            }
            SessionDisposalTarget::TabTarget { tab_target_id, .. } => {
                let removed_target_id = self
                    .remove_tab_session(cleanup_plan.session_id())
                    .ok_or(SessionDisposalError::InvalidSession)?;
                anyhow::ensure!(
                    removed_target_id == *tab_target_id,
                    SessionDisposalError::UnknownTarget
                );
            }
            SessionDisposalTarget::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let bc = self
                    .browser_context_by_id_mut(browser_context_id)
                    .ok_or(SessionDisposalError::InvalidSession)?;
                let removed_target_id = bc
                    .detach_shared_worker_target_session(cleanup_plan.session_id())
                    .ok_or(SessionDisposalError::InvalidSession)?;
                anyhow::ensure!(
                    removed_target_id == *target_id,
                    SessionDisposalError::UnknownTarget
                );
            }
            SessionDisposalTarget::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let bc = self
                    .browser_context_by_id_mut(browser_context_id)
                    .ok_or(SessionDisposalError::InvalidSession)?;
                let removed_target_id = bc
                    .detach_dedicated_worker_target_session(cleanup_plan.session_id())
                    .ok_or(SessionDisposalError::InvalidSession)?;
                anyhow::ensure!(
                    removed_target_id == *target_id,
                    SessionDisposalError::UnknownTarget
                );
            }
            SessionDisposalTarget::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            } => {
                let bc = self
                    .browser_context_by_id_mut(browser_context_id)
                    .ok_or(SessionDisposalError::InvalidSession)?;
                let removed_target_id = bc
                    .detach_service_worker_target_session(cleanup_plan.session_id())
                    .ok_or(SessionDisposalError::InvalidSession)?;
                anyhow::ensure!(
                    removed_target_id == *target_id,
                    SessionDisposalError::UnknownTarget
                );
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
