//! Pass-only diagnostics, metrics, and paint surrounding one frozen tree.

use std::{fmt::Debug, hash::Hash, ops::Deref, time::Duration};

use crate::{LayoutError, PaintDiagnostic, PaintSnapshot};

use super::{
    query::{LayoutAnswers, LayoutQueryBatch},
    tree::{FrozenLayoutTree, LayoutTreeRetentionMetrics},
};

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
    pub box_count: usize,
    pub fragment_count: usize,
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

    /// Consumes one pass into the sole retainable tree plus optional one-shot
    /// paint input. Embedded-frame composition uses this to nest the child tree
    /// into its parent while separately consuming paint into the parent frame.
    pub fn into_tree_and_paint_snapshot(self) -> (FrozenLayoutTree<N>, Option<PaintSnapshot>) {
        (self.tree, self.paint_snapshot)
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
    ) -> Self {
        Self {
            tree,
            diagnostics,
            metrics,
            paint_snapshot,
        }
    }
}
