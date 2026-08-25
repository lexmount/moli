use std::collections::HashSet;

use crate::devtools_runtime::{
    DevToolsSessionId, DevToolsTargetFilterEntry, DevToolsTargetId, DevToolsTargetInfo,
    DevToolsTargetKind, TargetAttachmentEvent, TargetDetachmentEvent,
};

use super::{
    CommittedAttachSession, DetachedTargetSession, PreparedAttachSession, TargetClosureCleanupPlan,
    TargetClosurePlan, TargetEventPlan, TargetHandlerStore, TargetHostDelta, TargetRegistry,
    TargetSessionRegistry,
};
use crate::conn::{BackgroundProtocolEvent, CdpSessionRoute, CdpTargetFilter};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TargetControlPlane {
    sessions: TargetSessionRegistry,
    registry: TargetRegistry,
    handlers: TargetHandlerStore,
}

impl TargetControlPlane {
    pub(crate) fn register_top_level_page(
        &mut self,
        page_target_id: String,
        tab_target_id: String,
    ) {
        self.registry
            .register_top_level_page(page_target_id, tab_target_id);
    }

    pub(crate) fn register_worker(&mut self, target_id: String, kind: DevToolsTargetKind) {
        self.registry.register_worker(target_id, kind);
    }

    pub(crate) fn remove_worker(&mut self, target_id: &str) -> bool {
        self.registry.remove_worker(target_id).is_some()
    }

    pub(crate) fn tab_target_id_for_page_target_id(&self, page_target_id: &str) -> Option<&str> {
        self.registry
            .tab_target_id_for_page_target_id(page_target_id)
    }

    pub(crate) fn page_target_id_for_tab_target_id(&self, tab_target_id: &str) -> Option<&str> {
        self.registry
            .page_target_id_for_tab_target_id(tab_target_id)
    }

    pub(crate) fn primary_session_id_for_tab_target_id(&self, tab_target_id: &str) -> Option<&str> {
        self.registry
            .primary_session_id_for_tab_target_id(tab_target_id)
    }

    pub(crate) fn assign_session_to_tab_target(
        &mut self,
        tab_target_id: &str,
        session_id: String,
        auxiliary: bool,
    ) -> bool {
        self.registry
            .assign_session_to_tab_target(tab_target_id, session_id, auxiliary)
    }

    pub(crate) fn remove_tab_session(&mut self, session_id: &str) -> Option<String> {
        self.registry.remove_tab_session(session_id)
    }

    pub(crate) fn remove_top_level_page_by_page_target_id(
        &mut self,
        page_target_id: &str,
    ) -> Option<TargetClosurePlan> {
        self.registry
            .remove_top_level_page_by_page_target_id(page_target_id)
    }

    pub(crate) fn tab_target_id_for_session_id(&self, session_id: &str) -> Option<&str> {
        self.registry.tab_target_id_for_session_id(session_id)
    }

    pub(crate) fn tab_target_info_for_page_target_info(
        &self,
        page_target_info: DevToolsTargetInfo,
    ) -> Option<DevToolsTargetInfo> {
        if page_target_info.kind != DevToolsTargetKind::Page {
            return None;
        }
        let page_target_id = page_target_info.target_id.as_ref()?.as_str();
        let target = self
            .registry
            .top_level_target_for_page_target_id(page_target_id)?;
        Some(super::projection::tab_target_info_from_page_target_info(
            target,
            page_target_info,
        ))
    }

    pub(crate) fn project_tab_page_target_infos(
        &self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<DevToolsTargetInfo> {
        let target = target_info.target_id.as_ref().and_then(|target_id| {
            self.registry
                .top_level_target_for_page_target_id(target_id.as_str())
        });
        super::projection::project_tab_page_target_infos(target, target_info)
    }

    pub(crate) fn target_deltas_for_target_id(
        &self,
        target_id: &str,
        build_delta: fn(String) -> TargetHostDelta,
    ) -> Vec<TargetHostDelta> {
        if let Some(target) = self
            .registry
            .top_level_target_for_page_target_id(target_id)
            .or_else(|| self.registry.top_level_target_for_tab_target_id(target_id))
        {
            return vec![
                build_delta(target.tab_target_id().to_owned()),
                build_delta(target.page_target_id().to_owned()),
            ];
        }
        vec![build_delta(target_id.to_owned())]
    }

    pub(crate) fn set_discover_targets(
        &mut self,
        owner_session_id: Option<&str>,
        filter: CdpTargetFilter,
    ) {
        self.handlers.set_discover_targets(owner_session_id, filter);
    }

    pub(crate) fn clear_discover_targets(&mut self, owner_session_id: Option<&str>) {
        self.handlers.clear_discover_targets(owner_session_id);
    }

    pub(crate) fn ensure_owner(&mut self, owner_session_id: Option<&str>) {
        self.handlers.ensure_owner(owner_session_id);
    }

    pub(crate) fn remove_owner(&mut self, owner_session_id: Option<&str>) {
        self.handlers.remove_owner(owner_session_id);
    }

    pub(crate) fn discover_filter_entries(
        &self,
        owner_session_id: Option<&str>,
    ) -> Option<Vec<DevToolsTargetFilterEntry>> {
        self.handlers.discover_filter_entries(owner_session_id)
    }

    pub(crate) fn initial_target_created_events_for_owner(
        &mut self,
        owner_session_id: Option<&str>,
        target_infos: Vec<DevToolsTargetInfo>,
    ) -> Vec<BackgroundProtocolEvent> {
        self.handlers
            .target_created_events(owner_session_id, target_infos)
    }

    pub(crate) fn has_any_discovery(&self) -> bool {
        self.handlers.has_any_discovery()
    }

    pub(crate) fn has_any_target_info_observer(&self) -> bool {
        !self
            .handlers
            .target_info_owner_session_ids(self.sessions.auto_attached_owner_session_ids())
            .is_empty()
    }

    pub(crate) fn target_created_events_for_all_discovery_owners(
        &mut self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<BackgroundProtocolEvent> {
        let owner_session_ids = self.handlers.discovery_owner_session_ids();
        if owner_session_ids.is_empty() {
            return Vec::new();
        }

        owner_session_ids
            .into_iter()
            .flat_map(|owner_session_id| {
                self.handlers
                    .target_created_events(owner_session_id.as_deref(), vec![target_info.clone()])
            })
            .collect()
    }

    pub(crate) fn target_info_changed_events_for_all_observer_owners(
        &self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<BackgroundProtocolEvent> {
        let owner_session_ids = self
            .handlers
            .target_info_owner_session_ids(self.sessions.auto_attached_owner_session_ids());
        if owner_session_ids.is_empty() {
            return Vec::new();
        }

        owner_session_ids
            .into_iter()
            .flat_map(|owner_session_id| {
                let auto_attached_target_ids = self
                    .sessions
                    .auto_attached_target_ids_for_owner(owner_session_id.as_deref())
                    .into_iter()
                    .collect::<HashSet<_>>();
                self.handlers.target_info_changed_events(
                    owner_session_id.as_deref(),
                    vec![target_info.clone()],
                    &auto_attached_target_ids,
                )
            })
            .collect()
    }

    pub(crate) fn target_info_changed_events_for_all_discovery_owners(
        &self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<BackgroundProtocolEvent> {
        self.handlers
            .discovery_owner_session_ids()
            .into_iter()
            .flat_map(|owner_session_id| {
                self.handlers.target_info_changed_events(
                    owner_session_id.as_deref(),
                    vec![target_info.clone()],
                    &HashSet::new(),
                )
            })
            .collect()
    }

    pub(crate) fn target_destroyed_events_for_all_discovery_owners(
        &mut self,
        target_info: DevToolsTargetInfo,
    ) -> Vec<BackgroundProtocolEvent> {
        let owner_session_ids = self.handlers.discovery_owner_session_ids();
        if owner_session_ids.is_empty() {
            return Vec::new();
        }

        owner_session_ids
            .into_iter()
            .flat_map(|owner_session_id| {
                self.handlers
                    .target_destroyed_events(owner_session_id.as_deref(), vec![target_info.clone()])
            })
            .collect()
    }

    pub(crate) fn target_crashed_events_for_all_discovery_owners(
        &self,
        target_id: &str,
        status: &str,
        error_code: i32,
    ) -> Vec<BackgroundProtocolEvent> {
        self.handlers
            .discovery_owner_session_ids()
            .into_iter()
            .filter_map(|owner_session_id| {
                self.handlers.target_crashed_event(
                    owner_session_id.as_deref(),
                    target_id,
                    status,
                    error_code,
                )
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn commit_auto_attached_session_for_target(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: Option<&str>,
        route: Option<CdpSessionRoute>,
        waiting_for_debugger: bool,
    ) {
        self.ensure_owner(owner_session_id);
        if let Some(target_id) = target_id {
            self.sessions
                .commit_attached_session(PreparedAttachSession::new(
                    session_id,
                    owner_session_id,
                    target_id,
                    route,
                    true,
                    waiting_for_debugger,
                ));
        } else {
            self.sessions
                .register_auto_attached_session(session_id, owner_session_id, None);
        }
    }

    pub(crate) fn commit_attached_session(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: &str,
        route: Option<CdpSessionRoute>,
        auto_attached: bool,
        waiting_for_debugger: bool,
    ) -> CommittedAttachSession {
        self.sessions
            .commit_attached_session(PreparedAttachSession::new(
                session_id,
                owner_session_id,
                target_id,
                route,
                auto_attached,
                waiting_for_debugger,
            ))
    }

    pub(crate) fn commit_attached_session_event(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: &str,
        route: Option<CdpSessionRoute>,
        auto_attached: bool,
        waiting_for_debugger: bool,
        target_info: DevToolsTargetInfo,
    ) -> TargetEventPlan {
        debug_assert_eq!(
            target_info
                .target_id
                .as_ref()
                .map(|target_id| target_id.as_str()),
            Some(target_id)
        );
        let committed_session = self.commit_attached_session(
            session_id.clone(),
            owner_session_id,
            target_id,
            route,
            auto_attached,
            waiting_for_debugger,
        );
        TargetEventPlan::from_committed_session_event(
            committed_session,
            target_attached_event(
                &session_id,
                owner_session_id,
                target_info,
                waiting_for_debugger,
            ),
        )
    }

    pub(crate) fn rollback_attached_session_without_event(
        &mut self,
        session_id: &str,
    ) -> TargetEventPlan {
        let rolled_back = self
            .sessions
            .rollback_attached_session_without_event(session_id);
        TargetEventPlan::from_rolled_back_session_ids(
            rolled_back
                .then(|| session_id.to_owned())
                .into_iter()
                .collect(),
        )
    }

    pub(crate) fn detach_attached_session_event_plan(
        &mut self,
        session_id: &str,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> Option<TargetEventPlan> {
        let detached_session = self.sessions.detach_attached_session(session_id)?;
        Some(TargetEventPlan::from_detached_session_event(
            detached_session.clone(),
            target_detached_event(
                detached_session.target_id(),
                detached_session.session_id(),
                reason,
                parent_session_id,
            ),
        ))
    }

    pub(crate) fn detach_known_session_event_plan(
        &mut self,
        target_id: &str,
        session_id: &str,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        if let Some(plan) =
            self.detach_attached_session_event_plan(session_id, reason, parent_session_id)
        {
            return plan;
        }
        let detached_session = DetachedTargetSession::from_detached_binding(session_id, target_id);
        TargetEventPlan::from_detached_session_event(
            detached_session.clone(),
            target_detached_event(target_id, session_id, reason, parent_session_id),
        )
    }

    #[cfg(test)]
    pub(crate) fn detach_target_closure_attached_sessions_event_plan(
        &mut self,
        target_id: &str,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let cleanup_plan = TargetClosureCleanupPlan::new(
            target_id.to_owned(),
            reason,
            self.sessions.attached_sessions_for_target(target_id),
        );
        self.detach_target_closure_cleanup_event_plan(cleanup_plan, parent_session_id)
    }

    pub(crate) fn detach_known_sessions_for_target_event_plan(
        &mut self,
        target_id: &str,
        session_ids: impl IntoIterator<Item = String>,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        let mut detached_sessions = Vec::new();
        let mut events = Vec::new();
        for session_id in session_ids {
            let detached_session = self
                .sessions
                .detach_attached_session(&session_id)
                .unwrap_or_else(|| {
                    DetachedTargetSession::from_detached_binding(&session_id, target_id)
                });
            events.push(target_detached_event(
                detached_session.target_id(),
                detached_session.session_id(),
                reason,
                parent_session_id,
            ));
            detached_sessions.push(detached_session);
        }
        TargetEventPlan::from_detached_sessions_events(detached_sessions, events)
    }

    pub(crate) fn detach_target_closure_cleanup_event_plan(
        &mut self,
        cleanup_plan: TargetClosureCleanupPlan,
        parent_session_id: Option<&str>,
    ) -> TargetEventPlan {
        if cleanup_plan.session_ids().next().is_none() {
            return TargetEventPlan::default();
        }
        let target_id = cleanup_plan.target_id().to_owned();
        let reason = cleanup_plan.reason().map(str::to_owned);
        self.detach_known_sessions_for_target_event_plan(
            &target_id,
            cleanup_plan.into_session_ids(),
            reason.as_deref(),
            parent_session_id,
        )
    }

    pub(crate) fn attached_sessions_for_target(&self, target_id: &str) -> Vec<String> {
        self.sessions.attached_sessions_for_target(target_id)
    }

    pub(crate) fn target_has_waiting_for_debugger_session(&self, target_id: &str) -> bool {
        self.sessions
            .target_has_waiting_for_debugger_session(target_id)
    }

    pub(crate) fn auto_attached_sessions_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        self.sessions
            .auto_attached_sessions_for_owner(owner_session_id)
    }

    pub(crate) fn attached_session_cascade_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        self.sessions
            .attached_session_cascade_for_owner(owner_session_id)
    }

    pub(crate) fn attached_session_cascade_for_root_frontend(&self) -> Vec<String> {
        self.sessions.attached_session_cascade_for_root_frontend()
    }

    pub(crate) fn attached_session_owner_session_id(&self, session_id: &str) -> Option<&str> {
        self.sessions.attached_session_owner_session_id(session_id)
    }

    pub(crate) fn auto_attached_session_cascade_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        self.sessions
            .auto_attached_session_cascade_for_owner(owner_session_id)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.registry.len()
    }

    pub(crate) fn host_kind(&self, target_id: &str) -> Option<DevToolsTargetKind> {
        self.registry.host(target_id).map(|host| host.kind())
    }
}

fn target_attached_event(
    session_id: &str,
    owner_session_id: Option<&str>,
    target_info: DevToolsTargetInfo,
    waiting_for_debugger: bool,
) -> BackgroundProtocolEvent {
    let target_id = target_info
        .target_id
        .clone()
        .unwrap_or_else(|| DevToolsTargetId::from(""));
    BackgroundProtocolEvent::target_attached(TargetAttachmentEvent {
        target_id,
        session_id: DevToolsSessionId::from(session_id),
        parent_session_id: owner_session_id.map(DevToolsSessionId::from),
        target_info,
        waiting_for_debugger,
    })
}

fn target_detached_event(
    target_id: &str,
    session_id: &str,
    reason: Option<&str>,
    parent_session_id: Option<&str>,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::target_detached(TargetDetachmentEvent {
        target_id: DevToolsTargetId::from(target_id),
        session_id: DevToolsSessionId::from(session_id),
        parent_session_id: parent_session_id.map(DevToolsSessionId::from),
        reason: reason.map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use crate::conn::{CdpSessionRoute, CdpTargetFilter, TargetControlPlane};
    use crate::devtools_runtime::{DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind};

    fn target_info(target_id: &str, kind: DevToolsTargetKind) -> DevToolsTargetInfo {
        DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from(target_id)),
            kind,
            title: String::new(),
            url: "about:blank".to_owned(),
            attached: false,
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: None,
            moli_popup_id: None,
        }
    }

    #[test]
    fn target_crashed_events_only_reach_discovery_owners_that_reported_the_host() {
        let mut control = TargetControlPlane::default();
        control.set_discover_targets(None, CdpTargetFilter::default_target_discovery());
        control.set_discover_targets(
            Some("SID-reported"),
            CdpTargetFilter::default_target_discovery(),
        );
        control.set_discover_targets(
            Some("SID-unreported"),
            CdpTargetFilter::default_target_discovery(),
        );
        assert_eq!(
            control
                .initial_target_created_events_for_owner(
                    None,
                    vec![target_info("TID-page", DevToolsTargetKind::Page)],
                )
                .len(),
            1
        );
        assert_eq!(
            control
                .initial_target_created_events_for_owner(
                    Some("SID-reported"),
                    vec![target_info("TID-page", DevToolsTargetKind::Page)],
                )
                .len(),
            1
        );

        let events =
            control.target_crashed_events_for_all_discovery_owners("TID-page", "crashed", 5);
        assert_eq!(events.len(), 2);
        let messages = events
            .into_iter()
            .map(|event| event.into_parts().0)
            .collect::<Vec<_>>();
        for message in &messages {
            assert_eq!(message["method"], "Target.targetCrashed");
            assert_eq!(message["params"]["targetId"], "TID-page");
            assert_eq!(message["params"]["status"], "crashed");
            assert_eq!(message["params"]["errorCode"], 5);
        }
        assert!(messages[0].get("sessionId").is_none());
        assert_eq!(messages[1]["sessionId"], "SID-reported");

        assert!(
            control
                .target_crashed_events_for_all_discovery_owners("TID-never-reported", "crashed", 5,)
                .is_empty()
        );
    }

    #[test]
    fn commit_auto_attached_session_event_records_committed_session_plan() {
        let mut control = TargetControlPlane::default();
        let route = CdpSessionRoute::ActiveTarget {
            browser_context_id: "BID-1".to_owned(),
            target_id: Some("TID-page".to_owned()),
        };

        control.ensure_owner(Some("SID-tab"));
        let plan = control.commit_attached_session_event(
            "SID-page".to_owned(),
            Some("SID-tab"),
            "TID-page",
            Some(route.clone()),
            true,
            true,
            target_info("TID-page", DevToolsTargetKind::Page),
        );

        assert_eq!(
            control.attached_sessions_for_target("TID-page"),
            vec!["SID-page".to_owned()]
        );
        assert_eq!(
            control.auto_attached_sessions_for_owner(Some("SID-tab")),
            vec!["SID-page".to_owned()]
        );

        let committed_sessions = plan.committed_sessions();
        assert_eq!(committed_sessions.len(), 1);
        let committed = &committed_sessions[0];
        assert_eq!(committed.session_id(), "SID-page");
        assert_eq!(committed.owner_session_id(), Some("SID-tab"));
        assert_eq!(committed.target_id(), "TID-page");
        assert_eq!(committed.route(), Some(&route));
        assert!(committed.auto_attached());
        assert!(committed.waiting_for_debugger());

        assert_eq!(plan.into_background_events().len(), 1);
    }

    #[test]
    fn detach_attached_session_event_plan_records_detached_session() {
        let mut control = TargetControlPlane::default();
        let route = CdpSessionRoute::ActiveTarget {
            browser_context_id: "BID-1".to_owned(),
            target_id: Some("TID-page".to_owned()),
        };
        control.commit_attached_session_event(
            "SID-page".to_owned(),
            Some("SID-owner"),
            "TID-page",
            Some(route.clone()),
            false,
            false,
            target_info("TID-page", DevToolsTargetKind::Page),
        );

        let plan = control
            .detach_attached_session_event_plan(
                "SID-page",
                Some("Target closed"),
                Some("SID-owner"),
            )
            .expect("attached session should detach");

        assert!(control.attached_sessions_for_target("TID-page").is_empty());
        let detached_sessions = plan.detached_sessions();
        assert_eq!(detached_sessions.len(), 1);
        let detached = &detached_sessions[0];
        assert_eq!(detached.session_id(), "SID-page");
        assert_eq!(detached.owner_session_id(), Some("SID-owner"));
        assert_eq!(detached.target_id(), "TID-page");
        assert_eq!(detached.route(), Some(&route));
        assert!(!detached.auto_attached());
        assert!(!detached.waiting_for_debugger());

        let events = plan.into_background_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events.as_slice(),
            [event] if event.is_target_detached()
        ));
    }

    #[test]
    fn target_closure_attached_sessions_event_plan_detaches_all_target_sessions() {
        let mut control = TargetControlPlane::default();
        for session_id in ["SID-a", "SID-b"] {
            control.commit_attached_session_event(
                session_id.to_owned(),
                None,
                "TID-page",
                Some(CdpSessionRoute::ActiveTarget {
                    browser_context_id: "BID-1".to_owned(),
                    target_id: Some("TID-page".to_owned()),
                }),
                false,
                false,
                target_info("TID-page", DevToolsTargetKind::Page),
            );
        }

        let plan = control.detach_target_closure_attached_sessions_event_plan(
            "TID-page",
            Some("Target closed"),
            None,
        );

        assert!(control.attached_sessions_for_target("TID-page").is_empty());
        assert_eq!(
            plan.detached_sessions()
                .iter()
                .map(|session| session.session_id())
                .collect::<Vec<_>>(),
            vec!["SID-a", "SID-b"]
        );
        assert_eq!(plan.into_background_events().len(), 2);
    }

    #[test]
    fn target_closure_cleanup_event_plan_detaches_declared_sessions() {
        let mut control = TargetControlPlane::default();
        for session_id in ["SID-primary", "SID-aux"] {
            control.commit_attached_session_event(
                session_id.to_owned(),
                None,
                "TID-page",
                Some(CdpSessionRoute::ActiveTarget {
                    browser_context_id: "BID-1".to_owned(),
                    target_id: Some("TID-page".to_owned()),
                }),
                false,
                false,
                target_info("TID-page", DevToolsTargetKind::Page),
            );
        }

        let plan = control.detach_target_closure_cleanup_event_plan(
            crate::conn::TargetClosureCleanupPlan::new(
                "TID-page",
                Some("Render process gone."),
                ["SID-primary".to_owned(), "SID-aux".to_owned()],
            ),
            None,
        );

        assert!(control.attached_sessions_for_target("TID-page").is_empty());
        assert_eq!(
            plan.detached_sessions()
                .iter()
                .map(|session| (session.target_id(), session.session_id()))
                .collect::<Vec<_>>(),
            vec![("TID-page", "SID-primary"), ("TID-page", "SID-aux")]
        );
        assert_eq!(plan.into_background_events().len(), 2);
    }

    #[test]
    fn rollback_attached_session_without_event_clears_session_indexes() {
        let mut control = TargetControlPlane::default();
        control.ensure_owner(Some("SID-tab"));
        control.commit_attached_session_event(
            "SID-page".to_owned(),
            Some("SID-tab"),
            "TID-page",
            Some(CdpSessionRoute::ActiveTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: Some("TID-page".to_owned()),
            }),
            true,
            false,
            target_info("TID-page", DevToolsTargetKind::Page),
        );

        let rollback_plan = control.rollback_attached_session_without_event("SID-page");
        assert_eq!(
            rollback_plan.rolled_back_session_ids(),
            &["SID-page".to_owned()]
        );
        assert!(rollback_plan.into_background_events().is_empty());

        assert!(control.attached_sessions_for_target("TID-page").is_empty());
        assert!(
            control
                .auto_attached_sessions_for_owner(Some("SID-tab"))
                .is_empty()
        );
        assert!(
            control
                .rollback_attached_session_without_event("SID-page")
                .rolled_back_session_ids()
                .is_empty()
        );
    }
}
