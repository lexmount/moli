use std::collections::{HashMap, HashSet};

use crate::conn::CdpSessionRoute;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedAttachSession {
    session_id: String,
    owner_session_id: Option<String>,
    target_id: String,
    route: Option<CdpSessionRoute>,
    auto_attached: bool,
    waiting_for_debugger: bool,
}

impl PreparedAttachSession {
    pub(crate) fn new(
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: &str,
        route: Option<CdpSessionRoute>,
        auto_attached: bool,
        waiting_for_debugger: bool,
    ) -> Self {
        Self {
            session_id,
            owner_session_id: owner_session_id.map(str::to_owned),
            target_id: target_id.to_owned(),
            route,
            auto_attached,
            waiting_for_debugger,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommittedAttachSession {
    session_id: String,
    owner_session_id: Option<String>,
    target_id: String,
    route: Option<CdpSessionRoute>,
    auto_attached: bool,
    waiting_for_debugger: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetachedTargetSession {
    session_id: String,
    owner_session_id: Option<String>,
    target_id: String,
    route: Option<CdpSessionRoute>,
    auto_attached: bool,
    waiting_for_debugger: bool,
}

impl DetachedTargetSession {
    pub(crate) fn from_detached_binding(session_id: &str, target_id: &str) -> Self {
        Self {
            session_id: session_id.to_owned(),
            owner_session_id: None,
            target_id: target_id.to_owned(),
            route: None,
            auto_attached: false,
            waiting_for_debugger: false,
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    #[cfg(test)]
    pub(crate) fn owner_session_id(&self) -> Option<&str> {
        self.owner_session_id.as_deref()
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    #[cfg(test)]
    pub(crate) fn route(&self) -> Option<&CdpSessionRoute> {
        self.route.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn auto_attached(&self) -> bool {
        self.auto_attached
    }

    pub(crate) fn was_waiting_for_debugger(&self) -> bool {
        self.waiting_for_debugger
    }
}

#[cfg(test)]
impl CommittedAttachSession {
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn owner_session_id(&self) -> Option<&str> {
        self.owner_session_id.as_deref()
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn route(&self) -> Option<&CdpSessionRoute> {
        self.route.as_ref()
    }

    pub(crate) fn auto_attached(&self) -> bool {
        self.auto_attached
    }

    pub(crate) fn waiting_for_debugger(&self) -> bool {
        self.waiting_for_debugger
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedAutoAttachSession {
    session_id: String,
    owner_session_id: Option<String>,
    target_id: Option<String>,
}

#[cfg(test)]
impl PreparedAutoAttachSession {
    pub(crate) fn new(
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: Option<&str>,
    ) -> Self {
        Self {
            session_id,
            owner_session_id: owner_session_id.map(str::to_owned),
            target_id: target_id.map(str::to_owned),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommittedAutoAttachSession {
    session_id: String,
    owner_session_id: Option<String>,
    target_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AutoAttachedTargetSession {
    owner_session_id: Option<String>,
    target_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AttachedTargetSession {
    owner_session_id: Option<String>,
    target_id: String,
    route: Option<CdpSessionRoute>,
    auto_attached: bool,
    waiting_for_debugger: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TargetSessionRegistry {
    attached_sessions: HashMap<String, AttachedTargetSession>,
    attached_sessions_by_target: HashMap<String, HashSet<String>>,
    attached_sessions_by_owner: HashMap<Option<String>, HashSet<String>>,
    auto_attached_sessions: HashMap<String, AutoAttachedTargetSession>,
    auto_attached_sessions_by_owner: HashMap<Option<String>, HashSet<String>>,
    auto_attached_sessions_by_target: HashMap<String, HashSet<String>>,
}

impl TargetSessionRegistry {
    pub(crate) fn commit_attached_session(
        &mut self,
        prepared: PreparedAttachSession,
    ) -> CommittedAttachSession {
        let session_id = prepared.session_id;
        let owner_session_id = prepared.owner_session_id;
        let target_id = prepared.target_id;
        let route = prepared.route;
        let auto_attached = prepared.auto_attached;
        let waiting_for_debugger = prepared.waiting_for_debugger;

        self.rollback_attached_session_without_event(&session_id);
        self.attached_sessions.insert(
            session_id.clone(),
            AttachedTargetSession {
                owner_session_id: owner_session_id.clone(),
                target_id: target_id.clone(),
                route: route.clone(),
                auto_attached,
                waiting_for_debugger,
            },
        );
        self.attached_sessions_by_target
            .entry(target_id.clone())
            .or_default()
            .insert(session_id.clone());
        self.attached_sessions_by_owner
            .entry(owner_session_id.clone())
            .or_default()
            .insert(session_id.clone());

        if auto_attached {
            self.index_auto_attached_session(
                session_id.clone(),
                owner_session_id.clone(),
                Some(target_id.clone()),
            );
        }

        CommittedAttachSession {
            session_id,
            owner_session_id,
            target_id,
            route,
            auto_attached,
            waiting_for_debugger,
        }
    }

    #[cfg(test)]
    pub(crate) fn commit_auto_attached_session(
        &mut self,
        prepared: PreparedAutoAttachSession,
    ) -> CommittedAutoAttachSession {
        let session_id = prepared.session_id;
        let owner_session_id = prepared.owner_session_id;
        let target_id = prepared.target_id;

        if let Some(target_id) = target_id.as_deref() {
            self.commit_attached_session(PreparedAttachSession::new(
                session_id.clone(),
                owner_session_id.as_deref(),
                target_id,
                None,
                true,
                false,
            ));
        } else {
            self.rollback_attached_session_without_event(&session_id);
            self.index_auto_attached_session(session_id.clone(), owner_session_id.clone(), None);
        }
        CommittedAutoAttachSession {
            session_id,
            owner_session_id,
            target_id,
        }
    }

    #[cfg(test)]
    pub(crate) fn register_auto_attached_session(
        &mut self,
        session_id: String,
        owner_session_id: Option<&str>,
        target_id: Option<&str>,
    ) {
        self.commit_auto_attached_session(PreparedAutoAttachSession::new(
            session_id,
            owner_session_id,
            target_id,
        ));
    }

    pub(crate) fn detach_attached_session(
        &mut self,
        session_id: &str,
    ) -> Option<DetachedTargetSession> {
        let session = self.attached_sessions.remove(session_id)?;
        remove_indexed_session(
            &mut self.attached_sessions_by_target,
            &session.target_id,
            session_id,
        );
        remove_indexed_session(
            &mut self.attached_sessions_by_owner,
            &session.owner_session_id,
            session_id,
        );
        self.clear_auto_attached_session_index(session_id);

        Some(DetachedTargetSession {
            session_id: session_id.to_owned(),
            owner_session_id: session.owner_session_id,
            target_id: session.target_id,
            route: session.route,
            auto_attached: session.auto_attached,
            waiting_for_debugger: session.waiting_for_debugger,
        })
    }

    pub(crate) fn rollback_attached_session_without_event(&mut self, session_id: &str) -> bool {
        let detached = self.detach_attached_session(session_id).is_some();
        self.clear_auto_attached_session_index(session_id) || detached
    }

    fn clear_auto_attached_session_index(&mut self, session_id: &str) -> bool {
        let Some(session) = self.auto_attached_sessions.remove(session_id) else {
            return false;
        };
        remove_indexed_session(
            &mut self.auto_attached_sessions_by_owner,
            &session.owner_session_id,
            session_id,
        );
        if let Some(target_id) = session.target_id {
            remove_indexed_session(
                &mut self.auto_attached_sessions_by_target,
                &target_id,
                session_id,
            );
        }
        true
    }

    fn index_auto_attached_session(
        &mut self,
        session_id: String,
        owner_session_id: Option<String>,
        target_id: Option<String>,
    ) {
        self.clear_auto_attached_session_index(&session_id);
        self.auto_attached_sessions.insert(
            session_id.clone(),
            AutoAttachedTargetSession {
                owner_session_id: owner_session_id.clone(),
                target_id: target_id.clone(),
            },
        );
        self.auto_attached_sessions_by_owner
            .entry(owner_session_id)
            .or_default()
            .insert(session_id.clone());
        if let Some(target_id) = target_id {
            self.auto_attached_sessions_by_target
                .entry(target_id)
                .or_default()
                .insert(session_id);
        }
    }

    pub(crate) fn attached_session_route(&self, session_id: &str) -> Option<&CdpSessionRoute> {
        self.attached_sessions.get(session_id)?.route.as_ref()
    }

    pub(crate) fn browser_session_count(&self) -> usize {
        self.attached_sessions
            .values()
            .filter(|session| session.route == Some(CdpSessionRoute::Browser))
            .count()
    }

    pub(crate) fn attached_sessions_for_target(&self, target_id: &str) -> Vec<String> {
        sorted_index_values(self.attached_sessions_by_target.get(target_id))
    }

    pub(crate) fn target_has_waiting_for_debugger_session(&self, target_id: &str) -> bool {
        self.attached_sessions_by_target
            .get(target_id)
            .is_some_and(|session_ids| {
                session_ids.iter().any(|session_id| {
                    self.attached_sessions
                        .get(session_id)
                        .is_some_and(|session| session.waiting_for_debugger)
                })
            })
    }

    /// Releases the debugger-on-start barrier contributed by one attached
    /// session.
    ///
    /// V8 keeps one barrier per inspector session and resumes the target only
    /// after every waiting session has run `Runtime.runIfWaitingForDebugger`
    /// (or detached). Keep that per-session transition in the attachment
    /// registry instead of treating `waitingForDebugger` as immutable event
    /// metadata.
    pub(crate) fn release_waiting_for_debugger_session(&mut self, session_id: &str) -> bool {
        let Some(session) = self.attached_sessions.get_mut(session_id) else {
            return false;
        };
        std::mem::take(&mut session.waiting_for_debugger)
    }

    pub(crate) fn attached_session_owner_session_id(&self, session_id: &str) -> Option<&str> {
        self.attached_sessions
            .get(session_id)?
            .owner_session_id
            .as_deref()
    }

    pub(crate) fn attached_session_cascade_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        let mut session_ids = Vec::new();
        let mut visited = HashSet::new();
        self.collect_attached_session_cascade(
            owner_session_id.map(str::to_owned),
            &mut visited,
            &mut session_ids,
        );
        session_ids
    }

    pub(crate) fn attached_session_cascade_for_root_frontend(&self) -> Vec<String> {
        let mut session_ids = Vec::new();
        let mut visited = HashSet::new();
        for session_id in sorted_index_values(self.attached_sessions_by_owner.get(&None)) {
            let is_root_browser_session = self
                .attached_sessions
                .get(&session_id)
                .is_some_and(|session| session.route == Some(CdpSessionRoute::Browser));
            if is_root_browser_session || !visited.insert(session_id.clone()) {
                continue;
            }
            self.collect_attached_session_cascade(
                Some(session_id.clone()),
                &mut visited,
                &mut session_ids,
            );
            session_ids.push(session_id);
        }
        session_ids
    }

    fn collect_attached_session_cascade(
        &self,
        owner_session_id: Option<String>,
        visited: &mut HashSet<String>,
        session_ids: &mut Vec<String>,
    ) {
        for session_id in
            sorted_index_values(self.attached_sessions_by_owner.get(&owner_session_id))
        {
            if !visited.insert(session_id.clone()) {
                continue;
            }
            self.collect_attached_session_cascade(Some(session_id.clone()), visited, session_ids);
            session_ids.push(session_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn auto_attached_session_target_id(&self, session_id: &str) -> Option<&str> {
        self.auto_attached_sessions
            .get(session_id)?
            .target_id
            .as_deref()
    }

    pub(crate) fn auto_attached_sessions_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        sorted_index_values(
            self.auto_attached_sessions_by_owner
                .get(&owner_session_id.map(str::to_owned)),
        )
    }

    pub(crate) fn auto_attached_owner_session_ids(&self) -> Vec<Option<String>> {
        let mut owner_session_ids = self
            .auto_attached_sessions_by_owner
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        owner_session_ids.sort();
        owner_session_ids
    }

    pub(crate) fn auto_attached_session_cascade_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        let mut session_ids = Vec::new();
        let mut visited = HashSet::new();
        self.collect_auto_attached_session_cascade(
            owner_session_id.map(str::to_owned),
            &mut visited,
            &mut session_ids,
        );
        session_ids
    }

    fn collect_auto_attached_session_cascade(
        &self,
        owner_session_id: Option<String>,
        visited: &mut HashSet<String>,
        session_ids: &mut Vec<String>,
    ) {
        for session_id in
            sorted_index_values(self.auto_attached_sessions_by_owner.get(&owner_session_id))
        {
            if !visited.insert(session_id.clone()) {
                continue;
            }
            self.collect_auto_attached_session_cascade(
                Some(session_id.clone()),
                visited,
                session_ids,
            );
            session_ids.push(session_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn auto_attached_sessions_for_target(&self, target_id: &str) -> Vec<String> {
        sorted_index_values(self.auto_attached_sessions_by_target.get(target_id))
    }

    pub(crate) fn auto_attached_target_ids_for_owner(
        &self,
        owner_session_id: Option<&str>,
    ) -> Vec<String> {
        let mut target_ids = self
            .auto_attached_sessions_for_owner(owner_session_id)
            .into_iter()
            .filter_map(|session_id| {
                self.auto_attached_sessions
                    .get(&session_id)?
                    .target_id
                    .clone()
            })
            .collect::<Vec<_>>();
        target_ids.sort();
        target_ids.dedup();
        target_ids
    }
}

fn remove_indexed_session<K>(index: &mut HashMap<K, HashSet<String>>, key: &K, session_id: &str)
where
    K: Clone + Eq + std::hash::Hash,
{
    let Some(sessions) = index.get_mut(key) else {
        return;
    };
    sessions.remove(session_id);
    if sessions.is_empty() {
        index.remove(key);
    }
}

fn sorted_index_values(values: Option<&HashSet<String>>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .flat_map(|values| values.iter().cloned())
        .collect::<Vec<_>>();
    values.sort();
    values
}

#[cfg(test)]
mod tests {
    use super::{PreparedAttachSession, PreparedAutoAttachSession, TargetSessionRegistry};
    use crate::conn::CdpSessionRoute;

    #[test]
    fn target_session_registry_indexes_auto_attached_sessions_by_owner() {
        let mut registry = TargetSessionRegistry::default();
        registry.register_auto_attached_session(
            "SID-root-child".to_owned(),
            None,
            Some("TID-root-child"),
        );
        registry.register_auto_attached_session(
            "SID-tab-child".to_owned(),
            Some("SID-tab"),
            Some("TID-tab-child"),
        );

        assert_eq!(
            registry.auto_attached_sessions_for_owner(None),
            vec!["SID-root-child".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_target_ids_for_owner(None),
            vec!["TID-root-child".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_sessions_for_owner(Some("SID-tab")),
            vec!["SID-tab-child".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_target_ids_for_owner(Some("SID-tab")),
            vec!["TID-tab-child".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_owner_session_ids(),
            vec![None, Some("SID-tab".to_owned())]
        );
    }

    #[test]
    fn target_session_registry_collects_auto_attached_owner_cascade_child_first() {
        let mut registry = TargetSessionRegistry::default();
        registry.register_auto_attached_session("SID-tab".to_owned(), None, Some("TAB-TID-page"));
        registry.register_auto_attached_session(
            "SID-page".to_owned(),
            Some("SID-tab"),
            Some("TID-page"),
        );
        registry.register_auto_attached_session(
            "SID-worker".to_owned(),
            Some("SID-page"),
            Some("TID-worker"),
        );
        registry.register_auto_attached_session(
            "SID-sibling".to_owned(),
            None,
            Some("TID-sibling"),
        );

        assert_eq!(
            registry.auto_attached_sessions_for_owner(None),
            vec!["SID-sibling".to_owned(), "SID-tab".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_session_cascade_for_owner(None),
            vec![
                "SID-sibling".to_owned(),
                "SID-worker".to_owned(),
                "SID-page".to_owned(),
                "SID-tab".to_owned()
            ]
        );
    }

    #[test]
    fn target_session_registry_collects_all_attached_owner_sessions_child_first() {
        let mut registry = TargetSessionRegistry::default();
        for (session_id, owner_session_id, target_id, auto_attached) in [
            ("SID-manual", Some("SID-browser"), "TID-page", false),
            ("SID-auto", Some("SID-browser"), "TID-worker", true),
            (
                "SID-grandchild",
                Some("SID-manual"),
                "TID-grandchild",
                false,
            ),
            ("SID-other", Some("SID-other-owner"), "TID-other", false),
        ] {
            registry.commit_attached_session(PreparedAttachSession::new(
                session_id.to_owned(),
                owner_session_id,
                target_id,
                Some(CdpSessionRoute::PageTarget {
                    browser_context_id: "BID-1".to_owned(),
                    target_id: target_id.to_owned(),
                    is_attached_session: auto_attached,
                }),
                auto_attached,
                false,
            ));
        }

        assert_eq!(
            registry.attached_session_cascade_for_owner(Some("SID-browser")),
            vec![
                "SID-auto".to_owned(),
                "SID-grandchild".to_owned(),
                "SID-manual".to_owned()
            ]
        );
        assert_eq!(
            registry.attached_session_cascade_for_owner(Some("SID-other-owner")),
            vec!["SID-other".to_owned()]
        );

        registry.detach_attached_session("SID-manual");
        assert_eq!(
            registry.attached_session_cascade_for_owner(Some("SID-browser")),
            vec!["SID-auto".to_owned()]
        );
    }

    #[test]
    fn target_session_registry_commits_prepared_auto_attached_session() {
        let mut registry = TargetSessionRegistry::default();
        let committed = registry.commit_auto_attached_session(PreparedAutoAttachSession::new(
            "SID-child".to_owned(),
            Some("SID-owner"),
            Some("TID-child"),
        ));

        assert_eq!(committed.session_id, "SID-child");
        assert_eq!(committed.owner_session_id.as_deref(), Some("SID-owner"));
        assert_eq!(committed.target_id.as_deref(), Some("TID-child"));
        assert_eq!(
            registry.auto_attached_sessions_for_owner(Some("SID-owner")),
            vec!["SID-child".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_sessions_for_target("TID-child"),
            vec!["SID-child".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_session_target_id("SID-child"),
            Some("TID-child")
        );
    }

    #[test]
    fn target_session_registry_commits_prepared_attached_session_with_route() {
        let mut registry = TargetSessionRegistry::default();
        let route = CdpSessionRoute::TabTarget {
            browser_context_id: "BID-1".to_owned(),
            tab_target_id: "TAB-TID-page".to_owned(),
        };
        let committed = registry.commit_attached_session(PreparedAttachSession::new(
            "SID-tab".to_owned(),
            Some("SID-browser"),
            "TAB-TID-page",
            Some(route.clone()),
            false,
            false,
        ));

        assert_eq!(committed.session_id, "SID-tab");
        assert_eq!(committed.owner_session_id.as_deref(), Some("SID-browser"));
        assert_eq!(committed.target_id, "TAB-TID-page");
        assert_eq!(committed.route.as_ref(), Some(&route));
        assert!(!committed.auto_attached);
        assert!(!committed.waiting_for_debugger);
        assert_eq!(
            registry.attached_sessions_for_target("TAB-TID-page"),
            vec!["SID-tab".to_owned()]
        );
        assert_eq!(registry.attached_session_route("SID-tab"), Some(&route));
        assert!(
            registry
                .auto_attached_sessions_for_owner(Some("SID-browser"))
                .is_empty()
        );
    }

    #[test]
    fn target_session_registry_commits_auto_attached_session_as_attached_session() {
        let mut registry = TargetSessionRegistry::default();
        registry.commit_attached_session(PreparedAttachSession::new(
            "SID-page".to_owned(),
            Some("SID-tab"),
            "TID-page",
            Some(CdpSessionRoute::PageTarget {
                browser_context_id: "BID-1".to_owned(),
                target_id: "TID-page".to_owned(),
                is_attached_session: false,
            }),
            true,
            true,
        ));

        assert_eq!(
            registry.attached_sessions_for_target("TID-page"),
            vec!["SID-page".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_sessions_for_owner(Some("SID-tab")),
            vec!["SID-page".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_target_ids_for_owner(Some("SID-tab")),
            vec!["TID-page".to_owned()]
        );
    }

    #[test]
    fn target_session_registry_scopes_waiting_for_debugger_to_attached_target() {
        let mut registry = TargetSessionRegistry::default();
        registry.commit_attached_session(PreparedAttachSession::new(
            "SID-waiting".to_owned(),
            Some("SID-owner"),
            "TID-waiting",
            None,
            true,
            true,
        ));
        registry.commit_attached_session(PreparedAttachSession::new(
            "SID-running".to_owned(),
            Some("SID-owner"),
            "TID-running",
            None,
            true,
            false,
        ));

        assert!(registry.target_has_waiting_for_debugger_session("TID-waiting"));
        assert!(!registry.target_has_waiting_for_debugger_session("TID-running"));
        assert!(!registry.target_has_waiting_for_debugger_session("TID-unattached"));
    }

    #[test]
    fn target_session_registry_releases_each_debugger_barrier_exactly_once() {
        let mut registry = TargetSessionRegistry::default();
        for session_id in ["SID-first", "SID-second"] {
            registry.commit_attached_session(PreparedAttachSession::new(
                session_id.to_owned(),
                Some("SID-owner"),
                "TID-page",
                None,
                true,
                true,
            ));
        }

        assert!(registry.target_has_waiting_for_debugger_session("TID-page"));
        assert!(registry.release_waiting_for_debugger_session("SID-first"));
        assert!(registry.target_has_waiting_for_debugger_session("TID-page"));
        assert!(!registry.release_waiting_for_debugger_session("SID-first"));
        assert!(registry.release_waiting_for_debugger_session("SID-second"));
        assert!(!registry.target_has_waiting_for_debugger_session("TID-page"));
        assert!(!registry.release_waiting_for_debugger_session("SID-missing"));
    }

    #[test]
    fn target_session_registry_clears_attached_session_and_auto_attached_indexes() {
        let mut registry = TargetSessionRegistry::default();
        registry.register_auto_attached_session(
            "SID-child".to_owned(),
            Some("SID-owner"),
            Some("TID-child"),
        );
        assert!(registry.rollback_attached_session_without_event("SID-child"));

        assert!(
            registry
                .auto_attached_sessions_for_owner(Some("SID-owner"))
                .is_empty()
        );
        assert!(
            registry
                .auto_attached_target_ids_for_owner(Some("SID-owner"))
                .is_empty()
        );
        assert!(
            registry
                .auto_attached_sessions_for_target("TID-child")
                .is_empty()
        );
        assert!(
            registry
                .attached_sessions_for_target("TID-child")
                .is_empty()
        );
    }

    #[test]
    fn target_session_registry_rekeys_existing_session() {
        let mut registry = TargetSessionRegistry::default();
        registry.register_auto_attached_session(
            "SID-child".to_owned(),
            Some("SID-owner-a"),
            Some("TID-a"),
        );
        registry.register_auto_attached_session(
            "SID-child".to_owned(),
            Some("SID-owner-b"),
            Some("TID-b"),
        );

        assert!(
            registry
                .auto_attached_sessions_for_owner(Some("SID-owner-a"))
                .is_empty()
        );
        assert!(
            registry
                .auto_attached_sessions_for_target("TID-a")
                .is_empty()
        );
        assert_eq!(
            registry.auto_attached_sessions_for_owner(Some("SID-owner-b")),
            vec!["SID-child".to_owned()]
        );
        assert_eq!(
            registry.auto_attached_sessions_for_target("TID-b"),
            vec!["SID-child".to_owned()]
        );
    }
}
