use super::*;
use crate::context_bootstrap::navigation_entry_public_token;
use crate::native_bridge::OwnerDispatchScope;
use moli_session_history::{
    JointSessionHistory, SessionHistoryContextId, SessionHistoryEntry, SessionHistoryStepId,
};

#[derive(Clone, Debug)]
pub(crate) struct NavigableHistory {
    context: SessionHistoryContextId,
    entries: Vec<(SessionHistoryStepId, SessionHistoryEntry)>,
}

impl NavigableHistory {
    fn contains(&self, key: &str) -> bool {
        self.entries
            .iter()
            .any(|(_, entry)| entry.key.as_str() == key)
    }
    fn entry_at(&self, step: SessionHistoryStepId) -> Option<&SessionHistoryEntry> {
        self.entries
            .iter()
            .find(|(id, _)| *id == step)
            .map(|(_, entry)| entry)
    }
    fn retain_in(&mut self, history: &JointSessionHistory) {
        self.entries
            .retain(|(id, _)| history.entries_at(*id).is_some());
    }
}

use parking_lot::Mutex;

/// A parser-created frame can be recreated when its parent Document is
/// repopulated. Script-created frames retain their own nested histories, but
/// a new script-created navigable must not claim an old one's history.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::native_bridge::context_host) struct ChildHistoryIdentity {
    parent: NavigationHistoryDocumentId,
    key: ChildHistoryKey,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ChildHistoryKey {
    Parser { name: String, ordinal: usize },
    Script(NavigationHistoryDocumentId),
}

#[derive(Clone, Debug)]
pub(super) struct ChildHistoryRecord {
    seed: NavigationHistoryEntrySeed,
    resources: HashMap<NavigationHistoryDocumentId, Arc<srcdoc_history::SrcdocHistoryResource>>,
    positions: NavigableHistory,
}

/// This store contains no live DOM handles or V8 objects. Retiring a Document
/// can release its entire subtree without discarding its nested histories.
#[derive(Clone, Debug, Default)]
pub(crate) struct NestedHistoryStore {
    records: Arc<Mutex<HashMap<ChildHistoryIdentity, ChildHistoryRecord>>>,
}

impl NestedHistoryStore {
    pub(crate) fn prune(
        &self,
        history: &JointSessionHistory,
        roots: &[NavigationHistoryDocumentId],
    ) {
        let mut records = self.records.lock();
        for record in records.values_mut() {
            record.positions.retain_in(history);
            record.seed.entries.retain(|entry| {
                record
                    .positions
                    .contains(&navigation_entry_public_token(entry.key.as_str()))
            });
            record.resources.retain(|id, _| {
                record
                    .seed
                    .entries
                    .iter()
                    .any(|entry| &entry.document_id == id)
            });
        }
        let mut reachable: HashSet<_> = roots.iter().cloned().collect();
        loop {
            let count = reachable.len();
            for (identity, record) in records.iter() {
                if reachable.contains(&identity.parent) {
                    reachable.extend(
                        record
                            .seed
                            .entries
                            .iter()
                            .map(|entry| entry.document_id.clone()),
                    );
                }
            }
            if reachable.len() == count {
                break;
            }
        }
        records.retain(|identity, record| {
            reachable.contains(&identity.parent) && !record.seed.entries.is_empty()
        });
    }
}

impl JsContextHost {
    pub(crate) fn nested_history_store(&mut self, root: OwnerDispatchScope) -> NestedHistoryStore {
        if root == OwnerDispatchScope::Top {
            return self.top_level_navigation_history.nested_history();
        }
        self.popup_nested_histories.entry(root).or_default().clone()
    }

    pub(super) fn child_history_identity(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) -> Option<ChildHistoryIdentity> {
        let node = self.dom_host().node(handle)?;
        let parser_created = node.flags().parser_created();
        let document = node.owner_document()?;
        let owner = self.owner_dispatch_scope_for_node(handle)?;
        let parent = match owner {
            OwnerDispatchScope::Top => self.top_level_navigation_history.current_document_id()?,
            OwnerDispatchScope::Child(parent) => {
                let seed = &self
                    .child_browsing_contexts
                    .get(&parent)?
                    .committed_navigation_entry_seed;
                seed.entries
                    .iter()
                    .find(|entry| entry.history_index == seed.current_index)?
                    .document_id
                    .clone()
            }
            OwnerDispatchScope::LightweightPopup(id) => {
                let window = self.lightweight_popup_window(scope, id)?;
                crate::context_bootstrap::history_document_id_for_holder(scope, window)?
            }
        };
        if !parser_created {
            return Some(ChildHistoryIdentity {
                parent,
                key: ChildHistoryKey::Script(NavigationHistoryDocumentId::allocate()),
            });
        }
        let name = self
            .dom_host()
            .get_attribute(handle, "name")
            .unwrap_or_default();
        let mut handles = Vec::new();
        self.collect_child_browsing_context_host_handles(document, &mut handles);
        let ordinal = handles
            .into_iter()
            .filter(|candidate| {
                self.dom_host()
                    .node(*candidate)
                    .is_some_and(|node| node.flags().parser_created())
                    && self
                        .dom_host()
                        .get_attribute(*candidate, "name")
                        .unwrap_or_default()
                        == name
            })
            .position(|candidate| candidate == handle)?;
        Some(ChildHistoryIdentity {
            parent,
            key: ChildHistoryKey::Parser { name, ordinal },
        })
    }

    pub(super) fn child_history_for_restoration(
        &mut self,
        handle: DomHandle,
        identity: &ChildHistoryIdentity,
    ) -> Option<ChildHistoryRecord> {
        // Consume each parser identity once in this native parent Document.
        // Removing/reinserting a frame in a still-live Document is not history traversal.
        let document = self.dom_host().node(handle)?.owner_document()?;
        if !self
            .claimed_child_histories
            .entry(document)
            .or_default()
            .insert(identity.clone())
        {
            return None;
        }
        let root = self.child_browsing_context_root_scope(handle)?;
        let popup = match root {
            OwnerDispatchScope::LightweightPopup(id) => Some(id),
            _ => None,
        };
        let step = self.session_histories.get_mut(popup).current_step();
        let store = self.nested_history_store(root);
        let mut record = store.records.lock().get(identity)?.clone();
        let key = &record.positions.entry_at(step)?.key;
        let target = record
            .seed
            .entries
            .iter()
            .find(|entry| navigation_entry_public_token(entry.key.as_str()) == key.as_str())?
            .clone();
        record.seed.current_index = target.history_index;
        // The parent already selected the joint position. Reattach the child's
        // historical view without replaying an older load's traversal admission.
        record.seed.session_history = Default::default();
        record.seed.activation = Some(NavigationActivationSeed {
            entry: target,
            from: None,
            navigation_type: Some("traverse".to_owned()),
        });
        Some(record)
    }

    pub(super) fn restore_child_history_after_initial_empty(
        &mut self,
        handle: DomHandle,
        record: ChildHistoryRecord,
    ) {
        let Some(target) = record
            .seed
            .entries
            .iter()
            .find(|entry| entry.history_index == record.seed.current_index)
        else {
            return;
        };
        let url = target.url.clone();
        let root = self
            .child_browsing_context_root_scope(handle)
            .unwrap_or(OwnerDispatchScope::Top);
        let popup = match root {
            OwnerDispatchScope::LightweightPopup(id) => Some(id),
            _ => None,
        };
        let parent = self.owner_dispatch_scope_for_node(handle).unwrap_or(root);
        let parent = self.session_histories.context(parent);
        self.session_histories
            .bind_context(OwnerDispatchScope::Child(handle), record.positions.context);
        self.session_histories.get_mut(popup).restore_context(
            record.positions.context,
            parent,
            &record.positions.entries,
        );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.srcdoc_history = record.resources;
            entry.restored_history_positions = Some(record.positions);
            entry.restoring_history = true;
        }
        self.queue_deferred_child_browsing_context_navigation_from_entry_seed(
            handle,
            &url,
            record.seed,
            None,
        );
    }

    pub(crate) fn remember_child_history(&mut self, handle: DomHandle) {
        let Some(root) = self.child_browsing_context_root_scope(handle) else {
            return;
        };
        let store = self.nested_history_store(root);
        let context = self
            .session_histories
            .context(OwnerDispatchScope::Child(handle));
        let popup = match root {
            OwnerDispatchScope::LightweightPopup(id) => Some(id),
            _ => None,
        };
        let live_positions = self
            .session_histories
            .get_mut(popup)
            .context_entries(context);
        let Some(entry) = self.child_browsing_contexts.get(&handle) else {
            return;
        };
        let Some(identity) = entry.history_identity.clone() else {
            return;
        };
        let seed = entry.committed_navigation_entry_seed();
        let positions = (!live_positions.is_empty())
            .then_some(NavigableHistory {
                context,
                entries: live_positions,
            })
            .or_else(|| entry.restored_history_positions.clone())
            .or_else(|| {
                store
                    .records
                    .lock()
                    .get(&identity)
                    .map(|record| record.positions.clone())
            });
        let Some(positions) = positions else {
            return;
        };
        // A new runtime can briefly expose its initial empty Document while
        // the historical target is pending. It must not overwrite that target.
        if !seed
            .entries
            .iter()
            .find(|entry| entry.history_index == seed.current_index)
            .is_some_and(|entry| {
                positions.contains(&navigation_entry_public_token(entry.key.as_str()))
            })
        {
            return;
        }
        let record = ChildHistoryRecord {
            seed,
            resources: entry.srcdoc_history.clone(),
            positions,
        };
        store.records.lock().insert(identity, record);
    }

    pub(crate) fn remember_nested_histories(&mut self, root: OwnerDispatchScope) {
        let handles: Vec<_> = self
            .child_browsing_contexts
            .keys()
            .copied()
            .filter(|handle| self.child_browsing_context_root_scope(*handle) == Some(root))
            .collect();
        for handle in handles {
            self.remember_child_history(handle);
        }
        if root == OwnerDispatchScope::Top {
            self.top_level_navigation_history
                .publish_joint_history(Some(self.session_histories.get_mut(None).clone()));
        }
    }
}
