use std::collections::{BTreeMap, BTreeSet};

use crate::{
    NavigationHistoryEntryKey, SessionHistoryContextId, SessionHistoryEntry,
    SessionHistoryPosition, SessionHistoryStepId,
};
use crate::{SessionHistoryContextChange, SessionHistoryRevision, SessionHistoryTraversalPlan};

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionHistoryStep {
    id: SessionHistoryStepId,
    entries: BTreeMap<SessionHistoryContextId, SessionHistoryEntry>,
}

/// Page-owned authority for joint history. No URL, Navigation API index, or
/// per-Window cached length participates in history bookkeeping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JointSessionHistory {
    revision: SessionHistoryRevision,
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
            revision: SessionHistoryRevision::allocate(),
            steps: (0..position.length)
                .map(|_| SessionHistoryStep {
                    id: SessionHistoryStepId::allocate(),
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
        next.id = SessionHistoryStepId::allocate();
        self.remove_replaced_document_children(&mut next, context, &entry);
        next.entries.insert(context, entry);
        self.steps.truncate(self.current + 1);
        self.steps.push(next);
        self.current += 1;
        self.revision = SessionHistoryRevision::allocate();
    }

    pub fn replace(&mut self, context: SessionHistoryContextId, entry: SessionHistoryEntry) {
        if self.entry(context) == Some(&entry) {
            return;
        }
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
        self.revision = SessionHistoryRevision::allocate();
    }

    pub fn step_by_delta(&self, delta: i64) -> Option<SessionHistoryStepId> {
        self.step_by_delta_from(self.current_step(), delta)
    }

    /// Resolve queued relative work without changing even a temporary cursor.
    pub fn step_by_delta_from(
        &self,
        source: SessionHistoryStepId,
        delta: i64,
    ) -> Option<SessionHistoryStepId> {
        let source = self.steps.iter().position(|step| step.id == source)?;
        let index = i64::try_from(source).ok()?.checked_add(delta)?;
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

    /// Capture the complete joint transition before an adapter runs admission.
    pub fn plan_traversal(
        &self,
        target: SessionHistoryStepId,
    ) -> Option<SessionHistoryTraversalPlan> {
        let next = self.steps.iter().position(|step| step.id == target)?;
        let delta = i64::try_from(next).ok()? - i64::try_from(self.current).ok()?;
        let source = &self.steps[self.current].entries;
        let destination = &self.steps[next].entries;
        let contexts = source
            .keys()
            .chain(destination.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let changes = contexts
            .into_iter()
            .filter_map(|context| {
                let from = source.get(&context);
                let to = destination.get(&context);
                (from != to).then(|| SessionHistoryContextChange {
                    context,
                    from: from.cloned(),
                    to: to.cloned(),
                })
            })
            .collect();
        Some(SessionHistoryTraversalPlan {
            revision: self.revision,
            source_step: self.current_step(),
            target_step: target,
            delta,
            changes,
            target_entries: destination.clone(),
        })
    }

    pub fn is_traversal_plan_current(&self, plan: &SessionHistoryTraversalPlan) -> bool {
        self.revision == plan.revision
            && self.current_step() == plan.source_step
            && self
                .plan_traversal(plan.target_step)
                .is_some_and(|current| current.changes == plan.changes)
    }

    /// Commit exactly the admitted proposal. A stale plan cannot be repaired
    /// by silently resolving its target again; admission must be restarted.
    pub fn commit_traversal(&mut self, plan: &SessionHistoryTraversalPlan) -> Option<i64> {
        if !self.is_traversal_plan_current(plan) {
            return None;
        }
        let next = self
            .steps
            .iter()
            .position(|step| step.id == plan.target_step)?;
        self.current = next;
        self.revision = SessionHistoryRevision::allocate();
        Some(plan.delta)
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
        if self.steps.len() == 1 {
            return;
        }
        let current = self.steps[self.current].clone();
        self.steps = vec![current];
        self.current = 0;
        self.revision = SessionHistoryRevision::allocate();
    }

    pub fn detach(&mut self, context: SessionHistoryContextId) {
        let mut removed = self.descendants(context);
        removed.push(context);
        if !self.parents.keys().any(|id| removed.contains(id))
            && !self
                .steps
                .iter()
                .any(|step| step.entries.keys().any(|id| removed.contains(id)))
        {
            return;
        }
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
