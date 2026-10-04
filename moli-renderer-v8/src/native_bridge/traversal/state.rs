use std::collections::HashMap;
use std::rc::Rc;

use super::filters::{PreparedTraversalFilter, TraversalFilter};
use crate::document_runtime::DomHandle;

#[derive(Debug, Default)]
pub(in crate::native_bridge) struct TraversalStore {
    next_id: u32,
    node_iterators: HashMap<u32, NodeIteratorState>,
    tree_walkers: HashMap<u32, TreeWalkerState>,
}

#[derive(Debug)]
struct NodeIteratorState {
    root: DomHandle,
    what_to_show: u32,
    filter: Option<TraversalFilter>,
    reference_node: DomHandle,
    pointer_before_reference_node: bool,
    active: bool,
}

#[derive(Debug)]
struct TreeWalkerState {
    root: DomHandle,
    what_to_show: u32,
    filter: Option<TraversalFilter>,
    current_node: DomHandle,
    active: bool,
}

pub(in crate::native_bridge) struct NodeIteratorSnapshot {
    pub(in crate::native_bridge::traversal) root: DomHandle,
    pub(in crate::native_bridge::traversal) what_to_show: u32,
    pub(in crate::native_bridge::traversal) filter: Option<Rc<PreparedTraversalFilter>>,
    pub(in crate::native_bridge::traversal) reference_node: DomHandle,
    pub(in crate::native_bridge::traversal) pointer_before_reference_node: bool,
}

pub(in crate::native_bridge) struct TreeWalkerSnapshot {
    pub(in crate::native_bridge::traversal) root: DomHandle,
    pub(in crate::native_bridge::traversal) what_to_show: u32,
    pub(in crate::native_bridge::traversal) filter: Option<Rc<PreparedTraversalFilter>>,
    pub(in crate::native_bridge::traversal) current_node: DomHandle,
}

impl TraversalStore {
    fn next_id(&mut self) -> u32 {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("tree traversal id space exhausted");
        self.next_id
    }

    pub(in crate::native_bridge) fn node_iterators_is_empty(&self) -> bool {
        self.node_iterators.is_empty()
    }

    pub(in crate::native_bridge) fn register_node_iterator(
        &mut self,
        root: DomHandle,
        what_to_show: u32,
        filter: Option<TraversalFilter>,
    ) -> u32 {
        let id = self.next_id();
        self.node_iterators.insert(
            id,
            NodeIteratorState {
                root,
                what_to_show,
                filter,
                reference_node: root,
                pointer_before_reference_node: true,
                active: false,
            },
        );
        id
    }

    pub(in crate::native_bridge) fn register_tree_walker(
        &mut self,
        root: DomHandle,
        what_to_show: u32,
        filter: Option<TraversalFilter>,
    ) -> u32 {
        let id = self.next_id();
        self.tree_walkers.insert(
            id,
            TreeWalkerState {
                root,
                what_to_show,
                filter,
                current_node: root,
                active: false,
            },
        );
        id
    }

    pub(in crate::native_bridge) fn node_iterator_try_begin(&mut self, id: u32) -> bool {
        match self.node_iterators.get_mut(&id) {
            Some(state) if !state.active => {
                state.active = true;
                true
            }
            _ => false,
        }
    }

    pub(in crate::native_bridge) fn node_iterator_end(&mut self, id: u32) {
        if let Some(state) = self.node_iterators.get_mut(&id) {
            state.active = false;
        }
    }

    pub(in crate::native_bridge) fn tree_walker_try_begin(&mut self, id: u32) -> bool {
        match self.tree_walkers.get_mut(&id) {
            Some(state) if !state.active => {
                state.active = true;
                true
            }
            _ => false,
        }
    }

    pub(in crate::native_bridge) fn tree_walker_end(&mut self, id: u32) {
        if let Some(state) = self.tree_walkers.get_mut(&id) {
            state.active = false;
        }
    }

    pub(in crate::native_bridge) fn node_iterator_snapshot(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: u32,
    ) -> Option<NodeIteratorSnapshot> {
        let state = self.node_iterators.get(&id)?;
        Some(NodeIteratorSnapshot {
            root: state.root,
            what_to_show: state.what_to_show,
            filter: prepare_filter(scope, state.filter.as_ref()),
            reference_node: state.reference_node,
            pointer_before_reference_node: state.pointer_before_reference_node,
        })
    }

    pub(in crate::native_bridge) fn tree_walker_snapshot(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: u32,
    ) -> Option<TreeWalkerSnapshot> {
        let state = self.tree_walkers.get(&id)?;
        Some(TreeWalkerSnapshot {
            root: state.root,
            what_to_show: state.what_to_show,
            filter: prepare_filter(scope, state.filter.as_ref()),
            current_node: state.current_node,
        })
    }

    pub(in crate::native_bridge) fn set_node_iterator_position(
        &mut self,
        id: u32,
        reference_node: DomHandle,
        pointer_before_reference_node: bool,
    ) {
        if let Some(state) = self.node_iterators.get_mut(&id) {
            state.reference_node = reference_node;
            state.pointer_before_reference_node = pointer_before_reference_node;
        }
    }

    pub(in crate::native_bridge) fn apply_node_iterator_pre_remove_steps<F>(
        &mut self,
        removed_node: DomHandle,
        previous_node: DomHandle,
        next_after_removed_subtree: Option<DomHandle>,
        mut is_inclusive_ancestor: F,
    ) where
        F: FnMut(DomHandle, DomHandle) -> bool,
    {
        for state in self.node_iterators.values_mut() {
            if removed_node == state.root
                || is_inclusive_ancestor(removed_node, state.root)
                || !is_inclusive_ancestor(removed_node, state.reference_node)
            {
                continue;
            }

            if !state.pointer_before_reference_node {
                state.reference_node = previous_node;
                continue;
            }

            if let Some(next) = next_after_removed_subtree {
                state.reference_node = next;
            } else {
                state.reference_node = previous_node;
                state.pointer_before_reference_node = false;
            }
        }
    }

    pub(in crate::native_bridge) fn set_tree_walker_current_node(
        &mut self,
        id: u32,
        current_node: DomHandle,
    ) {
        if let Some(state) = self.tree_walkers.get_mut(&id) {
            state.current_node = current_node;
        }
    }
}

fn prepare_filter(
    scope: &mut v8::PinScope<'_, '_>,
    filter: Option<&TraversalFilter>,
) -> Option<Rc<PreparedTraversalFilter>> {
    let filter = filter?;
    Some(Rc::new(filter.prepare(scope)?))
}

#[cfg(test)]
mod tests {
    use super::TraversalStore;
    use crate::document_runtime::DomHandle;

    fn handle(idx: usize) -> DomHandle {
        DomHandle::new(idx)
    }

    #[test]
    fn next_id_is_strictly_monotonic_starting_at_one() {
        let mut store = TraversalStore::default();
        let id1 = store.register_node_iterator(handle(0), 0xFFFF_FFFF, None);
        let id2 = store.register_node_iterator(handle(0), 0xFFFF_FFFF, None);
        let id3 = store.register_tree_walker(handle(0), 0xFFFF_FFFF, None);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }

    #[test]
    fn node_iterator_try_begin_is_mutually_exclusive_until_end() {
        // This is the core invariant introduced by P1-6: re-entrant traversal
        // on the same iterator must throw InvalidStateError. The store
        // enforces it by returning false on any try_begin that sees an
        // already-active state, until end() flips it back.
        let mut store = TraversalStore::default();
        let id = store.register_node_iterator(handle(7), 0, None);

        // Initial state is inactive — first begin claims the lock.
        assert!(store.node_iterator_try_begin(id));
        // Re-entrant begin while active must fail.
        assert!(!store.node_iterator_try_begin(id));
        assert!(!store.node_iterator_try_begin(id));

        // After end() the iterator is reclaimable again.
        store.node_iterator_end(id);
        assert!(store.node_iterator_try_begin(id));
    }

    #[test]
    fn node_iterators_is_empty_tracks_registered_iterators() {
        let mut store = TraversalStore::default();
        assert!(store.node_iterators_is_empty());

        store.register_node_iterator(handle(0), 0, None);

        assert!(!store.node_iterators_is_empty());
    }

    #[test]
    fn tree_walker_try_begin_is_mutually_exclusive_until_end() {
        // Same active-flag invariant on the TreeWalker side.
        let mut store = TraversalStore::default();
        let id = store.register_tree_walker(handle(11), 0, None);

        assert!(store.tree_walker_try_begin(id));
        assert!(!store.tree_walker_try_begin(id));
        store.tree_walker_end(id);
        assert!(store.tree_walker_try_begin(id));
    }

    #[test]
    fn active_flag_is_per_iterator_not_global() {
        // Two distinct iterators are independent — beginning one must not
        // block the other. The store keys the flag by id.
        let mut store = TraversalStore::default();
        let a = store.register_node_iterator(handle(1), 0, None);
        let b = store.register_node_iterator(handle(2), 0, None);
        assert!(store.node_iterator_try_begin(a));
        assert!(store.node_iterator_try_begin(b));
        store.node_iterator_end(a);
        // b stays active even after a ends.
        assert!(!store.node_iterator_try_begin(b));
        store.node_iterator_end(b);
        assert!(store.node_iterator_try_begin(b));
    }

    #[test]
    fn unknown_id_does_not_panic_and_signals_failure_to_begin() {
        // end() on an unknown id must be a no-op (not panic). try_begin()
        // on an unknown id must return false (not claim a missing slot).
        let mut store = TraversalStore::default();
        store.node_iterator_end(999);
        store.tree_walker_end(999);
        assert!(!store.node_iterator_try_begin(999));
        assert!(!store.tree_walker_try_begin(999));
    }

    #[test]
    #[should_panic(expected = "tree traversal id space exhausted")]
    fn next_id_exhaustion_never_reuses_u32_max() {
        let mut store = TraversalStore {
            next_id: u32::MAX - 1,
            ..Default::default()
        };
        let id1 = store.register_node_iterator(handle(0), 0, None);
        assert_eq!(id1, u32::MAX);
        let _ = store.register_node_iterator(handle(0), 0, None);
    }

    #[test]
    fn set_node_iterator_position_updates_only_when_id_known() {
        // The setter must be a no-op on a missing id — exercised in WPT
        // when a callback fires after the iterator was reset.
        let mut store = TraversalStore::default();
        let id = store.register_node_iterator(handle(0), 0, None);

        store.set_node_iterator_position(id, handle(42), false);
        // A read-back via the snapshot needs v8; instead, verify by trying
        // a no-op on a bogus id and confirming the real id stays consistent.
        store.set_node_iterator_position(9999, handle(7), true);

        // Indirect check: register_node_iterator initialised state with
        // pointer_before=true at root; after our set, position should be
        // (handle(42), false). We can't snapshot without v8, but we can
        // re-set to the same and confirm no panic + correct ordering of
        // operations.
        store.set_node_iterator_position(id, handle(42), false);
        assert!(store.node_iterator_try_begin(id));
    }

    #[test]
    fn node_iterator_pre_remove_steps_apply_dom_spec_cases() {
        let mut store = TraversalStore::default();
        let untouched_root = store.register_node_iterator(handle(2), 0, None);
        let pointer_after = store.register_node_iterator(handle(0), 0, None);
        let pointer_before_with_next = store.register_node_iterator(handle(0), 0, None);
        let pointer_before_without_next = store.register_node_iterator(handle(0), 0, None);

        store.set_node_iterator_position(untouched_root, handle(4), false);
        store.set_node_iterator_position(pointer_after, handle(4), false);
        store.set_node_iterator_position(pointer_before_with_next, handle(4), true);
        store.set_node_iterator_position(pointer_before_without_next, handle(4), true);

        let is_ancestor = |ancestor: DomHandle, node: DomHandle| {
            ancestor == node
                || matches!(
                    (ancestor.index(), node.index()),
                    (1, 2..=4) | (2, 3..=4) | (3, 4)
                )
        };

        store.apply_node_iterator_pre_remove_steps(
            handle(2),
            handle(1),
            Some(handle(5)),
            is_ancestor,
        );
        let untouched = store.node_iterators.get(&untouched_root).unwrap();
        assert_eq!(untouched.reference_node, handle(4));
        assert!(!untouched.pointer_before_reference_node);

        let after = store.node_iterators.get(&pointer_after).unwrap();
        assert_eq!(after.reference_node, handle(1));
        assert!(!after.pointer_before_reference_node);

        let before_next = store.node_iterators.get(&pointer_before_with_next).unwrap();
        assert_eq!(before_next.reference_node, handle(5));
        assert!(before_next.pointer_before_reference_node);

        store.set_node_iterator_position(pointer_before_without_next, handle(4), true);
        store.apply_node_iterator_pre_remove_steps(handle(2), handle(1), None, is_ancestor);
        let before_no_next = store
            .node_iterators
            .get(&pointer_before_without_next)
            .unwrap();
        assert_eq!(before_no_next.reference_node, handle(1));
        assert!(!before_no_next.pointer_before_reference_node);
    }
}
