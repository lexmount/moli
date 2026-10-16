//! Owned computation state, separate from the published immutable geometry.
//!
//! The shared box constructor can restore a supported graph after validating
//! its source dependencies. Supported graphs retain their owned box storage;
//! changed or unsupported graphs use fresh construction. Exact old inputs
//! validate retained slot identities and unchanged inline preparation. Eligible inline numeric results additionally reuse their exact
//! input keys; all remaining numeric work uses the same shared backend.

use std::{collections::HashMap, fmt::Debug, hash::Hash, sync::Arc};

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
#[cfg(test)]
mod owned_construction_tests;
mod partial;
mod retention;
#[cfg(test)]
mod tests;

/// Default admission charge for one document's optional computation state.
pub const DEFAULT_DOCUMENT_WORKING_SET_BYTES: usize = 16 * 1024 * 1024;

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
            max_estimated_retained_bytes: DEFAULT_DOCUMENT_WORKING_SET_BYTES,
        }
    }
}

/// Outcome of admitting a completed pass into its optional working set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutWorkingSetAdmission {
    /// No completed pass has attempted retention since the last take/clear.
    #[default]
    NotAttempted,
    /// The owned or compact graph and eligible inline state fit all limits.
    Retained,
    /// The construction graph exceeds the box count limit.
    BoxLimit,
    /// Source text exceeds the source-complexity limit.
    TextLimit,
    /// Even the estimated compact-state charge exceeds its admission limit.
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
    /// Construction slots whose retained identities matched the fresh pass.
    /// A complete topology match counts the graph; partial restoration counts
    /// only restored plain IFC owners and their validated direct text leaves.
    /// This does not count reused allocations, preparation skips or numeric work.
    pub reused_box_slots: usize,
    /// Boxes answered by a validated retained construction graph, through
    /// either in-place owned updates or compact graph materialization. This is
    /// zero for cold construction, including inline-only restoration.
    pub reused_construction_boxes: usize,
    /// Box objects reused in place after complete source-frontier validation.
    pub reused_owned_boxes: usize,
    /// Retained box objects, rather than compact comparison records.
    pub retained_owned_boxes: usize,
    /// Owned candidate charge after reclaiming spare slots, before a possible
    /// compact fallback. This is not a physical-memory measurement.
    pub candidate_owned_bytes: usize,
    /// Actual unused box slots released under estimated-byte pressure.
    pub owned_box_capacity_trimmed: usize,
    pub reused_inline_contexts: usize,
    /// Final numeric calls answered by a validated result from a previous
    /// pass. This counts calls, not unique owners or hits on newly computed
    /// results within the current pass.
    pub reused_inline_numeric_layouts: usize,
    /// Size-only pure-text probes answered by exact constraints from a
    /// previous pass. Separate from final layout and same-pass cache hits.
    pub reused_inline_measurements: usize,
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
/// private to their construction identities. A changed graph is built fresh;
/// only independently validated, unmoved inline closures may retain their
/// preparation. No old ID is remapped to a different source. Clearing affects cost.
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
    graph: RetainedGraph<N>,
    // Pass-local validation result prepared before updating owned box inputs.
    prepared_inline_inputs: Option<(Vec<bool>, usize)>,
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

/// A single graph owner. Unsupported or oversized graphs retain compact
/// comparison inputs; supported graphs may keep the actual construction
/// objects. Neither representation retains paint resources or valid Taffy work.
pub(crate) enum RetainedGraph<N> {
    Compact(Vec<RetainedBox<N>>),
    Owned {
        boxes: Vec<LayoutBox<N>>,
        source_mapping: HashMap<N, LayoutBoxId>,
    },
}

/// Borrowed comparison inputs shared by the two retention representations.
pub(crate) struct RetainedBoxRef<'a, N> {
    pub(crate) source: Option<N>,
    pub(crate) owner: Option<N>,
    pub(crate) pseudo: Option<LayoutPseudo>,
    pub(crate) kind: LayoutBoxKind,
    pub(crate) anonymous_reason: Option<LayoutAnonymousReason>,
    pub(crate) structural_parent: Option<LayoutBoxId>,
    pub(crate) parent: Option<LayoutBoxId>,
    pub(crate) children: &'a [LayoutBoxId],
    pub(crate) inline_formatting_context: bool,
    pub(crate) outside_list_marker: bool,
    computed: Option<&'a ServoArc<ComputedValues>>,
    inline_style: InlineStyleInputs,
    pub(crate) element_semantics: Option<&'a LayoutElementSemantics>,
    pub(crate) text: Option<&'a Arc<str>>,
    text_selection: Option<LayoutTextSelection>,
}

impl<N: Copy> RetainedBoxRef<'_, N> {
    fn from_compact(old: &RetainedBox<N>) -> RetainedBoxRef<'_, N> {
        RetainedBoxRef {
            source: old.source,
            owner: old.owner,
            pseudo: old.pseudo,
            kind: old.kind,
            anonymous_reason: old.anonymous_reason,
            structural_parent: old.structural_parent,
            parent: old.parent,
            children: &old.children,
            inline_formatting_context: old.inline_formatting_context,
            outside_list_marker: old.outside_list_marker,
            computed: old.computed.as_ref(),
            inline_style: old.inline_style,
            element_semantics: old.element_semantics.as_ref(),
            text: old.text.as_ref(),
            text_selection: old.text_selection,
        }
    }

    fn from_owned(old: &LayoutBox<N>) -> RetainedBoxRef<'_, N> {
        RetainedBoxRef {
            source: old.source,
            owner: old.owner,
            pseudo: old.pseudo,
            kind: old.kind,
            anonymous_reason: old.anonymous_reason,
            structural_parent: old.structural_parent,
            parent: old.parent,
            children: &old.children,
            inline_formatting_context: old.inline_formatting_context,
            outside_list_marker: old.outside_list_marker,
            computed: old.style.stylo_computed_values(),
            inline_style: InlineStyleInputs::from_style(&old.style),
            element_semantics: old.element_semantics.as_ref(),
            text: old.text.as_ref(),
            text_selection: old.text_selection,
        }
    }
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

    pub(crate) fn box_count(&self) -> usize {
        match &self.graph {
            RetainedGraph::Compact(boxes) => boxes.len(),
            RetainedGraph::Owned { boxes, .. } => boxes.len(),
        }
    }

    pub(crate) fn box_at(&self, index: usize) -> Option<RetainedBoxRef<'_, N>> {
        match &self.graph {
            RetainedGraph::Compact(boxes) => boxes.get(index).map(RetainedBoxRef::from_compact),
            RetainedGraph::Owned { boxes, .. } => boxes.get(index).map(RetainedBoxRef::from_owned),
        }
    }

    pub(crate) fn construction_boxes(
        &self,
    ) -> impl ExactSizeIterator<Item = RetainedBoxRef<'_, N>> {
        (0..self.box_count()).map(|index| self.box_at(index).expect("construction slot exists"))
    }

    pub(crate) fn owned_construction(&self) -> Option<(&[LayoutBox<N>], &HashMap<N, LayoutBoxId>)> {
        match &self.graph {
            RetainedGraph::Owned {
                boxes,
                source_mapping,
            } => Some((boxes, source_mapping)),
            RetainedGraph::Compact(_) => None,
        }
    }

    pub(crate) fn take_owned_construction(&mut self) -> Option<LayoutWorld<N>> {
        self.owned_construction()?;
        let RetainedGraph::Owned {
            boxes,
            source_mapping,
        } = std::mem::replace(&mut self.graph, RetainedGraph::Compact(Vec::new()))
        else {
            unreachable!()
        };
        let mut world =
            LayoutWorld::from_construction(boxes, source_mapping, self.root_is_document_element);
        world.quirks_mode = self.quirks_mode;
        Some(world)
    }

    /// Discard dependent old outputs while their computed-style owners still
    /// reside on the retained graph. Never compare after installing new inputs.
    pub(crate) fn prepare_owned_inline_inputs(
        &mut self,
        unchanged: Vec<bool>,
        reused_boxes: usize,
    ) {
        self.inline_contexts
            .retain(|(owner, _)| unchanged.get(owner.index()).copied().unwrap_or(false));
        if let Some(numeric) = &mut self.inline_numeric_reuse {
            numeric.retain_owners(|owner| unchanged.get(owner.index()).copied().unwrap_or(false));
        }
        self.prepared_inline_inputs = Some((unchanged, reused_boxes));
    }

    pub(crate) fn discard_prepared_inline_inputs(&mut self) {
        self.prepared_inline_inputs = None;
        self.inline_contexts.clear();
        self.inline_numeric_reuse = None;
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
                reused_owned_boxes: 0,
                retained_owned_boxes: 0,
                candidate_owned_bytes: 0,
                owned_box_capacity_trimmed: 0,
                reused_inline_contexts: 0,
                reused_inline_numeric_layouts: 0,
                reused_inline_measurements: 0,
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
        world.inline_layouts.truncate(construction_box_count);
        let text_bytes = world.boxes.iter().fold(0usize, |bytes, layout_box| {
            bytes.saturating_add(layout_box.text.as_deref().map_or(0, str::len))
        });
        self.stats.reused_box_slots = reused_box_slots;
        self.stats.reused_construction_boxes = world
            .construction_inputs
            .as_ref()
            .map_or(0, |inputs| inputs.reused_boxes());
        self.stats.reused_owned_boxes = world
            .construction_inputs
            .as_ref()
            .map_or(0, |inputs| inputs.reused_owned_boxes());
        self.stats.reused_inline_contexts = reused_inline_contexts;
        self.stats.reused_inline_numeric_layouts = world
            .inline_numeric_reuse
            .as_ref()
            .map_or(0, |numeric| numeric.hits());
        self.stats.reused_inline_measurements = world
            .inline_numeric_reuse
            .as_ref()
            .map_or(0, |numeric| numeric.measurement_hits());
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
        let mut retained = RetainedWork::from_world(world, fonts, pass_box_count);
        let mut estimate = retained.estimated_retention();
        if retained.owned_construction().is_some() {
            if estimate.total() > self.limits.max_estimated_retained_bytes {
                self.stats.owned_box_capacity_trimmed = retained.trim_owned_box_capacity();
                estimate = retained.estimated_retention();
            }
            self.stats.candidate_owned_bytes = estimate.total();
            if estimate.total() > self.limits.max_estimated_retained_bytes {
                retained.compact_graph();
                estimate = retained.estimated_retention();
            }
        }
        if estimate.total() > self.limits.max_estimated_retained_bytes {
            // Prefer a compact validated structure with useful text results
            // over keeping full box storage by prematurely evicting them.
            // Both graph representations use the same numeric backend.
            // Discard the measurement layer only if the compact candidate
            // also exceeds the unchanged document admission limit.
            retained.discard_inline_measurements();
            estimate = retained.estimated_retention();
        }
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
        self.stats.retained_boxes = retained.box_count();
        self.stats.retained_owned_boxes = retained
            .owned_construction()
            .map_or(0, |(boxes, _)| boxes.len());
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
    let Some(mut previous) = previous else {
        return (current, 0);
    };
    if previous.fonts != *fonts
        || previous.root != current.root
        || previous.root_is_document_element != current.root_is_document_element
        || previous.quirks_mode != current.quirks_mode
    {
        return (current, 0);
    }

    let (unchanged, count) = if let Some(prepared) = previous.prepared_inline_inputs.take() {
        prepared
    } else if same_topology(&previous, &current) {
        (
            unchanged_inline_subtrees(&previous, &current),
            current.len(),
        )
    } else {
        // The shared constructor has already rebuilt the changed graph. Do
        // not revive that graph or remap IDs: only a bounded, independently
        // validated local IFC can keep its owned preparation and output.
        partial::unchanged_plain_inline_contexts(&previous, &current)
    };
    current.prepare_inline_storage();
    for (owner, inline) in previous.inline_contexts {
        if unchanged.get(owner.index()).copied().unwrap_or(false) {
            // Restore only valid owned preparation. All newly constructed
            // style, source, intrinsic, scroll and resource inputs stay fresh.
            current.inline_layouts[owner.index()] = Some(inline);
        }
    }
    if let Some(mut numeric) = previous.inline_numeric_reuse {
        let mut has_retained_output = false;
        numeric.retain_owners_for_box_count(current.len(), |owner| {
            let keep = unchanged.get(owner.index()).copied().unwrap_or(false)
                && current
                    .inline_layouts
                    .get(owner.index())
                    .and_then(Option::as_ref)
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

fn unchanged_inline_subtrees<N>(previous: &RetainedWork<N>, current: &LayoutWorld<N>) -> Vec<bool>
where
    N: Copy + Debug + Eq + Hash,
{
    let unchanged = previous
        .construction_boxes()
        .zip(&current.boxes)
        .map(|(old, new)| same_inline_input(&old, new))
        .collect::<Vec<_>>();
    propagate_inline_changes(current, unchanged)
}

pub(crate) fn propagate_inline_changes<N>(
    current: &LayoutWorld<N>,
    mut unchanged: Vec<bool>,
) -> Vec<bool>
where
    N: Copy + Debug + Eq + Hash,
{
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

    unchanged
}

fn same_topology<N>(old: &RetainedWork<N>, new: &LayoutWorld<N>) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    old.root == new.root
        && old.root_is_document_element == new.root_is_document_element
        && old.quirks_mode == new.quirks_mode
        && old.box_count() == new.boxes.len()
        && old
            .construction_boxes()
            .zip(&new.boxes)
            .all(|(old, new)| same_box_identity(&old, new) && old.children == new.children)
}

fn same_box_identity<N: PartialEq>(old: &RetainedBoxRef<'_, N>, new: &LayoutBox<N>) -> bool {
    old.source == new.source
        && old.owner == new.owner
        && old.pseudo == new.pseudo
        && old.kind == new.kind
        && old.anonymous_reason == new.anonymous_reason
        && old.structural_parent == new.structural_parent
        && old.parent == new.parent
        && old.inline_formatting_context == new.inline_formatting_context
        && old.outside_list_marker == new.outside_list_marker
}

fn same_inline_input<N>(old: &RetainedBoxRef<'_, N>, new: &LayoutBox<N>) -> bool {
    same_inline_style_input(old, &new.style, new.text_selection)
        && old.element_semantics == new.element_semantics.as_ref()
        && old.text == new.text.as_ref()
}

pub(crate) fn same_inline_style_input<N>(
    old: &RetainedBoxRef<'_, N>,
    style: &ResolvedLayoutStyle,
    selection: Option<LayoutTextSelection>,
) -> bool {
    let (Some(old_computed), Some(new_computed)) = (old.computed, style.stylo_computed_values())
    else {
        // Synthetic styles may be adjusted independently of a computed owner.
        // Keep their normal builder path until they have an explicit input key.
        return false;
    };
    ServoArc::ptr_eq(old_computed, new_computed)
        && old.text_selection == selection
        && old.inline_style == InlineStyleInputs::from_style(style)
}
