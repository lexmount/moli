//! Chunked cache-backed trampoline bounding native call-stack depth inside
//! Taffy's recursive layout algorithms.
//!
//! Taffy's `compute_block_layout`, `compute_flexbox_layout`,
//! `compute_grid_layout`, and `compute_oof_layout` recurse once per tree
//! level through `LayoutPartialTree::compute_child_layout` and
//! `LayoutBlockContainer::compute_block_child_layout`. On normal documents
//! (depth < `DEFAULT_MAX_CALL_STACK_DEPTH`) this module stays completely
//! dormant: zero heap allocations and zero hash lookups. When a deep DOM
//! crosses the chunk boundary, the offending call is recorded with its exact
//! inputs (and live `BlockContext` float state when inside a shared BFC), the
//! active call stack unwinds cleanly without writing partial caches or
//! layouts, the suspended subtree runs from call-stack depth 0 on an explicit
//! heap work stack, and its `LayoutOutput` plus any newly placed BFC floats
//! are memoized so the parent resumes in O(1) stack space and propagates
//! float state back into the live parent `BlockContext`.

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::Hash;
use std::mem::{Discriminant, discriminant};

use style::Atom;
use taffy::{
    AutoSizeBehavior, AvailableSpace, BlockContext, BlockFormattingContext, Clear, Direction,
    FloatDirection, FloatMark, LayoutInput, LayoutOutput, NodeId, RequestedAxis, ResolveOrZero,
    RunMode, Size, SizingMode, SizingPurpose, Style, WritingMode,
};

use crate::{LayoutBoxId, LayoutBoxKind, LayoutWorld};

/// Maximum Taffy recursion depth permitted on the native call stack per chunk.
///
/// Sized conservatively so even unoptimized debug frames (`cargo test`) stay
/// well under a 2 MiB thread stack and release frames stay well under
/// `render_runtime`'s 8 MiB stack on Linux, macOS, and Windows.
pub(crate) const DEFAULT_MAX_CALL_STACK_DEPTH: u32 = 96;

/// Small grace window past `max_depth` for a box whose `style.taffy` is
/// temporarily swapped by table cell layout (`with_grid_cell_style` or
/// `restrict_scrollable_percentage_children`) so suspension normally lands on
/// an unpinned child without cloning `Style<Atom>`.
const PINNED_STYLE_GRACE_DEPTH: u32 = 4;

/// Synthetic float ID used solely when `apply_block_context_delta` must set
/// `ctx.adjoining_floats[side]` via a `Size::ZERO` placement without inflating
/// `ctx.float_content_contribution` (e.g. when an escaped positive `margin-top`
/// shifted the real float's `rel_y + height` above `delta.float_content_contribution`).
/// Real `LayoutBoxId` values fit in `u32`, so `u64::MAX` can never collide.
const ADJOINING_SENTINEL_ID: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum FloatDirectionKey {
    Left,
    Right,
}

impl From<FloatDirection> for FloatDirectionKey {
    fn from(direction: FloatDirection) -> Self {
        match direction {
            FloatDirection::Left => Self::Left,
            FloatDirection::Right => Self::Right,
        }
    }
}

impl FloatDirectionKey {
    const fn to_taffy(self) -> FloatDirection {
        match self {
            Self::Left => FloatDirection::Left,
            Self::Right => FloatDirection::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ClearKey {
    None,
    Left,
    Right,
    Both,
}

impl From<Clear> for ClearKey {
    fn from(clear: Clear) -> Self {
        match clear {
            Clear::None => Self::None,
            Clear::Left => Self::Left,
            Clear::Right => Self::Right,
            Clear::Both => Self::Both,
        }
    }
}

impl ClearKey {
    const fn to_taffy(self) -> Clear {
        match self {
            Self::None => Clear::None,
            Self::Left => Clear::Left,
            Self::Right => Clear::Right,
            Self::Both => Clear::Both,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FloatPlacementMeta {
    margin_box: Size<f32>,
    direction: FloatDirection,
    clear: Clear,
    seq: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SnapshotFloat {
    id: u64,
    rel_x_bits: u32,
    rel_y_bits: u32,
    width_bits: u32,
    height_bits: u32,
    direction: FloatDirectionKey,
    clear: ClearKey,
    /// Containing-block inset on the side opposite `direction`, relative to
    /// the snapshotted `BlockContext`, when non-zero and not masked by an
    /// opposite-side float. Preserves Taffy's segment-seeding behavior in
    /// `FloatContext::place_floated_box_inner`.
    opposite_cb_inset_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BlockContextSnapshot {
    is_root: bool,
    ctx_width_bits: u32,
    adjoining_floats: [bool; 2],
    float_content_contribution_bits: u32,
    floats: Vec<SnapshotFloat>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TrampolineBlockContextState {
    None,
    Empty { is_root: bool, ctx_width_bits: u32 },
    Populated(Box<BlockContextSnapshot>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TrampolineCallKind {
    Child,
    BlockChild(TrampolineBlockContextState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum AvailableSpaceKey {
    Definite(u32),
    MinContent,
    MaxContent,
}

impl From<AvailableSpace> for AvailableSpaceKey {
    fn from(space: AvailableSpace) -> Self {
        match space {
            AvailableSpace::Definite(value) => Self::Definite(value.to_bits()),
            AvailableSpace::MinContent => Self::MinContent,
            AvailableSpace::MaxContent => Self::MaxContent,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LayoutInputKey {
    sizing_purpose: Discriminant<SizingPurpose>,
    block_auto_behavior: Discriminant<AutoSizeBehavior>,
    parent_writing_mode: Discriminant<WritingMode>,
    run_mode: Discriminant<RunMode>,
    sizing_mode: Discriminant<SizingMode>,
    axis: Discriminant<RequestedAxis>,
    known_width: Option<u32>,
    known_height: Option<u32>,
    definite_width: bool,
    definite_height: bool,
    parent_width: Option<u32>,
    parent_height: Option<u32>,
    available_width: AvailableSpaceKey,
    available_height: AvailableSpaceKey,
    collapsible_start: bool,
    collapsible_end: bool,
}

impl From<LayoutInput> for LayoutInputKey {
    fn from(inputs: LayoutInput) -> Self {
        Self {
            sizing_purpose: discriminant(&inputs.sizing_purpose),
            block_auto_behavior: discriminant(&inputs.block_auto_behavior),
            parent_writing_mode: discriminant(&inputs.parent_writing_mode),
            run_mode: discriminant(&inputs.run_mode),
            sizing_mode: discriminant(&inputs.sizing_mode),
            axis: discriminant(&inputs.axis),
            known_width: inputs.known_dimensions.width.map(f32::to_bits),
            known_height: inputs.known_dimensions.height.map(f32::to_bits),
            definite_width: inputs.known_dimensions_are_definite.width,
            definite_height: inputs.known_dimensions_are_definite.height,
            parent_width: inputs.parent_size.width.map(f32::to_bits),
            parent_height: inputs.parent_size.height.map(f32::to_bits),
            available_width: inputs.available_space.width.into(),
            available_height: inputs.available_space.height.into(),
            collapsible_start: inputs.vertical_margins_are_collapsible.start,
            collapsible_end: inputs.vertical_margins_are_collapsible.end,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TrampolineKey {
    node_id: usize,
    kind: TrampolineCallKind,
    inputs: LayoutInputKey,
    measure_baselines: bool,
    table_cell_percentage_height: Option<(LayoutBoxId, bool)>,
    pinned_style: bool,
}

#[derive(Clone, Debug)]
struct SuspendedLayoutCall {
    key: TrampolineKey,
    node_id: NodeId,
    inputs: LayoutInput,
    measure_baselines: bool,
    table_cell_percentage_height: Option<(LayoutBoxId, bool)>,
    style_override: Option<Box<Style<Atom>>>,
}

#[derive(Clone, Debug)]
enum PendingTrampolineAction {
    /// A deep subtree crossed the trampoline boundary while sharing a
    /// populated BFC before float metadata tracking was enabled. Unwind once
    /// and replay with `track_floats = true` so ancestor floats are recorded
    /// with exact margin-box sizes and placement order.
    RetryWithFloatTracking,
    Suspend(Box<SuspendedLayoutCall>),
}

#[derive(Clone, Debug)]
struct BlockContextDelta {
    placed_floats: Vec<SnapshotFloat>,
    float_content_contribution: f32,
    added_adjoining_floats: [bool; 2],
}

impl Default for BlockContextDelta {
    fn default() -> Self {
        Self {
            placed_floats: Vec::new(),
            float_content_contribution: f32::NEG_INFINITY,
            added_adjoining_floats: [false, false],
        }
    }
}

impl BlockContextDelta {
    fn is_empty(&self) -> bool {
        self.placed_floats.is_empty()
            && !self.float_content_contribution.is_finite()
            && !self.added_adjoining_floats[0]
            && !self.added_adjoining_floats[1]
    }
}

#[derive(Clone, Debug)]
struct CompletedLayout {
    output: LayoutOutput,
    delta: BlockContextDelta,
}

#[derive(Debug)]
pub(crate) struct LayoutTrampoline {
    active: bool,
    depth: u32,
    max_depth: u32,
    aborted: bool,
    /// Enabled lazily only after the trampoline actually suspends or encounters
    /// a populated BFC at the depth limit, keeping shallow trees allocation-free.
    track_floats: bool,
    /// True while executing a resumed job on `work_stack`.
    in_job: bool,
    job_top_adjoining_floats: [bool; 2],
    /// Depth of the deepest active block frame inside the running job whose
    /// top margin strut is still open and collapsing with the job root's top
    /// margin strut (`0` once the job's top strut has committed/closed).
    job_open_strut_depth: u32,
    float_seq: u64,
    float_meta: HashMap<u64, FloatPlacementMeta>,
    pending: Option<PendingTrampolineAction>,
    completed: HashMap<TrampolineKey, CompletedLayout>,
    /// Fast membership filter so retries skip snapshotting `BlockContext` for
    /// intermediate nodes that have never been memoized in `completed`.
    completed_node_ids: HashSet<usize>,
    pinned_style_nodes: Vec<LayoutBoxId>,
    /// Single-call cache bypass guards for same-BFC block children that read
    /// from or mutate shared `BlockContext` float state.
    skip_cache_get_node: Option<NodeId>,
    skip_cache_store_node: Option<NodeId>,
}

impl Default for LayoutTrampoline {
    fn default() -> Self {
        Self {
            active: false,
            depth: 0,
            max_depth: DEFAULT_MAX_CALL_STACK_DEPTH,
            aborted: false,
            track_floats: false,
            in_job: false,
            job_top_adjoining_floats: [false, false],
            job_open_strut_depth: 0,
            float_seq: 0,
            float_meta: HashMap::new(),
            pending: None,
            completed: HashMap::new(),
            completed_node_ids: HashSet::new(),
            pinned_style_nodes: Vec::new(),
            skip_cache_get_node: None,
            skip_cache_store_node: None,
        }
    }
}

impl LayoutTrampoline {
    #[inline(always)]
    pub(crate) const fn is_aborted(&self) -> bool {
        self.aborted
    }

    #[inline(always)]
    pub(crate) const fn track_floats(&self) -> bool {
        self.track_floats
    }

    #[cfg(test)]
    pub(crate) fn set_max_depth_for_test(&mut self, max_depth: u32) {
        self.max_depth = max_depth.max(1);
    }

    pub(crate) fn push_pinned_style(&mut self, id: LayoutBoxId) {
        self.pinned_style_nodes.push(id);
    }

    pub(crate) fn pop_pinned_style(&mut self) {
        if let Some(id) = self.pinned_style_nodes.pop()
            && !self.aborted
            && !self.completed.is_empty()
        {
            let raw_id = usize::from(id.to_taffy());
            self.completed
                .retain(|key, _| !(key.pinned_style && key.node_id == raw_id));
        }
    }

    #[inline(always)]
    pub(super) fn set_skip_cache_get(&mut self, node_id: NodeId) {
        self.skip_cache_get_node = Some(node_id);
    }

    #[inline(always)]
    pub(super) fn take_skip_cache_get(&mut self, node_id: NodeId) -> bool {
        if self.skip_cache_get_node == Some(node_id) {
            self.skip_cache_get_node = None;
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub(super) fn set_skip_cache_store(&mut self, node_id: NodeId) {
        self.skip_cache_store_node = Some(node_id);
    }

    #[inline(always)]
    pub(super) fn take_skip_cache_store(&mut self, node_id: NodeId) -> bool {
        if self.skip_cache_store_node == Some(node_id) {
            self.skip_cache_store_node = None;
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub(super) fn clear_skip_cache(&mut self, node_id: NodeId) {
        if self.skip_cache_get_node == Some(node_id) {
            self.skip_cache_get_node = None;
        }
        if self.skip_cache_store_node == Some(node_id) {
            self.skip_cache_store_node = None;
        }
    }

    pub(super) fn record_float_meta(
        &mut self,
        id: u64,
        margin_box: Size<f32>,
        direction: FloatDirection,
        clear: Clear,
    ) {
        self.float_seq = self.float_seq.saturating_add(1);
        self.float_meta.insert(
            id,
            FloatPlacementMeta {
                margin_box,
                direction,
                clear,
                seq: self.float_seq,
            },
        );
    }

    #[inline(always)]
    pub(super) const fn is_observing_open_strut(&self) -> bool {
        self.in_job && self.job_open_strut_depth > 0 && self.depth == self.job_open_strut_depth
    }

    #[inline(always)]
    pub(super) const fn is_in_open_strut_scope(&self) -> bool {
        self.in_job && self.job_open_strut_depth > 0 && self.job_open_strut_depth >= self.depth
    }

    fn record_completed(&mut self, key: TrampolineKey, completed: CompletedLayout) {
        self.completed_node_ids.insert(key.node_id);
        self.completed.insert(key, completed);
    }

    #[inline(always)]
    fn is_pinned(&self, node_id: NodeId, is_viewport: bool) -> bool {
        if is_viewport || self.pinned_style_nodes.is_empty() {
            return false;
        }
        let id = LayoutBoxId::from_taffy(node_id);
        self.pinned_style_nodes.contains(&id)
    }
}

fn f32_key_bits(value: f32) -> u32 {
    if value == 0.0 {
        0.0_f32.to_bits()
    } else {
        value.to_bits()
    }
}

fn zero_float_mark() -> FloatMark {
    // INVARIANT: Taffy's `FloatMark` stores only `(left_len, right_len)` indices
    // into a `FloatContext`'s float lists, so a mark captured from a fresh BFC
    // always starts at `(0, 0)` and enumerates every float in any `BlockContext`.
    let mut empty = BlockFormattingContext::new();
    let root_ctx = empty.root_block_context();
    root_ctx.float_mark()
}

/// Probe `ctx`'s segments covering `[rel_y, rel_y + height)` to recover any
/// non-zero containing-block inset seeded onto the side opposite `direction`
/// when no actual float occupies that opposite side.
fn probe_opposite_cb_inset(
    ctx: &BlockContext<'_>,
    ctx_width: f32,
    rel_y: f32,
    height: f32,
    direction: FloatDirection,
) -> f32 {
    if height <= 0.0 || !ctx.has_active_floats(rel_y) {
        return 0.0;
    }
    let end_y = rel_y + height;
    let opposite_dir = match direction {
        FloatDirection::Left => Direction::Rtl,
        FloatDirection::Right => Direction::Ltr,
    };
    let probe_step = ctx_width.abs().max(1.0) * 16.0 + 1024.0;
    let probe_m1 = -probe_step;
    let probe_m2 = probe_m1 * 2.0;
    let (margins_1, margins_2) = match direction {
        FloatDirection::Left => ([0.0, probe_m1], [0.0, probe_m2]),
        FloatDirection::Right => ([probe_m1, 0.0], [probe_m2, 0.0]),
    };

    let mut after: Option<usize> = None;
    loop {
        let slot = ctx.find_content_slot(rel_y, Clear::None, after);
        let Some(seg_id) = slot.segment_id else {
            return 0.0;
        };
        if slot.y >= end_y {
            return 0.0;
        }
        let bfc_1 = ctx.find_bfc_slot(rel_y, margins_1, opposite_dir, Clear::None, after);
        let bfc_2 = ctx.find_bfc_slot(rel_y, margins_2, opposite_dir, Clear::None, after);
        // When no float occupies the opposite side, `find_bfc_slot` resolves
        // that edge purely from the probed negative margin, so doubling the
        // negative margin expands `border_width` by `probe_step`.
        let has_opposite_float = (bfc_2.border_width - bfc_1.border_width) < probe_step * 0.5;
        if !has_opposite_float {
            let inset = match direction {
                FloatDirection::Left => ctx_width - slot.x - slot.width,
                FloatDirection::Right => slot.x,
            };
            return inset.max(0.0);
        }
        after = Some(seg_id);
    }
}

const fn adjoining_matches_clear(adjoining: [bool; 2], clear: Clear) -> bool {
    match clear {
        Clear::Left => adjoining[0],
        Clear::Right => adjoining[1],
        Clear::Both => adjoining[0] || adjoining[1],
        Clear::None => false,
    }
}

fn apply_block_context_delta(
    trampoline: &mut LayoutTrampoline,
    ctx: &mut BlockContext<'_>,
    delta: &BlockContextDelta,
) {
    if delta.is_empty() {
        return;
    }
    let ctx_width = ctx
        .find_content_slot(f32::INFINITY, Clear::None, None)
        .width;
    let needs_adjoin = [
        delta.added_adjoining_floats[0] && !ctx.has_adjoining_float(Clear::Left),
        delta.added_adjoining_floats[1] && !ctx.has_adjoining_float(Clear::Right),
    ];
    // Only place a real float directly on `ctx` with `adjoins_unresolved_strut = true`
    // when its `rel_y + height` does not exceed `delta.float_content_contribution`;
    // otherwise escaped positive top margins would double-count into
    // `ctx.float_content_contribution`.
    let direct_adjoin_idx = |side: FloatDirectionKey| -> Option<usize> {
        let side_idx = side.to_taffy() as usize;
        if !needs_adjoin[side_idx] {
            return None;
        }
        delta.placed_floats.iter().rposition(|f| {
            f.direction == side
                && (!delta.float_content_contribution.is_finite()
                    || f32::from_bits(f.rel_y_bits) + f32::from_bits(f.height_bits)
                        <= delta.float_content_contribution)
        })
    };
    let direct_left_idx = direct_adjoin_idx(FloatDirectionKey::Left);
    let direct_right_idx = direct_adjoin_idx(FloatDirectionKey::Right);

    // If an adjoining side has no real float eligible for direct placement on
    // `ctx`, place a `Size::ZERO` sentinel before the real floats so
    // `ctx.adjoining_floats[side]` is set without touching `segments` or
    // `last_placed_floats` and without inflating `ctx.float_content_contribution`.
    if (needs_adjoin[0] && direct_left_idx.is_none())
        || (needs_adjoin[1] && direct_right_idx.is_none())
    {
        let min_delta_rel_y = delta
            .placed_floats
            .iter()
            .map(|f| f32::from_bits(f.rel_y_bits))
            .fold(f32::INFINITY, f32::min);
        let sentinel_min_y = if delta.float_content_contribution.is_finite() {
            min_delta_rel_y.min(delta.float_content_contribution)
        } else {
            min_delta_rel_y
        };
        if sentinel_min_y.is_finite() {
            for (side_idx, direction, direct_idx) in [
                (0usize, FloatDirection::Left, direct_left_idx),
                (1usize, FloatDirection::Right, direct_right_idx),
            ] {
                if needs_adjoin[side_idx] && direct_idx.is_none() {
                    ctx.place_floated_box(
                        ADJOINING_SENTINEL_ID,
                        Size::ZERO,
                        sentinel_min_y,
                        direction,
                        Clear::None,
                        true,
                    );
                }
            }
        }
    }

    for (idx, f) in delta.placed_floats.iter().enumerate() {
        let rel_x = f32::from_bits(f.rel_x_bits);
        let rel_y = f32::from_bits(f.rel_y_bits);
        let margin_box = Size {
            width: f32::from_bits(f.width_bits),
            height: f32::from_bits(f.height_bits),
        };
        let opp_inset = f32::from_bits(f.opposite_cb_inset_bits);
        let direction = f.direction.to_taffy();
        let clear = f.clear.to_taffy();
        // Refresh placement sequence on replay so nested chunk retries keep
        // chronological ordering relative to floats placed earlier in the
        // retrying parent chunk.
        if trampoline.track_floats {
            trampoline.record_float_meta(f.id, margin_box, direction, clear);
        }
        let (inset_left_delta, inset_right_delta) = match direction {
            FloatDirection::Left => {
                let right = if opp_inset > 0.0 {
                    opp_inset
                } else {
                    (ctx_width - (rel_x + margin_box.width)).min(0.0)
                };
                (rel_x, right)
            }
            FloatDirection::Right => {
                let right = ctx_width - (rel_x + margin_box.width);
                let left = if opp_inset > 0.0 {
                    opp_inset
                } else {
                    rel_x.min(0.0)
                };
                (left, right)
            }
        };
        let adjoins = match direction {
            FloatDirection::Left => direct_left_idx == Some(idx),
            FloatDirection::Right => direct_right_idx == Some(idx),
        };
        if adjoins {
            ctx.apply_content_box_inset([inset_left_delta, inset_right_delta]);
            ctx.place_floated_box(f.id, margin_box, rel_y, direction, clear, true);
            ctx.apply_content_box_inset([0.0, 0.0]);
        } else {
            let mut target = ctx.sub_context(0.0, [inset_left_delta, inset_right_delta]);
            target.place_floated_box(f.id, margin_box, rel_y, direction, clear, false);
        }
    }

    if delta.float_content_contribution.is_finite() {
        ctx.add_child_floated_content_height_contribution(delta.float_content_contribution);
    }
}

impl<N> LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    #[inline(always)]
    pub(crate) const fn trampoline_aborted(&self) -> bool {
        self.trampoline.is_aborted()
    }

    pub(crate) fn push_trampoline_pinned_style(&mut self, id: LayoutBoxId) {
        self.trampoline.push_pinned_style(id);
    }

    pub(crate) fn pop_trampoline_pinned_style(&mut self) {
        self.trampoline.pop_pinned_style();
    }

    #[inline(never)]
    pub(super) fn record_trampoline_block_float_meta(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        output: &LayoutOutput,
    ) {
        if inputs.axis != RequestedAxis::Both
            || inputs.sizing_purpose != SizingPurpose::Layout
            || (inputs.run_mode != RunMode::PerformLayout && !self.measure_baselines)
            || self.is_viewport_taffy_node(node_id)
        {
            return;
        }
        let id = LayoutBoxId::from_taffy(node_id);
        if !super::box_is_effectively_floated(self, id) {
            return;
        }
        let style = &self.boxes[id.index()].style.taffy;
        let Some(direction) = style.float.float_direction() else {
            return;
        };
        let margin = style
            .margin
            .resolve_or_zero(inputs.parent_size.width, super::resolve_stylo_calc_value);
        let margin_box = output.size + margin.sum_axes();
        self.trampoline
            .record_float_meta(id.index() as u64, margin_box, direction, style.clear);
    }

    fn resolve_float_meta(&self, id: u64) -> Option<FloatPlacementMeta> {
        if let Some(meta) = self.trampoline.float_meta.get(&id).copied() {
            return Some(meta);
        }
        let index = usize::try_from(id).ok()?;
        let layout_box = self.boxes.get(index)?;
        let direction = layout_box.style.taffy.float.float_direction()?;
        let margin_box =
            layout_box.unrounded_layout.size + layout_box.unrounded_layout.margin.sum_axes();
        Some(FloatPlacementMeta {
            margin_box,
            direction,
            clear: layout_box.style.taffy.clear,
            seq: 0,
        })
    }

    fn capture_floats_since(
        &self,
        ctx: &BlockContext<'_>,
        mark: FloatMark,
        ctx_width: f32,
    ) -> Vec<SnapshotFloat> {
        let mut entries: Vec<(u64, usize, SnapshotFloat)> = Vec::new();
        for (order_idx, (id, rel_pos)) in ctx.floats_placed_since(mark).enumerate() {
            if id == ADJOINING_SENTINEL_ID {
                continue;
            }
            let Some(meta) = self.resolve_float_meta(id) else {
                continue;
            };
            let opposite_cb_inset = probe_opposite_cb_inset(
                ctx,
                ctx_width,
                rel_pos.y,
                meta.margin_box.height,
                meta.direction,
            );
            entries.push((
                meta.seq,
                order_idx,
                SnapshotFloat {
                    id,
                    rel_x_bits: f32_key_bits(rel_pos.x),
                    rel_y_bits: f32_key_bits(rel_pos.y),
                    width_bits: f32_key_bits(meta.margin_box.width),
                    height_bits: f32_key_bits(meta.margin_box.height),
                    direction: meta.direction.into(),
                    clear: meta.clear.into(),
                    opposite_cb_inset_bits: f32_key_bits(opposite_cb_inset),
                },
            ));
        }
        // Replay in exact chronological placement order across left/right sides
        // so `last_placed_floats`, `float_ceiling`, and `segments` match 1:1.
        entries.sort_unstable_by_key(|(seq, order_idx, _)| (*seq, *order_idx));
        entries.into_iter().map(|(_, _, item)| item).collect()
    }

    fn capture_block_context_state(
        &self,
        block_context: Option<&BlockContext<'_>>,
    ) -> Option<TrampolineBlockContextState> {
        let Some(ctx) = block_context else {
            return Some(TrampolineBlockContextState::None);
        };
        let is_root = ctx.is_bfc_root();
        let ctx_width = ctx
            .find_content_slot(f32::INFINITY, Clear::None, None)
            .width;
        if !ctx.has_floats() {
            return Some(TrampolineBlockContextState::Empty {
                is_root,
                ctx_width_bits: f32_key_bits(ctx_width),
            });
        }
        if !self.trampoline.track_floats {
            return None;
        }
        let floats = self.capture_floats_since(ctx, zero_float_mark(), ctx_width);
        let adjoining_floats = [
            ctx.has_adjoining_float(Clear::Left),
            ctx.has_adjoining_float(Clear::Right),
        ];
        let float_content_contribution = if is_root {
            ctx.floated_content_height_contribution()
        } else {
            f32::NEG_INFINITY
        };
        Some(TrampolineBlockContextState::Populated(Box::new(
            BlockContextSnapshot {
                is_root,
                ctx_width_bits: f32_key_bits(ctx_width),
                adjoining_floats,
                float_content_contribution_bits: f32_key_bits(float_content_contribution),
                floats,
            },
        )))
    }

    /// Check whether a `compute_child_layout` call should return a memoized
    /// trampoline result or suspend onto the heap work stack.
    #[inline(always)]
    pub(super) fn trampoline_before_child_call(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
    ) -> Option<LayoutOutput> {
        if self.trampoline.aborted {
            return Some(LayoutOutput::HIDDEN);
        }
        if !self.trampoline.active {
            return None;
        }
        let at_limit = self.trampoline.depth >= self.trampoline.max_depth;
        if !at_limit
            && (self.trampoline.completed.is_empty()
                || !self
                    .trampoline
                    .completed_node_ids
                    .contains(&usize::from(node_id)))
        {
            return None;
        }
        self.trampoline_before_call_slow(node_id, inputs, false, None, at_limit)
    }

    /// Check whether a `compute_block_child_layout` call should return a
    /// memoized trampoline result (replaying any BFC float delta into the live
    /// `block_context`) or suspend onto the heap work stack.
    #[inline(always)]
    pub(super) fn trampoline_before_block_child_call(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_context: Option<&mut BlockContext<'_>>,
    ) -> Option<LayoutOutput> {
        if self.trampoline.aborted {
            return Some(LayoutOutput::HIDDEN);
        }
        if !self.trampoline.active {
            return None;
        }
        let at_limit = self.trampoline.depth >= self.trampoline.max_depth;
        if !at_limit
            && (self.trampoline.completed.is_empty()
                || !self
                    .trampoline
                    .completed_node_ids
                    .contains(&usize::from(node_id)))
        {
            return None;
        }
        self.trampoline_before_call_slow(node_id, inputs, true, block_context, at_limit)
    }

    // Keep all key/snapshot/suspension locals out of recursive Taffy frames so
    // unoptimized debug builds stay well within bounded thread stacks.
    #[inline(never)]
    #[cold]
    fn trampoline_before_call_slow(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        is_block_child: bool,
        block_context: Option<&mut BlockContext<'_>>,
        at_limit: bool,
    ) -> Option<LayoutOutput> {
        let raw_node_id = usize::from(node_id);
        let kind = if is_block_child {
            let Some(state) = self.capture_block_context_state(block_context.as_deref()) else {
                // Floats already exist in the shared BFC before tracking was
                // enabled: replay once from the current root with tracking on.
                self.trampoline.track_floats = true;
                self.trampoline.pending = Some(PendingTrampolineAction::RetryWithFloatTracking);
                self.trampoline.aborted = true;
                return Some(LayoutOutput::HIDDEN);
            };
            TrampolineCallKind::BlockChild(state)
        } else {
            TrampolineCallKind::Child
        };
        let is_viewport = self.is_viewport_taffy_node(node_id);
        let pinned = self.trampoline.is_pinned(node_id, is_viewport);
        let key = TrampolineKey {
            node_id: raw_node_id,
            kind,
            inputs: inputs.into(),
            measure_baselines: self.measure_baselines,
            table_cell_percentage_height: self.table_cell_percentage_height,
            pinned_style: pinned,
        };
        if let Some(completed) = self.trampoline.completed.get(&key).cloned() {
            if let Some(ctx) = block_context {
                apply_block_context_delta(&mut self.trampoline, ctx, &completed.delta);
            }
            if self.trampoline.is_in_open_strut_scope() {
                self.trampoline.job_top_adjoining_floats[0] |=
                    completed.delta.added_adjoining_floats[0];
                self.trampoline.job_top_adjoining_floats[1] |=
                    completed.delta.added_adjoining_floats[1];
            }
            return Some(completed.output);
        }
        if !at_limit {
            return None;
        }
        if pinned
            && self.trampoline.depth
                < self
                    .trampoline
                    .max_depth
                    .saturating_add(PINNED_STYLE_GRACE_DEPTH)
        {
            return None;
        }
        let style_override = (pinned && !is_viewport).then(|| {
            let id = LayoutBoxId::from_taffy(node_id);
            Box::new(self.boxes[id.index()].style.taffy.clone())
        });
        self.trampoline.pending = Some(PendingTrampolineAction::Suspend(Box::new(
            SuspendedLayoutCall {
                key,
                node_id,
                inputs,
                measure_baselines: self.measure_baselines,
                table_cell_percentage_height: self.table_cell_percentage_height,
                style_override,
            },
        )));
        self.trampoline.aborted = true;
        Some(LayoutOutput::HIDDEN)
    }

    /// Execute a top-level Taffy layout operation with bounded native
    /// call-stack depth, transparently driving any suspended deep subtrees on
    /// an explicit heap stack.
    pub(super) fn run_trampolined<R>(&mut self, mut root_op: impl FnMut(&mut Self) -> R) -> R {
        if self.trampoline.active {
            return root_op(self);
        }
        debug_assert_eq!(self.trampoline.depth, 0);
        self.trampoline.active = true;
        self.trampoline.aborted = false;
        self.trampoline.track_floats = false;
        self.trampoline.in_job = false;
        self.trampoline.job_top_adjoining_floats = [false, false];
        self.trampoline.job_open_strut_depth = 0;
        self.trampoline.float_seq = 0;
        self.trampoline.float_meta.clear();
        self.trampoline.pending = None;
        self.trampoline.completed.clear();
        self.trampoline.completed_node_ids.clear();
        self.trampoline.skip_cache_get_node = None;
        self.trampoline.skip_cache_store_node = None;

        let mut work_stack: Vec<SuspendedLayoutCall> = Vec::new();
        let result = loop {
            if let Some(job) = work_stack.last().cloned() {
                let prev_measure = self.measure_baselines;
                let prev_table_pct = self.table_cell_percentage_height;
                self.measure_baselines = job.measure_baselines;
                self.table_cell_percentage_height = job.table_cell_percentage_height;
                let restored_style = job.style_override.as_ref().map(|override_style| {
                    let id = LayoutBoxId::from_taffy(job.node_id);
                    let prev = std::mem::replace(
                        &mut self.boxes[id.index()].style.taffy,
                        (**override_style).clone(),
                    );
                    self.trampoline.push_pinned_style(id);
                    (id, prev)
                });
                self.trampoline.aborted = false;
                self.trampoline.pending = None;

                let completed = self.execute_trampoline_job(&job);

                if let Some((id, prev_style)) = restored_style {
                    self.boxes[id.index()].style.taffy = prev_style;
                    self.trampoline.pinned_style_nodes.pop();
                }
                self.measure_baselines = prev_measure;
                self.table_cell_percentage_height = prev_table_pct;

                if self.trampoline.aborted {
                    match self.trampoline.pending.take() {
                        Some(PendingTrampolineAction::RetryWithFloatTracking) => {
                            continue;
                        }
                        Some(PendingTrampolineAction::Suspend(next_job)) => {
                            // Guard against impossible non-DAG re-suspension cycles.
                            if work_stack.iter().any(|active| active.key == next_job.key) {
                                self.trampoline.record_completed(
                                    next_job.key,
                                    CompletedLayout {
                                        output: LayoutOutput::HIDDEN,
                                        delta: BlockContextDelta::default(),
                                    },
                                );
                            } else {
                                self.trampoline.track_floats = true;
                                work_stack.push(*next_job);
                            }
                            continue;
                        }
                        None => {
                            self.trampoline.record_completed(
                                job.key,
                                CompletedLayout {
                                    output: LayoutOutput::HIDDEN,
                                    delta: BlockContextDelta::default(),
                                },
                            );
                            work_stack.pop();
                            continue;
                        }
                    }
                }
                self.trampoline.record_completed(job.key, completed);
                work_stack.pop();
            } else {
                self.trampoline.aborted = false;
                self.trampoline.pending = None;
                let value = root_op(self);
                if self.trampoline.aborted {
                    match self.trampoline.pending.take() {
                        Some(PendingTrampolineAction::RetryWithFloatTracking) => {
                            continue;
                        }
                        Some(PendingTrampolineAction::Suspend(next_job)) => {
                            self.trampoline.track_floats = true;
                            work_stack.push(*next_job);
                            continue;
                        }
                        None => {}
                    }
                }
                break value;
            }
        };

        self.trampoline.active = false;
        self.trampoline.aborted = false;
        self.trampoline.track_floats = false;
        self.trampoline.in_job = false;
        self.trampoline.job_top_adjoining_floats = [false, false];
        self.trampoline.job_open_strut_depth = 0;
        self.trampoline.float_seq = 0;
        self.trampoline.float_meta.clear();
        self.trampoline.pending = None;
        self.trampoline.completed.clear();
        self.trampoline.completed_node_ids.clear();
        self.trampoline.skip_cache_get_node = None;
        self.trampoline.skip_cache_store_node = None;
        result
    }

    fn block_collapses_top_margin_with_children(
        &self,
        node_id: NodeId,
        inputs: LayoutInput,
    ) -> bool {
        if !inputs.vertical_margins_are_collapsible.start || self.is_viewport_taffy_node(node_id) {
            return false;
        }
        let id = LayoutBoxId::from_taffy(node_id);
        let layout_box = &self.boxes[id.index()];
        if layout_box.inline_formatting_context || layout_box.is_replaced() {
            return false;
        }
        let display = layout_box.style.display();
        if display.is_flex_container()
            || display.is_grid_container()
            || matches!(
                layout_box.kind,
                LayoutBoxKind::TableWrapper
                    | LayoutBoxKind::InlineTableWrapper
                    | LayoutBoxKind::AnonymousTableWrapper
                    | LayoutBoxKind::PrincipalFlex
                    | LayoutBoxKind::PrincipalInlineFlex
                    | LayoutBoxKind::PrincipalGrid
                    | LayoutBoxKind::PrincipalInlineGrid
                    | LayoutBoxKind::PrincipalInline
                    | LayoutBoxKind::InlineContinuation
                    | LayoutBoxKind::TableColumn
                    | LayoutBoxKind::Text
                    | LayoutBoxKind::LineBreak
                    | LayoutBoxKind::PseudoMarker
                    | LayoutBoxKind::PseudoBefore
                    | LayoutBoxKind::PseudoAfter
                    | LayoutBoxKind::Replaced
            )
        {
            return false;
        }
        let style = &layout_box.style.taffy;
        if style.position.is_out_of_flow()
            || style.overflow.x.is_scroll_container()
            || style.overflow.y.is_scroll_container()
            || style.align_content.keyword != taffy::AlignContentKeyword::Normal
            || style.contain.establishes_independent_formatting_context()
        {
            return false;
        }
        let basis = inputs
            .parent_writing_mode
            .to_logical(inputs.parent_size)
            .inline_size;
        let padding = style
            .padding
            .resolve_or_zero(basis, super::resolve_stylo_calc_value);
        let border = style
            .border
            .resolve_or_zero(basis, super::resolve_stylo_calc_value);
        padding.top == 0.0 && border.top == 0.0
    }

    #[inline(never)]
    pub(super) fn observe_open_strut_block_child_enter(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_context: Option<&BlockContext<'_>>,
    ) {
        if let Some(ctx) = block_context {
            self.trampoline.job_top_adjoining_floats[0] |= ctx.has_adjoining_float(Clear::Left);
            self.trampoline.job_top_adjoining_floats[1] |= ctx.has_adjoining_float(Clear::Right);
        }
        if self.block_collapses_top_margin_with_children(node_id, inputs) {
            self.trampoline.job_open_strut_depth = self.trampoline.depth.saturating_add(1);
        }
    }

    #[inline(never)]
    pub(super) fn observe_open_strut_block_child_exit(
        &mut self,
        node_id: NodeId,
        block_context: Option<&BlockContext<'_>>,
        output: &LayoutOutput,
    ) {
        let child_kept_strut_open = self.trampoline.job_open_strut_depth > self.trampoline.depth;
        self.trampoline.job_open_strut_depth = self.trampoline.depth;
        if child_kept_strut_open && let Some(ctx) = block_context {
            self.trampoline.job_top_adjoining_floats[0] |= ctx.has_adjoining_float(Clear::Left);
            self.trampoline.job_top_adjoining_floats[1] |= ctx.has_adjoining_float(Clear::Right);
        }
        let mut committed = !output.margins_can_collapse_through;
        if !committed
            && !self.is_viewport_taffy_node(node_id)
            && let Some(ctx) = block_context
        {
            let clear = self.boxes[LayoutBoxId::from_taffy(node_id).index()]
                .style
                .taffy
                .clear;
            if clear != Clear::None
                && let Some(threshold) = ctx.cleared_threshold(clear)
            {
                let forced = ctx.has_adjoining_float(clear)
                    || adjoining_matches_clear(self.trampoline.job_top_adjoining_floats, clear);
                let top_margin = output.top_margin.resolve();
                // `ctx.y_offset = estimated_y.max(clear_pos)`, so `threshold <= 0`
                // and `threshold == 0` indicates `estimated_y <= clear_pos`.
                if forced || threshold >= 0.0 || (top_margin < 0.0 && top_margin < threshold) {
                    committed = true;
                }
            }
        }
        if committed {
            self.trampoline.job_open_strut_depth = 0;
        }
    }

    #[inline(never)]
    pub(super) fn observe_open_strut_child_exit(&mut self, node_id: NodeId, inputs: LayoutInput) {
        if inputs.axis != RequestedAxis::Both
            || inputs.sizing_purpose != SizingPurpose::Layout
            || inputs.sizing_mode != SizingMode::InherentSize
            || inputs.run_mode == RunMode::PerformHiddenLayout
            || self.is_viewport_taffy_node(node_id)
        {
            return;
        }
        let id = LayoutBoxId::from_taffy(node_id);
        let style = &self.boxes[id.index()].style.taffy;
        if style.position.is_out_of_flow() || style.display == taffy::Display::None {
            return;
        }
        if super::box_is_effectively_floated(self, id) {
            if let Some(direction) = style.float.float_direction() {
                self.trampoline.job_top_adjoining_floats[direction as usize] = true;
            }
            return;
        }
        // Any non-floated, non-OOF child laid out via `compute_child_layout`
        // establishes an independent formatting context (or is a table/replaced
        // leaf) and cannot collapse through, committing the parent's top strut.
        self.trampoline.job_open_strut_depth = 0;
    }

    fn run_block_child_with_context(
        &mut self,
        job: &SuspendedLayoutCall,
        ctx: &mut BlockContext<'_>,
        ctx_width: f32,
    ) -> CompletedLayout {
        let before_mark = ctx.float_mark();
        let before_adjoining = [
            ctx.has_adjoining_float(Clear::Left),
            ctx.has_adjoining_float(Clear::Right),
        ];
        self.trampoline.in_job = true;
        self.trampoline.job_top_adjoining_floats = before_adjoining;
        self.trampoline.job_open_strut_depth =
            u32::from(self.block_collapses_top_margin_with_children(job.node_id, job.inputs));

        let output = self.compute_block_child_layout_inner(job.node_id, job.inputs, Some(ctx));

        let observed_adjoining = self.trampoline.job_top_adjoining_floats;
        let job_still_open = self.trampoline.job_open_strut_depth > 0;
        self.trampoline.in_job = false;
        self.trampoline.job_top_adjoining_floats = [false, false];
        self.trampoline.job_open_strut_depth = 0;

        if self.trampoline.aborted {
            return CompletedLayout {
                output,
                delta: BlockContextDelta::default(),
            };
        }

        let placed_floats = if ctx.float_mark() != before_mark {
            self.capture_floats_since(ctx, before_mark, ctx_width)
        } else {
            Vec::new()
        };
        let float_content_contribution = ctx.floated_content_height_contribution();
        let final_adjoining = if job_still_open {
            [
                observed_adjoining[0] || ctx.has_adjoining_float(Clear::Left),
                observed_adjoining[1] || ctx.has_adjoining_float(Clear::Right),
            ]
        } else {
            observed_adjoining
        };
        let added_adjoining_floats = [
            final_adjoining[0] && !before_adjoining[0],
            final_adjoining[1] && !before_adjoining[1],
        ];
        CompletedLayout {
            output,
            delta: BlockContextDelta {
                placed_floats,
                float_content_contribution,
                added_adjoining_floats,
            },
        }
    }

    fn execute_trampoline_job(&mut self, job: &SuspendedLayoutCall) -> CompletedLayout {
        debug_assert_eq!(self.trampoline.depth, 0);
        self.trampoline.depth = 1;
        let completed = match &job.key.kind {
            TrampolineCallKind::Child => CompletedLayout {
                output: self.compute_child_layout_inner(job.node_id, job.inputs),
                delta: BlockContextDelta::default(),
            },
            TrampolineCallKind::BlockChild(TrampolineBlockContextState::None) => CompletedLayout {
                output: self.compute_block_child_layout_inner(job.node_id, job.inputs, None),
                delta: BlockContextDelta::default(),
            },
            TrampolineCallKind::BlockChild(TrampolineBlockContextState::Empty {
                is_root,
                ctx_width_bits,
            }) => {
                let ctx_width = f32::from_bits(*ctx_width_bits);
                let mut root_bfc = BlockFormattingContext::new();
                let mut root_ctx = root_bfc.root_block_context();
                root_ctx.set_width(ctx_width);
                if *is_root {
                    self.run_block_child_with_context(job, &mut root_ctx, ctx_width)
                } else {
                    let mut sub_ctx = root_ctx.sub_context(0.0, [0.0, 0.0]);
                    self.run_block_child_with_context(job, &mut sub_ctx, ctx_width)
                }
            }
            TrampolineCallKind::BlockChild(TrampolineBlockContextState::Populated(snapshot)) => {
                let ctx_width = f32::from_bits(snapshot.ctx_width_bits);
                let mut shift_left: f32 = 0.0;
                let mut shift_right: f32 = 0.0;
                for f in &snapshot.floats {
                    let rel_x = f32::from_bits(f.rel_x_bits);
                    let width = f32::from_bits(f.width_bits);
                    if rel_x < 0.0 {
                        shift_left = shift_left.max(-rel_x);
                    }
                    let right_overhang = (rel_x + width) - ctx_width;
                    if right_overhang > 0.0 {
                        shift_right = shift_right.max(right_overhang);
                    }
                }
                let syn_bfc_width = shift_left + ctx_width + shift_right;
                let mut root_bfc = BlockFormattingContext::new();
                let mut root_ctx = root_bfc.root_block_context();
                root_ctx.set_width(syn_bfc_width);

                let last_left_idx = snapshot
                    .floats
                    .iter()
                    .rposition(|f| f.direction == FloatDirectionKey::Left);
                let last_right_idx = snapshot
                    .floats
                    .iter()
                    .rposition(|f| f.direction == FloatDirectionKey::Right);

                for (idx, f) in snapshot.floats.iter().enumerate() {
                    let rel_x = f32::from_bits(f.rel_x_bits);
                    let rel_y = f32::from_bits(f.rel_y_bits);
                    let margin_box = Size {
                        width: f32::from_bits(f.width_bits),
                        height: f32::from_bits(f.height_bits),
                    };
                    let opp_inset = f32::from_bits(f.opposite_cb_inset_bits);
                    let direction = f.direction.to_taffy();
                    let clear = f.clear.to_taffy();
                    let (cb_left, cb_right) = match direction {
                        FloatDirection::Left => {
                            let left = (shift_left + rel_x).max(0.0);
                            let right = if opp_inset > 0.0 {
                                shift_right + opp_inset
                            } else {
                                0.0
                            };
                            (left, right)
                        }
                        FloatDirection::Right => {
                            let right =
                                (shift_right + ctx_width - (rel_x + margin_box.width)).max(0.0);
                            let left = if opp_inset > 0.0 {
                                shift_left + opp_inset
                            } else {
                                0.0
                            };
                            (left, right)
                        }
                    };
                    let adjoins = match direction {
                        FloatDirection::Left => {
                            snapshot.adjoining_floats[0] && last_left_idx == Some(idx)
                        }
                        FloatDirection::Right => {
                            snapshot.adjoining_floats[1] && last_right_idx == Some(idx)
                        }
                    };
                    if self.trampoline.track_floats {
                        self.trampoline
                            .record_float_meta(f.id, margin_box, direction, clear);
                    }
                    root_ctx.apply_content_box_inset([cb_left, cb_right]);
                    root_ctx.place_floated_box(f.id, margin_box, rel_y, direction, clear, adjoins);
                }
                root_ctx.apply_content_box_inset([0.0, 0.0]);

                if snapshot.is_root {
                    let prior_contrib = f32::from_bits(snapshot.float_content_contribution_bits);
                    if prior_contrib.is_finite() {
                        root_ctx.add_child_floated_content_height_contribution(prior_contrib);
                    }
                    self.run_block_child_with_context(job, &mut root_ctx, ctx_width)
                } else {
                    let mut sub_ctx = root_ctx.sub_context(0.0, [shift_left, shift_right]);
                    self.run_block_child_with_context(job, &mut sub_ctx, ctx_width)
                }
            }
        };
        self.trampoline.depth = 0;
        completed
    }

    #[inline(always)]
    pub(super) fn trampoline_enter(&mut self) {
        self.trampoline.depth = self.trampoline.depth.saturating_add(1);
    }

    #[inline(always)]
    pub(super) fn trampoline_exit(&mut self) {
        self.trampoline.depth = self.trampoline.depth.saturating_sub(1);
    }
}
