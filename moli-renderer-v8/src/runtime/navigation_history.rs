use std::sync::Arc;

use anyhow::{Result, anyhow};
use moli_page_types::{
    NavigationHistoryMutation, cross_document_navigation_seed, reload_navigation_seed,
    traversal_navigation_seed_candidate,
};
use parking_lot::Mutex;
use url::Url;

use crate::native_bridge::{NavigationHistoryEntrySeed, NavigationHistorySerializedEntry};

/// The committed history of a renderer's top-level Document. Publishing a
/// snapshot happens at a history mutation, never while capturing Page state.
/// Browser navigation can read this handle without entering V8, including
/// state updates made while the destination request is waiting for a response.
#[derive(Clone, Debug, Default)]
pub struct RendererNavigationHistory {
    snapshot: Arc<Mutex<Option<Arc<NavigationHistoryEntrySeed>>>>,
}

impl PartialEq for RendererNavigationHistory {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.snapshot, &other.snapshot)
    }
}

impl Eq for RendererNavigationHistory {}

impl RendererNavigationHistory {
    pub(crate) fn publish(&self, seed: NavigationHistoryEntrySeed) {
        *self.snapshot.lock() = Some(Arc::new(seed));
    }

    fn snapshot(&self) -> Option<Arc<NavigationHistoryEntrySeed>> {
        self.snapshot.lock().clone()
    }

    pub fn navigate(
        &self,
        url: &Url,
        mutation: NavigationHistoryMutation,
    ) -> Option<RendererNavigationHistoryRequest> {
        let source = self.snapshot()?;
        let current = current_entry(&source)?;
        let seed = cross_document_navigation_seed(
            source.entries.clone(),
            source.current_index,
            current.index,
            url,
            mutation,
        );
        Some(self.request(seed))
    }

    pub fn reload(&self) -> Option<RendererNavigationHistoryRequest> {
        let source = self.snapshot()?;
        Some(self.request(reload_navigation_seed(
            source.entries.clone(),
            source.current_index,
        )?))
    }

    pub fn traverse(&self, delta: i64) -> Option<RendererNavigationHistoryRequest> {
        let source = self.snapshot()?;
        let target_index =
            u32::try_from(i64::from(source.current_index).checked_add(delta)?).ok()?;
        let candidate = traversal_navigation_seed_candidate(
            source.entries.clone(),
            source.current_index,
            target_index,
        )?;
        Some(self.request(candidate.seed))
    }

    pub fn traversal_is_same_document(&self, delta: i64) -> Option<bool> {
        let source = self.snapshot()?;
        let target_index =
            u32::try_from(i64::from(source.current_index).checked_add(delta)?).ok()?;
        let current = current_entry(&source)?;
        let target = source
            .entries
            .iter()
            .find(|entry| entry.history_index == target_index)?;
        Some(current.document_id == target.document_id)
    }

    pub(crate) fn request(
        &self,
        seed: NavigationHistoryEntrySeed,
    ) -> RendererNavigationHistoryRequest {
        let source_index = self.snapshot().map(|source| source.current_index);
        RendererNavigationHistoryRequest {
            source: self.clone(),
            requested: Arc::new(seed),
            source_index,
            initial_empty_source: false,
        }
    }
}

/// One frozen navigation operation with a live source-history handle. The
/// request retains its destination identity/state while reading the latest
/// committed source entries at Document creation. Equality compares the same
/// captured request, not the contents of JavaScript structured-clone payloads.
#[derive(Clone, Debug)]
pub struct RendererNavigationHistoryRequest {
    source: RendererNavigationHistory,
    requested: Arc<NavigationHistoryEntrySeed>,
    source_index: Option<u32>,
    initial_empty_source: bool,
}

impl PartialEq for RendererNavigationHistoryRequest {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.requested, &other.requested)
            && self.initial_empty_source == other.initial_empty_source
    }
}

impl Eq for RendererNavigationHistoryRequest {}

impl RendererNavigationHistoryRequest {
    /// The browser owns the initial-empty Document lifecycle. Its first
    /// navigation has no activation source. Same-document URL updates do not
    /// change that lifecycle state.
    pub fn with_initial_empty_source(mut self) -> Self {
        self.initial_empty_source = true;
        self
    }

    fn finish_seed(
        &self,
        mut seed: NavigationHistoryEntrySeed,
        final_url: &Url,
    ) -> NavigationHistoryEntrySeed {
        // A new top-level Document rebuilds its joint history from this list.
        // Bootstrap snapshots may give about:blank and its successor the same
        // Navigation ordinal. Retained browser entries need distinct steps,
        // otherwise forward traversal can skip that successor. Public entry
        // indices are computed separately from the exposed same-origin list.
        for entry in &mut seed.entries {
            entry.index = entry.history_index;
        }
        if let Some(from) = seed
            .activation
            .as_mut()
            .and_then(|activation| activation.from.as_mut())
        {
            from.index = from.history_index;
        }
        update_destination_url(&mut seed, final_url);
        if self.initial_empty_source
            && let Some(activation) = &mut seed.activation
        {
            activation.from = None;
        }
        seed
    }

    pub fn navigation_type(&self) -> Option<&str> {
        self.requested
            .activation
            .as_ref()?
            .navigation_type
            .as_deref()
    }

    pub fn traversal_delta(&self) -> Option<i64> {
        if self.navigation_type() != Some("traverse") {
            return None;
        }
        Some(i64::from(self.requested.current_index) - i64::from(self.source_index?))
    }

    pub(crate) fn resolve(&self, final_url: &Url) -> Result<NavigationHistoryEntrySeed> {
        let Some(source) = self.source.snapshot() else {
            return Ok(self.finish_seed(self.requested.as_ref().clone(), final_url));
        };
        let requested = current_entry(&self.requested)
            .ok_or_else(|| anyhow!("missing navigation history destination"))?;
        let current =
            current_entry(&source).ok_or_else(|| anyhow!("missing navigation history source"))?;
        let seed = match self.navigation_type() {
            Some("push" | "replace") => {
                let mutation = if self.navigation_type() == Some("replace") {
                    NavigationHistoryMutation::Replace
                } else {
                    NavigationHistoryMutation::Push
                };
                let mut seed = cross_document_navigation_seed(
                    source.entries.clone(),
                    source.current_index,
                    current.index,
                    final_url,
                    mutation,
                );
                let entry = seed
                    .entries
                    .iter_mut()
                    .find(|entry| entry.history_index == seed.current_index)
                    .expect("a new navigation has a destination entry");
                entry.id = requested.id.clone();
                entry.document_id = requested.document_id.clone();
                if mutation == NavigationHistoryMutation::Push {
                    entry.key = requested.key.clone();
                }
                entry.history_state = requested.history_state.clone();
                entry.navigation_state = requested.navigation_state.clone();
                seed
            }
            Some("reload") => reload_navigation_seed(source.entries.clone(), source.current_index)
                .ok_or_else(|| anyhow!("missing reload history entry"))?,
            Some("traverse") => {
                let target = source
                    .entries
                    .iter()
                    .find(|entry| entry.key == requested.key)
                    .ok_or_else(|| anyhow!("history traversal destination was removed"))?;
                traversal_navigation_seed_candidate(
                    source.entries.clone(),
                    source.current_index,
                    target.history_index,
                )
                .ok_or_else(|| {
                    anyhow!("history traversal destination no longer requires a new Document")
                })?
                .seed
            }
            _ => self.requested.as_ref().clone(),
        };
        Ok(self.finish_seed(seed, final_url))
    }
}

fn current_entry(seed: &NavigationHistoryEntrySeed) -> Option<&NavigationHistorySerializedEntry> {
    seed.entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
}

fn update_destination_url(seed: &mut NavigationHistoryEntrySeed, final_url: &Url) {
    if let Some(entry) = seed
        .entries
        .iter_mut()
        .find(|entry| entry.history_index == seed.current_index)
    {
        entry.url = final_url.to_string();
        if let Some(activation) = &mut seed.activation {
            activation.entry = entry.clone();
        }
    }
}
