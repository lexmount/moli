use super::*;
use crate::context_bootstrap::navigation_entry_public_token;
use crate::native_bridge::{
    OwnerDispatchScope,
    joint_history::{
        JointHistoryEntry, JointHistorySnapshot, NavigableHistory, SessionHistoryStep,
    },
};
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
    pub(crate) fn prune(&self, step: SessionHistoryStep, roots: &[NavigationHistoryDocumentId]) {
        let mut records = self.records.lock();
        for record in records.values_mut() {
            record.positions.retain_through(step);
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

pub(crate) fn joint_snapshot(seed: &NavigationHistoryEntrySeed) -> JointHistorySnapshot {
    JointHistorySnapshot {
        entries: seed
            .entries
            .iter()
            .map(|entry| JointHistoryEntry {
                index: entry.history_index,
                navigation_index: entry.index,
                key: navigation_entry_public_token(entry.key.as_str()),
            })
            .collect(),
        current_index: seed.current_index,
    }
}

impl JsContextHost {
    fn nested_history_root(&self, handle: DomHandle) -> Option<OwnerDispatchScope> {
        let mut owner = self.owner_dispatch_scope_for_node(handle)?;
        while let OwnerDispatchScope::Child(parent) = owner {
            owner = self.owner_dispatch_scope_for_node(parent)?;
        }
        Some(owner)
    }

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
        let root = self.nested_history_root(handle)?;
        let step = self.joint_histories.get(root)?.destination_step();
        let store = self.nested_history_store(root);
        let mut record = store.records.lock().get(identity)?.clone();
        let key = &record.positions.entry_at(step)?.key;
        let target = record
            .seed
            .entries
            .iter()
            .find(|entry| navigation_entry_public_token(entry.key.as_str()) == *key)?
            .clone();
        record.seed.current_index = target.history_index;
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
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.srcdoc_history = record.resources;
            entry.restored_history_positions = Some(record.positions);
            entry.restoring_history = true;
        }
        self.queue_deferred_child_browsing_context_navigation_from_entry_seed(
            handle,
            &url,
            record.seed,
            false,
            None,
        );
    }

    pub(crate) fn take_restored_child_history_positions(
        &mut self,
        handle: DomHandle,
        key: &str,
    ) -> Option<NavigableHistory> {
        let entry = self.child_browsing_contexts.get_mut(&handle)?;
        if !entry.restored_history_positions.as_ref()?.contains(key) {
            return None;
        }
        entry.restored_history_positions.take()
    }

    pub(crate) fn saved_child_history_positions(
        &mut self,
        handle: DomHandle,
        key: &str,
    ) -> Option<NavigableHistory> {
        let identity = self
            .child_browsing_contexts
            .get(&handle)?
            .history_identity
            .clone()?;
        let root = self.nested_history_root(handle)?;
        let store = self.nested_history_store(root);
        let records = store.records.lock();
        let positions = &records.get(&identity)?.positions;
        positions.contains(key).then(|| positions.clone())
    }

    pub(crate) fn remember_child_history(&mut self, handle: DomHandle) {
        let Some(root) = self.nested_history_root(handle) else {
            return;
        };
        let store = self.nested_history_store(root);
        let Some(entry) = self.child_browsing_contexts.get(&handle) else {
            return;
        };
        let Some(identity) = entry.history_identity.clone() else {
            return;
        };
        let seed = entry.committed_navigation_entry_seed();
        let joint = self.joint_histories.get(root);
        let positions = entry
            .restored_history_positions
            .clone()
            .or_else(|| {
                joint.and_then(|joint| joint.navigable_history(OwnerDispatchScope::Child(handle)))
            })
            // Lazy or retiring realms may have no live joint-ledger entry.
            // Their committed positions still belong to the navigable and
            // must not be rebased to the traversable's current step.
            .or_else(|| {
                store
                    .records
                    .lock()
                    .get(&identity)
                    .map(|record| record.positions.clone())
            })
            .or_else(|| {
                NavigableHistory::new(
                    joint_snapshot(&seed),
                    joint.map_or(SessionHistoryStep::INITIAL, |joint| {
                        joint.destination_step()
                    }),
                )
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

    pub(in crate::native_bridge::context_host) fn take_child_history_restoration(
        &mut self,
        handle: DomHandle,
    ) -> bool {
        self.child_browsing_contexts
            .get_mut(&handle)
            .is_some_and(|entry| std::mem::take(&mut entry.restoring_history))
    }

    pub(crate) fn remember_nested_histories(&mut self, root: OwnerDispatchScope) {
        let handles: Vec<_> = self
            .child_browsing_contexts
            .keys()
            .copied()
            .filter(|handle| self.nested_history_root(*handle) == Some(root))
            .collect();
        for handle in handles {
            self.remember_child_history(handle);
        }
        if root == OwnerDispatchScope::Top {
            self.top_level_navigation_history.publish_joint_history(
                self.joint_histories
                    .get(root)
                    .and_then(|joint| joint.restoration(root)),
            );
        }
    }
}
