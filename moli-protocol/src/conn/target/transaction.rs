use crate::conn::{BackgroundProtocolEvent, CdpSessionRoute};
use crate::devtools_runtime::DevToolsTargetInfo;
use moli_page_types::DevToolsSessionKey;

use super::{CommittedAttachSession, DetachedTargetSession, TargetHostDelta};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TargetEventPlan {
    events: Vec<BackgroundProtocolEvent>,
    committed_sessions: Vec<CommittedAttachSession>,
    detached_sessions: Vec<DetachedTargetSession>,
    rolled_back_session_ids: Vec<String>,
}

impl TargetEventPlan {
    pub(crate) fn from_background_events(events: Vec<BackgroundProtocolEvent>) -> Self {
        Self {
            events,
            committed_sessions: Vec::new(),
            detached_sessions: Vec::new(),
            rolled_back_session_ids: Vec::new(),
        }
    }

    pub(crate) fn from_rolled_back_session_ids(rolled_back_session_ids: Vec<String>) -> Self {
        Self {
            events: Vec::new(),
            committed_sessions: Vec::new(),
            detached_sessions: Vec::new(),
            rolled_back_session_ids,
        }
    }

    pub(crate) fn from_committed_session_event(
        committed_session: CommittedAttachSession,
        event: BackgroundProtocolEvent,
    ) -> Self {
        Self {
            events: vec![event],
            committed_sessions: vec![committed_session],
            detached_sessions: Vec::new(),
            rolled_back_session_ids: Vec::new(),
        }
    }

    pub(crate) fn from_detached_session_event(
        detached_session: DetachedTargetSession,
        event: BackgroundProtocolEvent,
    ) -> Self {
        Self {
            events: vec![event],
            committed_sessions: Vec::new(),
            detached_sessions: vec![detached_session],
            rolled_back_session_ids: Vec::new(),
        }
    }

    pub(crate) fn from_detached_sessions_events(
        detached_sessions: Vec<DetachedTargetSession>,
        events: Vec<BackgroundProtocolEvent>,
    ) -> Self {
        Self {
            events,
            committed_sessions: Vec::new(),
            detached_sessions,
            rolled_back_session_ids: Vec::new(),
        }
    }

    pub(crate) fn into_background_events(self) -> Vec<BackgroundProtocolEvent> {
        self.events
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.events.extend(other.events);
        self.committed_sessions.extend(other.committed_sessions);
        self.detached_sessions.extend(other.detached_sessions);
        self.rolled_back_session_ids
            .extend(other.rolled_back_session_ids);
    }

    #[cfg(test)]
    pub(crate) fn committed_sessions(&self) -> &[CommittedAttachSession] {
        &self.committed_sessions
    }

    pub(crate) fn detached_sessions(&self) -> &[DetachedTargetSession] {
        &self.detached_sessions
    }

    pub(crate) fn rolled_back_session_ids(&self) -> &[String] {
        &self.rolled_back_session_ids
    }
}

impl IntoIterator for TargetEventPlan {
    type Item = BackgroundProtocolEvent;
    type IntoIter = std::vec::IntoIter<BackgroundProtocolEvent>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedTargetHostDelta {
    delta: TargetHostDelta,
    snapshot: Option<DevToolsTargetInfo>,
}

impl PreparedTargetHostDelta {
    pub(crate) fn created(
        target_id: impl Into<String>,
        snapshot: Option<DevToolsTargetInfo>,
    ) -> Self {
        Self::new(TargetHostDelta::created(target_id), snapshot)
    }

    pub(crate) fn info_changed(
        target_id: impl Into<String>,
        snapshot: Option<DevToolsTargetInfo>,
    ) -> Self {
        Self::new(TargetHostDelta::info_changed(target_id), snapshot)
    }

    pub(crate) fn destroyed(
        target_id: impl Into<String>,
        snapshot: Option<DevToolsTargetInfo>,
    ) -> Self {
        Self::new(TargetHostDelta::destroyed(target_id), snapshot)
    }

    pub(crate) fn without_snapshot(delta: TargetHostDelta) -> Self {
        Self::new(delta, None)
    }

    fn new(delta: TargetHostDelta, snapshot: Option<DevToolsTargetInfo>) -> Self {
        if let Some(snapshot) = snapshot.as_ref() {
            debug_assert_eq!(
                snapshot
                    .target_id
                    .as_ref()
                    .map(|target_id| target_id.as_str()),
                Some(delta.target_id())
            );
        }
        Self { delta, snapshot }
    }

    pub(crate) fn into_parts(self) -> (TargetHostDelta, Option<DevToolsTargetInfo>) {
        (self.delta, self.snapshot)
    }

    pub(crate) fn target_id(&self) -> &str {
        self.delta.target_id()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedTargetHostClosure {
    detached_info_deltas: Vec<PreparedTargetHostDelta>,
    destroyed_deltas: Vec<PreparedTargetHostDelta>,
}

impl PreparedTargetHostClosure {
    pub(crate) fn new(
        detached_info_deltas: Vec<PreparedTargetHostDelta>,
        destroyed_deltas: Vec<PreparedTargetHostDelta>,
    ) -> Self {
        Self {
            detached_info_deltas,
            destroyed_deltas,
        }
    }

    pub(crate) fn into_parts(self) -> (Vec<PreparedTargetHostDelta>, Vec<PreparedTargetHostDelta>) {
        (self.detached_info_deltas, self.destroyed_deltas)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetAttachSessionCommit {
    session_id: String,
    owner_session_id: Option<String>,
    route: CdpSessionRoute,
    auto_attached: bool,
    waiting_for_debugger: bool,
}

impl TargetAttachSessionCommit {
    pub(crate) fn new(
        session_id: impl Into<String>,
        owner_session_id: Option<String>,
        route: CdpSessionRoute,
        auto_attached: bool,
        waiting_for_debugger: bool,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            owner_session_id,
            route,
            auto_attached,
            waiting_for_debugger,
        }
    }

    pub(crate) fn auto_attached(
        session_id: impl Into<String>,
        owner_session_id: Option<String>,
        route: CdpSessionRoute,
        waiting_for_debugger: bool,
    ) -> Self {
        Self::new(
            session_id,
            owner_session_id,
            route,
            true,
            waiting_for_debugger,
        )
    }

    pub(crate) fn direct(
        session_id: impl Into<String>,
        owner_session_id: Option<String>,
        route: CdpSessionRoute,
        waiting_for_debugger: bool,
    ) -> Self {
        Self::new(
            session_id,
            owner_session_id,
            route,
            false,
            waiting_for_debugger,
        )
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn route(&self) -> &CdpSessionRoute {
        &self.route
    }

    #[cfg(test)]
    pub(crate) fn waiting_for_debugger(&self) -> bool {
        self.waiting_for_debugger
    }

    pub(crate) fn into_parts(self) -> (String, Option<String>, CdpSessionRoute, bool, bool) {
        (
            self.session_id,
            self.owner_session_id,
            self.route,
            self.auto_attached,
            self.waiting_for_debugger,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedTargetAttach {
    target_id: String,
    target_info: DevToolsTargetInfo,
    sessions: Vec<TargetAttachSessionCommit>,
}

impl PreparedTargetAttach {
    pub(crate) fn new(
        target_id: impl Into<String>,
        target_info: DevToolsTargetInfo,
        sessions: impl IntoIterator<Item = TargetAttachSessionCommit>,
    ) -> Self {
        let target_id = target_id.into();
        debug_assert_eq!(
            target_info
                .target_id
                .as_ref()
                .map(|target_id| target_id.as_str()),
            Some(target_id.as_str())
        );
        Self {
            target_id,
            target_info,
            sessions: sessions.into_iter().collect(),
        }
    }

    pub(crate) fn into_parts(self) -> (String, DevToolsTargetInfo, Vec<TargetAttachSessionCommit>) {
        (self.target_id, self.target_info, self.sessions)
    }

    #[cfg(test)]
    pub(crate) fn target_info(&self) -> &DevToolsTargetInfo {
        &self.target_info
    }

    #[cfg(test)]
    pub(crate) fn sessions(&self) -> &[TargetAttachSessionCommit] {
        &self.sessions
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionDisposalPlan {
    session_id: String,
    target: SessionDisposalTarget,
}

impl SessionDisposalPlan {
    /// Freezes the exact binding that must remain alive while every domain
    /// handler disables its session-owned state.
    ///
    /// Callers must execute the domain cleanup phase before committing this
    /// plan's binding removal. Keeping both phases tied to one value prevents
    /// detach paths from resolving the session again after asynchronous work.
    pub(crate) fn for_session_route(session_id: &str, route: &CdpSessionRoute) -> Option<Self> {
        Some(Self {
            session_id: session_id.to_owned(),
            target: SessionDisposalTarget::from_route(route)?,
        })
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn browser_context_id(&self) -> Option<&str> {
        self.target.browser_context_id()
    }

    pub(crate) fn target(&self) -> &SessionDisposalTarget {
        &self.target
    }

    pub(crate) fn target_id(&self) -> Option<&str> {
        self.target.target_id()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionDisposalTarget {
    Browser,
    PageTarget {
        browser_context_id: String,
        target_id: String,
        session_key: DevToolsSessionKey,
    },
    TabTarget {
        browser_context_id: String,
        tab_target_id: String,
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

impl SessionDisposalTarget {
    fn from_route(route: &CdpSessionRoute) -> Option<Self> {
        Some(match route {
            CdpSessionRoute::PageTarget {
                browser_context_id,
                target_id,
                session_key,
            } => Self::PageTarget {
                browser_context_id: browser_context_id.clone(),
                target_id: target_id.clone(),
                session_key: session_key.clone(),
            },
            CdpSessionRoute::TabTarget {
                browser_context_id,
                tab_target_id,
            } => Self::TabTarget {
                browser_context_id: browser_context_id.clone(),
                tab_target_id: tab_target_id.clone(),
            },
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id,
                target_id,
            } => Self::SharedWorkerTarget {
                browser_context_id: browser_context_id.clone(),
                target_id: target_id.clone(),
            },
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id,
                target_id,
            } => Self::DedicatedWorkerTarget {
                browser_context_id: browser_context_id.clone(),
                target_id: target_id.clone(),
            },
            CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id,
                target_id,
            } => Self::ServiceWorkerTarget {
                browser_context_id: browser_context_id.clone(),
                target_id: target_id.clone(),
            },
            CdpSessionRoute::Browser => Self::Browser,
            CdpSessionRoute::BrowserContext { .. } => return None,
        })
    }

    pub(crate) fn browser_context_id(&self) -> Option<&str> {
        match self {
            Self::Browser => None,
            Self::PageTarget {
                browser_context_id, ..
            }
            | Self::TabTarget {
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

    pub(crate) fn target_id(&self) -> Option<&str> {
        match self {
            Self::PageTarget { target_id, .. }
            | Self::SharedWorkerTarget { target_id, .. }
            | Self::DedicatedWorkerTarget { target_id, .. }
            | Self::ServiceWorkerTarget { target_id, .. } => Some(target_id),
            Self::TabTarget { tab_target_id, .. } => Some(tab_target_id),
            Self::Browser => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetAttachRollbackPlan {
    session_id: String,
    cleanup_plan: Option<SessionDisposalPlan>,
}

impl TargetAttachRollbackPlan {
    pub(crate) fn from_prepared_attach_session(prepared: &TargetAttachSessionCommit) -> Self {
        Self::from_session_route(prepared.session_id(), Some(prepared.route().clone()))
    }

    pub(crate) fn from_session_route(
        session_id: impl Into<String>,
        route: Option<CdpSessionRoute>,
    ) -> Self {
        let session_id = session_id.into();
        let cleanup_plan = route
            .as_ref()
            .and_then(|route| SessionDisposalPlan::for_session_route(&session_id, route));
        Self {
            session_id,
            cleanup_plan,
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn cleanup_plan(&self) -> Option<&SessionDisposalPlan> {
        self.cleanup_plan.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetAutoAttachedSessionDetachPlan {
    Rollback { session_id: String },
    Detach { cleanup_plan: SessionDisposalPlan },
}

impl TargetAutoAttachedSessionDetachPlan {
    pub(crate) fn from_session_route(
        session_id: impl Into<String>,
        route: Option<CdpSessionRoute>,
    ) -> Self {
        let session_id = session_id.into();
        let Some(route) = route else {
            return Self::Rollback { session_id };
        };
        if matches!(route, CdpSessionRoute::Browser) {
            return Self::Rollback { session_id };
        }
        match SessionDisposalPlan::for_session_route(&session_id, &route) {
            Some(cleanup_plan) => Self::Detach { cleanup_plan },
            None => Self::Rollback { session_id },
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        match self {
            Self::Rollback { session_id } => session_id,
            Self::Detach { cleanup_plan } => cleanup_plan.session_id(),
        }
    }

    pub(crate) fn cleanup_plan(&self) -> Option<&SessionDisposalPlan> {
        match self {
            Self::Rollback { .. } => None,
            Self::Detach { cleanup_plan } => Some(cleanup_plan),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetClosureCleanupPlan {
    target_id: String,
    reason: Option<String>,
    session_ids: Vec<String>,
}

impl TargetClosureCleanupPlan {
    pub(crate) fn new(
        target_id: impl Into<String>,
        reason: Option<&str>,
        session_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            target_id: target_id.into(),
            reason: reason.map(str::to_owned),
            session_ids: session_ids.into_iter().collect(),
        }
    }

    pub(crate) fn from_primary_and_attached_sessions(
        target_id: impl Into<String>,
        reason: Option<&str>,
        primary_session_id: Option<String>,
        attached_session_ids: Vec<String>,
    ) -> Self {
        Self::new(
            target_id,
            reason,
            primary_session_id.into_iter().chain(attached_session_ids),
        )
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub(crate) fn session_ids(&self) -> impl Iterator<Item = &str> {
        self.session_ids.iter().map(String::as_str)
    }

    pub(crate) fn into_session_ids(self) -> Vec<String> {
        self.session_ids
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetSessionDetachCleanupPlan {
    target_id: String,
    session_id: String,
    reason: Option<String>,
    parent_session_id: Option<String>,
}

impl TargetSessionDetachCleanupPlan {
    pub(crate) fn new(
        target_id: impl Into<String>,
        session_id: impl Into<String>,
        reason: Option<&str>,
        parent_session_id: Option<&str>,
    ) -> Self {
        Self {
            target_id: target_id.into(),
            session_id: session_id.into(),
            reason: reason.map(str::to_owned),
            parent_session_id: parent_session_id.map(str::to_owned),
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub(crate) fn parent_session_id(&self) -> Option<&str> {
        self.parent_session_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PreparedTargetAttach, PreparedTargetHostDelta, SessionDisposalPlan, SessionDisposalTarget,
        TargetAttachRollbackPlan, TargetAttachSessionCommit, TargetAutoAttachedSessionDetachPlan,
        TargetClosureCleanupPlan, TargetEventPlan, TargetSessionDetachCleanupPlan,
    };
    use crate::conn::CdpSessionRoute;
    use crate::devtools_runtime::{DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind};
    use moli_page_types::DevToolsSessionKey;

    #[test]
    fn target_binding_cleanup_plan_maps_route_to_cleanup_action() {
        let browser =
            SessionDisposalPlan::for_session_route("SID-browser", &CdpSessionRoute::Browser)
                .expect("Browser sessions require domain cleanup");
        assert_eq!(browser.target(), &SessionDisposalTarget::Browser);
        assert_eq!(browser.browser_context_id(), None);
        assert_eq!(browser.target_id(), None);

        assert_eq!(
            SessionDisposalPlan::for_session_route(
                "SID-active",
                &CdpSessionRoute::PageTarget {
                    browser_context_id: "BID-1".to_owned(),
                    target_id: "TID-active".to_owned(),
                    session_key: DevToolsSessionKey::Primary,
                },
            )
            .expect("Page routes require binding cleanup")
            .target(),
            &SessionDisposalTarget::PageTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-active".to_owned(),
                session_key: DevToolsSessionKey::Primary,
            },
        );

        assert_eq!(
            SessionDisposalPlan::for_session_route(
                "SID-bg",
                &CdpSessionRoute::PageTarget {
                    browser_context_id: "BID-1".to_owned(),
                    target_id: "TID-bg".to_owned(),
                    session_key: DevToolsSessionKey::Primary,
                },
            )
            .expect("Page routes require binding cleanup")
            .target(),
            &SessionDisposalTarget::PageTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-bg".to_owned(),
                session_key: DevToolsSessionKey::Primary,
            },
        );

        assert_eq!(
            SessionDisposalPlan::for_session_route(
                "SID-tab",
                &CdpSessionRoute::TabTarget {
                    browser_context_id: "BID-1".to_owned(),
                    tab_target_id: "TAB-TID-page".to_owned(),
                },
            )
            .expect("Tab routes require binding cleanup")
            .target(),
            &SessionDisposalTarget::TabTarget {
                browser_context_id: "BID-1".to_owned(),
                tab_target_id: "TAB-TID-page".to_owned()
            },
        );
    }

    #[test]
    fn target_closure_cleanup_plan_preserves_target_reason_and_sessions() {
        let plan = TargetClosureCleanupPlan::from_primary_and_attached_sessions(
            "TID-page",
            Some("Render process gone."),
            Some("SID-primary".to_owned()),
            vec!["SID-attached".to_owned()],
        );

        assert_eq!(plan.target_id(), "TID-page");
        assert_eq!(plan.reason(), Some("Render process gone."));
        assert_eq!(
            plan.session_ids().collect::<Vec<_>>(),
            vec!["SID-primary", "SID-attached"]
        );
        assert_eq!(
            plan.into_session_ids(),
            vec!["SID-primary".to_owned(), "SID-attached".to_owned()]
        );
    }

    #[test]
    fn target_session_detach_cleanup_plan_preserves_route_and_event_context() {
        let plan = TargetSessionDetachCleanupPlan::new(
            "TID-page",
            "SID-page",
            Some("Render process gone."),
            Some("SID-parent"),
        );

        assert_eq!(plan.target_id(), "TID-page");
        assert_eq!(plan.session_id(), "SID-page");
        assert_eq!(plan.reason(), Some("Render process gone."));
        assert_eq!(plan.parent_session_id(), Some("SID-parent"));
    }

    #[test]
    fn target_event_plan_extend_preserves_rollback_metadata() {
        let mut plan = TargetEventPlan::from_rolled_back_session_ids(vec!["SID-1".to_owned()]);
        plan.extend(TargetEventPlan::from_rolled_back_session_ids(vec![
            "SID-2".to_owned(),
        ]));

        assert_eq!(
            plan.rolled_back_session_ids(),
            &["SID-1".to_owned(), "SID-2".to_owned()]
        );
    }

    #[test]
    fn target_attach_session_commit_preserves_owner_route_kind_and_waiting_flag() {
        let commit = TargetAttachSessionCommit::auto_attached(
            "SID-child",
            Some("SID-owner".to_owned()),
            CdpSessionRoute::Browser,
            true,
        );

        assert_eq!(
            commit.into_parts(),
            (
                "SID-child".to_owned(),
                Some("SID-owner".to_owned()),
                CdpSessionRoute::Browser,
                true,
                true
            )
        );
    }

    #[test]
    fn prepared_target_attach_preserves_snapshot_and_sessions() {
        let target_info = DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from("TID-page")),
            kind: DevToolsTargetKind::Page,
            title: "Title".to_owned(),
            url: "https://example.test/".to_owned(),
            attached: false,
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: None,
            moli_popup_id: None,
        };
        let prepared = PreparedTargetAttach::new(
            "TID-page",
            target_info.clone(),
            [
                TargetAttachSessionCommit::direct("SID-1", None, CdpSessionRoute::Browser, false),
                TargetAttachSessionCommit::auto_attached(
                    "SID-2",
                    Some("SID-owner".to_owned()),
                    CdpSessionRoute::Browser,
                    true,
                ),
            ],
        );

        let (target_id, snapshot, sessions) = prepared.into_parts();

        assert_eq!(target_id, "TID-page");
        assert_eq!(snapshot, target_info);
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            sessions[1].clone().into_parts(),
            (
                "SID-2".to_owned(),
                Some("SID-owner".to_owned()),
                CdpSessionRoute::Browser,
                true,
                true
            )
        );
    }

    #[test]
    fn target_attach_rollback_plan_preserves_route_cleanup_boundary() {
        let prepared = TargetAttachSessionCommit::auto_attached(
            "SID-worker",
            Some("SID-owner".to_owned()),
            CdpSessionRoute::SharedWorkerTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-worker".to_owned(),
            },
            false,
        );

        let rollback = TargetAttachRollbackPlan::from_prepared_attach_session(&prepared);

        assert_eq!(rollback.session_id(), "SID-worker");
        let cleanup = rollback
            .cleanup_plan()
            .expect("route-backed rollback should include binding cleanup");
        assert_eq!(cleanup.session_id(), "SID-worker");
        assert_eq!(cleanup.browser_context_id(), Some("BID-1"));
        assert_eq!(
            cleanup.target(),
            &SessionDisposalTarget::SharedWorkerTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-worker".to_owned()
            }
        );
    }

    #[test]
    fn target_auto_attached_session_detach_plan_preserves_route_cleanup_boundary() {
        let plan = TargetAutoAttachedSessionDetachPlan::from_session_route(
            "SID-worker",
            Some(CdpSessionRoute::SharedWorkerTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-worker".to_owned(),
            }),
        );

        assert_eq!(plan.session_id(), "SID-worker");
        let cleanup = plan
            .cleanup_plan()
            .expect("auto-attached worker detach should include binding cleanup");
        assert_eq!(cleanup.session_id(), "SID-worker");
        assert_eq!(cleanup.browser_context_id(), Some("BID-1"));
        assert_eq!(
            cleanup.target(),
            &SessionDisposalTarget::SharedWorkerTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-worker".to_owned()
            }
        );
    }

    #[test]
    fn target_auto_attached_session_detach_plan_rolls_back_missing_or_browser_route() {
        for route in [None, Some(CdpSessionRoute::Browser)] {
            let plan =
                TargetAutoAttachedSessionDetachPlan::from_session_route("SID-browser", route);
            assert_eq!(plan.session_id(), "SID-browser");
            assert!(plan.cleanup_plan().is_none());
        }
    }

    #[test]
    fn prepared_target_host_delta_preserves_delta_and_snapshot() {
        let target_info = DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from("TID-worker")),
            kind: DevToolsTargetKind::SharedWorker,
            title: "worker".to_owned(),
            url: "https://example.test/worker.js".to_owned(),
            attached: false,
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: None,
            moli_popup_id: None,
        };
        let prepared = PreparedTargetHostDelta::created("TID-worker", Some(target_info.clone()));

        assert_eq!(prepared.target_id(), "TID-worker");
        let (delta, snapshot) = prepared.into_parts();
        assert_eq!(delta.target_id(), "TID-worker");
        assert_eq!(snapshot, Some(target_info));
    }
}
