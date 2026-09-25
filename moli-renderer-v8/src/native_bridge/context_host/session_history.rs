use super::OwnerDispatchScope;
use moli_session_history::{
    JointSessionHistory, NavigationHistoryEntryKey, SessionHistoryContextId, SessionHistoryStepId,
};
use std::collections::HashMap;

/// Each traversable owns one history. Lightweight popup Windows share a V8
/// host with their opener, but have independent traversables.
#[derive(Default)]
pub(crate) struct RendererSessionHistories {
    main: JointSessionHistory,
    popups: HashMap<u64, JointSessionHistory>,
    contexts: HashMap<OwnerDispatchScope, SessionHistoryContextId>,
    pending: HashMap<Option<u64>, PendingSessionHistoryTraversal>,
}

struct PendingSessionHistoryTraversal {
    step: SessionHistoryStepId,
    committed: bool,
    remaining: HashMap<SessionHistoryContextId, NavigationHistoryEntryKey>,
    rollback: Option<moli_session_history::SessionHistoryTraversalPlan>,
}

impl RendererSessionHistories {
    pub(crate) fn entry_is_current(
        &self,
        owner: OwnerDispatchScope,
        entry: &moli_session_history::SessionHistoryEntry,
    ) -> bool {
        let Some(context) = self.contexts.get(&owner).copied() else {
            return false;
        };
        self.main.entry(context) == Some(entry)
            || self
                .popups
                .values()
                .any(|history| history.entry(context) == Some(entry))
    }

    pub(crate) fn popup_for_owner(&self, owner: OwnerDispatchScope) -> Option<u64> {
        match owner {
            OwnerDispatchScope::Top => None,
            OwnerDispatchScope::LightweightPopup(id) => Some(id),
            OwnerDispatchScope::Child(_) => {
                let context = self.contexts.get(&owner)?;
                self.popups.iter().find_map(|(popup, history)| {
                    (!history.context_entries(*context).is_empty()).then_some(*popup)
                })
            }
        }
    }

    pub(crate) fn has_pending_traversal(&self, popup: Option<u64>) -> bool {
        self.pending.contains_key(&popup)
    }

    pub(crate) fn begin_traversal(
        &mut self,
        popup: Option<u64>,
        step: SessionHistoryStepId,
        targets: Vec<(SessionHistoryContextId, NavigationHistoryEntryKey)>,
    ) {
        self.pending.insert(
            popup,
            PendingSessionHistoryTraversal {
                step,
                committed: false,
                remaining: targets.into_iter().collect(),
                rollback: None,
            },
        );
    }

    pub(crate) fn commit_traversal(
        &mut self,
        popup: Option<u64>,
        plan: &moli_session_history::SessionHistoryTraversalPlan,
    ) -> Option<i64> {
        let step = plan.target_step();
        let delta = self.get_mut(popup).commit_traversal(plan)?;
        let rollback = self.get_mut(popup).plan_traversal(plan.source_step());
        if let Some(pending) = self.pending.get_mut(&popup)
            && pending.step == step
        {
            pending.committed = true;
            pending.rollback = rollback;
        }
        Some(delta)
    }

    pub(crate) fn finish_traversal_entry(
        &mut self,
        popup: Option<u64>,
        context: SessionHistoryContextId,
        key: Option<&NavigationHistoryEntryKey>,
    ) -> Option<SessionHistoryStepId> {
        let pending = self.pending.get_mut(&popup)?;
        if key.is_none_or(|key| pending.remaining.get(&context) == Some(key)) {
            pending.rollback = None;
            pending.remaining.remove(&context);
        }
        if !pending.remaining.is_empty() {
            return None;
        }
        let pending = self.pending.remove(&popup)?;
        Some(if pending.committed {
            pending.step
        } else {
            self.get_mut(popup).current_step()
        })
    }

    pub(crate) fn finish_traversal_without_document_commit(
        &mut self,
        popup: Option<u64>,
        context: SessionHistoryContextId,
    ) -> Option<(SessionHistoryStepId, Option<i64>)> {
        let pending = self.pending.get_mut(&popup)?;
        pending.remaining.remove(&context)?;
        if !pending.remaining.is_empty() {
            return None;
        }
        let pending = self.pending.remove(&popup)?;
        // No participant materialized its destination. An ignored response
        // must not consume a history.back(), but a newer mutation wins over
        // this reservation: never repair a stale rollback plan.
        let delta = pending
            .rollback
            .as_ref()
            .and_then(|plan| self.get_mut(popup).commit_traversal(plan));
        let step = if delta.is_some() || !pending.committed {
            self.get_mut(popup).current_step()
        } else {
            pending.step
        };
        Some((step, delta))
    }

    pub(crate) fn cancel_traversal_step(
        &mut self,
        popup: Option<u64>,
        step: SessionHistoryStepId,
    ) -> Option<SessionHistoryStepId> {
        if !self
            .pending
            .get(&popup)
            .is_some_and(|pending| pending.step == step)
        {
            return None;
        }
        self.cancel_traversal(popup)
    }

    pub(crate) fn cancel_traversal(&mut self, popup: Option<u64>) -> Option<SessionHistoryStepId> {
        self.pending.remove(&popup)?;
        Some(self.get_mut(popup).current_step())
    }

    pub(crate) fn detach(&mut self, owner: OwnerDispatchScope) {
        if let Some(context) = self.contexts.remove(&owner) {
            self.main.detach(context);
            for history in self.popups.values_mut() {
                history.detach(context);
            }
        }
    }
    pub(crate) fn get_mut(&mut self, popup: Option<u64>) -> &mut JointSessionHistory {
        match popup {
            None => &mut self.main,
            Some(id) => self.popups.entry(id).or_default(),
        }
    }

    pub(crate) fn bind_context(
        &mut self,
        owner: OwnerDispatchScope,
        context: SessionHistoryContextId,
    ) {
        if let Some(previous) = self.contexts.insert(owner, context)
            && previous != context
        {
            self.main.detach(previous);
            for history in self.popups.values_mut() {
                history.detach(previous);
            }
        }
    }

    pub(crate) fn context(&mut self, owner: OwnerDispatchScope) -> SessionHistoryContextId {
        match owner {
            OwnerDispatchScope::Top | OwnerDispatchScope::LightweightPopup(_) => {
                SessionHistoryContextId::ROOT
            }
            OwnerDispatchScope::Child(_) => *self
                .contexts
                .entry(owner)
                .or_insert_with(SessionHistoryContextId::allocate),
        }
    }

    pub(crate) fn owner(
        &self,
        context: SessionHistoryContextId,
        popup: Option<u64>,
    ) -> Option<OwnerDispatchScope> {
        if context == SessionHistoryContextId::ROOT {
            return Some(popup.map_or(
                OwnerDispatchScope::Top,
                OwnerDispatchScope::LightweightPopup,
            ));
        }
        self.contexts
            .iter()
            .find_map(|(owner, id)| (*id == context).then_some(*owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_session_history::{NavigationHistoryDocumentId, SessionHistoryEntry};

    fn entry() -> SessionHistoryEntry {
        SessionHistoryEntry {
            key: NavigationHistoryEntryKey::allocate(),
            document: NavigationHistoryDocumentId::allocate(),
        }
    }

    #[test]
    fn ignored_joint_loads_restore_only_an_unmaterialized_unchanged_reservation() {
        for mode in ["ignored", "partial", "successor"] {
            let mut histories = RendererSessionHistories::default();
            let a = SessionHistoryContextId::allocate();
            let b = SessionHistoryContextId::allocate();
            let first_a = entry();
            let first_b = entry();
            let model = histories.get_mut(None);
            model.attach(SessionHistoryContextId::ROOT, None, entry());
            model.attach(a, Some(SessionHistoryContextId::ROOT), first_a.clone());
            model.attach(b, Some(SessionHistoryContextId::ROOT), first_b.clone());
            let destination = model.current_step();
            model.push(a, entry());
            model.push(b, entry());
            let original = model.current_step();
            let plan = model.plan_traversal(destination).unwrap();
            histories.begin_traversal(
                None,
                destination,
                vec![(a, first_a.key.clone()), (b, first_b.key.clone())],
            );
            assert_eq!(histories.commit_traversal(None, &plan), Some(-2));
            if mode == "partial" {
                assert_eq!(
                    histories.finish_traversal_entry(None, a, Some(&first_a.key)),
                    None
                );
            } else {
                assert_eq!(
                    histories.finish_traversal_without_document_commit(None, a),
                    None
                );
            }
            if mode == "successor" {
                histories.get_mut(None).push(a, entry());
            }
            let accepted = histories.get_mut(None).current_step();
            let (_, delta) = histories
                .finish_traversal_without_document_commit(None, b)
                .unwrap();
            assert_eq!(delta, (mode == "ignored").then_some(2), "{mode}");
            assert_eq!(
                histories.get_mut(None).current_step(),
                if mode == "ignored" {
                    original
                } else {
                    accepted
                },
                "{mode}"
            );
            assert!(!histories.has_pending_traversal(None));
        }
    }
}
