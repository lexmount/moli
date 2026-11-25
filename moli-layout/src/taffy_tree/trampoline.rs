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
//! inputs, the active call stack unwinds cleanly without writing partial
//! caches or layouts, the suspended subtree runs from call-stack depth 0 on
//! an explicit heap work stack, and its `LayoutOutput` is memoized so the
//! parent resumes in O(1) stack space.

use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::mem::{Discriminant, discriminant};

use style::Atom;
use taffy::{
    AutoSizeBehavior, AvailableSpace, BlockFormattingContext, LayoutInput, LayoutOutput, NodeId,
    RequestedAxis, RunMode, SizingMode, SizingPurpose, Style, WritingMode,
};

use crate::{LayoutBoxId, LayoutWorld};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TrampolineBlockContextKind {
    Root,
    Sub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TrampolineCallKind {
    Child,
    BlockChild {
        block_context: Option<TrampolineBlockContextKind>,
    },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TrampolineKey {
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
    kind: TrampolineCallKind,
    inputs: LayoutInput,
    measure_baselines: bool,
    table_cell_percentage_height: Option<(LayoutBoxId, bool)>,
    style_override: Option<Box<Style<Atom>>>,
}

#[derive(Debug)]
pub(crate) struct LayoutTrampoline {
    active: bool,
    depth: u32,
    max_depth: u32,
    aborted: bool,
    pending: Option<SuspendedLayoutCall>,
    completed: HashMap<TrampolineKey, LayoutOutput>,
    /// Most recent `RunMode::PerformLayout` key memoized per node, matching
    /// Taffy's single-slot `final_layout_entry` so re-laying out a subtree
    /// with different inputs invalidates earlier `PerformLayout` entries.
    last_perform_layout: HashMap<usize, TrampolineKey>,
    pinned_style_nodes: Vec<LayoutBoxId>,
}

impl Default for LayoutTrampoline {
    fn default() -> Self {
        Self {
            active: false,
            depth: 0,
            max_depth: DEFAULT_MAX_CALL_STACK_DEPTH,
            aborted: false,
            pending: None,
            completed: HashMap::new(),
            last_perform_layout: HashMap::new(),
            pinned_style_nodes: Vec::new(),
        }
    }
}

impl LayoutTrampoline {
    #[inline(always)]
    pub(crate) const fn is_aborted(&self) -> bool {
        self.aborted
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
            if self
                .last_perform_layout
                .get(&raw_id)
                .is_some_and(|key| key.pinned_style)
            {
                self.last_perform_layout.remove(&raw_id);
            }
        }
    }

    fn record_completed(&mut self, key: TrampolineKey, run_mode: RunMode, output: LayoutOutput) {
        if run_mode == RunMode::PerformLayout
            && let Some(prev) = self.last_perform_layout.insert(key.node_id, key)
            && prev != key
        {
            self.completed.remove(&prev);
        }
        self.completed.insert(key, output);
    }

    fn invalidate_perform_layout_for(&mut self, node_id: usize) {
        if let Some(prev) = self.last_perform_layout.remove(&node_id) {
            self.completed.remove(&prev);
        }
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

    /// Check whether this call should return a memoized trampoline result or
    /// suspend onto the heap work stack. Returns `Some(output)` when the
    /// caller must return immediately without descending further.
    #[inline(always)]
    pub(super) fn trampoline_before_call(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        kind: TrampolineCallKind,
    ) -> Option<LayoutOutput> {
        if self.trampoline.aborted {
            return Some(LayoutOutput::HIDDEN);
        }
        if !self.trampoline.active {
            return None;
        }
        let at_limit = self.trampoline.depth >= self.trampoline.max_depth;
        if !at_limit && self.trampoline.completed.is_empty() {
            return None;
        }
        let is_viewport = self.is_viewport_taffy_node(node_id);
        let pinned = self.trampoline.is_pinned(node_id, is_viewport);
        let key = TrampolineKey {
            node_id: usize::from(node_id),
            kind,
            inputs: inputs.into(),
            measure_baselines: self.measure_baselines,
            table_cell_percentage_height: self.table_cell_percentage_height,
            pinned_style: pinned,
        };
        if let Some(output) = self.trampoline.completed.get(&key) {
            return Some(output.clone());
        }
        if !at_limit {
            if inputs.run_mode == RunMode::PerformLayout {
                self.trampoline.invalidate_perform_layout_for(key.node_id);
            }
            return None;
        }
        if pinned
            && self.trampoline.depth
                < self
                    .trampoline
                    .max_depth
                    .saturating_add(PINNED_STYLE_GRACE_DEPTH)
        {
            if inputs.run_mode == RunMode::PerformLayout {
                self.trampoline.invalidate_perform_layout_for(key.node_id);
            }
            return None;
        }
        let style_override = (pinned && !is_viewport).then(|| {
            let id = LayoutBoxId::from_taffy(node_id);
            Box::new(self.boxes[id.index()].style.taffy.clone())
        });
        self.trampoline.pending = Some(SuspendedLayoutCall {
            key,
            node_id,
            kind,
            inputs,
            measure_baselines: self.measure_baselines,
            table_cell_percentage_height: self.table_cell_percentage_height,
            style_override,
        });
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
        self.trampoline.pending = None;
        self.trampoline.completed.clear();
        self.trampoline.last_perform_layout.clear();

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

                let output = self.execute_trampoline_job(&job);

                if let Some((id, prev_style)) = restored_style {
                    self.boxes[id.index()].style.taffy = prev_style;
                    self.trampoline.pinned_style_nodes.pop();
                }
                self.measure_baselines = prev_measure;
                self.table_cell_percentage_height = prev_table_pct;

                if self.trampoline.aborted {
                    if let Some(next_job) = self.trampoline.pending.take() {
                        // Guard against impossible non-DAG re-suspension cycles.
                        if work_stack.iter().any(|active| active.key == next_job.key) {
                            self.trampoline.record_completed(
                                next_job.key,
                                next_job.inputs.run_mode,
                                LayoutOutput::HIDDEN,
                            );
                        } else {
                            work_stack.push(next_job);
                        }
                        continue;
                    }
                    self.trampoline.record_completed(
                        job.key,
                        job.inputs.run_mode,
                        LayoutOutput::HIDDEN,
                    );
                    work_stack.pop();
                    continue;
                }
                self.trampoline
                    .record_completed(job.key, job.inputs.run_mode, output);
                work_stack.pop();
            } else {
                self.trampoline.aborted = false;
                self.trampoline.pending = None;
                let value = root_op(self);
                if self.trampoline.aborted
                    && let Some(next_job) = self.trampoline.pending.take()
                {
                    work_stack.push(next_job);
                    continue;
                }
                break value;
            }
        };

        self.trampoline.active = false;
        self.trampoline.aborted = false;
        self.trampoline.pending = None;
        self.trampoline.completed.clear();
        self.trampoline.last_perform_layout.clear();
        result
    }

    fn execute_trampoline_job(&mut self, job: &SuspendedLayoutCall) -> LayoutOutput {
        debug_assert_eq!(self.trampoline.depth, 0);
        self.trampoline.depth = 1;
        let output = match job.kind {
            TrampolineCallKind::Child => self.compute_child_layout_inner(job.node_id, job.inputs),
            TrampolineCallKind::BlockChild { block_context } => match block_context {
                None => self.compute_block_child_layout_inner(job.node_id, job.inputs, None),
                Some(TrampolineBlockContextKind::Root) => {
                    let mut root_bfc = BlockFormattingContext::new();
                    let mut root_ctx = root_bfc.root_block_context();
                    self.compute_block_child_layout_inner(
                        job.node_id,
                        job.inputs,
                        Some(&mut root_ctx),
                    )
                }
                Some(TrampolineBlockContextKind::Sub) => {
                    let mut root_bfc = BlockFormattingContext::new();
                    let mut root_ctx = root_bfc.root_block_context();
                    let mut sub_ctx = root_ctx.sub_context(0.0, [0.0, 0.0]);
                    self.compute_block_child_layout_inner(
                        job.node_id,
                        job.inputs,
                        Some(&mut sub_ctx),
                    )
                }
            },
        };
        self.trampoline.depth = 0;
        output
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
