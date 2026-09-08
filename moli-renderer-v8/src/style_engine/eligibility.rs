use std::collections::HashSet;

use moli_selector::stylo_attribute_change_can_use_retained_invalidator;

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
};

use super::{StyleAttributeImpact, StyleMutationEffect};

/// Retained sibling traversal can restore one removed element's links, not a
/// historical tree. Only use it while each recorded splice still agrees with
/// the live tree, excluding subsequent insertions validated in reverse order.
/// Check at drain time: later mutations can move either the changed nodes or
/// their old neighbors, including within the same parent.
pub(super) fn child_list_effects_have_current_topology(
    host: &DomHost,
    effects: &[StyleMutationEffect],
) -> bool {
    let mut later_insertions = HashSet::new();
    effects.iter().rev().all(|effect| {
        let StyleMutationEffect::ChildList {
            parent,
            added_nodes,
            removed_nodes,
            removed_element_snapshots,
            previous_sibling,
            next_sibling,
        } = effect
        else {
            return true;
        };
        if removed_nodes.iter().any(|&root| {
            host.node(root)
                .is_none_or(|node| node.parent_node().is_some())
        }) {
            return false;
        }
        // A removed root can stay detached while one of its snapshotted
        // descendants is moved back into the live tree.
        if removed_element_snapshots.iter().any(|snapshot| {
            let mut current = Some(snapshot.handle());
            while let Some(handle) = current {
                if removed_nodes.contains(&handle) {
                    return false;
                }
                current = host.node(handle).and_then(Node::parent_node);
            }
            true
        }) {
            return false;
        }
        if previous_sibling
            .iter()
            .chain(next_sibling)
            .any(|&sibling| host.node(sibling).and_then(Node::parent_node) != Some(*parent))
        {
            return false;
        }
        let Some(parent_node) = host.node(*parent) else {
            return false;
        };
        let mut current = match previous_sibling {
            Some(previous) => host.next_sibling(*previous),
            None => parent_node.first_child(),
        };
        current = skip_later_insertions(host, current, &later_insertions);
        for &added in added_nodes {
            if current != Some(added) {
                return false;
            }
            current = skip_later_insertions(host, host.next_sibling(added), &later_insertions);
        }
        if current != *next_sibling {
            return false;
        }
        // Pure insertion batches are safe: every new subtree is also queried.
        // Do not mistake an appended suffix for a rearranged old sibling chain.
        later_insertions.extend(added_nodes.iter().copied());
        true
    })
}

fn skip_later_insertions(
    host: &DomHost,
    mut current: Option<DomHandle>,
    later_insertions: &HashSet<DomHandle>,
) -> Option<DomHandle> {
    while let Some(handle) = current.filter(|handle| later_insertions.contains(handle)) {
        current = host.next_sibling(handle);
    }
    current
}

pub(super) fn attribute_effect_can_use_retained_stylo_invalidator(name: &str) -> bool {
    stylo_attribute_change_can_use_retained_invalidator(
        name,
        attribute_has_non_css_runtime_side_effect(name),
    )
}

pub(super) fn attribute_has_non_css_runtime_side_effect(name: &str) -> bool {
    matches!(
        StyleAttributeImpact::for_attribute_name(name),
        StyleAttributeImpact::LayoutMetric
            | StyleAttributeImpact::DescendantComputedStyle
            | StyleAttributeImpact::StylesheetLinkage
            | StyleAttributeImpact::LayoutMetricAndStylesheetLinkage
    )
}
