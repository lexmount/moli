use super::{
    ChildBrowsingContextBootstrap, ChildBrowsingContextEntry, ChildBrowsingContextSnapshot,
    NavigationHistoryEntrySeed, NavigationHistorySerializedEntry, WindowAccessOrigin,
};
use crate::document_runtime::DocumentSandboxPolicy;
use std::sync::Arc;
use url::Url;

/// A local-scheme resource belongs to its historical Document. It cannot be
/// reconstructed from a mutable owner attribute or a possibly revoked blob URL.
/// Same-document entries share the original source,
/// fallback base URL and policy, while each entry retains its own URL/state.
#[derive(Debug)]
pub(super) struct LocalHistoryResource {
    snapshot: ChildBrowsingContextSnapshot,
    origin: WindowAccessOrigin,
}

fn current_entry(seed: &NavigationHistoryEntrySeed) -> Option<&NavigationHistorySerializedEntry> {
    seed.entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
}

impl ChildBrowsingContextEntry {
    pub(in crate::native_bridge::context_host) fn local_history_bootstrap(
        &self,
        seed: &NavigationHistoryEntrySeed,
    ) -> Option<ChildBrowsingContextBootstrap> {
        let entry = current_entry(seed)?;
        let resource = self.local_history.get(&entry.document_id)?;
        if resource.snapshot.url.scheme() == "about" && resource.snapshot.url.path() == "srcdoc" {
            Some(ChildBrowsingContextBootstrap::Srcdoc {
                base_url: resource.snapshot.fallback_base_url.clone()?,
                markup: resource.snapshot.markup().into_owned(),
            })
        } else {
            Some(ChildBrowsingContextBootstrap::Url(
                Url::parse(&entry.url).ok()?,
            ))
        }
    }

    pub(in crate::native_bridge::context_host) fn pending_local_history_snapshot(
        &self,
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> Option<ChildBrowsingContextSnapshot> {
        let target =
            super::super::JsContextHost::child_browsing_context_navigation_entry_url(bootstrap)?;
        if !is_local_history_url(&target) {
            return None;
        }
        let entry = current_entry(&self.navigation_entry_seed)?;
        let mut snapshot = self.local_history.get(&entry.document_id)?.snapshot.clone();
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

    pub(in crate::native_bridge::context_host) fn current_local_history_origin(
        &self,
    ) -> Option<&WindowAccessOrigin> {
        if !self.live_bootstrap.security_origin_inherited() {
            return None;
        }
        let entry = current_entry(&self.committed_navigation_entry_seed)?;
        Some(&self.local_history.get(&entry.document_id)?.origin)
    }

    pub(in crate::native_bridge::context_host) fn remember_local_history_resource(
        &mut self,
        snapshot: &ChildBrowsingContextSnapshot,
        origin: WindowAccessOrigin,
    ) {
        if !is_local_history_url(&snapshot.url) {
            return;
        }
        let Some(entry) = current_entry(&self.committed_navigation_entry_seed) else {
            return;
        };
        self.local_history
            .entry(entry.document_id.clone())
            .or_insert_with(|| {
                Arc::new(LocalHistoryResource {
                    snapshot: snapshot.clone(),
                    origin,
                })
            });
    }

    pub(super) fn prune_local_history(&mut self) {
        self.local_history.retain(|document_id, _| {
            self.committed_navigation_entry_seed
                .entries
                .iter()
                .any(|entry| &entry.document_id == document_id)
        });
    }
}

fn is_local_history_url(url: &Url) -> bool {
    matches!(url.scheme(), "blob" | "data")
        || (url.scheme() == "about" && matches!(url.path(), "blank" | "srcdoc"))
}
