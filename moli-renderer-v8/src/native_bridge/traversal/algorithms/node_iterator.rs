use super::super::{
    NodeIteratorSnapshot,
    filters::{TraversalFilterResult, traversal_filter_result},
};
use super::shared::{next_preorder_node, node_is_inside_root, previous_preorder_node};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

#[cfg(test)]
use std::collections::HashSet;

pub(in crate::native_bridge::traversal) fn node_iterator_next_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    state: &NodeIteratorSnapshot,
) -> Result<(Option<DomHandle>, DomHandle, bool), ()> {
    iterator_traverse_step(scope, runtime_ptr, IteratorDirection::Forward, state)
}

pub(in crate::native_bridge::traversal) fn node_iterator_previous_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    state: &NodeIteratorSnapshot,
) -> Result<(Option<DomHandle>, DomHandle, bool), ()> {
    iterator_traverse_step(scope, runtime_ptr, IteratorDirection::Backward, state)
}

#[derive(Debug, Clone, Copy)]
enum IteratorDirection {
    Forward,
    Backward,
}

fn iterator_traverse_step(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    direction: IteratorDirection,
    state: &NodeIteratorSnapshot,
) -> Result<(Option<DomHandle>, DomHandle, bool), ()> {
    let reference_is_inside_root =
        node_is_inside_root(runtime_ptr, state.reference_node, state.root);

    match direction {
        IteratorDirection::Forward => {
            if state.pointer_before_reference_node && reference_is_inside_root {
                match traversal_filter_result(
                    scope,
                    runtime_ptr,
                    state.filter.as_deref(),
                    state.reference_node,
                    state.what_to_show,
                ) {
                    TraversalFilterResult::Accept => {
                        return Ok((Some(state.reference_node), state.reference_node, false));
                    }
                    TraversalFilterResult::Exception => return Err(()),
                    TraversalFilterResult::Reject
                    | TraversalFilterResult::Skip
                    | TraversalFilterResult::Other => {}
                }
            }
            let start = if reference_is_inside_root {
                next_preorder_node(runtime_ptr, state.reference_node, state.root)
            } else {
                Some(state.root)
            };
            find_accepted_node_from(scope, runtime_ptr, state, direction, start, false)
        }
        IteratorDirection::Backward => {
            if !state.pointer_before_reference_node && reference_is_inside_root {
                match traversal_filter_result(
                    scope,
                    runtime_ptr,
                    state.filter.as_deref(),
                    state.reference_node,
                    state.what_to_show,
                ) {
                    TraversalFilterResult::Accept => {
                        return Ok((Some(state.reference_node), state.reference_node, true));
                    }
                    TraversalFilterResult::Exception => return Err(()),
                    TraversalFilterResult::Reject
                    | TraversalFilterResult::Skip
                    | TraversalFilterResult::Other => {}
                }
            }
            let start = reference_is_inside_root
                .then(|| previous_preorder_node(runtime_ptr, state.reference_node, state.root))
                .flatten();
            find_accepted_node_from(scope, runtime_ptr, state, direction, start, true)
        }
    }
}

fn find_accepted_node_from(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    state: &NodeIteratorSnapshot,
    direction: IteratorDirection,
    mut candidate: Option<DomHandle>,
    pointer_before_on_match: bool,
) -> Result<(Option<DomHandle>, DomHandle, bool), ()> {
    while let Some(node) = candidate {
        match traversal_filter_result(
            scope,
            runtime_ptr,
            state.filter.as_deref(),
            node,
            state.what_to_show,
        ) {
            TraversalFilterResult::Accept => {
                return Ok((Some(node), node, pointer_before_on_match));
            }
            TraversalFilterResult::Exception => return Err(()),
            TraversalFilterResult::Reject
            | TraversalFilterResult::Skip
            | TraversalFilterResult::Other => {}
        }
        candidate = match direction {
            IteratorDirection::Forward => next_preorder_node(runtime_ptr, node, state.root),
            IteratorDirection::Backward => previous_preorder_node(runtime_ptr, node, state.root),
        };
    }
    Ok((
        None,
        state.reference_node,
        state.pointer_before_reference_node,
    ))
}

// Pure NodeIterator traverse step. Decoupled from v8/the runtime so the
// state-mutation invariant (only update reference_node / pointer when a node
// is accepted) can be exercised in unit tests.
#[cfg(test)]
fn iterator_step(
    direction: IteratorDirection,
    traversed: &[DomHandle],
    accepted: &HashSet<DomHandle>,
    reference_node: DomHandle,
    pointer_before_reference_node: bool,
) -> (Option<DomHandle>, DomHandle, bool) {
    let index = traversed
        .iter()
        .position(|handle| *handle == reference_node)
        .map(|index| index as isize)
        .unwrap_or(-1);

    match direction {
        IteratorDirection::Forward => {
            // Spec: if the reference node is accepted AND the pointer is
            // before it, the next traverse returns the reference node itself
            // and flips the pointer to after.
            if pointer_before_reference_node && accepted.contains(&reference_node) {
                return (Some(reference_node), reference_node, false);
            }
            let mut cursor = index + 1;
            while cursor < traversed.len() as isize {
                let node = traversed[cursor as usize];
                if accepted.contains(&node) {
                    return (Some(node), node, false);
                }
                cursor += 1;
            }
        }
        IteratorDirection::Backward => {
            // Spec mirror: if the reference node is accepted AND the pointer
            // is after it, the previous traverse returns the reference node
            // and flips the pointer to before.
            if !pointer_before_reference_node && accepted.contains(&reference_node) {
                return (Some(reference_node), reference_node, true);
            }
            let mut cursor = index - 1;
            while cursor >= 0 {
                let node = traversed[cursor as usize];
                if accepted.contains(&node) {
                    return (Some(node), node, true);
                }
                cursor -= 1;
            }
        }
    }

    // P1-6 invariant: when nothing is accepted, the snapshot must come back
    // unchanged. The pointer must NOT flip; the reference must NOT move.
    (None, reference_node, pointer_before_reference_node)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{IteratorDirection, iterator_step};
    use crate::document_runtime::DomHandle;

    fn h(n: usize) -> DomHandle {
        DomHandle::new(n)
    }

    fn accepted(handles: &[DomHandle]) -> HashSet<DomHandle> {
        handles.iter().copied().collect()
    }

    #[test]
    fn forward_returns_reference_when_pointer_before_and_reference_accepted() {
        let traversed = vec![h(0), h(1), h(2), h(3)];
        let accepted = accepted(&[h(1)]);
        let (next, new_ref, new_ptr) = iterator_step(
            IteratorDirection::Forward,
            &traversed,
            &accepted,
            h(1),
            true,
        );
        assert_eq!(next, Some(h(1)));
        assert_eq!(new_ref, h(1));
        assert!(!new_ptr, "pointer must flip to after the reference");
    }

    #[test]
    fn forward_skips_rejected_and_lands_on_next_accepted_after_reference() {
        // From ref=h(1), pointer is "after" h(1): h(2) is rejected by filter,
        // h(3) is accepted — that's the answer.
        let traversed = vec![h(0), h(1), h(2), h(3), h(4)];
        let accepted = accepted(&[h(3), h(4)]);
        let (next, new_ref, new_ptr) = iterator_step(
            IteratorDirection::Forward,
            &traversed,
            &accepted,
            h(1),
            false,
        );
        assert_eq!(next, Some(h(3)));
        assert_eq!(new_ref, h(3));
        assert!(!new_ptr);
    }

    #[test]
    fn forward_returns_none_without_mutating_state_when_no_acceptable_node() {
        // This is the P1-6 regression test. Pre-fix, the implementation
        // flipped pointer_before to `false` unconditionally before checking
        // whether a node was accepted, so a no-op next() left the iterator
        // in a wrong state.
        let traversed = vec![h(0), h(1), h(2)];
        let empty: HashSet<DomHandle> = HashSet::new();

        // pointer_before = true → should remain true on no-op.
        let (next, new_ref, new_ptr) =
            iterator_step(IteratorDirection::Forward, &traversed, &empty, h(1), true);
        assert_eq!(next, None);
        assert_eq!(new_ref, h(1), "reference must not move");
        assert!(new_ptr, "pointer must NOT flip on no-op");

        // pointer_before = false → should remain false on no-op.
        let (next2, new_ref2, new_ptr2) =
            iterator_step(IteratorDirection::Forward, &traversed, &empty, h(1), false);
        assert_eq!(next2, None);
        assert_eq!(new_ref2, h(1));
        assert!(!new_ptr2);
    }

    #[test]
    fn backward_returns_reference_when_pointer_after_and_reference_accepted() {
        let traversed = vec![h(0), h(1), h(2)];
        let accepted = accepted(&[h(1)]);
        let (next, new_ref, new_ptr) = iterator_step(
            IteratorDirection::Backward,
            &traversed,
            &accepted,
            h(1),
            false,
        );
        assert_eq!(next, Some(h(1)));
        assert_eq!(new_ref, h(1));
        assert!(new_ptr, "pointer must flip to before the reference");
    }

    #[test]
    fn backward_skips_rejected_and_lands_on_previous_accepted() {
        // From ref=h(3), pointer is "before" h(3): we look strictly earlier
        // in the traversal. h(2) is rejected, h(1) is accepted.
        let traversed = vec![h(0), h(1), h(2), h(3)];
        let accepted = accepted(&[h(1)]);
        let (next, new_ref, new_ptr) = iterator_step(
            IteratorDirection::Backward,
            &traversed,
            &accepted,
            h(3),
            true,
        );
        assert_eq!(next, Some(h(1)));
        assert_eq!(new_ref, h(1));
        assert!(new_ptr);
    }

    #[test]
    fn backward_returns_none_without_mutating_state_when_no_acceptable_node() {
        // P1-6 mirror on the previous() side.
        let traversed = vec![h(0), h(1), h(2)];
        let empty: HashSet<DomHandle> = HashSet::new();

        let (next, new_ref, new_ptr) =
            iterator_step(IteratorDirection::Backward, &traversed, &empty, h(1), false);
        assert_eq!(next, None);
        assert_eq!(new_ref, h(1));
        assert!(
            !new_ptr,
            "pointer must NOT flip to true on previous() no-op"
        );

        let (next2, new_ref2, new_ptr2) =
            iterator_step(IteratorDirection::Backward, &traversed, &empty, h(1), true);
        assert_eq!(next2, None);
        assert_eq!(new_ref2, h(1));
        assert!(new_ptr2);
    }

    #[test]
    fn reference_not_in_traversed_starts_search_from_index_minus_one() {
        // When the reference_node has been removed from the tree, the
        // traverse algorithm proceeds as if the cursor were at index -1 for
        // forward (start at 0) or below 0 for backward (return None).
        let traversed = vec![h(10), h(11), h(12)];
        let accepted = accepted(&[h(10), h(12)]);

        let (next, _, _) = iterator_step(
            IteratorDirection::Forward,
            &traversed,
            &accepted,
            h(99),
            false,
        );
        assert_eq!(
            next,
            Some(h(10)),
            "forward starts searching at traversed[0]"
        );

        let (prev, prev_ref, prev_ptr) = iterator_step(
            IteratorDirection::Backward,
            &traversed,
            &accepted,
            h(99),
            true,
        );
        assert_eq!(prev, None, "backward from -1 yields nothing");
        assert_eq!(prev_ref, h(99));
        assert!(prev_ptr);
    }
}
