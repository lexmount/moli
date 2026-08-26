use std::collections::HashMap;

use super::graph::{TabTarget, TargetGraph};
use super::host::TargetHost;
use crate::devtools_runtime::DevToolsTargetKind;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetClosurePlan {
    tab_target: TabTarget,
    deltas: Vec<TargetHostDelta>,
}

impl TargetClosurePlan {
    fn from_tab_target(target: TabTarget) -> Self {
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
pub(crate) struct TargetRegistry {
    hosts: HashMap<String, TargetHost>,
    graph: TargetGraph,
}

impl TargetRegistry {
    pub(crate) fn register_tab(&mut self, tab_target_id: String, primary_page_target_id: String) {
        self.remove_tab_by_page_target_id(&primary_page_target_id);
        self.remove_tab_by_target_id(&tab_target_id);
        self.hosts.insert(
            tab_target_id.clone(),
            TargetHost::new(tab_target_id.clone(), DevToolsTargetKind::Tab),
        );
        self.hosts.insert(
            primary_page_target_id.clone(),
            TargetHost::new(primary_page_target_id.clone(), DevToolsTargetKind::Page),
        );
        self.graph
            .register_tab(tab_target_id, primary_page_target_id);
    }

    pub(crate) fn register_worker(&mut self, target_id: String, kind: DevToolsTargetKind) {
        debug_assert!(matches!(
            kind,
            DevToolsTargetKind::Worker
                | DevToolsTargetKind::SharedWorker
                | DevToolsTargetKind::ServiceWorker
        ));
        self.hosts
            .insert(target_id.clone(), TargetHost::new(target_id, kind));
    }

    pub(crate) fn remove_worker(&mut self, target_id: &str) -> Option<TargetHost> {
        let host = self.hosts.get(target_id)?;
        if !matches!(
            host.kind(),
            DevToolsTargetKind::Worker
                | DevToolsTargetKind::SharedWorker
                | DevToolsTargetKind::ServiceWorker
        ) {
            return None;
        }
        self.hosts.remove(target_id)
    }

    pub(crate) fn remove_tab_by_page_target_id(
        &mut self,
        page_target_id: &str,
    ) -> Option<TargetClosurePlan> {
        let target = self.graph.remove_tab_by_page_target_id(page_target_id)?;
        self.remove_tab_hosts(&target);
        Some(TargetClosurePlan::from_tab_target(target))
    }

    pub(crate) fn remove_tab_by_target_id(
        &mut self,
        tab_target_id: &str,
    ) -> Option<TargetClosurePlan> {
        let target = self.graph.remove_tab_by_target_id(tab_target_id)?;
        self.remove_tab_hosts(&target);
        Some(TargetClosurePlan::from_tab_target(target))
    }

    fn remove_tab_hosts(&mut self, target: &TabTarget) {
        self.hosts.remove(target.id());
        self.hosts.remove(target.primary_page_target_id());
    }

    pub(crate) fn tab_target_id_for_page_target_id(&self, page_target_id: &str) -> Option<&str> {
        let host = self.hosts.get(page_target_id)?;
        debug_assert_eq!(host.id(), page_target_id);
        if host.kind() != DevToolsTargetKind::Page {
            return None;
        }
        self.graph.tab_target_id_for_page_target_id(page_target_id)
    }

    pub(crate) fn primary_page_target_id_for_tab_target_id(
        &self,
        tab_target_id: &str,
    ) -> Option<&str> {
        let host = self.hosts.get(tab_target_id)?;
        debug_assert_eq!(host.id(), tab_target_id);
        if host.kind() != DevToolsTargetKind::Tab {
            return None;
        }
        self.graph
            .primary_page_target_id_for_tab_target_id(tab_target_id)
    }

    pub(crate) fn primary_session_id_for_tab_target_id(&self, tab_target_id: &str) -> Option<&str> {
        self.graph
            .primary_session_id_for_tab_target_id(tab_target_id)
    }

    pub(crate) fn assign_session_to_tab_target(
        &mut self,
        tab_target_id: &str,
        session_id: String,
        auxiliary: bool,
    ) -> bool {
        if self
            .primary_page_target_id_for_tab_target_id(tab_target_id)
            .is_none()
        {
            return false;
        }
        self.graph
            .assign_session_to_tab_target(tab_target_id, session_id, auxiliary)
    }

    pub(crate) fn remove_tab_session(&mut self, session_id: &str) -> Option<String> {
        self.graph.remove_tab_session(session_id)
    }

    pub(crate) fn tab_target_id_for_session_id(&self, session_id: &str) -> Option<&str> {
        self.graph.tab_target_id_for_session_id(session_id)
    }

    pub(crate) fn tab_for_page_target_id(&self, page_target_id: &str) -> Option<&TabTarget> {
        self.graph.tab_for_page_target_id(page_target_id)
    }

    pub(crate) fn tab(&self, tab_target_id: &str) -> Option<&TabTarget> {
        self.graph.tab(tab_target_id)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.graph.len()
    }

    #[cfg(test)]
    pub(crate) fn host_count(&self) -> usize {
        self.hosts.len()
    }

    pub(crate) fn host(&self, target_id: &str) -> Option<&TargetHost> {
        self.hosts.get(target_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::devtools_runtime::DevToolsTargetKind;

    use super::TargetRegistry;

    #[test]
    fn target_registry_registers_distinct_page_and_tab_hosts() {
        let mut registry = TargetRegistry::default();
        registry.register_tab("TAB-1".to_owned(), "TID-page".to_owned());

        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry.tab_target_id_for_page_target_id("TID-page"),
            Some("TAB-1")
        );
        assert_eq!(
            registry.primary_page_target_id_for_tab_target_id("TAB-1"),
            Some("TID-page")
        );

        let page = registry.host("TID-page").expect("page host");
        assert_eq!(page.id(), "TID-page");
        assert_eq!(page.kind(), DevToolsTargetKind::Page);

        let tab = registry.host("TAB-1").expect("tab host");
        assert_eq!(tab.id(), "TAB-1");
        assert_eq!(tab.kind(), DevToolsTargetKind::Tab);
    }

    #[test]
    fn target_registry_removes_page_and_tab_hosts_together() {
        let mut registry = TargetRegistry::default();
        registry.register_tab("TAB-1".to_owned(), "TID-page".to_owned());

        let removed = registry
            .remove_tab_by_target_id("TAB-1")
            .expect("removed tab target");
        assert_eq!(removed.tab_target().primary_page_target_id(), "TID-page");
        assert_eq!(removed.tab_target().id(), "TAB-1");
        assert_eq!(
            removed.destroyed_target_ids().collect::<Vec<_>>(),
            vec!["TID-page", "TAB-1"]
        );
        assert_eq!(registry.host("TID-page"), None);
        assert_eq!(registry.host("TAB-1"), None);
        assert_eq!(registry.tab_target_id_for_page_target_id("TID-page"), None);
        assert_eq!(
            registry.primary_page_target_id_for_tab_target_id("TAB-1"),
            None
        );
    }

    #[test]
    fn target_registry_registers_and_removes_worker_host_without_tab_graph() {
        let mut registry = TargetRegistry::default();
        registry.register_worker(
            "TID-shared-worker".to_owned(),
            DevToolsTargetKind::SharedWorker,
        );

        assert_eq!(registry.len(), 0);
        assert_eq!(registry.host_count(), 1);
        let host = registry.host("TID-shared-worker").expect("worker host");
        assert_eq!(host.id(), "TID-shared-worker");
        assert_eq!(host.kind(), DevToolsTargetKind::SharedWorker);

        let removed = registry
            .remove_worker("TID-shared-worker")
            .expect("removed worker host");
        assert_eq!(removed.id(), "TID-shared-worker");
        assert_eq!(registry.host("TID-shared-worker"), None);
        assert_eq!(registry.host_count(), 0);
    }
}
