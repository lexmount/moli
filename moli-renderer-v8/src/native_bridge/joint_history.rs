use super::OwnerDispatchScope;
use indexmap::IndexMap;
use std::collections::{BTreeSet, HashMap, VecDeque};

/// A position in a traversable's session history, independent of the local
/// indexes and opaque entry keys of any one Window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SessionHistoryStep(u64);

impl SessionHistoryStep {
    pub(crate) const INITIAL: Self = Self(0);
}

#[derive(Clone, Debug)]
pub(crate) struct JointHistoryEntry {
    pub(crate) index: u32,
    pub(crate) navigation_index: u32,
    pub(crate) key: String,
}

#[derive(Clone, Debug)]
pub(crate) struct JointHistorySnapshot {
    pub(crate) entries: Vec<JointHistoryEntry>,
    pub(crate) current_index: u32,
}

#[derive(Clone, Debug)]
struct EntryPosition {
    entry: JointHistoryEntry,
    step: SessionHistoryStep,
}

#[derive(Clone, Debug)]
pub(crate) struct NavigableHistory {
    entries: Vec<EntryPosition>,
    current_key: String,
}

impl NavigableHistory {
    pub(crate) fn new(
        snapshot: JointHistorySnapshot,
        first_step: SessionHistoryStep,
    ) -> Option<Self> {
        let current_key = snapshot
            .entries
            .iter()
            .find(|entry| entry.index == snapshot.current_index)?
            .key
            .clone();
        Some(Self {
            entries: snapshot
                .entries
                .into_iter()
                .map(|entry| EntryPosition {
                    step: SessionHistoryStep(first_step.0 + u64::from(entry.navigation_index)),
                    entry,
                })
                .collect(),
            current_key,
        })
    }
    pub(crate) fn entry_at(&self, step: SessionHistoryStep) -> Option<&JointHistoryEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.step <= step)
            .map(|entry| &entry.entry)
    }

    pub(crate) fn retain_through(&mut self, step: SessionHistoryStep) {
        self.entries.retain(|entry| entry.step <= step);
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|entry| entry.entry.key == key)
    }
}

/// Committed history positions survive the renderer and DOM handles that
/// happened to present them. Child entries are restored separately when their
/// parent Document recreates its navigables.
#[derive(Clone, Debug)]
pub(crate) struct JointHistoryRestoration {
    root: NavigableHistory,
    current_step: SessionHistoryStep,
    steps: BTreeSet<SessionHistoryStep>,
}

impl JointHistoryRestoration {
    pub(crate) fn current_step(&self) -> SessionHistoryStep {
        self.current_step
    }
    pub(crate) fn for_navigation(
        self,
        snapshot: JointHistorySnapshot,
        navigation_type: Option<&str>,
        target_step: Option<SessionHistoryStep>,
    ) -> Option<Self> {
        let root = OwnerDispatchScope::Top;
        let mut histories = JointSessionHistories::default();
        histories.restore_root(root, self);
        let joint = histories.get_mut(root)?;
        match navigation_type {
            Some("push") => {
                joint.push(root, snapshot);
            }
            Some("traverse") => {
                let key = &snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.index == snapshot.current_index)?
                    .key;
                let target = joint.plan_entry(root, key)?;
                let entry_step = joint
                    .navigables
                    .get(&root)?
                    .entries
                    .iter()
                    .find(|entry| &entry.entry.key == key)?
                    .step;
                joint.update_current(root, snapshot, Some(entry_step));
                joint.current_step = target_step.unwrap_or(target.step);
            }
            _ => joint.replace(root, snapshot),
        }
        joint.restoration(root)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct JointHistoryTarget {
    pub(crate) owner: OwnerDispatchScope,
    pub(crate) index: u32,
    pub(crate) key: String,
}

#[derive(Clone, Debug)]
pub(crate) struct JointHistoryTraversal {
    pub(crate) step: SessionHistoryStep,
    pub(crate) targets: Vec<JointHistoryTarget>,
}

#[derive(Debug)]
struct PendingTraversal {
    step: SessionHistoryStep,
    source_revision: u64,
    remaining: Vec<JointHistoryTarget>,
    committed: bool,
    ignored_response: bool,
}

#[derive(Debug)]
struct DeferredMutation {
    owner: OwnerDispatchScope,
    snapshot: JointHistorySnapshot,
    push: bool,
}

#[derive(Debug)]
pub(crate) struct JointSessionHistory {
    current_step: SessionHistoryStep,
    revision: u64,
    navigables: IndexMap<OwnerDispatchScope, NavigableHistory>,
    retired_steps: BTreeSet<SessionHistoryStep>,
    pending_traversal: Option<PendingTraversal>,
    deferred_mutations: VecDeque<DeferredMutation>,
    completed_traversal: Option<(SessionHistoryStep, u64)>,
}

#[derive(Clone, Copy)]
pub(crate) struct JointHistoryPosition {
    pub(crate) root: OwnerDispatchScope,
    pub(crate) step: SessionHistoryStep,
    pub(crate) revision: u64,
}

#[derive(Default)]
pub(crate) struct JointSessionHistories {
    histories: HashMap<OwnerDispatchScope, JointSessionHistory>,
}

impl JointSessionHistories {
    pub(crate) fn restore_root(
        &mut self,
        owner: OwnerDispatchScope,
        restoration: JointHistoryRestoration,
    ) {
        self.histories.insert(
            owner,
            JointSessionHistory {
                current_step: restoration.current_step,
                revision: 0,
                navigables: IndexMap::from([(owner, restoration.root)]),
                retired_steps: restoration.steps,
                pending_traversal: None,
                deferred_mutations: VecDeque::new(),
                completed_traversal: None,
            },
        );
    }
    pub(crate) fn root_for_owner(&self, owner: OwnerDispatchScope) -> Option<OwnerDispatchScope> {
        self.histories
            .iter()
            .find_map(|(root, history)| history.navigables.contains_key(&owner).then_some(*root))
    }

    pub(crate) fn get(&self, root: OwnerDispatchScope) -> Option<&JointSessionHistory> {
        self.histories.get(&root)
    }

    pub(crate) fn get_mut(&mut self, root: OwnerDispatchScope) -> Option<&mut JointSessionHistory> {
        self.histories.get_mut(&root)
    }

    pub(crate) fn ensure_root(
        &mut self,
        root: OwnerDispatchScope,
        snapshot: JointHistorySnapshot,
    ) -> Option<&mut JointSessionHistory> {
        let current = snapshot
            .entries
            .iter()
            .find(|entry| entry.index == snapshot.current_index)?;
        let current_step = SessionHistoryStep(u64::from(current.navigation_index));
        let history = self
            .histories
            .entry(root)
            .or_insert_with(|| JointSessionHistory {
                current_step,
                revision: 0,
                navigables: IndexMap::new(),
                retired_steps: BTreeSet::new(),
                pending_traversal: None,
                deferred_mutations: VecDeque::new(),
                completed_traversal: None,
            });
        history.ensure_navigable(root, snapshot, SessionHistoryStep(0));
        Some(history)
    }
}

impl JointSessionHistory {
    pub(crate) fn restoration(&self, root: OwnerDispatchScope) -> Option<JointHistoryRestoration> {
        Some(JointHistoryRestoration {
            root: self.navigables.get(&root)?.clone(),
            current_step: self.current_step,
            steps: self.used_steps().into_iter().collect(),
        })
    }

    pub(crate) fn navigable_history(&self, owner: OwnerDispatchScope) -> Option<NavigableHistory> {
        self.navigables.get(&owner).cloned()
    }

    pub(crate) fn restore_child(
        &mut self,
        owner: OwnerDispatchScope,
        mut history: NavigableHistory,
        current_key: &str,
    ) {
        history.current_key = current_key.to_owned();
        self.navigables.insert(owner, history);
    }

    pub(crate) fn destination_step(&self) -> SessionHistoryStep {
        self.pending_traversal
            .as_ref()
            .map_or(self.current_step, |pending| pending.step)
    }
    pub(crate) fn reset(&mut self, snapshots: Vec<(OwnerDispatchScope, JointHistorySnapshot)>) {
        self.navigables.clear();
        self.retired_steps.clear();
        self.pending_traversal = None;
        self.deferred_mutations.clear();
        self.current_step = SessionHistoryStep(0);
        self.revision += 1;
        for (owner, mut snapshot) in snapshots {
            snapshot
                .entries
                .retain(|entry| entry.index == snapshot.current_index);
            for entry in &mut snapshot.entries {
                entry.navigation_index = 0;
            }
            self.ensure_navigable(owner, snapshot, self.current_step);
        }
        self.completed_traversal = Some((self.current_step, self.revision));
    }

    #[cfg(test)]
    pub(crate) fn current_step(&self) -> SessionHistoryStep {
        self.current_step
    }

    /// Synchronous mutations are already visible to their caller even while
    /// the central history is waiting for another navigable to commit.
    pub(crate) fn source_step(&self) -> SessionHistoryStep {
        if let Some(traversal) = &self.pending_traversal
            && !self.deferred_mutations.is_empty()
        {
            return SessionHistoryStep(traversal.step.0 + self.deferred_push_count() as u64);
        }
        self.current_step
    }

    fn deferred_push_count(&self) -> usize {
        self.deferred_mutations
            .iter()
            .filter(|mutation| mutation.push)
            .count()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn has_pending_traversal(&self) -> bool {
        self.pending_traversal.is_some()
    }

    pub(crate) fn follows_pending_traversal(&self, revision: u64) -> bool {
        self.pending_traversal
            .as_ref()
            .is_some_and(|traversal| traversal.source_revision == revision)
    }

    pub(crate) fn pending_target(&self, owner: OwnerDispatchScope) -> Option<&JointHistoryTarget> {
        self.pending_traversal
            .as_ref()?
            .remaining
            .iter()
            .find(|target| target.owner == owner)
    }

    pub(crate) fn take_completed_traversal(&mut self) -> Option<(SessionHistoryStep, u64)> {
        self.completed_traversal.take()
    }

    pub(crate) fn cancel_traversal(&mut self) -> Vec<JointHistoryTarget> {
        if self.pending_traversal.take().is_none() {
            return Vec::new();
        }
        self.completed_traversal = Some((self.current_step, self.revision));
        self.apply_deferred_mutations()
    }

    fn must_defer_mutation(&self, owner: OwnerDispatchScope) -> bool {
        self.pending_traversal.as_ref().is_some_and(|traversal| {
            !traversal
                .remaining
                .iter()
                .any(|target| target.owner == owner)
        })
    }

    fn used_steps(&self) -> Vec<SessionHistoryStep> {
        self.navigables
            .values()
            .flat_map(|history| history.entries.iter().map(|entry| entry.step))
            .chain(self.retired_steps.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn length(&self) -> usize {
        if let Some(traversal) = &self.pending_traversal
            && self.deferred_push_count() > 0
        {
            return self
                .used_steps()
                .iter()
                .filter(|step| **step <= traversal.step)
                .count()
                + self.deferred_push_count();
        }
        self.used_steps().len()
    }

    pub(crate) fn ensure_child(
        &mut self,
        owner: OwnerDispatchScope,
        snapshot: JointHistorySnapshot,
    ) {
        let current_index = snapshot
            .entries
            .iter()
            .find(|entry| entry.index == snapshot.current_index)
            .map_or(0, |entry| u64::from(entry.navigation_index));
        self.ensure_navigable(
            owner,
            snapshot,
            SessionHistoryStep(self.current_step.0.saturating_sub(current_index)),
        );
    }

    fn ensure_navigable(
        &mut self,
        owner: OwnerDispatchScope,
        snapshot: JointHistorySnapshot,
        first_step: SessionHistoryStep,
    ) {
        if self.navigables.contains_key(&owner) {
            return;
        }
        let Some(current) = snapshot
            .entries
            .iter()
            .find(|entry| entry.index == snapshot.current_index)
        else {
            return;
        };
        let current_key = current.key.clone();
        let entries = snapshot
            .entries
            .into_iter()
            .map(|entry| EntryPosition {
                step: SessionHistoryStep(first_step.0 + u64::from(entry.navigation_index)),
                entry,
            })
            .collect();
        self.navigables.insert(
            owner,
            NavigableHistory {
                entries,
                current_key,
            },
        );
    }

    pub(crate) fn retain_navigables(&mut self, mut retain: impl FnMut(OwnerDispatchScope) -> bool) {
        // Keep committed positions after frame removal, as browsers do for
        // classic History. The removed frame is no longer a traversal target,
        // and a later push still prunes its positions on the forward branch.
        self.navigables.retain(|owner, history| {
            if retain(*owner) {
                return true;
            }
            self.retired_steps
                .extend(history.entries.iter().map(|entry| entry.step));
            false
        });
    }

    /// Only a committed push creates a step and clears the forward entries of
    /// every navigable. Pending network requests never enter this operation.
    pub(crate) fn push(
        &mut self,
        owner: OwnerDispatchScope,
        snapshot: JointHistorySnapshot,
    ) -> Vec<JointHistoryTarget> {
        if self.must_defer_mutation(owner) {
            self.revision += 1;
            self.deferred_mutations.push_back(DeferredMutation {
                owner,
                snapshot,
                push: true,
            });
            return Vec::new();
        }
        let mut removed = Vec::new();
        self.retired_steps.retain(|step| *step <= self.current_step);
        for (entry_owner, history) in &mut self.navigables {
            history.entries.retain(|position| {
                let retain = position.step <= self.current_step;
                if !retain {
                    removed.push(JointHistoryTarget {
                        owner: *entry_owner,
                        index: position.entry.index,
                        key: position.entry.key.clone(),
                    });
                }
                retain
            });
        }
        let step = SessionHistoryStep(
            self.current_step
                .0
                .checked_add(1)
                .expect("session history step overflow"),
        );
        self.update_current(owner, snapshot, Some(step));
        self.current_step = step;
        self.revision += 1;
        removed
    }

    pub(crate) fn replace(&mut self, owner: OwnerDispatchScope, snapshot: JointHistorySnapshot) {
        if self.must_defer_mutation(owner) {
            self.revision += 1;
            self.deferred_mutations.push_back(DeferredMutation {
                owner,
                snapshot,
                push: false,
            });
            return;
        }
        self.update_current(owner, snapshot, None);
        self.revision += 1;
    }

    fn update_current(
        &mut self,
        owner: OwnerDispatchScope,
        snapshot: JointHistorySnapshot,
        new_step: Option<SessionHistoryStep>,
    ) {
        let Some(current_key) = snapshot
            .entries
            .iter()
            .find(|entry| entry.index == snapshot.current_index)
            .map(|entry| entry.key.clone())
        else {
            return;
        };
        let Some(history) = self.navigables.get_mut(&owner) else {
            return;
        };
        let previous = history
            .entries
            .iter()
            .find(|entry| entry.entry.key == history.current_key);
        let replacement_step = previous.map_or(self.current_step, |entry| entry.step);
        let entries = snapshot
            .entries
            .into_iter()
            .filter_map(|entry| {
                let step = if entry.index == snapshot.current_index {
                    new_step.unwrap_or(replacement_step)
                } else {
                    history
                        .entries
                        .iter()
                        .find(|known| known.entry.key == entry.key)
                        .map(|known| known.step)?
                };
                Some(EntryPosition { entry, step })
            })
            .collect();
        history.current_key = current_key;
        history.entries = entries;
    }

    pub(crate) fn plan_delta(
        &self,
        source: SessionHistoryStep,
        delta: i64,
    ) -> Option<JointHistoryTraversal> {
        let steps = self.used_steps();
        let source_index = steps.iter().rposition(|step| *step <= source)?;
        let index = i64::try_from(source_index).ok()?.checked_add(delta)?;
        let step = *steps.get(usize::try_from(index).ok()?)?;
        Some(self.plan_step(step))
    }

    pub(crate) fn plan_entry(
        &self,
        owner: OwnerDispatchScope,
        key: &str,
    ) -> Option<JointHistoryTraversal> {
        let history = self.navigables.get(&owner)?;
        // A local entry may be active at several joint steps. The Navigation
        // API selects the nearest such step in the requested direction.
        let step = self
            .used_steps()
            .into_iter()
            .filter(|step| {
                history
                    .entries
                    .iter()
                    .rev()
                    .find(|entry| entry.step <= *step)
                    .is_some_and(|entry| entry.entry.key == key)
            })
            .min_by_key(|step| step.0.abs_diff(self.current_step.0))?;
        Some(self.plan_step(step))
    }

    fn plan_step(&self, step: SessionHistoryStep) -> JointHistoryTraversal {
        let targets = self
            .navigables
            .iter()
            .filter_map(|(owner, history)| {
                let target = history
                    .entries
                    .iter()
                    .rev()
                    .find(|entry| entry.step <= step)?;
                (target.entry.key != history.current_key).then(|| JointHistoryTarget {
                    owner: *owner,
                    index: target.entry.index,
                    key: target.entry.key.clone(),
                })
            })
            .collect();
        JointHistoryTraversal { step, targets }
    }

    pub(crate) fn commit_traversal_entry(
        &mut self,
        owner: OwnerDispatchScope,
        key: &str,
    ) -> Vec<JointHistoryTarget> {
        if let Some(history) = self.navigables.get_mut(&owner)
            && let Some(entry) = history.entries.iter().find(|entry| entry.entry.key == key)
        {
            history.current_key = key.to_owned();
            if let Some(traversal) = &mut self.pending_traversal {
                traversal.committed = true;
            } else {
                self.current_step = entry.step;
                self.revision += 1;
            }
        }
        self.complete_traversal_entry(owner, key)
    }

    /// A non-displayable response completes the traversal without changing
    /// the active Document or settling the Navigation API's promises.
    pub(crate) fn finish_traversal_entry(
        &mut self,
        owner: OwnerDispatchScope,
        key: &str,
    ) -> Vec<JointHistoryTarget> {
        if let Some(traversal) = &mut self.pending_traversal
            && traversal
                .remaining
                .iter()
                .any(|target| target.owner == owner && target.key == key)
        {
            traversal.ignored_response = true;
        }
        self.complete_traversal_entry(owner, key)
    }

    fn complete_traversal_entry(
        &mut self,
        owner: OwnerDispatchScope,
        key: &str,
    ) -> Vec<JointHistoryTarget> {
        if let Some(traversal) = &mut self.pending_traversal {
            traversal
                .remaining
                .retain(|target| target.owner != owner || target.key != key);
        }
        self.finish_traversal()
    }

    pub(crate) fn begin_traversal(&mut self, traversal: JointHistoryTraversal) {
        debug_assert!(self.pending_traversal.is_none());
        self.completed_traversal = None;
        self.pending_traversal = Some(PendingTraversal {
            step: traversal.step,
            source_revision: self.revision,
            remaining: traversal.targets,
            committed: false,
            ignored_response: false,
        });
    }

    pub(crate) fn finish_traversal(&mut self) -> Vec<JointHistoryTarget> {
        if let Some(traversal) = &self.pending_traversal
            && traversal
                .remaining
                .iter()
                .all(|target| !self.navigables.contains_key(&target.owner))
        {
            // A 204/205 or download that leaves every Document in place must
            // also leave the position from which a new History.back() retries.
            if traversal.committed || !traversal.ignored_response {
                self.current_step = traversal.step;
            }
            self.revision += 1;
            self.pending_traversal = None;
            self.completed_traversal = Some((self.current_step, self.revision));
            // A synchronous navigation in an already traversed navigable must
            // wait for the other navigables before choosing its new step.
            return self.apply_deferred_mutations();
        }
        Vec::new()
    }

    fn apply_deferred_mutations(&mut self) -> Vec<JointHistoryTarget> {
        let mut removed = Vec::new();
        while let Some(mutation) = self.deferred_mutations.pop_front() {
            if !self.navigables.contains_key(&mutation.owner) {
                continue;
            }
            if mutation.push {
                removed.extend(self.push(mutation.owner, mutation.snapshot));
            } else {
                self.replace(mutation.owner, mutation.snapshot);
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_runtime::DomHandle;

    fn snapshot(keys: &[&str], current_index: u32) -> JointHistorySnapshot {
        JointHistorySnapshot {
            entries: keys
                .iter()
                .enumerate()
                .map(|(index, key)| JointHistoryEntry {
                    index: index as u32,
                    navigation_index: index as u32,
                    key: (*key).to_owned(),
                })
                .collect(),
            current_index,
        }
    }

    #[test]
    fn restored_root_and_children_preserve_interleaved_session_history_steps() {
        let root = OwnerDispatchScope::Top;
        let old_a = OwnerDispatchScope::Child(DomHandle::new(1));
        let old_b = OwnerDispatchScope::Child(DomHandle::new(2));
        let mut histories = JointSessionHistories::default();
        let original = histories
            .ensure_root(root, snapshot(&["parent"], 0))
            .unwrap();
        original.ensure_child(old_a, snapshot(&["a0"], 0));
        original.ensure_child(old_b, snapshot(&["b0"], 0));
        original.push(old_a, snapshot(&["a0", "a1"], 1));
        original.push(old_b, snapshot(&["b0", "b1"], 1));
        original.push(old_a, snapshot(&["a0", "a1", "a2"], 2));
        let a = original.navigable_history(old_a).unwrap();
        let b = original.navigable_history(old_b).unwrap();
        let source = original.restoration(root).unwrap();
        let away = source
            .for_navigation(snapshot(&["parent", "away"], 1), Some("push"), None)
            .unwrap();
        let back = away
            .for_navigation(
                snapshot(&["parent", "away"], 0),
                Some("traverse"),
                Some(SessionHistoryStep(3)),
            )
            .unwrap();
        drop(histories);

        let mut histories = JointSessionHistories::default();
        histories.restore_root(root, back);
        let restored = histories.get_mut(root).unwrap();
        let new_a = OwnerDispatchScope::Child(DomHandle::new(101));
        let new_b = OwnerDispatchScope::Child(DomHandle::new(102));
        restored.restore_child(new_a, a.clone(), "a2");
        restored.restore_child(new_b, b.clone(), "b1");
        assert_eq!(restored.length(), 5);
        for (owner, key) in [(new_a, "a1"), (new_b, "b0"), (new_a, "a0")] {
            let plan = restored.plan_delta(restored.current_step(), -1).unwrap();
            assert_eq!(plan.targets.len(), 1);
            assert_eq!(plan.targets[0].owner, owner);
            assert_eq!(plan.targets[0].key, key);
            restored.begin_traversal(plan);
            restored.commit_traversal_entry(owner, key);
        }
        assert_eq!(restored.current_step(), SessionHistoryStep(0));
        assert_eq!(
            restored
                .plan_delta(restored.current_step(), 4)
                .unwrap()
                .step,
            SessionHistoryStep(4)
        );
        assert_eq!(a.entry_at(SessionHistoryStep(1)).unwrap().key, "a1");
        assert_eq!(b.entry_at(SessionHistoryStep(1)).unwrap().key, "b0");
    }

    #[test]
    fn interleaved_children_share_steps_and_prune_each_others_forward_entries() {
        let root = OwnerDispatchScope::Top;
        let a = OwnerDispatchScope::Child(DomHandle::new(1));
        let b = OwnerDispatchScope::Child(DomHandle::new(2));
        let mut histories = JointSessionHistories::default();
        let history = histories.ensure_root(root, snapshot(&["root"], 0)).unwrap();
        history.ensure_child(a, snapshot(&["a0"], 0));
        history.ensure_child(b, snapshot(&["b0"], 0));
        history.push(a, snapshot(&["a0", "a1"], 1));
        history.push(b, snapshot(&["b0", "b1"], 1));
        history.push(a, snapshot(&["a0", "a1", "a2"], 2));
        assert_eq!(history.length(), 4);
        let one = history.plan_delta(history.current_step(), -1).unwrap();
        assert_eq!(one.targets.len(), 1);
        assert_eq!(one.targets[0].owner, a);
        assert_eq!(one.targets[0].key, "a1");
        let two = history.plan_delta(history.current_step(), -2).unwrap();
        assert_eq!(
            two.targets
                .iter()
                .map(|target| target.key.as_str())
                .collect::<Vec<_>>(),
            ["a1", "b0"]
        );
        history.begin_traversal(two.clone());
        history.commit_traversal_entry(a, "a1");
        assert_eq!(history.current_step(), SessionHistoryStep(3));
        history.commit_traversal_entry(b, "b0");
        assert_eq!(history.current_step(), two.step);
        let removed = history.push(a, snapshot(&["a0", "a1", "fork"], 2));
        assert_eq!(
            removed
                .iter()
                .map(|target| target.key.as_str())
                .collect::<Vec<_>>(),
            ["a2", "b1"]
        );
        assert_eq!(history.length(), 3);
        assert!(history.plan_delta(history.current_step(), 1).is_none());
    }

    #[test]
    fn removing_a_navigable_keeps_positions_without_retaining_traversal_targets() {
        let root = OwnerDispatchScope::Top;
        let a = OwnerDispatchScope::Child(DomHandle::new(1));
        let b = OwnerDispatchScope::Child(DomHandle::new(2));
        let mut histories = JointSessionHistories::default();
        let history = histories.ensure_root(root, snapshot(&["root"], 0)).unwrap();
        history.ensure_child(a, snapshot(&["a0"], 0));
        history.ensure_child(b, snapshot(&["b0"], 0));
        history.push(a, snapshot(&["a0", "a1"], 1));
        history.push(b, snapshot(&["b0", "b1"], 1));
        history.push(a, snapshot(&["a0", "a1", "a2"], 2));
        history.retain_navigables(|owner| owner != b);
        assert_eq!(history.length(), 4);
        let previous = history.plan_delta(history.current_step(), -1).unwrap();
        assert_eq!(previous.step, SessionHistoryStep(2));
        assert_eq!(previous.targets[0].key, "a1");
        assert_eq!(previous.targets.len(), 1);
        history.begin_traversal(previous);
        history.commit_traversal_entry(a, "a1");
        let unchanged = history.plan_delta(history.current_step(), -1).unwrap();
        assert_eq!(unchanged.step, SessionHistoryStep(1));
        assert!(unchanged.targets.is_empty());
        history.begin_traversal(unchanged);
        history.finish_traversal();
        history.push(a, snapshot(&["a0", "a1", "fork"], 2));
        assert_eq!(history.length(), 3);
        assert!(history.plan_delta(history.current_step(), 1).is_none());
    }

    #[test]
    fn a_push_after_one_child_commits_waits_for_the_remaining_child() {
        let root = OwnerDispatchScope::Top;
        let a = OwnerDispatchScope::Child(DomHandle::new(1));
        let b = OwnerDispatchScope::Child(DomHandle::new(2));
        let mut histories = JointSessionHistories::default();
        let history = histories.ensure_root(root, snapshot(&["root"], 0)).unwrap();
        history.ensure_child(a, snapshot(&["a0"], 0));
        history.ensure_child(b, snapshot(&["b0"], 0));
        history.push(a, snapshot(&["a0", "a1"], 1));
        history.push(b, snapshot(&["b0", "b1"], 1));
        let traversal = history.plan_delta(history.current_step(), -2).unwrap();
        history.begin_traversal(traversal);
        assert!(history.commit_traversal_entry(a, "a0").is_empty());
        assert!(history.push(a, snapshot(&["a0", "during"], 1)).is_empty());
        assert!(history.has_pending_traversal());
        assert_eq!(history.current_step(), SessionHistoryStep(2));
        assert_eq!(history.source_step(), SessionHistoryStep(1));
        assert_eq!(history.length(), 2);

        let removed = history.commit_traversal_entry(b, "b0");
        assert_eq!(
            removed
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            ["a1", "b1"]
        );
        assert!(!history.has_pending_traversal());
        assert_eq!(history.current_step(), SessionHistoryStep(1));
        assert_eq!(history.length(), 2);
        assert_eq!(
            history.take_completed_traversal().unwrap().0,
            SessionHistoryStep(0)
        );
        let previous = history.plan_delta(history.current_step(), -1).unwrap();
        assert_eq!(previous.targets.len(), 1);
        assert_eq!(previous.targets[0].key, "a0");
    }

    #[test]
    fn canceled_traversal_keeps_the_source_for_the_next_delta() {
        let root = OwnerDispatchScope::Top;
        let mut histories = JointSessionHistories::default();
        let history = histories
            .ensure_root(root, snapshot(&["zero", "one", "two"], 2))
            .unwrap();
        let traversal = history.plan_delta(history.current_step(), -1).unwrap();
        history.begin_traversal(traversal.clone());
        assert!(history.cancel_traversal().is_empty());
        assert!(!history.has_pending_traversal());
        let (source, _) = history.take_completed_traversal().unwrap();
        let retry = history.plan_delta(source, -1).unwrap();
        assert_eq!(retry.step, traversal.step);
        assert_eq!(retry.targets[0].key, "one");
    }

    #[test]
    fn an_ignored_response_completes_the_group_without_changing_the_active_entry() {
        let root = OwnerDispatchScope::Top;
        let child = OwnerDispatchScope::Child(DomHandle::new(1));
        let mut histories = JointSessionHistories::default();
        let history = histories.ensure_root(root, snapshot(&["top"], 0)).unwrap();
        history.ensure_child(child, snapshot(&["child0"], 0));
        history.push(child, snapshot(&["child0", "child1"], 1));
        let revision = history.revision();
        let traversal = history.plan_entry(child, "child0").unwrap();
        history.begin_traversal(traversal);
        assert!(history.follows_pending_traversal(revision));
        history.finish_traversal_entry(child, "child0");
        assert!(!history.has_pending_traversal());
        assert_eq!(history.current_step(), SessionHistoryStep(1));
        assert_eq!(history.navigables[&child].current_key, "child1");
        let retry = history.plan_delta(history.current_step(), -1).unwrap();
        assert_eq!(retry.targets.len(), 1);
        assert_eq!(retry.targets[0].key, "child0");
    }

    #[test]
    fn keyed_traversal_selects_the_nearest_joint_step_and_all_affected_windows() {
        let root = OwnerDispatchScope::Top;
        let child = OwnerDispatchScope::Child(DomHandle::new(1));
        let mut histories = JointSessionHistories::default();
        let history = histories.ensure_root(root, snapshot(&["top0"], 0)).unwrap();
        history.ensure_child(child, snapshot(&["child0"], 0));
        history.push(root, snapshot(&["top0", "top1"], 1));
        history.push(child, snapshot(&["child0", "child1"], 1));
        let child_back = history.plan_entry(child, "child0").unwrap();
        assert_eq!(child_back.step, SessionHistoryStep(1));
        assert_eq!(child_back.targets.len(), 1);
        assert_eq!(child_back.targets[0].key, "child0");
        let top_back = history.plan_entry(root, "top0").unwrap();
        assert_eq!(
            top_back
                .targets
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            ["top0", "child0"]
        );
        history.begin_traversal(top_back);
        history.commit_traversal_entry(root, "top0");
        history.commit_traversal_entry(child, "child0");
        let child_forward = history.plan_entry(child, "child1").unwrap();
        assert_eq!(
            child_forward
                .targets
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            ["top1", "child1"]
        );
    }
}
