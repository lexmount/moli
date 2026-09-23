//! Rust-owned admission state. Promise callbacks retain only a transaction ID.

use super::{PendingNavigationResult, WindowExecutionContextOwner};
use moli_session_history::{SessionHistoryEntry, SessionHistoryTraversalPlan};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct HistoryTraversalId(u64);

impl HistoryTraversalId {
    pub(crate) fn allocate() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self(
            NEXT_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("history traversal ID overflow"),
        )
    }

    pub(crate) fn raw(self) -> u64 {
        self.0
    }

    pub(crate) fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Default)]
pub(crate) struct TraversalParticipantOutcome {
    pub(crate) intercepted: bool,
    pub(crate) destination: Option<v8::Global<v8::Object>>,
    pub(crate) signal: Option<v8::Global<v8::Object>>,
    pub(crate) event: Option<v8::Global<v8::Object>>,
    pub(crate) intercept_result: Option<v8::Global<v8::Value>>,
    pub(crate) intercept_error: Option<v8::Global<v8::Value>>,
}

pub(crate) struct HistoryTraversalParticipant {
    pub(crate) execution_owner: WindowExecutionContextOwner,
    pub(crate) history: v8::Global<v8::Object>,
    pub(crate) navigation: Option<v8::Global<v8::Object>>,
    pub(crate) source: SessionHistoryEntry,
    pub(crate) source_url: String,
    pub(crate) destination: SessionHistoryEntry,
    pub(crate) outcome: TraversalParticipantOutcome,
}

pub(crate) struct PendingHistoryTraversalAdmission {
    pub(crate) id: HistoryTraversalId,
    pub(crate) active: Cell<bool>,
    pub(crate) plan: SessionHistoryTraversalPlan,
    pub(crate) initiator: v8::Global<v8::Object>,
    pub(crate) execution_owner: WindowExecutionContextOwner,
    pub(crate) participants: Vec<HistoryTraversalParticipant>,
    pub(crate) results: Vec<PendingNavigationResult>,
    pub(crate) remaining_precommit: Cell<usize>,
}

/// Pending admissions remain discoverable through unload script until acceptance.
/// Callbacks clone an Rc before invoking script, never retaining a table borrow.
#[derive(Default)]
pub(crate) struct PendingHistoryTraversalAdmissions {
    entries: HashMap<HistoryTraversalId, Rc<PendingHistoryTraversalAdmission>>,
    retired: Vec<Rc<PendingHistoryTraversalAdmission>>,
}

impl PendingHistoryTraversalAdmissions {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.retired.is_empty()
    }

    pub(crate) fn insert(
        &mut self,
        admission: PendingHistoryTraversalAdmission,
    ) -> HistoryTraversalId {
        let id = admission.id;
        self.entries.insert(id, Rc::new(admission));
        id
    }

    pub(crate) fn take(
        &mut self,
        id: HistoryTraversalId,
    ) -> Option<Rc<PendingHistoryTraversalAdmission>> {
        self.entries.remove(&id)
    }

    pub(crate) fn fulfill_precommit(
        &mut self,
        id: HistoryTraversalId,
    ) -> Option<Rc<PendingHistoryTraversalAdmission>> {
        let admission = self.entries.get(&id)?;
        let remaining = admission.remaining_precommit.get().checked_sub(1)?;
        admission.remaining_precommit.set(remaining);
        (remaining == 0).then(|| Rc::clone(admission))
    }

    pub(crate) fn take_for_navigation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        navigation: v8::Local<'_, v8::Object>,
    ) -> Option<Rc<PendingHistoryTraversalAdmission>> {
        let id = self.entries.iter().find_map(|(id, admission)| {
            admission
                .participants
                .iter()
                .any(|participant| {
                    participant.navigation.as_ref().is_some_and(|candidate| {
                        v8::Local::new(scope, candidate).strict_equals(navigation.into())
                    })
                })
                .then_some(*id)
        })?;
        self.entries.remove(&id)
    }

    pub(crate) fn take_for_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> Vec<Rc<PendingHistoryTraversalAdmission>> {
        self.entries
            .extract_if(|_, admission| {
                admission.execution_owner == owner
                    || admission
                        .participants
                        .iter()
                        .any(|participant| participant.execution_owner == owner)
            })
            .map(|(_, admission)| {
                // Invalidate even an Rc already held by an executing callback.
                admission.active.set(false);
                admission
            })
            .collect()
    }

    /// Native retirement cannot run author script while borrowing JsContextHost.
    /// Remove callbacks' tokens now; the VM settles the owned batch after the
    /// native stack has returned. No cleanup of a successor runs after events.
    pub(crate) fn retire_owner(&mut self, owner: WindowExecutionContextOwner) {
        let admissions = self.take_for_owner(owner);
        self.retired.extend(admissions);
    }

    pub(crate) fn take_retired(&mut self) -> Vec<Rc<PendingHistoryTraversalAdmission>> {
        std::mem::take(&mut self.retired)
    }

    pub(crate) fn clear(&mut self) {
        for admission in self.entries.values() {
            admission.active.set(false);
        }
        self.entries.clear();
        self.retired.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}
