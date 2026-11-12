use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    native_bridge::PageScriptHost,
};

pub(super) struct NodeIteratorRemovalPlan {
    pub(super) removed_node: DomHandle,
    pub(super) previous_node: DomHandle,
    pub(super) next_after_removed_subtree: Option<DomHandle>,
}

impl DocumentRuntime {
    pub(super) fn node_iterator_pre_insert_remove_plan(
        &self,
        insertion_roots: &[DomHandle],
        reference_child: Option<DomHandle>,
    ) -> Vec<NodeIteratorRemovalPlan> {
        insertion_roots
            .iter()
            .filter_map(|&child| {
                if Some(child) == reference_child {
                    return None;
                }
                let parent = self.dom_host.node(child).and_then(Node::parent_node)?;
                self.node_iterator_pre_remove_plan(parent, child)
            })
            .collect()
    }

    pub(super) fn node_iterator_pre_remove_plan(
        &self,
        parent: DomHandle,
        child: DomHandle,
    ) -> Option<NodeIteratorRemovalPlan> {
        if self.dom_host.node(child).and_then(Node::parent_node) != Some(parent) {
            return None;
        }
        Some(NodeIteratorRemovalPlan {
            removed_node: child,
            previous_node: self.node_preceding_removed_child(parent, child),
            next_after_removed_subtree: self.node_following_removed_subtree(child),
        })
    }

    pub(super) fn apply_node_iterator_pre_remove_plans(
        &self,
        host_ptr: *mut PageScriptHost,
        plans: &[NodeIteratorRemovalPlan],
    ) {
        for plan in plans {
            self.apply_node_iterator_pre_remove_plan(host_ptr, plan);
        }
    }

    pub(super) fn apply_node_iterator_pre_remove_plan(
        &self,
        host_ptr: *mut PageScriptHost,
        plan: &NodeIteratorRemovalPlan,
    ) {
        unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .apply_node_iterator_pre_remove_steps(
                plan.removed_node,
                plan.previous_node,
                plan.next_after_removed_subtree,
                |ancestor, node| self.is_ancestor_of(ancestor, node),
            );
    }

    fn node_preceding_removed_child(&self, parent: DomHandle, child: DomHandle) -> DomHandle {
        self.dom_host
            .previous_sibling(child)
            .map(|sibling| self.last_preorder_descendant(sibling))
            .unwrap_or(parent)
    }

    fn node_following_removed_subtree(&self, node: DomHandle) -> Option<DomHandle> {
        let mut current = Some(node);
        while let Some(handle) = current {
            if let Some(sibling) = self.dom_host.next_sibling(handle) {
                return Some(sibling);
            }
            current = self.dom_host.node(handle).and_then(Node::parent_node);
        }
        None
    }

    fn last_preorder_descendant(&self, node: DomHandle) -> DomHandle {
        let mut current = node;
        while let Some(child) = self.dom_host.last_child(current) {
            current = child;
        }
        current
    }
}
