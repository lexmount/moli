//! Pass-only diagnostics, metrics, and paint surrounding one frozen tree.

use std::{fmt::Debug, hash::Hash, ops::Deref, time::Duration};

use crate::{LayoutError, PaintDiagnostic, PaintSnapshot};

use super::{
    query::{LayoutAnswers, LayoutQueryBatch},
    tree::{FrozenLayoutTree, LayoutTreeRetentionMetrics},
};

/// One direct computed CSS image URL observed while constructing a box.
///
/// The source node keeps the owning `Document` recoverable at the renderer
/// boundary without teaching this crate about browsing contexts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutCssImageReference<N> {
    pub source: N,
    pub resolved_url: String,
}

/// Why a full, synchronous layout pass was forced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutFlushReason {
    Screenshot,
    Screencast,
    SynchronousGeometry,
    CdpGeometry,
    ObserverDelivery,
    HitTest,
    Paint,
    Test,
}

/// Diagnostics and cost counters for exactly one full pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutPassMetrics {
    pub reason: LayoutFlushReason,
    pub elapsed: Duration,
    pub box_tree_elapsed: Duration,
    pub list_marker_elapsed: Duration,
    pub form_control_elapsed: Duration,
    pub inline_preparation_elapsed: Duration,
    pub numeric_layout_elapsed: Duration,
    pub numeric_first_pass_elapsed: Duration,
    pub numeric_followup_passes_elapsed: Duration,
    pub overflow_detection_elapsed: Duration,
    pub scrollbar_feedback_elapsed: Duration,
    pub embedded_frame_elapsed: Duration,
    pub projection_elapsed: Duration,
    pub numeric_layout_pass_count: usize,
    /// Number of box-cache entries explicitly invalidated across automatic
    /// scrollbar feedback iterations. Unchanged subtrees stay cacheable.
    pub numeric_feedback_invalidated_node_count: usize,
    /// Boxes whose local or descendant overflow was recomputed after the
    /// initial linear overflow projection. Unaffected branches are excluded.
    pub numeric_feedback_overflow_recomputed_node_count: usize,
    pub box_count: usize,
    pub fragment_count: usize,
    /// Paint-order events considered for this capture before viewport culling.
    pub paint_event_count: usize,
    /// Paint-order events skipped because their conservative ink bounds did
    /// not intersect the capture cull rect.
    pub paint_culled_event_count: usize,
    /// Final inline lines considered across foreground and text-mask paint.
    pub paint_text_line_count: usize,
    /// Inline lines whose conservative ink bounds missed the capture.
    pub paint_culled_text_line_count: usize,
    pub paint_operation_count: usize,
    pub fallback_count: usize,
}

/// Transient products of exactly one complete layout demand.
///
/// Consumers may inspect the tree and take an optional paint snapshot while
/// handling the demand. Only [`FrozenLayoutTree`] crosses the latest-layout
/// retention boundary; diagnostics, metrics, and paint remain pass-owned.
pub struct LayoutPassResult<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub tree: FrozenLayoutTree<N>,
    pub diagnostics: Vec<PaintDiagnostic>,
    pub metrics: LayoutPassMetrics,
    paint_snapshot: Option<PaintSnapshot>,
    css_image_references: Vec<LayoutCssImageReference<N>>,
}

impl<N> Deref for LayoutPassResult<N>
where
    N: Copy + Debug + Eq + Hash,
{
    type Target = FrozenLayoutTree<N>;

    fn deref(&self) -> &Self::Target {
        &self.tree
    }
}

impl<N> LayoutPassResult<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub fn paint_snapshot(&self) -> Option<&PaintSnapshot> {
        self.paint_snapshot.as_ref()
    }

    pub fn css_image_references(&self) -> &[LayoutCssImageReference<N>] {
        &self.css_image_references
    }

    pub fn take_paint_snapshot(&mut self) -> Result<PaintSnapshot, LayoutError> {
        self.paint_snapshot
            .take()
            .ok_or(LayoutError::PaintProjectionNotRequested)
    }

    pub fn into_paint_snapshot(self) -> Result<PaintSnapshot, LayoutError> {
        self.paint_snapshot
            .ok_or(LayoutError::PaintProjectionNotRequested)
    }

    /// Consumes every pass-only product and returns the sole retainable tree.
    pub fn into_tree(self) -> FrozenLayoutTree<N> {
        self.tree
    }

    /// Consumes all products needed to compose an embedded frame into its
    /// parent pass, including resource discovery metadata from that child.
    pub fn into_embedded_parts(
        self,
    ) -> (
        FrozenLayoutTree<N>,
        Option<PaintSnapshot>,
        Vec<LayoutCssImageReference<N>>,
    ) {
        (self.tree, self.paint_snapshot, self.css_image_references)
    }

    pub fn retention_metrics(&self) -> LayoutTreeRetentionMetrics {
        self.tree.retention_metrics()
    }

    pub fn validate_retention_budget(&self) -> Result<(), LayoutError> {
        self.tree.validate_retention_budget()
    }

    pub fn answer_queries(&self, batch: &LayoutQueryBatch<N>) -> LayoutAnswers<N> {
        self.tree.answer_queries(batch, self.metrics)
    }
}

impl<N> LayoutPassResult<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) fn new(
        tree: FrozenLayoutTree<N>,
        diagnostics: Vec<PaintDiagnostic>,
        metrics: LayoutPassMetrics,
        paint_snapshot: Option<PaintSnapshot>,
        css_image_references: Vec<LayoutCssImageReference<N>>,
    ) -> Self {
        Self {
            tree,
            diagnostics,
            metrics,
            paint_snapshot,
            css_image_references,
        }
    }
}
