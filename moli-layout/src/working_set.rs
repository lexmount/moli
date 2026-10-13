//! Owned computation state, separate from the published immutable geometry.
//!
//! An input update still uses the shared box constructor. Compatible topology
//! keeps its box slots and unchanged inline preparation; incompatible topology
//! retires the complete old graph. Numeric layout is conservatively recomputed.

use std::{fmt::Debug, hash::Hash};

use crate::{LayoutBox, LayoutBoxId, LayoutWorld, text::LayoutFontInputs};

mod retention;
#[cfg(test)]
mod tests;

/// Admission limits for a working tree. These bound source complexity, not
/// total allocator bytes or shared font/style payloads. The estimated-byte
/// charge additionally limits owned buffers and models opaque text-layout
/// storage; it is not an allocator measurement or a physical-memory ceiling.
/// Embedders must also aggregate working sets across their live documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutWorkingSetLimits {
    pub max_boxes: usize,
    pub max_text_bytes: usize,
    pub max_estimated_retained_bytes: usize,
}

impl Default for LayoutWorkingSetLimits {
    fn default() -> Self {
        Self {
            max_boxes: 4096,
            max_text_bytes: 256 * 1024,
            max_estimated_retained_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutWorkingSetStats {
    pub retained_boxes: usize,
    pub retained_text_bytes: usize,
    /// Admission charge for retained buffers. Includes visible allocation
    /// capacities and an opaque Parley storage model, but excludes shared
    /// computed-style/font payloads and allocator metadata. Shared source text
    /// may be charged more than once; this is not resident-memory accounting.
    pub estimated_retained_bytes: usize,
    pub reused_box_slots: usize,
    pub reused_inline_contexts: usize,
}

/// Optional, caller-owned state for one exact document and layout view.
///
/// The caller clears this at owner retirement and keeps transient views out
/// of the canonical slot. It contains no source/resolver borrow. Box IDs are
/// private to a compatible graph; a structural mismatch retires that graph
/// instead of aliasing old IDs to new nodes. Clearing only affects cost.
pub struct LayoutWorkingSet<N>
where
    N: Copy + Debug + Eq + Hash,
{
    retained: Option<RetainedWork<N>>,
    limits: LayoutWorkingSetLimits,
    stats: LayoutWorkingSetStats,
}

pub(crate) struct RetainedWork<N>
where
    N: Copy + Debug + Eq + Hash,
{
    boxes: Vec<LayoutBox<N>>,
    root: LayoutBoxId,
    root_is_document_element: bool,
    quirks_mode: style::context::QuirksMode,
    fonts: LayoutFontInputs,
}

impl<N> Default for LayoutWorkingSet<N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn default() -> Self {
        Self::new(LayoutWorkingSetLimits::default())
    }
}

impl<N> LayoutWorkingSet<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub const fn new(limits: LayoutWorkingSetLimits) -> Self {
        Self {
            retained: None,
            limits,
            stats: LayoutWorkingSetStats {
                retained_boxes: 0,
                retained_text_bytes: 0,
                estimated_retained_bytes: 0,
                reused_box_slots: 0,
                reused_inline_contexts: 0,
            },
        }
    }

    pub fn clear(&mut self) {
        self.retained = None;
        self.stats = LayoutWorkingSetStats::default();
    }

    pub const fn stats(&self) -> LayoutWorkingSetStats {
        self.stats
    }

    pub(crate) fn take(&mut self) -> Option<RetainedWork<N>> {
        self.stats = LayoutWorkingSetStats::default();
        self.retained.take()
    }

    pub(crate) fn publish(
        &mut self,
        mut world: LayoutWorld<N>,
        fonts: LayoutFontInputs,
        reused_box_slots: usize,
        reused_inline_contexts: usize,
        construction_box_count: usize,
    ) {
        self.clear();
        // Numeric layout can append placeholders which are neither source
        // boxes nor part of the next construction graph. Their only incoming
        // edges are layout_children, cleared below along with numeric output.
        assert!(construction_box_count <= world.boxes.len());
        world.boxes.truncate(construction_box_count);
        let text_bytes = world.boxes.iter().fold(0usize, |bytes, layout_box| {
            bytes.saturating_add(layout_box.text.as_deref().map_or(0, str::len))
        });
        self.stats.reused_box_slots = reused_box_slots;
        self.stats.reused_inline_contexts = reused_inline_contexts;
        if world.len() > self.limits.max_boxes || text_bytes > self.limits.max_text_bytes {
            return;
        }
        let mut retained = RetainedWork {
            boxes: std::mem::take(&mut world.boxes),
            root: world.root,
            root_is_document_element: world.root_is_document_element,
            quirks_mode: world.quirks_mode,
            fonts,
        };
        // Everything else in LayoutWorld is a pass-only index or numeric
        // workspace. Retain only the graph and preparation actually reused.
        drop(world);
        let estimated_retained_bytes = retained.prune_and_estimate();
        if estimated_retained_bytes > self.limits.max_estimated_retained_bytes {
            return;
        }
        self.stats.retained_boxes = retained.boxes.len();
        self.stats.retained_text_bytes = text_bytes;
        self.stats.estimated_retained_bytes = estimated_retained_bytes;
        self.retained = Some(retained);
    }
}

pub(crate) fn update_working_tree<N>(
    previous: Option<RetainedWork<N>>,
    mut current: LayoutWorld<N>,
    fonts: &LayoutFontInputs,
) -> (LayoutWorld<N>, usize)
where
    N: Copy + Debug + Eq + Hash,
{
    let Some(mut previous) = previous else {
        return (current, 0);
    };
    if previous.fonts != *fonts || !same_topology(&previous, &current) {
        return (current, 0);
    }

    let mut unchanged = previous
        .boxes
        .iter()
        .zip(&current.boxes)
        .map(|(old, new)| same_inline_input(old, new))
        .collect::<Vec<_>>();
    // Construction indices need not be preorder: anonymous wrappers can be
    // allocated after their children. Propagate dependency changes bottom-up
    // through the actual normalized forest, once per edge.
    let mut remaining = current
        .boxes
        .iter()
        .map(|layout_box| layout_box.children.len())
        .collect::<Vec<_>>();
    let mut leaves = remaining
        .iter()
        .enumerate()
        .filter_map(|(index, children)| (*children == 0).then_some(index))
        .collect::<Vec<_>>();
    while let Some(index) = leaves.pop() {
        if let Some(parent) = current.boxes[index].parent {
            let parent = parent.index();
            unchanged[parent] &= unchanged[index];
            remaining[parent] -= 1;
            if remaining[parent] == 0 {
                leaves.push(parent);
            }
        }
    }

    let count = current.len();
    let fresh = std::mem::replace(&mut current.boxes, std::mem::take(&mut previous.boxes));
    for (index, (slot, input)) in current.boxes.iter_mut().zip(fresh).enumerate() {
        let prepared_inline = unchanged[index]
            .then(|| slot.inline_layout.take())
            .flatten();
        // Fresh source/style/intrinsic/scroll data replaces the old input.
        // No numeric result or stale paint resource is copied into this pass.
        *slot = input;
        slot.inline_layout = prepared_inline;
    }
    (current, count)
}

fn same_topology<N>(old: &RetainedWork<N>, new: &LayoutWorld<N>) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    old.root == new.root
        && old.root_is_document_element == new.root_is_document_element
        && old.quirks_mode == new.quirks_mode
        && old.boxes.len() == new.boxes.len()
        && old.boxes.iter().zip(&new.boxes).all(|(old, new)| {
            old.source == new.source
                && old.owner == new.owner
                && old.pseudo == new.pseudo
                && old.kind == new.kind
                && old.anonymous_reason == new.anonymous_reason
                && old.structural_parent == new.structural_parent
                && old.parent == new.parent
                && old.children == new.children
                && old.inline_formatting_context == new.inline_formatting_context
                && old.outside_list_marker == new.outside_list_marker
        })
}

fn same_inline_input<N>(old: &LayoutBox<N>, new: &LayoutBox<N>) -> bool {
    let (Some(old_computed), Some(new_computed)) = (
        old.style.stylo_computed_values(),
        new.style.stylo_computed_values(),
    ) else {
        // Synthetic styles may be adjusted independently of a computed owner.
        // Keep their normal builder path until they have an explicit input key.
        return false;
    };
    style::servo_arc::Arc::ptr_eq(old_computed, new_computed)
        && old.element_semantics == new.element_semantics
        && old.text == new.text
        && old.text_selection == new.text_selection
        && old.style.display() == new.style.display()
        && old.style.is_out_of_flow() == new.style.is_out_of_flow()
        && old.style.is_floated() == new.style.is_floated()
        && old.style.white_space_collapse() == new.style.white_space_collapse()
        && old.style.text_transform() == new.style.text_transform()
        && old.style.direction() == new.style.direction()
        && old.style.unicode_bidi() == new.style.unicode_bidi()
        && old.style.vertical_align() == new.style.vertical_align()
        && old.style.writing_mode() == new.style.writing_mode()
        && old.style.includes_used_font_metrics() == new.style.includes_used_font_metrics()
}
