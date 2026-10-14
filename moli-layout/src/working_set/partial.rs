//! Conservative IFC transfer after the shared constructor changes topology.
//!
//! This does not preserve construction or remap IDs. The first supported local
//! closure is an ordinary source-backed block with direct text children. More
//! complex inline/anonymous/list/table/control structure takes the cold path.

use std::{fmt::Debug, hash::Hash};

use super::{RetainedBox, RetainedWork, same_box_identity, same_inline_input};
use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutDisplay, LayoutElementCategory, LayoutNamespace, LayoutWorld,
};

pub(super) fn unchanged_plain_inline_contexts<N>(
    previous: &RetainedWork<N>,
    current: &LayoutWorld<N>,
) -> (Vec<bool>, usize)
where
    N: Copy + Debug + Eq + Hash,
{
    let mut unchanged = vec![false; current.len()];
    let mut matched_slots = 0;
    for (owner, inline) in &previous.inline_contexts {
        let Some(old) = previous.boxes.get(owner.index()) else {
            continue;
        };
        let Some(new) = current.boxes.get(owner.index()) else {
            continue;
        };
        if !plain_block(old)
            || !old.inline_formatting_context
            || old.children.is_empty()
            || !same_box_identity(old, new)
            || old.children != new.children
            || !same_inline_input(old, new)
            || inline.root_style != *owner
            || !inline.objects.is_empty()
            || !inline.structural_boxes.is_empty()
            || !unchanged_ancestor_context(previous, current, *owner)
        {
            continue;
        }
        let unchanged_children = old.children.iter().copied().all(|child| {
            let Some(old_child) = previous.boxes.get(child.index()) else {
                return false;
            };
            let Some(new_child) = current.boxes.get(child.index()) else {
                return false;
            };
            old_child.kind == LayoutBoxKind::Text
                && old_child.source.is_some()
                && old_child.owner.is_none()
                && old_child.pseudo.is_none()
                && old_child.anonymous_reason.is_none()
                && !old_child.outside_list_marker
                && !old_child.inline_formatting_context
                && old_child.parent == Some(*owner)
                && old_child.structural_parent == Some(*owner)
                && old_child.children.is_empty()
                && same_box_identity(old_child, new_child)
                && old_child.children == new_child.children
                && same_inline_input(old_child, new_child)
        });
        if !unchanged_children
            || !inline.references_only(|id| id == *owner || old.children.contains(&id))
        {
            continue;
        }
        unchanged[owner.index()] = true;
        // These closures cannot overlap: each consists of one block and its
        // direct Text leaves. Changed ancestor child lists are not counted as
        // reused construction identities, nor is the shared builder skipped.
        matched_slots += 1 + old.children.len();
    }
    (unchanged, matched_slots)
}

fn plain_block<N>(layout_box: &RetainedBox<N>) -> bool {
    layout_box.source.is_some()
        && layout_box.owner.is_none()
        && layout_box.pseudo.is_none()
        && layout_box.anonymous_reason.is_none()
        && !layout_box.outside_list_marker
        && layout_box.structural_parent == layout_box.parent
        && matches!(
            layout_box.kind,
            LayoutBoxKind::PrincipalBlock | LayoutBoxKind::PrincipalFlowRoot
        )
        && matches!(
            layout_box.inline_style.display,
            LayoutDisplay::Block | LayoutDisplay::FlowRoot
        )
        && !layout_box.inline_style.out_of_flow
        && !layout_box.inline_style.floated
        && layout_box
            .element_semantics
            .as_ref()
            .is_some_and(|semantics| {
                semantics.namespace == LayoutNamespace::Html
                    && semantics.category == LayoutElementCategory::Generic
                    && semantics.replaced.is_none()
                    && semantics.metadata.form_control.is_none()
            })
}

fn unchanged_ancestor_context<N>(
    previous: &RetainedWork<N>,
    current: &LayoutWorld<N>,
    owner: LayoutBoxId,
) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    let mut child = owner;
    let mut parent = previous.boxes[owner.index()].parent;
    // Both graphs came from the shared constructor. Parent identities name
    // their actual edges; do not scan every sibling list once per paragraph.
    // The bound also makes a malformed cycle an ordinary reuse rejection.
    for _ in 0..previous.boxes.len() {
        let Some(id) = parent else {
            return child == previous.root;
        };
        let Some(old) = previous.boxes.get(id.index()) else {
            return false;
        };
        let Some(new) = current.boxes.get(id.index()) else {
            return false;
        };
        if !plain_block(old) || !same_box_identity(old, new) || !same_inline_input(old, new) {
            return false;
        }
        // Siblings are not plain-text shaping inputs. Their construction,
        // numeric constraints, floats, positioning and projection are fresh.
        // Eligible final numeric reuse still checks its existing full key.
        child = id;
        parent = old.parent;
    }
    false
}
