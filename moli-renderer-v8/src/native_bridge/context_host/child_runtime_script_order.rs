use std::collections::{HashMap, VecDeque};

use crate::{
    document_runtime::DomHandle,
    frame_owner_model::{FrameDocumentScriptReadyTaskWork, FrameDocumentTaskOwner, FrameRealmId},
    types::ScriptMode,
};

struct RuntimeScriptEntry {
    node: DomHandle,
    position: usize,
    dispatched: bool,
    ready: Option<(FrameRealmId, FrameDocumentScriptReadyTaskWork)>,
}

/// The HTML in-order list retains scripts until earlier elements have run.
/// Eligible work then enters the stable Page source, which owns task FIFO.
#[derive(Default)]
pub(super) struct ChildRuntimeScriptOrder {
    next_position: usize,
    documents: HashMap<FrameDocumentTaskOwner, VecDeque<RuntimeScriptEntry>>,
}

impl ChildRuntimeScriptOrder {
    pub(super) fn register(
        &mut self,
        owner: FrameDocumentTaskOwner,
        node: DomHandle,
        mode: ScriptMode,
    ) -> usize {
        if let Some(entry) = self
            .documents
            .get(&owner)
            .and_then(|entries| entries.iter().find(|entry| entry.node == node))
        {
            return entry.position;
        }
        let position = self.next_position;
        self.next_position = position
            .checked_add(1)
            .expect("child runtime script position overflow");
        if matches!(mode, ScriptMode::InOrder | ScriptMode::ModuleInOrder) {
            self.documents
                .entry(owner)
                .or_default()
                .push_back(RuntimeScriptEntry {
                    node,
                    position,
                    dispatched: false,
                    ready: None,
                });
        }
        position
    }

    pub(super) fn waiting_for_predecessor(
        &self,
        owner: FrameDocumentTaskOwner,
        node: DomHandle,
    ) -> bool {
        self.documents.get(&owner).is_some_and(|entries| {
            entries
                .iter()
                .position(|entry| entry.node == node)
                .is_some_and(|index| index != 0)
        })
    }

    pub(super) fn admit_or_retain(
        &mut self,
        realm_id: FrameRealmId,
        work: FrameDocumentScriptReadyTaskWork,
    ) -> Option<FrameDocumentScriptReadyTaskWork> {
        let route = work.route();
        let Some(entries) = self.documents.get_mut(&route.task_owner()) else {
            return Some(work);
        };
        let Some(index) = entries
            .iter()
            .position(|entry| entry.node == route.script_handle())
        else {
            return Some(work);
        };
        let entry = &mut entries[index];
        // Evaluation reactions may arrive after initial execution was admitted.
        if index == 0 || entry.dispatched {
            entry.dispatched = true;
            return Some(work);
        }
        debug_assert!(
            entry.ready.is_none(),
            "an ordered script has one initial ready result"
        );
        entry.ready = Some((realm_id, work));
        None
    }

    pub(super) fn finish(
        &mut self,
        owner: FrameDocumentTaskOwner,
        node: DomHandle,
    ) -> Option<(FrameRealmId, FrameDocumentScriptReadyTaskWork)> {
        let entries = self.documents.get_mut(&owner)?;
        let index = entries.iter().position(|entry| entry.node == node)?;
        entries.remove(index);
        let next = entries.front_mut().and_then(|entry| {
            let ready = entry.ready.take()?;
            entry.dispatched = true;
            Some(ready)
        });
        if entries.is_empty() {
            self.documents.remove(&owner);
        }
        next
    }

    pub(super) fn remove_document(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Vec<FrameDocumentScriptReadyTaskWork> {
        self.documents
            .remove(&owner)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ready.map(|(_, work)| work))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_owner_model::{
        DocumentId, FrameDocumentRealmBoundScriptWork, FrameSchedulerLaneId, LocalWindowId,
        PendingChildDynamicDocumentScript,
    };

    fn owner(document: u64) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(1),
            LocalWindowId(2),
            DocumentId(document),
        )
    }

    fn work(owner: FrameDocumentTaskOwner, node: DomHandle) -> FrameDocumentScriptReadyTaskWork {
        FrameDocumentRealmBoundScriptWork::DynamicClassic(PendingChildDynamicDocumentScript {
            child_handle: DomHandle::new(1),
            owner,
            realm_id: Some(FrameRealmId(2)),
            script_handle: node,
            source: String::new(),
            script_nonce: None,
            script_integrity: None,
        })
        .into()
    }

    #[test]
    fn runtime_order_uses_admission_not_node_order_and_does_not_hold_async() {
        let mut order = ChildRuntimeScriptOrder::default();
        let owner = owner(3);
        let (first, second, third, independent) = (
            DomHandle::new(40),
            DomHandle::new(20),
            DomHandle::new(10),
            DomHandle::new(5),
        );
        assert!(
            order.register(owner, first, ScriptMode::InOrder)
                < order.register(owner, second, ScriptMode::ModuleInOrder)
        );
        order.register(owner, third, ScriptMode::InOrder);
        order.register(owner, independent, ScriptMode::Async);
        let realm = FrameRealmId(2);
        assert!(order.admit_or_retain(realm, work(owner, third)).is_none());
        assert!(order.admit_or_retain(realm, work(owner, second)).is_none());
        assert!(
            order
                .admit_or_retain(realm, work(owner, independent))
                .is_some()
        );
        assert!(order.admit_or_retain(realm, work(owner, first)).is_some());
        assert_eq!(
            order
                .finish(owner, first)
                .unwrap()
                .1
                .route()
                .script_handle(),
            second
        );
        // A TLA reaction for the admitted module is independent of later elements.
        assert!(order.admit_or_retain(realm, work(owner, second)).is_some());
        assert_eq!(
            order
                .finish(owner, second)
                .unwrap()
                .1
                .route()
                .script_handle(),
            third
        );
        assert!(order.finish(owner, third).is_none());
        assert!(order.documents.is_empty());
    }

    #[test]
    fn runtime_order_rollback_and_document_retirement_leave_other_owners_runnable() {
        let mut order = ChildRuntimeScriptOrder::default();
        let (first, rejected, last) = (DomHandle::new(3), DomHandle::new(4), DomHandle::new(5));
        let (old, current) = (owner(10), owner(11));
        for node in [first, rejected, last] {
            order.register(old, node, ScriptMode::ModuleInOrder);
        }
        let realm = FrameRealmId(2);
        assert!(order.admit_or_retain(realm, work(old, last)).is_none());
        assert!(order.finish(old, rejected).is_none());
        order.register(current, first, ScriptMode::ModuleInOrder);
        assert!(order.admit_or_retain(realm, work(current, first)).is_some());
        let retired = order.remove_document(old);
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].route().script_handle(), last);
        assert!(order.finish(old, first).is_none());
        assert!(order.finish(current, first).is_none());
        assert!(order.documents.is_empty());
    }
}
