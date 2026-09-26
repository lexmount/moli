use std::sync::Arc;

use super::super::DomHandle;

mod observer;
mod script;
mod slot;
mod style;
mod stylesheet;
mod text_control;
mod tree;

pub use observer::{DomMutationRecord, DomMutationRecordBatch, DomMutationRecordKind};
pub use script::{DomScriptMutationEffects, ScriptPrepareTrigger, ScriptPrepareTriggerKind};
pub use slot::{DomSlotAssignmentChange, DomSlotMutationEffects};
pub use style::{DomAttributeMutation, DomChildListMutation, DomStyleInvalidationInputs};
pub use stylesheet::{
    DomStylesheetOwnerChange, DomStylesheetOwnerChangeKind, DomStylesheetOwnerTransitions,
    DomStylesheetOwnerTreeScopes,
};
pub use text_control::DomTextareaValueChange;
pub use tree::DomTreeMutationEffects;

/// One synchronous DOM mutation result, partitioned by semantic followup
/// domain. A consumer may read several domains, but each domain owns its
/// payload and merge invariants here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomMutationEffects {
    changed: bool,
    tree: DomTreeMutationEffects,
    scripts: DomScriptMutationEffects,
    slots: DomSlotMutationEffects,
    style: DomStyleInvalidationInputs,
    stylesheet_owners: DomStylesheetOwnerTransitions,
    observer_records: DomMutationRecordBatch,
    textarea_values: Vec<DomTextareaValueChange>,
}

/// Result of an attribute operation that shares the captured pre-mutation
/// value with style invalidation and MutationObserver payloads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomAttributeMutationOutcome {
    effects: DomMutationEffects,
    old_value: Option<Arc<str>>,
}

impl DomAttributeMutationOutcome {
    pub(super) fn new(effects: DomMutationEffects, old_value: Option<Arc<str>>) -> Self {
        Self { effects, old_value }
    }

    pub fn effects(&self) -> &DomMutationEffects {
        &self.effects
    }

    pub fn old_value(&self) -> Option<&str> {
        self.old_value.as_deref()
    }

    pub fn into_effects(self) -> DomMutationEffects {
        self.effects
    }

    pub fn into_parts(self) -> (DomMutationEffects, Option<Arc<str>>) {
        (self.effects, self.old_value)
    }
}

impl DomMutationEffects {
    pub(super) fn changed() -> Self {
        Self {
            changed: true,
            ..Self::default()
        }
    }

    pub fn did_change(&self) -> bool {
        self.changed
    }

    pub fn tree(&self) -> &DomTreeMutationEffects {
        &self.tree
    }

    pub fn scripts(&self) -> &DomScriptMutationEffects {
        &self.scripts
    }

    pub fn slots(&self) -> &DomSlotMutationEffects {
        &self.slots
    }

    pub fn style(&self) -> &DomStyleInvalidationInputs {
        &self.style
    }

    pub fn stylesheet_owners(&self) -> &DomStylesheetOwnerTransitions {
        &self.stylesheet_owners
    }

    pub fn observer_records(&self) -> &DomMutationRecordBatch {
        &self.observer_records
    }

    pub fn textarea_values(&self) -> &[DomTextareaValueChange] {
        &self.textarea_values
    }

    pub fn coalesce_child_list_replacement(
        &mut self,
        target: DomHandle,
        added_nodes: &[DomHandle],
        removed_node: DomHandle,
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    ) {
        self.observer_records
            .remove_replacement_parts(target, added_nodes, removed_node);
        self.mark_child_list_mutation(
            target,
            added_nodes,
            std::slice::from_ref(&removed_node),
            previous_sibling,
            next_sibling,
        );
    }

    pub(super) fn mark_child_list_mutation(
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
        let mutation = DomChildListMutation::new(
            target,
            added_nodes,
            removed_nodes,
            previous_sibling,
            next_sibling,
        );
        self.push_style_child_list_mutation(mutation.clone());
        self.push_mutation_record(DomMutationRecord {
            target,
            kind: DomMutationRecordKind::ChildList(mutation),
        });
    }

    pub(in crate::native::host::mutation) fn mark_attribute_change(
        &mut self,
        target: DomHandle,
        attribute_name: &str,
        attribute_namespace: Option<&str>,
        old_value: Option<Arc<str>>,
        new_value: Option<&str>,
        queues_observer_record: bool,
    ) {
        let mutation = DomAttributeMutation::new(
            target,
            attribute_name,
            attribute_namespace,
            old_value,
            new_value,
        );
        self.changed = true;
        self.style.push_attribute_mutation(mutation.clone());
        if queues_observer_record {
            self.observer_records
                .push_attribute_mutation_record(mutation);
        }
    }

    pub(in crate::native::host::mutation) fn queue_attribute_mutation_record(
        &mut self,
        target: DomHandle,
        attribute_name: &str,
        attribute_namespace: Option<&str>,
        old_value: Option<Arc<str>>,
        new_value: Option<&str>,
    ) {
        self.changed = true;
        self.observer_records
            .push_attribute_mutation_record(DomAttributeMutation::new(
                target,
                attribute_name,
                attribute_namespace,
                old_value,
                new_value,
            ));
    }

    pub fn merge(&mut self, other: Self) {
        self.changed |= other.changed;
        self.tree.merge(other.tree);
        self.scripts.merge(other.scripts);
        self.slots.merge(other.slots);
        self.style.merge(other.style);
        self.stylesheet_owners.merge(other.stylesheet_owners);
        self.observer_records.merge(other.observer_records);
        self.textarea_values.extend(other.textarea_values);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_and_observer_child_list_batches_share_one_payload() {
        let target = DomHandle::new(1);
        let added = [DomHandle::new(2), DomHandle::new(3)];
        let removed = [DomHandle::new(4)];
        let mut effects = DomMutationEffects::default();
        effects.mark_child_list_mutation(target, &added, &removed, None, None);

        let style = &effects.style.child_list_mutations[0];
        let DomMutationRecordKind::ChildList(observer) = &effects.observer_records.records[0].kind
        else {
            panic!("expected child-list observer record");
        };
        assert!(Arc::ptr_eq(&style.added_nodes, &observer.added_nodes));
        assert!(Arc::ptr_eq(&style.removed_nodes, &observer.removed_nodes));
    }

    #[test]
    fn merged_style_and_observer_batches_keep_the_shared_payload() {
        let target = DomHandle::new(1);
        let added = [DomHandle::new(2), DomHandle::new(3)];
        let removed = [DomHandle::new(4)];
        let mut appended = DomMutationEffects::default();
        appended.mark_child_list_mutation(target, &added, &removed, None, None);

        let mut effects = DomMutationEffects::default();
        effects.merge(appended);

        let style = &effects.style.child_list_mutations[0];
        let DomMutationRecordKind::ChildList(observer) = &effects.observer_records.records[0].kind
        else {
            panic!("expected child-list observer record");
        };
        assert!(Arc::ptr_eq(&style.added_nodes, &observer.added_nodes));
        assert!(Arc::ptr_eq(&style.removed_nodes, &observer.removed_nodes));
    }

    #[test]
    fn style_and_observer_attribute_batches_share_the_string_payload() {
        let target = DomHandle::new(1);
        let mut effects = DomMutationEffects::default();
        effects.mark_attribute_change(
            target,
            "data-state",
            Some("urn:test"),
            Some(Arc::from("old")),
            Some("new"),
            true,
        );

        let style = &effects.style.attribute_mutations[0];
        let DomMutationRecordKind::Attributes(observer) = &effects.observer_records.records[0].kind
        else {
            panic!("expected attribute observer record");
        };
        assert!(Arc::ptr_eq(&style.local_name, &observer.local_name));
        assert!(Arc::ptr_eq(
            style.namespace.as_ref().expect("style namespace"),
            observer.namespace.as_ref().expect("observer namespace"),
        ));
        assert!(Arc::ptr_eq(
            style.old_value.as_ref().expect("style old value"),
            observer.old_value.as_ref().expect("observer old value"),
        ));
        assert!(Arc::ptr_eq(
            style.new_value.as_ref().expect("style new value"),
            observer.new_value.as_ref().expect("observer new value"),
        ));
    }

    #[test]
    fn attribute_style_input_does_not_depend_on_observer_recording() {
        let target = DomHandle::new(1);
        let mut effects = DomMutationEffects::default();
        effects.mark_attribute_change(
            target,
            "class",
            None,
            Some(Arc::from("before")),
            Some("after"),
            false,
        );

        assert_eq!(effects.style.attribute_mutations.len(), 1);
        assert!(effects.observer_records.records.is_empty());
        assert_eq!(
            effects.style.attribute_mutations[0].old_value(),
            Some("before")
        );
        assert_eq!(
            effects.style.attribute_mutations[0].new_value(),
            Some("after")
        );
    }
}
