use super::{
    ChildBrowsingContextBootstrap, ChildBrowsingContextEntry, ChildBrowsingContextSnapshot,
    NavigationHistoryEntrySeed, NavigationHistorySerializedEntry, WindowAccessOrigin,
};
use crate::document_runtime::DocumentSandboxPolicy;
use std::sync::Arc;
use url::Url;

/// The resource belongs to a historical Document, not to its owner element's
/// current srcdoc attribute. Same-document entries share the original source,
/// fallback base URL and policy, while each entry retains its own URL/state.
#[derive(Debug)]
pub(super) struct SrcdocHistoryResource {
    snapshot: ChildBrowsingContextSnapshot,
    origin: WindowAccessOrigin,
}

fn current_entry(seed: &NavigationHistoryEntrySeed) -> Option<&NavigationHistorySerializedEntry> {
    seed.entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
}

impl ChildBrowsingContextEntry {
    pub(in crate::native_bridge::context_host) fn srcdoc_history_bootstrap(
        &self,
        seed: &NavigationHistoryEntrySeed,
    ) -> Option<ChildBrowsingContextBootstrap> {
        let resource = self.srcdoc_history.get(&current_entry(seed)?.document_id)?;
        Some(ChildBrowsingContextBootstrap::Srcdoc {
            base_url: resource.snapshot.fallback_base_url.clone()?,
            markup: resource.snapshot.markup.clone(),
        })
    }

    pub(in crate::native_bridge::context_host) fn pending_srcdoc_history_snapshot(
        &self,
    ) -> Option<ChildBrowsingContextSnapshot> {
        let entry = current_entry(&self.navigation_entry_seed)?;
        let mut snapshot = self
            .srcdoc_history
            .get(&entry.document_id)?
            .snapshot
            .clone();
        snapshot.url = Url::parse(&entry.url).ok()?;
        // The iframe's current sandbox attribute is applied at commit. Only
        // CSP restrictions belong to the historical policy container; keeping
        // the old owner flags would prevent a later relaxation from taking
        // effect when the historical Document is reconstructed.
        snapshot.policy_container.sandbox =
            DocumentSandboxPolicy::from_response_content_security_policies(
                &snapshot.policy_container.response_content_security_policies,
            );
        Some(snapshot)
    }

    pub(in crate::native_bridge::context_host) fn current_srcdoc_history_origin(
        &self,
    ) -> Option<&WindowAccessOrigin> {
        if !matches!(
            self.live_bootstrap,
            ChildBrowsingContextBootstrap::Srcdoc { .. }
        ) {
            return None;
        }
        let entry = current_entry(&self.committed_navigation_entry_seed)?;
        Some(&self.srcdoc_history.get(&entry.document_id)?.origin)
    }

    pub(in crate::native_bridge::context_host) fn remember_srcdoc_history_resource(
        &mut self,
        snapshot: &ChildBrowsingContextSnapshot,
        origin: WindowAccessOrigin,
    ) {
        if !matches!(
            self.live_bootstrap,
            ChildBrowsingContextBootstrap::Srcdoc { .. }
        ) {
            return;
        }
        let Some(entry) = current_entry(&self.committed_navigation_entry_seed) else {
            return;
        };
        self.srcdoc_history
            .entry(entry.document_id.clone())
            .or_insert_with(|| {
                Arc::new(SrcdocHistoryResource {
                    snapshot: snapshot.clone(),
                    origin,
                })
            });
    }

    pub(super) fn prune_srcdoc_history(&mut self) {
        self.srcdoc_history.retain(|document_id, _| {
            self.committed_navigation_entry_seed
                .entries
                .iter()
                .any(|entry| &entry.document_id == document_id)
        });
    }
}
