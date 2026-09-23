//! The history of a traversable, independent of any Document's Navigation API
//! view. A step records the active entry of each participating browsing context.
//! Indices are positions, never identities: pushing after traversal discards the
//! forward steps, whereas replacement preserves the current step.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{NavigationHistoryDocumentId, NavigationHistoryEntryKey};

static NEXT_CONTEXT: AtomicU64 = AtomicU64::new(1);
static NEXT_STEP: AtomicU64 = AtomicU64::new(1);

fn allocate(counter: &AtomicU64) -> u64 {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("session history identity allocator exhausted")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionHistoryContextId(u64);

impl SessionHistoryContextId {
    pub const ROOT: Self = Self(0);

    pub fn allocate() -> Self {
        Self(allocate(&NEXT_CONTEXT))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionHistoryStepId(u64);

impl SessionHistoryStepId {
    pub fn raw(self) -> u64 {
        self.0
    }

    /// Restore an opaque identity held by a renderer's private task slot.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Browser supplied position. It includes steps outside the current renderer's
/// Navigation API view, including entries in other Documents and origins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionHistoryPosition {
    index: usize,
    length: usize,
}

impl SessionHistoryPosition {
    pub const INITIAL: Self = Self {
        index: 0,
        length: 1,
    };

    pub fn new(index: usize, length: usize) -> Option<Self> {
        (index < length).then_some(Self { index, length })
    }

    pub fn index(self) -> usize {
        self.index
    }

    pub fn length(self) -> usize {
        self.length
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHistoryEntry {
    pub key: NavigationHistoryEntryKey,
    pub document: NavigationHistoryDocumentId,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionHistoryCommit {
    #[default]
    Attach,
    Push,
    Replace,
    Traverse,
}

/// The history effect of a document commit, carried separately from the
/// document's entry view. A top-level renderer replacement also carries its
/// traversable; subframe commits keep using the page's existing owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionHistorySeed {
    pub commit: SessionHistoryCommit,
    pub traversable: Option<Box<JointSessionHistory>>,
    pub target_step: Option<SessionHistoryStepId>,
    /// A cross-Document participant of an already accepted joint traversal.
    /// Its load may materialize this entry, but may not move the shared cursor.
    pub admitted_entry: Option<SessionHistoryEntry>,
}

/// One already-committed traversable mutation, published in renderer FIFO
/// order before navigation observers can initiate another mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHistoryUpdate {
    pub position: SessionHistoryPosition,
    pub update: crate::SessionHistoryUpdateKind,
    pub root_url: String,
    /// Joint steps that share the current root entry. A root replacement also
    /// changes the URL exposed by steps introduced by child navigations.
    pub root_entry_steps: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionHistoryStep {
    id: SessionHistoryStepId,
    entries: BTreeMap<SessionHistoryContextId, SessionHistoryEntry>,
}

/// Page-owned authority for joint history. No URL, Navigation API index, or
/// per-Window cached length participates in history bookkeeping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JointSessionHistory {
    steps: Vec<SessionHistoryStep>,
    current: usize,
    parents: BTreeMap<SessionHistoryContextId, SessionHistoryContextId>,
}

impl Default for JointSessionHistory {
    fn default() -> Self {
        Self::new(SessionHistoryPosition::INITIAL)
    }
}

impl JointSessionHistory {
    pub fn new(position: SessionHistoryPosition) -> Self {
        Self {
            steps: (0..position.length)
                .map(|_| SessionHistoryStep {
                    id: SessionHistoryStepId(allocate(&NEXT_STEP)),
                    entries: BTreeMap::new(),
                })
                .collect(),
            current: position.index,
            parents: BTreeMap::new(),
        }
    }

    pub fn position(&self) -> SessionHistoryPosition {
        SessionHistoryPosition {
            index: self.current,
            length: self.steps.len(),
        }
    }

    pub fn current_step(&self) -> SessionHistoryStepId {
        self.steps[self.current].id
    }

    pub fn entry(&self, context: SessionHistoryContextId) -> Option<&SessionHistoryEntry> {
        self.steps[self.current].entries.get(&context)
    }

    /// Attaching an initial document adds no step. Its entry is also active at
    /// other steps in the same parent Document, including its forward history.
    pub fn attach(
        &mut self,
        context: SessionHistoryContextId,
        parent: Option<SessionHistoryContextId>,
        entry: SessionHistoryEntry,
    ) {
        if self.entry(context).is_some() {
            return;
        }
        let parent_document =
            parent.and_then(|parent| self.entry(parent).map(|e| e.document.clone()));
        if let Some(parent) = parent {
            self.parents.insert(context, parent);
        }
        for (index, step) in self.steps.iter_mut().enumerate() {
            let same_parent_document =
                parent
                    .zip(parent_document.as_ref())
                    .is_some_and(|(parent, document)| {
                        step.entries
                            .get(&parent)
                            .is_some_and(|e| &e.document == document)
                    });
            if index == self.current || same_parent_document {
                step.entries.entry(context).or_insert_with(|| entry.clone());
            }
        }
    }

    pub fn push(&mut self, context: SessionHistoryContextId, entry: SessionHistoryEntry) {
        let mut next = self.steps[self.current].clone();
        next.id = SessionHistoryStepId(allocate(&NEXT_STEP));
        self.remove_replaced_document_children(&mut next, context, &entry);
        next.entries.insert(context, entry);
        self.steps.truncate(self.current + 1);
        self.steps.push(next);
        self.current += 1;
    }

    pub fn replace(&mut self, context: SessionHistoryContextId, entry: SessionHistoryEntry) {
        let old_key = self.entry(context).map(|entry| entry.key.clone());
        let descendants = self.descendants(context);
        for (index, step) in self.steps.iter_mut().enumerate() {
            if index == self.current
                || old_key
                    .as_ref()
                    .is_some_and(|key| step.entries.get(&context).is_some_and(|e| &e.key == key))
            {
                if step
                    .entries
                    .get(&context)
                    .is_some_and(|old| old.document != entry.document)
                {
                    step.entries.retain(|id, _| !descendants.contains(id));
                }
                step.entries.insert(context, entry.clone());
            }
        }
    }

    pub fn step_by_delta(&self, delta: i64) -> Option<SessionHistoryStepId> {
        let index = i64::try_from(self.current).ok()?.checked_add(delta)?;
        self.steps
            .get(usize::try_from(index).ok()?)
            .map(|step| step.id)
    }

    pub fn delta_to(&self, target: SessionHistoryStepId) -> Option<i64> {
        let next = self.steps.iter().position(|step| step.id == target)?;
        Some(i64::try_from(next).ok()? - i64::try_from(self.current).ok()?)
    }

    pub fn steps_for_entry(
        &self,
        context: SessionHistoryContextId,
        key: &NavigationHistoryEntryKey,
    ) -> Vec<usize> {
        self.steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| {
                step.entries
                    .get(&context)
                    .is_some_and(|entry| &entry.key == key)
                    .then_some(index)
            })
            .collect()
    }

    /// An entry can span several joint steps made by other frames. Traversal
    /// chooses the nearest such step, preserving intervening frame history.
    pub fn step_for_entry(
        &self,
        context: SessionHistoryContextId,
        key: &NavigationHistoryEntryKey,
    ) -> Option<SessionHistoryStepId> {
        let matches = |step: &&SessionHistoryStep| {
            step.entries
                .get(&context)
                .is_some_and(|entry| &entry.key == key)
        };
        self.steps[..=self.current]
            .iter()
            .rev()
            .find(matches)
            .or_else(|| self.steps[self.current + 1..].iter().find(matches))
            .map(|step| step.id)
    }

    pub fn entries_at(
        &self,
        target: SessionHistoryStepId,
    ) -> Option<&BTreeMap<SessionHistoryContextId, SessionHistoryEntry>> {
        self.steps
            .iter()
            .find(|step| step.id == target)
            .map(|step| &step.entries)
    }

    /// Commit by identity so queued traversals cannot land in a newly pushed
    /// entry that happens to reuse an index after forward-history pruning.
    pub fn traverse(&mut self, target: SessionHistoryStepId) -> Option<i64> {
        let next = self.steps.iter().position(|step| step.id == target)?;
        let delta = i64::try_from(next).ok()? - i64::try_from(self.current).ok()?;
        self.current = next;
        Some(delta)
    }

    pub fn contains_entry(
        &self,
        context: SessionHistoryContextId,
        key: &NavigationHistoryEntryKey,
    ) -> bool {
        self.steps.iter().any(|step| {
            step.entries
                .get(&context)
                .is_some_and(|entry| &entry.key == key)
        })
    }

    pub fn prune_all_but_current(&mut self) {
        let current = self.steps[self.current].clone();
        self.steps = vec![current];
        self.current = 0;
    }

    pub fn detach(&mut self, context: SessionHistoryContextId) {
        let mut removed = self.descendants(context);
        removed.push(context);
        for step in &mut self.steps {
            step.entries.retain(|id, _| !removed.contains(id));
        }
        self.parents.retain(|id, _| !removed.contains(id));
    }

    fn descendants(&self, context: SessionHistoryContextId) -> Vec<SessionHistoryContextId> {
        let mut found = vec![context];
        let mut index = 0;
        while index < found.len() {
            let parent = found[index];
            found.extend(
                self.parents
                    .iter()
                    .filter_map(|(child, candidate)| (*candidate == parent).then_some(*child)),
            );
            index += 1;
        }
        found.remove(0);
        found
    }

    fn remove_replaced_document_children(
        &self,
        step: &mut SessionHistoryStep,
        context: SessionHistoryContextId,
        entry: &SessionHistoryEntry,
    ) {
        if step
            .entries
            .get(&context)
            .is_some_and(|old| old.document != entry.document)
        {
            let descendants = self.descendants(context);
            step.entries.retain(|id, _| !descendants.contains(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, document: &str) -> SessionHistoryEntry {
        SessionHistoryEntry {
            key: NavigationHistoryEntryKey::from_serialized(key.into()),
            document: NavigationHistoryDocumentId::from_serialized(document.into()),
        }
    }

    #[test]
    fn branch_truncates_forward_steps_and_invalidates_queued_targets() {
        let mut history = JointSessionHistory::default();
        let root = SessionHistoryContextId::ROOT;
        history.attach(root, None, entry("a", "top"));
        for key in ["b", "c", "d"] {
            history.push(root, entry(key, "top"));
        }
        let stale = history.current_step();
        let b = history.step_by_delta(-2).unwrap();
        assert_eq!(history.traverse(b), Some(-2));
        history.push(root, entry("e", "top"));
        assert_eq!(
            history.position(),
            SessionHistoryPosition::new(2, 3).unwrap()
        );
        assert_eq!(history.traverse(stale), None);
        assert_eq!(history.entry(root), Some(&entry("e", "top")));
    }

    #[test]
    fn joint_steps_follow_commit_order_and_navigation_traversal_uses_nearest_step() {
        let mut history = JointSessionHistory::default();
        let root = SessionHistoryContextId::ROOT;
        let a = SessionHistoryContextId::allocate();
        let b = SessionHistoryContextId::allocate();
        history.attach(root, None, entry("top", "top"));
        history.attach(a, Some(root), entry("a0", "a"));
        history.attach(b, Some(root), entry("b0", "b"));
        history.push(a, entry("a1", "a"));
        history.push(b, entry("b1", "b"));
        history.push(a, entry("a2", "a"));
        let target = history.step_for_entry(a, &entry("a1", "a").key).unwrap();
        assert_eq!(history.traverse(target), Some(-1));
        assert_eq!(history.entry(b), Some(&entry("b1", "b")));
        history.traverse(history.step_by_delta(-1).unwrap());
        assert_eq!(history.entry(b), Some(&entry("b0", "b")));
        history.push(b, entry("b2", "b"));
        assert_eq!(
            history.position(),
            SessionHistoryPosition::new(2, 3).unwrap()
        );
        assert!(!history.contains_entry(a, &entry("a2", "a").key));
        assert!(!history.contains_entry(b, &entry("b1", "b").key));
    }

    #[test]
    fn replace_preserves_steps_and_updates_shared_entry_references() {
        let mut history = JointSessionHistory::default();
        let root = SessionHistoryContextId::ROOT;
        let child = SessionHistoryContextId::allocate();
        history.attach(root, None, entry("top", "top"));
        history.attach(child, Some(root), entry("child0", "child"));
        history.push(child, entry("child1", "child"));
        let position = history.position();
        history.replace(root, entry("top-replaced", "top"));
        assert_eq!(history.position(), position);
        history.traverse(history.step_by_delta(-1).unwrap());
        assert_eq!(history.entry(root), Some(&entry("top-replaced", "top")));
        history.replace(root, entry("new-document", "new-document"));
        assert!(history.entry(child).is_none());
        assert_eq!(history.position().length(), 2);
    }

    #[test]
    fn browser_position_includes_opaque_steps_and_push_uses_the_cursor() {
        let mut history = JointSessionHistory::new(SessionHistoryPosition::new(1, 4).unwrap());
        let root = SessionHistoryContextId::ROOT;
        history.attach(root, None, entry("current", "document"));
        history.push(root, entry("next", "document"));
        assert_eq!(
            history.position(),
            SessionHistoryPosition::new(2, 3).unwrap()
        );
        assert_eq!(
            history
                .entries_at(history.step_by_delta(-2).unwrap())
                .unwrap()
                .len(),
            0
        );
        assert!(SessionHistoryPosition::new(0, 0).is_none());
        assert!(SessionHistoryPosition::new(3, 3).is_none());
    }
}
