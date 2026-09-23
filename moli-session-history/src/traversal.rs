use std::collections::BTreeMap;

use crate::{
    SessionHistoryContextId, SessionHistoryEntry, SessionHistoryRevision, SessionHistoryStepId,
};

/// One context's state change in a joint traversal. A missing destination
/// belongs to a context removed by an ancestor Document replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHistoryContextChange {
    pub context: SessionHistoryContextId,
    pub from: Option<SessionHistoryEntry>,
    pub to: Option<SessionHistoryEntry>,
}

/// An immutable proposal against one timeline revision and set of context
/// transitions. Only the history can construct it; commit revalidates both.
/// Script admission and document loading remain the adapter's responsibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHistoryTraversalPlan {
    pub(crate) revision: SessionHistoryRevision,
    pub(crate) source_step: SessionHistoryStepId,
    pub(crate) target_step: SessionHistoryStepId,
    pub(crate) delta: i64,
    pub(crate) changes: Vec<SessionHistoryContextChange>,
    pub(crate) target_entries: BTreeMap<SessionHistoryContextId, SessionHistoryEntry>,
}

impl SessionHistoryTraversalPlan {
    pub fn revision(&self) -> SessionHistoryRevision {
        self.revision
    }

    pub fn source_step(&self) -> SessionHistoryStepId {
        self.source_step
    }

    pub fn target_step(&self) -> SessionHistoryStepId {
        self.target_step
    }

    pub fn delta(&self) -> i64 {
        self.delta
    }

    pub fn changes(&self) -> &[SessionHistoryContextChange] {
        &self.changes
    }

    /// Complete destination projection, including unchanged contexts. An
    /// adapter may still be materializing an earlier accepted Document load,
    /// so its live views can lag behind the model's source entries.
    pub fn target_entries(&self) -> &BTreeMap<SessionHistoryContextId, SessionHistoryEntry> {
        &self.target_entries
    }
}
