//! Owned computation state, separate from the published immutable geometry.
//!
//! The shared box constructor can restore a supported graph after validating
//! its source dependencies. Every pass still materializes fresh boxes, whose
//! compatible topology validates retained slot identities and unchanged inline
//! preparation. Eligible inline numeric results additionally reuse their exact
//! input keys; all remaining numeric work uses the same shared backend.

use std::{fmt::Debug, hash::Hash, sync::Arc};

use style::{properties::ComputedValues, servo_arc::Arc as ServoArc};

use crate::{
    LayoutAnonymousReason, LayoutBox, LayoutBoxId, LayoutBoxKind, LayoutDisplay,
    LayoutElementSemantics, LayoutPseudo, LayoutTextSelection, LayoutWorld, ResolvedLayoutStyle,
    builder::ConstructionInputs,
    inline::{InlineFormattingContext, InlineNumericReuse},
    style::{
        InlineDirection, InlineTextTransform, InlineUnicodeBidi, InlineVerticalAlign,
        InlineWhiteSpaceCollapse,
    },
    text::LayoutFontInputs,
};

#[cfg(test)]
mod construction_budget_tests;
#[cfg(test)]
mod numeric_tests;
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

/// Outcome of admitting a completed pass into its optional working set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutWorkingSetAdmission {
    /// No completed pass has attempted retention since the last take/clear.
    #[default]
    NotAttempted,
    /// The compact graph and eligible inline state fit all configured limits.
    Retained,
    /// The construction graph exceeds the box count limit.
    BoxLimit,
    /// Source text exceeds the source-complexity limit.
    TextLimit,
    /// The estimated compact-state charge exceeds its admission limit.
    EstimatedByteLimit,
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
    /// Construction slots whose retained graph identities matched the fresh
    /// pass. This does not count reused LayoutBox allocations or numeric work.
    pub reused_box_slots: usize,
    /// Fresh box slots materialized from a validated retained construction
    /// graph instead of running the complete box constructor. This is zero
    /// for cold construction, including passes that reuse only inline state.
    pub reused_construction_boxes: usize,
    pub reused_inline_contexts: usize,
    /// Final numeric calls answered by a validated result from a previous
    /// pass. This counts calls, not unique owners or hits on newly computed
    /// results within the current pass.
    pub reused_inline_numeric_layouts: usize,
    pub admission: LayoutWorkingSetAdmission,
    /// Source-complexity counts, rather than retained allocation sizes.
    pub candidate_boxes: usize,
    pub candidate_text_bytes: usize,
    /// Zero when a source-complexity limit rejects before compact-state
    /// estimation. `admission` distinguishes that case from an empty charge.
    pub candidate_estimated_bytes: usize,
    /// Candidate estimate components, including a candidate rejected by its
    /// estimated-byte limit. These do not describe currently retained bytes.
    pub estimated_graph_bytes: usize,
    /// The graph charge attributable to optional source dependencies used to
    /// validate construction reuse. Already included in `estimated_graph_bytes`.
    pub estimated_construction_bytes: usize,
    /// Total candidate inline charge, including `estimated_inline_numeric_bytes`.
    pub estimated_inline_bytes: usize,
    /// The inline charge attributable to retained numeric keys/results and
    /// visible placement/fragment capacities. Opaque Parley storage remains
    /// in the preparation model and is not charged a second time here.
    pub estimated_inline_numeric_bytes: usize,
    pub estimated_font_metadata_bytes: usize,
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
    boxes: Vec<RetainedBox<N>>,
    construction_inputs: Option<Box<ConstructionInputs<N>>>,
    // Allocation hint only. Numeric-only boxes are rebuilt, never retained as
    // construction identities, but their previous count avoids late growth.
    pass_box_count: usize,
    // Most construction boxes do not own an IFC. Store only the real owners,
    // without embedding a large Option<IFC> or allocating one Box per owner.
    inline_contexts: Vec<(LayoutBoxId, InlineFormattingContext)>,
    inline_numeric_reuse: Option<Box<InlineNumericReuse>>,
    root: LayoutBoxId,
    root_is_document_element: bool,
    quirks_mode: style::context::QuirksMode,
    fonts: LayoutFontInputs,
}

/// The retained construction graph and exact inputs used by inline matching.
/// Numeric styles/results, paint resources and diagnostic strings are omitted.
pub(crate) struct RetainedBox<N> {
    pub(crate) source: Option<N>,
    pub(crate) owner: Option<N>,
    pub(crate) pseudo: Option<LayoutPseudo>,
    pub(crate) kind: LayoutBoxKind,
    pub(crate) anonymous_reason: Option<LayoutAnonymousReason>,
    pub(crate) structural_parent: Option<LayoutBoxId>,
    pub(crate) parent: Option<LayoutBoxId>,
    pub(crate) children: Vec<LayoutBoxId>,
    pub(crate) inline_formatting_context: bool,
    pub(crate) outside_list_marker: bool,
    // Owning the Arc prevents address reuse from turning a replaced computed
    // style into a false pointer-equality match on a later pass.
    computed: Option<ServoArc<ComputedValues>>,
    inline_style: InlineStyleInputs,
    pub(crate) element_semantics: Option<LayoutElementSemantics>,
    pub(crate) text: Option<Arc<str>>,
    text_selection: Option<LayoutTextSelection>,
}

#[derive(Clone, Copy, PartialEq)]
struct InlineStyleInputs {
    display: LayoutDisplay,
    out_of_flow: bool,
    floated: bool,
    white_space_collapse: InlineWhiteSpaceCollapse,
    text_transform: InlineTextTransform,
    direction: InlineDirection,
    unicode_bidi: InlineUnicodeBidi,
    vertical_align: InlineVerticalAlign,
    writing_mode: taffy::WritingMode,
    includes_used_font_metrics: bool,
}

impl InlineStyleInputs {
    fn from_style(style: &ResolvedLayoutStyle) -> Self {
        Self {
            display: style.display(),
            out_of_flow: style.is_out_of_flow(),
            floated: style.is_floated(),
            white_space_collapse: style.white_space_collapse(),
            text_transform: style.text_transform(),
            direction: style.direction(),
            unicode_bidi: style.unicode_bidi(),
            vertical_align: style.vertical_align(),
            writing_mode: style.writing_mode(),
            includes_used_font_metrics: style.includes_used_font_metrics(),
        }
    }
}

impl<N> RetainedWork<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) fn construction_inputs(&self) -> Option<&ConstructionInputs<N>> {
        self.construction_inputs.as_deref()
    }

    pub(crate) fn construction_boxes(&self) -> &[RetainedBox<N>] {
        &self.boxes
    }

    pub(crate) fn take_construction_inputs(&mut self) -> Option<Box<ConstructionInputs<N>>> {
        self.construction_inputs.take()
    }

    pub(crate) fn box_capacity_hint(&self) -> usize {
        self.pass_box_count
    }

    pub(crate) const fn root(&self) -> LayoutBoxId {
        self.root
    }

    pub(crate) const fn root_is_document_element(&self) -> bool {
        self.root_is_document_element
    }

    pub(crate) const fn quirks_mode(&self) -> style::context::QuirksMode {
        self.quirks_mode
    }
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
                reused_construction_boxes: 0,
                reused_inline_contexts: 0,
                reused_inline_numeric_layouts: 0,
                admission: LayoutWorkingSetAdmission::NotAttempted,
                candidate_boxes: 0,
                candidate_text_bytes: 0,
                candidate_estimated_bytes: 0,
                estimated_graph_bytes: 0,
                estimated_construction_bytes: 0,
                estimated_inline_bytes: 0,
                estimated_inline_numeric_bytes: 0,
                estimated_font_metadata_bytes: 0,
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

    pub const fn limits(&self) -> LayoutWorkingSetLimits {
        self.limits
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
        // edges are layout_children, absent from retained construction input.
        assert!(construction_box_count <= world.boxes.len());
        let pass_box_count = world.boxes.len();
        world.boxes.truncate(construction_box_count);
        let text_bytes = world.boxes.iter().fold(0usize, |bytes, layout_box| {
            bytes.saturating_add(layout_box.text.as_deref().map_or(0, str::len))
        });
        self.stats.reused_box_slots = reused_box_slots;
        self.stats.reused_construction_boxes = world
            .construction_inputs
            .as_ref()
            .map_or(0, |inputs| inputs.reused_boxes());
        self.stats.reused_inline_contexts = reused_inline_contexts;
        self.stats.reused_inline_numeric_layouts = world
            .inline_numeric_reuse
            .as_ref()
            .map_or(0, |numeric| numeric.hits());
        self.stats.candidate_boxes = world.len();
        self.stats.candidate_text_bytes = text_bytes;
        if world.len() > self.limits.max_boxes {
            self.stats.admission = LayoutWorkingSetAdmission::BoxLimit;
            return;
        }
        if text_bytes > self.limits.max_text_bytes {
            self.stats.admission = LayoutWorkingSetAdmission::TextLimit;
            return;
        }
        let retained = RetainedWork::from_world(world, fonts, pass_box_count);
        let estimate = retained.estimated_retention();
        let estimated_retained_bytes = estimate.total();
        self.stats.candidate_estimated_bytes = estimated_retained_bytes;
        self.stats.estimated_graph_bytes = estimate.graph;
        self.stats.estimated_construction_bytes = estimate.construction;
        self.stats.estimated_inline_bytes = estimate.inline;
        self.stats.estimated_inline_numeric_bytes = estimate.inline_numeric;
        self.stats.estimated_font_metadata_bytes = estimate.font_metadata;
        if estimated_retained_bytes > self.limits.max_estimated_retained_bytes {
            self.stats.admission = LayoutWorkingSetAdmission::EstimatedByteLimit;
            return;
        }
        self.stats.retained_boxes = retained.boxes.len();
        self.stats.retained_text_bytes = text_bytes;
        self.stats.estimated_retained_bytes = estimated_retained_bytes;
        self.stats.admission = LayoutWorkingSetAdmission::Retained;
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
    let Some(previous) = previous else {
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
    for (owner, inline) in previous.inline_contexts {
        if unchanged[owner.index()] {
            // Restore only valid owned preparation. All newly constructed
            // style, source, intrinsic, scroll and resource inputs stay fresh.
            current.boxes[owner.index()].inline_layout = Some(inline);
        }
    }
    if let Some(mut numeric) = previous.inline_numeric_reuse {
        let mut has_retained_output = false;
        numeric.retain_owners(|owner| {
            let keep = unchanged.get(owner.index()).copied().unwrap_or(false)
                && current
                    .boxes
                    .get(owner.index())
                    .and_then(|layout_box| layout_box.inline_layout.as_ref())
                    .is_some_and(|inline| inline.laid_out.is_some());
            has_retained_output |= keep;
            keep
        });
        if has_retained_output {
            // Only entries surviving graph, font and inline-input validation
            // count as results restored from a previous pass.
            numeric.reset_hits();
            current.inline_numeric_reuse = Some(numeric);
        }
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

fn same_inline_input<N>(old: &RetainedBox<N>, new: &LayoutBox<N>) -> bool {
    let (Some(old_computed), Some(new_computed)) =
        (old.computed.as_ref(), new.style.stylo_computed_values())
    else {
        // Synthetic styles may be adjusted independently of a computed owner.
        // Keep their normal builder path until they have an explicit input key.
        return false;
    };
    ServoArc::ptr_eq(old_computed, new_computed)
        && old.element_semantics == new.element_semantics
        && old.text == new.text
        && old.text_selection == new.text_selection
        && old.inline_style == InlineStyleInputs::from_style(&new.style)
}
