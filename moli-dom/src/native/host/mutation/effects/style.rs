use std::sync::Arc;

use super::{DomHandle, DomMutationEffects};
use crate::native::DomStringValue;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomStyleInvalidationInputs {
    pub(super) attribute_mutations: Vec<DomAttributeMutation>,
    pub(super) child_list_mutations: Vec<DomChildListMutation>,
    pub(super) character_data_mutations: Vec<DomHandle>,
}

impl DomStyleInvalidationInputs {
    pub fn attribute_mutations(&self) -> &[DomAttributeMutation] {
        &self.attribute_mutations
    }

    pub fn child_list_mutations(&self) -> &[DomChildListMutation] {
        &self.child_list_mutations
    }

    pub fn character_data_mutations(&self) -> &[DomHandle] {
        &self.character_data_mutations
    }

    fn mark_character_data_mutation(&mut self, target: DomHandle) {
        if !self.character_data_mutations.contains(&target) {
            self.character_data_mutations.push(target);
        }
    }

    pub(super) fn push_attribute_mutation(&mut self, mutation: DomAttributeMutation) {
        if !self.attribute_mutations.contains(&mutation) {
            self.attribute_mutations.push(mutation);
        }
    }

    fn push_child_list_mutation(&mut self, mutation: DomChildListMutation) {
        if !self.child_list_mutations.contains(&mutation) {
            self.child_list_mutations.push(mutation);
        }
    }

    pub(super) fn merge(&mut self, other: Self) {
        for mutation in other.attribute_mutations {
            self.push_attribute_mutation(mutation);
        }
        for target in other.character_data_mutations {
            self.mark_character_data_mutation(target);
        }
        for mutation in other.child_list_mutations {
            self.push_child_list_mutation(mutation);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomAttributeMutation {
    pub(super) target: DomHandle,
    pub(super) local_name: Arc<str>,
    pub(super) namespace: Option<Arc<str>>,
    pub(super) old_value: Option<Arc<DomStringValue>>,
    pub(super) new_value: Option<Arc<DomStringValue>>,
}

impl DomAttributeMutation {
    pub(super) fn new(
        target: DomHandle,
        local_name: &str,
        namespace: Option<&str>,
        old_value: Option<Arc<DomStringValue>>,
        new_value: Option<Arc<DomStringValue>>,
    ) -> Self {
        Self {
            target,
            local_name: Arc::from(local_name),
            namespace: namespace.map(Arc::from),
            old_value,
            new_value,
        }
    }

    pub fn target(&self) -> DomHandle {
        self.target
    }

    pub fn local_name(&self) -> &str {
        &self.local_name
    }

    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Scalar view for style consumers; DOMString consumers use `shared_old_value`.
    pub fn old_value(&self) -> Option<&str> {
        self.old_value.as_deref().map(DomStringValue::as_str_lossy)
    }

    /// Scalar view for style consumers; DOMString consumers use `shared_new_value`.
    pub fn new_value(&self) -> Option<&str> {
        self.new_value.as_deref().map(DomStringValue::as_str_lossy)
    }

    pub fn shared_old_value(&self) -> Option<Arc<DomStringValue>> {
        self.old_value.clone()
    }

    pub fn shared_new_value(&self) -> Option<Arc<DomStringValue>> {
        self.new_value.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomChildListMutation {
    pub(super) target: DomHandle,
    pub(super) added_nodes: Arc<[DomHandle]>,
    pub(super) removed_nodes: Arc<[DomHandle]>,
    pub(super) previous_sibling: Option<DomHandle>,
    pub(super) next_sibling: Option<DomHandle>,
}

impl DomChildListMutation {
    pub(super) fn new(
        target: DomHandle,
        added_nodes: &[DomHandle],
        removed_nodes: &[DomHandle],
        previous_sibling: Option<DomHandle>,
        next_sibling: Option<DomHandle>,
    ) -> Self {
        Self {
            target,
            added_nodes: Arc::from(added_nodes),
            removed_nodes: Arc::from(removed_nodes),
            previous_sibling,
            next_sibling,
        }
    }

    pub fn target(&self) -> DomHandle {
        self.target
    }

    pub fn added_nodes(&self) -> &[DomHandle] {
        &self.added_nodes
    }

    pub fn shared_added_nodes(&self) -> Arc<[DomHandle]> {
        Arc::clone(&self.added_nodes)
    }

    pub fn removed_nodes(&self) -> &[DomHandle] {
        &self.removed_nodes
    }

    pub fn shared_removed_nodes(&self) -> Arc<[DomHandle]> {
        Arc::clone(&self.removed_nodes)
    }

    pub fn previous_sibling(&self) -> Option<DomHandle> {
        self.previous_sibling
    }

    pub fn next_sibling(&self) -> Option<DomHandle> {
        self.next_sibling
    }
}

impl DomMutationEffects {
    pub(in crate::native::host::mutation) fn mark_style_character_data_mutation(
        &mut self,
        target: DomHandle,
    ) {
        self.changed = true;
        self.style.mark_character_data_mutation(target);
    }

    pub(in crate::native::host::mutation) fn mark_style_child_list_mutation(
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
        self.changed = true;
        let mutation = DomChildListMutation::new(
            target,
            added_nodes,
            removed_nodes,
            previous_sibling,
            next_sibling,
        );
        self.push_style_child_list_mutation(mutation);
    }

    pub(super) fn push_style_child_list_mutation(&mut self, mutation: DomChildListMutation) {
        self.style.push_child_list_mutation(mutation);
    }
}
