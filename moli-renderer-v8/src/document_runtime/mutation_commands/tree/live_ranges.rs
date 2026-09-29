use std::collections::HashSet;

use crate::{
    context_bootstrap,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    native_bridge::JsContextHost,
};

struct LiveRangeRemoval {
    parent: DomHandle,
    child: DomHandle,
    index: u32,
    previous_sibling: Option<DomHandle>,
}

pub(super) struct LiveRangePreInsertPlan {
    removals: Vec<LiveRangeRemoval>,
    pub(super) insertion_index: u32,
}

impl DocumentRuntime {
    fn prospective_child_index_and_count(
        &self,
        parent: DomHandle,
        excluded_children: &HashSet<DomHandle>,
        target: Option<DomHandle>,
    ) -> (Option<usize>, usize) {
        let mut target_index = None;
        let mut count = 0;
        for candidate in self.dom_host.child_handles(parent) {
            if !excluded_children.contains(&candidate) {
                if Some(candidate) == target {
                    target_index = Some(count);
                }
                count += 1;
            }
        }
        (target_index, count)
    }

    pub(super) fn live_range_pre_insert_plan(
        &self,
        parent: DomHandle,
        insertion_roots: &[DomHandle],
        reference_child: Option<DomHandle>,
    ) -> LiveRangePreInsertPlan {
        let removals = insertion_roots
            .iter()
            .filter_map(|child| {
                let old_parent = self.dom_host.node(*child)?.parent_node()?;
                let old_index = self.dom_host.child_index(old_parent, *child)? as u32;
                Some(LiveRangeRemoval {
                    parent: old_parent,
                    child: *child,
                    index: old_index,
                    previous_sibling: self.dom_host.node(*child).and_then(Node::prev_sibling),
                })
            })
            .collect::<Vec<_>>();
        let excluded_children = insertion_roots.iter().copied().collect::<HashSet<_>>();
        let (reference_index, prospective_child_count) =
            self.prospective_child_index_and_count(parent, &excluded_children, reference_child);
        let insertion_index = reference_index.unwrap_or(prospective_child_count) as u32;
        LiveRangePreInsertPlan {
            removals,
            insertion_index,
        }
    }

    pub(super) fn apply_live_range_pre_insert_plan(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        plan: &LiveRangePreInsertPlan,
    ) {
        for removal in &plan.removals {
            context_bootstrap::live_ranges_child_removal(
                scope,
                host_ptr,
                &self.dom_host,
                removal.parent,
                removal.child,
                removal.index,
                removal.previous_sibling,
            );
        }
    }
}

/// Applies the per-insertion live-range offset adjustment for each inserted
/// child, starting at `insertion_index`. A DocumentFragment hoists all of its
/// children at once but the underlying live-range bookkeeping is "shift any
/// offset > index by 1", so we have to fire it once per inserted child for
/// fragments to land on the same offsets that an equivalent sequence of
/// single-child inserts would have produced.
pub(super) fn apply_live_ranges_child_insertion(
    scope: &mut v8::PinScope<'_, '_>,
    parent: DomHandle,
    insertion_index: u32,
    insertion_roots: &[DomHandle],
) {
    for (offset, inserted_child) in insertion_roots.iter().copied().enumerate() {
        context_bootstrap::live_ranges_child_insertion(
            scope,
            parent,
            insertion_index + offset as u32,
            inserted_child,
        );
    }
}
