use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetHostDelta {
    Created { target_id: String },
    InfoChanged { target_id: String },
    Destroyed { target_id: String },
}

impl TargetHostDelta {
    pub(crate) fn created(target_id: impl Into<String>) -> Self {
        Self::Created {
            target_id: target_id.into(),
        }
    }

    pub(crate) fn info_changed(target_id: impl Into<String>) -> Self {
        Self::InfoChanged {
            target_id: target_id.into(),
        }
    }

    pub(crate) fn destroyed(target_id: impl Into<String>) -> Self {
        Self::Destroyed {
            target_id: target_id.into(),
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        match self {
            Self::Created { target_id }
            | Self::InfoChanged { target_id }
            | Self::Destroyed { target_id } => target_id,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TargetSessionSet {
    primary_session_id: Option<String>,
    attached_session_ids: HashSet<String>,
}

impl TargetSessionSet {
    pub(crate) fn has_session(&self) -> bool {
        self.primary_session_id.is_some() || !self.attached_session_ids.is_empty()
    }

    fn primary_session_id(&self) -> Option<&str> {
        self.primary_session_id.as_deref()
    }

    fn insert_session(&mut self, session_id: String, is_attached_session: bool) {
        if !is_attached_session && self.primary_session_id.is_none() {
            self.primary_session_id = Some(session_id);
        } else {
            self.attached_session_ids.insert(session_id);
        }
    }

    fn remove_session(&mut self, session_id: &str) -> bool {
        if self.primary_session_id.as_deref() == Some(session_id) {
            self.primary_session_id = None;
            return true;
        }
        self.attached_session_ids.remove(session_id)
    }

    fn session_ids(&self) -> Vec<String> {
        self.primary_session_id
            .iter()
            .cloned()
            .chain(self.attached_session_ids.iter().cloned())
            .collect()
    }
}

/// One stable browser tab and its current top-level page target.
///
/// Moli keeps the same page target across ordinary document navigations and
/// does not yet expose prerender page targets, so the supported relationship
/// is intentionally one-to-one. The tab identity is still independent: it is
/// the durable browser surface and owns its own DevTools sessions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabTarget {
    tab_target_id: String,
    primary_page_target_id: String,
    sessions: TargetSessionSet,
}

impl TabTarget {
    fn new(tab_target_id: String, primary_page_target_id: String) -> Self {
        Self {
            tab_target_id,
            primary_page_target_id,
            sessions: TargetSessionSet::default(),
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.tab_target_id
    }

    pub(crate) fn primary_page_target_id(&self) -> &str {
        &self.primary_page_target_id
    }

    pub(crate) fn session_ids(&self) -> Vec<String> {
        self.sessions.session_ids()
    }

    pub(crate) fn has_session(&self) -> bool {
        self.sessions.has_session()
    }

    /// Chromium publishes a WebContents-backed Tab host before its primary
    /// RenderFrame-backed Page host becomes observable.
    pub(crate) fn target_ids_in_creation_order(&self) -> [&str; 2] {
        [self.id(), self.primary_page_target_id()]
    }

    /// The primary Page host goes away before the WebContents-backed Tab host.
    pub(crate) fn target_ids_in_destruction_order(&self) -> [&str; 2] {
        [self.primary_page_target_id(), self.id()]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetClosurePlan {
    tab_target: TabTarget,
    deltas: Vec<TargetHostDelta>,
}

impl TargetClosurePlan {
    pub(super) fn from_tab_target(target: TabTarget) -> Self {
        let deltas = target
            .target_ids_in_destruction_order()
            .into_iter()
            .map(TargetHostDelta::destroyed)
            .collect();
        Self {
            tab_target: target,
            deltas,
        }
    }

    pub(crate) fn tab_target(&self) -> &TabTarget {
        &self.tab_target
    }

    pub(crate) fn destroyed_target_ids(&self) -> impl Iterator<Item = &str> {
        self.deltas.iter().filter_map(|delta| match delta {
            TargetHostDelta::Destroyed { target_id } => Some(target_id.as_str()),
            TargetHostDelta::Created { .. } | TargetHostDelta::InfoChanged { .. } => None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TargetGraph {
    tabs: HashMap<String, TabTarget>,
    page_to_tab: HashMap<String, String>,
    tab_session_to_tab: HashMap<String, String>,
}

impl TargetGraph {
    pub(crate) fn register_tab(&mut self, tab_target_id: String, primary_page_target_id: String) {
        self.remove_tab_by_page_target_id(&primary_page_target_id);
        self.remove_tab_by_target_id(&tab_target_id);
        self.page_to_tab
            .insert(primary_page_target_id.clone(), tab_target_id.clone());
        self.tabs.insert(
            tab_target_id.clone(),
            TabTarget::new(tab_target_id, primary_page_target_id),
        );
    }

    pub(crate) fn remove_tab_by_page_target_id(
        &mut self,
        page_target_id: &str,
    ) -> Option<TabTarget> {
        let tab_target_id = self.page_to_tab.get(page_target_id)?.clone();
        self.remove_tab_by_target_id(&tab_target_id)
    }

    pub(crate) fn remove_tab_by_target_id(&mut self, tab_target_id: &str) -> Option<TabTarget> {
        let target = self.tabs.remove(tab_target_id)?;
        for session_id in target.sessions.session_ids() {
            self.tab_session_to_tab.remove(&session_id);
        }
        self.page_to_tab.remove(target.primary_page_target_id());
        Some(target)
    }

    pub(crate) fn tab_target_id_for_page_target_id(&self, page_target_id: &str) -> Option<&str> {
        self.page_to_tab.get(page_target_id).map(String::as_str)
    }

    pub(crate) fn primary_page_target_id_for_tab_target_id(
        &self,
        tab_target_id: &str,
    ) -> Option<&str> {
        self.tabs
            .get(tab_target_id)
            .map(TabTarget::primary_page_target_id)
    }

    pub(crate) fn primary_session_id_for_tab_target_id(&self, tab_target_id: &str) -> Option<&str> {
        self.tabs.get(tab_target_id)?.sessions.primary_session_id()
    }

    pub(crate) fn assign_session_to_tab_target(
        &mut self,
        tab_target_id: &str,
        session_id: String,
        is_attached_session: bool,
    ) -> bool {
        self.remove_tab_session(&session_id);
        let Some(target) = self.tabs.get_mut(tab_target_id) else {
            return false;
        };
        target
            .sessions
            .insert_session(session_id.clone(), is_attached_session);
        self.tab_session_to_tab
            .insert(session_id, tab_target_id.to_owned());
        true
    }

    pub(crate) fn remove_tab_session(&mut self, session_id: &str) -> Option<String> {
        let tab_target_id = self.tab_session_to_tab.remove(session_id)?;
        let target = self.tabs.get_mut(&tab_target_id)?;
        target.sessions.remove_session(session_id);
        Some(tab_target_id)
    }

    pub(crate) fn tab_target_id_for_session_id(&self, session_id: &str) -> Option<&str> {
        self.tab_session_to_tab.get(session_id).map(String::as_str)
    }

    pub(crate) fn tab_for_page_target_id(&self, page_target_id: &str) -> Option<&TabTarget> {
        let tab_target_id = self.page_to_tab.get(page_target_id)?;
        self.tabs.get(tab_target_id)
    }

    pub(crate) fn tab(&self, tab_target_id: &str) -> Option<&TabTarget> {
        self.tabs.get(tab_target_id)
    }

    pub(crate) fn contains_target_id(&self, target_id: &str) -> bool {
        self.tabs.contains_key(target_id) || self.page_to_tab.contains_key(target_id)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.tabs.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::TargetGraph;

    #[test]
    fn target_graph_registers_stable_tab_with_primary_page() {
        let mut graph = TargetGraph::default();
        graph.register_tab("TAB-1".to_owned(), "TID-page".to_owned());

        assert_eq!(graph.len(), 1);
        assert_eq!(
            graph.tab_target_id_for_page_target_id("TID-page"),
            Some("TAB-1")
        );
        assert_eq!(
            graph.primary_page_target_id_for_tab_target_id("TAB-1"),
            Some("TID-page")
        );
        let tab = graph
            .tab_for_page_target_id("TID-page")
            .expect("tab target");
        assert_eq!(tab.id(), "TAB-1");
        assert_eq!(tab.primary_page_target_id(), "TID-page");
        assert!(!tab.has_session());
    }

    #[test]
    fn target_graph_rekey_removes_stale_reverse_entries() {
        let mut graph = TargetGraph::default();
        graph.register_tab("TAB-a".to_owned(), "TID-a".to_owned());
        graph.register_tab("TAB-b".to_owned(), "TID-a".to_owned());

        assert_eq!(graph.len(), 1);
        assert_eq!(
            graph.tab_target_id_for_page_target_id("TID-a"),
            Some("TAB-b")
        );
        assert_eq!(
            graph.primary_page_target_id_for_tab_target_id("TAB-a"),
            None
        );

        graph.register_tab("TAB-b".to_owned(), "TID-b".to_owned());
        assert_eq!(graph.len(), 1);
        assert_eq!(graph.tab_target_id_for_page_target_id("TID-a"), None);
        assert_eq!(
            graph.tab_target_id_for_page_target_id("TID-b"),
            Some("TAB-b")
        );
        assert_eq!(
            graph.primary_page_target_id_for_tab_target_id("TAB-b"),
            Some("TID-b")
        );
    }

    #[test]
    fn target_graph_removes_tab_from_either_identity() {
        let mut graph = TargetGraph::default();
        graph.register_tab("TAB-1".to_owned(), "TID-page".to_owned());

        let removed = graph.remove_tab_by_target_id("TAB-1").expect("removed tab");
        assert_eq!(removed.primary_page_target_id(), "TID-page");
        assert!(graph.is_empty());
        assert_eq!(graph.tab_target_id_for_page_target_id("TID-page"), None);
        assert_eq!(
            graph.primary_page_target_id_for_tab_target_id("TAB-1"),
            None
        );
    }

    #[test]
    fn target_graph_exposes_chromium_host_lifecycle_order() {
        let mut graph = TargetGraph::default();
        graph.register_tab("TAB-1".to_owned(), "TID-page".to_owned());
        let target = graph.tab("TAB-1").expect("tab target");

        assert_eq!(target.target_ids_in_creation_order(), ["TAB-1", "TID-page"]);
        assert_eq!(
            target.target_ids_in_destruction_order(),
            ["TID-page", "TAB-1"]
        );
    }
}
