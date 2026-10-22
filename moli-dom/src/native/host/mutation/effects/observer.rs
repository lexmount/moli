use super::{DomAttributeMutation, DomChildListMutation, DomHandle, DomMutationEffects};
use std::sync::Arc;

/// Observer work in mutation order. Subtree removal is independent of the
/// suppress-observers flag and must survive child-list record coalescing.
#[derive(Debug, Clone, Copy)]
pub enum DomMutationObserverEffect<'a> {
    Record(&'a DomMutationRecord),
    SubtreeRemoval {
        node: DomHandle,
        ancestors: &'a [DomHandle],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ObserverStep {
    Record(usize),
    SubtreeRemoval {
        node: DomHandle,
        ancestors: Arc<[DomHandle]>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomMutationRecordBatch {
    pub(super) records: Vec<DomMutationRecord>,
    steps: Vec<ObserverStep>,
}

impl DomMutationRecordBatch {
    pub fn records(&self) -> &[DomMutationRecord] {
        &self.records
    }

    pub fn effects(&self) -> impl Iterator<Item = DomMutationObserverEffect<'_>> {
        self.steps.iter().map(|step| match step {
            ObserverStep::Record(index) => DomMutationObserverEffect::Record(&self.records[*index]),
            ObserverStep::SubtreeRemoval { node, ancestors } => {
                DomMutationObserverEffect::SubtreeRemoval {
                    node: *node,
                    ancestors,
                }
            }
        })
    }

    fn push(&mut self, record: DomMutationRecord) {
        self.steps.push(ObserverStep::Record(self.records.len()));
        self.records.push(record);
    }

    fn retain_records(&mut self, mut retain: impl FnMut(&DomMutationRecord) -> bool) {
        let mut indices = Vec::with_capacity(self.records.len());
        let mut next_index = 0;
        self.records.retain(|record| {
            let keep = retain(record);
            indices.push(keep.then(|| {
                let index = next_index;
                next_index += 1;
                index
            }));
            keep
        });
        self.steps.retain_mut(|step| match step {
            ObserverStep::Record(index) => {
                if let Some(new_index) = indices[*index] {
                    *index = new_index;
                    true
                } else {
                    false
                }
            }
            ObserverStep::SubtreeRemoval { .. } => true,
        });
    }

    pub(super) fn remove_replacement_parts(
        &mut self,
        target: DomHandle,
        added_nodes: &[DomHandle],
        removed_node: DomHandle,
    ) {
        self.retain_records(|record| {
            if record.target != target {
                return true;
            }
            match &record.kind {
                DomMutationRecordKind::ChildList(mutation) => {
                    let record_added_nodes = mutation.added_nodes();
                    let removed_nodes = mutation.removed_nodes();
                    let is_addition = record_added_nodes == added_nodes && removed_nodes.is_empty();
                    let is_removal =
                        record_added_nodes.is_empty() && removed_nodes == [removed_node];
                    !is_addition && !is_removal
                }
                _ => true,
            }
        });
    }

    pub(super) fn merge(&mut self, other: Self) {
        let offset = self.records.len();
        self.steps
            .extend(other.steps.into_iter().map(|step| match step {
                ObserverStep::Record(index) => ObserverStep::Record(index + offset),
                removal @ ObserverStep::SubtreeRemoval { .. } => removal,
            }));
        self.records.extend(other.records);
    }

    pub(super) fn coalesce_child_list_removals(
        &mut self,
        target: DomHandle,
        removed_nodes: &[DomHandle],
    ) {
        if removed_nodes.is_empty() {
            return;
        }

        let mut coalesced_nodes = Vec::new();
        self.retain_records(|record| {
            if record.target != target {
                return true;
            }
            let DomMutationRecordKind::ChildList(mutation) = &record.kind else {
                return true;
            };
            if !mutation.added_nodes().is_empty()
                || mutation
                    .removed_nodes()
                    .iter()
                    .any(|node| !removed_nodes.contains(node))
            {
                return true;
            }
            coalesced_nodes.extend_from_slice(mutation.removed_nodes());
            false
        });

        if coalesced_nodes.is_empty() {
            return;
        }
        debug_assert_eq!(
            coalesced_nodes, removed_nodes,
            "all-children removal records must retain tree order"
        );
        self.push(DomMutationRecord {
            target,
            kind: DomMutationRecordKind::ChildList(DomChildListMutation::new(
                target,
                &[],
                removed_nodes,
                None,
                None,
            )),
        });
    }

    pub(super) fn coalesce_child_list_additions(
        &mut self,
        target: DomHandle,
        added_nodes: &[DomHandle],
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    ) {
        if added_nodes.is_empty() {
            return;
        }

        let mut coalesced_nodes = Vec::new();
        self.retain_records(|record| {
            if record.target != target {
                return true;
            }
            let DomMutationRecordKind::ChildList(mutation) = &record.kind else {
                return true;
            };
            if !mutation.removed_nodes().is_empty()
                || mutation
                    .added_nodes()
                    .iter()
                    .any(|node| !added_nodes.contains(node))
            {
                return true;
            }
            coalesced_nodes.extend_from_slice(mutation.added_nodes());
            false
        });

        if coalesced_nodes.is_empty() {
            return;
        }
        debug_assert_eq!(
            coalesced_nodes, added_nodes,
            "batched insertion records must retain tree order"
        );
        self.push(DomMutationRecord {
            target,
            kind: DomMutationRecordKind::ChildList(DomChildListMutation::new(
                target,
                added_nodes,
                &[],
                previous_sibling,
                next_sibling,
            )),
        });
    }

    pub(super) fn push_attribute_mutation_record(&mut self, mutation: DomAttributeMutation) {
        self.push(DomMutationRecord {
            target: mutation.target,
            kind: DomMutationRecordKind::Attributes(mutation),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomMutationRecord {
    pub(super) target: DomHandle,
    pub(super) kind: DomMutationRecordKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomMutationRecordKind {
    Attributes(DomAttributeMutation),
    CharacterData {
        old_value: Option<crate::native::DomStringValue>,
    },
    ChildList(DomChildListMutation),
}

impl DomMutationRecord {
    pub fn target(&self) -> DomHandle {
        self.target
    }

    pub fn kind(&self) -> &DomMutationRecordKind {
        &self.kind
    }
}

impl DomMutationEffects {
    pub(super) fn push_mutation_record(&mut self, record: DomMutationRecord) {
        self.changed = true;
        self.observer_records.push(record);
    }

    pub(in crate::native::host::mutation) fn clear_mutation_records(&mut self) {
        self.observer_records.retain_records(|_| false);
    }

    pub(in crate::native::host::mutation) fn mark_observer_subtree_removal(
        &mut self,
        node: DomHandle,
        ancestors: Arc<[DomHandle]>,
    ) {
        self.observer_records
            .steps
            .push(ObserverStep::SubtreeRemoval { node, ancestors });
    }

    /// Apply the DOM suppress-observers flag to this operation's target. Source
    /// removals and non-observer mutation effects must remain observable.
    pub fn suppress_child_list_mutations_for_target(&mut self, target: DomHandle) {
        self.observer_records.retain_records(|record| {
            record.target != target || !matches!(record.kind, DomMutationRecordKind::ChildList(_))
        });
    }

    pub(in crate::native::host::mutation) fn mark_character_data_mutation(
        &mut self,
        target: DomHandle,
        old_value: Option<crate::native::DomStringValue>,
    ) {
        self.push_mutation_record(DomMutationRecord {
            target,
            kind: DomMutationRecordKind::CharacterData { old_value },
        });
    }

    pub fn queue_character_data_mutation(
        &mut self,
        target: DomHandle,
        old_value: Option<crate::native::DomStringValue>,
    ) {
        self.mark_character_data_mutation(target, old_value);
    }

    fn push_child_list_mutation_record(
        &mut self,
        target: DomHandle,
        added_nodes: &[DomHandle],
        removed_nodes: &[DomHandle],
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    ) {
        self.push_mutation_record(DomMutationRecord {
            target,
            kind: DomMutationRecordKind::ChildList(DomChildListMutation::new(
                target,
                added_nodes,
                removed_nodes,
                previous_sibling,
                next_sibling,
            )),
        });
    }

    pub fn queue_child_list_mutation(
        &mut self,
        target: DomHandle,
        added_nodes: &[DomHandle],
        removed_nodes: &[DomHandle],
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    ) {
        if added_nodes.is_empty() && removed_nodes.is_empty() {
            return;
        }
        self.push_child_list_mutation_record(
            target,
            added_nodes,
            removed_nodes,
            previous_sibling,
            next_sibling,
        );
    }

    /// Coalesces the per-child observer records produced while implementing a
    /// single all-children removal into the one record required by the DOM
    /// replace-all semantics. Style/tree payloads remain precise per child.
    pub fn coalesce_child_list_removals(&mut self, target: DomHandle, removed_nodes: &[DomHandle]) {
        self.observer_records
            .coalesce_child_list_removals(target, removed_nodes);
    }

    /// Coalesces the per-child observer records produced while committing an
    /// already-detached insertion batch. Style/tree payloads remain precise per
    /// child; only the web-observable MutationObserver record is batched.
    pub fn coalesce_child_list_additions(
        &mut self,
        target: DomHandle,
        added_nodes: &[DomHandle],
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    ) {
        self.observer_records.coalesce_child_list_additions(
            target,
            added_nodes,
            previous_sibling,
            next_sibling,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtree_removals_keep_their_order_when_record_batches_merge_and_coalesce() {
        let parent = DomHandle::new(1);
        let first = DomHandle::new(2);
        let second = DomHandle::new(3);
        let text = DomHandle::new(4);
        let ancestors: Arc<[DomHandle]> = Arc::from([parent]);
        let mut effects = DomMutationEffects::default();
        effects.queue_character_data_mutation(text, None);
        effects.mark_observer_subtree_removal(first, Arc::clone(&ancestors));
        effects.queue_child_list_mutation(parent, &[], &[first], None, Some(second));
        let mut next = DomMutationEffects::default();
        next.mark_observer_subtree_removal(second, ancestors);
        next.queue_child_list_mutation(parent, &[], &[second], None, None);
        effects.merge(next);
        effects.coalesce_child_list_removals(parent, &[first, second]);

        let steps = effects.observer_records().effects().collect::<Vec<_>>();
        assert_eq!(steps.len(), 4);
        assert!(
            matches!(steps[0], DomMutationObserverEffect::Record(record) if record.target() == text)
        );
        assert!(
            matches!(steps[1], DomMutationObserverEffect::SubtreeRemoval { node, ancestors } if node == first && ancestors == [parent])
        );
        assert!(
            matches!(steps[2], DomMutationObserverEffect::SubtreeRemoval { node, ancestors } if node == second && ancestors == [parent])
        );
        assert!(
            matches!(steps[3], DomMutationObserverEffect::Record(record) if record.target() == parent)
        );

        effects.suppress_child_list_mutations_for_target(parent);
        assert_eq!(effects.observer_records().records().len(), 1);
        assert_eq!(effects.observer_records().effects().count(), 3);
        effects.clear_mutation_records();
        assert!(effects.observer_records().records().is_empty());
        assert_eq!(effects.observer_records().effects().count(), 2);
    }

    #[test]
    fn coalesces_insertion_batch_without_discarding_other_records() {
        let target = DomHandle::new(1);
        let first = DomHandle::new(2);
        let second = DomHandle::new(3);
        let unrelated_target = DomHandle::new(4);
        let previous = DomHandle::new(5);
        let next = DomHandle::new(6);
        let mut effects = DomMutationEffects::default();
        effects.queue_child_list_mutation(target, &[first], &[], previous.into(), next.into());
        effects.queue_child_list_mutation(target, &[second], &[], first.into(), next.into());
        effects.queue_child_list_mutation(unrelated_target, &[first], &[], None, None);

        effects.coalesce_child_list_additions(target, &[first, second], Some(previous), Some(next));

        let records = effects.observer_records().records();
        assert_eq!(records.len(), 2);
        let coalesced = records
            .iter()
            .find(|record| record.target() == target)
            .expect("coalesced target record");
        let DomMutationRecordKind::ChildList(coalesced) = coalesced.kind() else {
            panic!("expected child-list record");
        };
        assert_eq!(coalesced.added_nodes(), &[first, second]);
        assert!(coalesced.removed_nodes().is_empty());
        assert_eq!(coalesced.previous_sibling(), Some(previous));
        assert_eq!(coalesced.next_sibling(), Some(next));
        assert!(
            records
                .iter()
                .any(|record| record.target() == unrelated_target)
        );
    }
}
