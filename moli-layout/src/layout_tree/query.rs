//! Public geometry request/answer types and the batch dispatcher.

use std::{fmt::Debug, hash::Hash, ops::Range};

use crate::LayoutError;

use super::{
    hit_test::{LayoutCaretPosition, LayoutHit},
    model::{
        LayoutBoxModel, LayoutFragmentId, LayoutOutputBoxId, LayoutPoint, LayoutQuad, LayoutSize,
        LayoutViewport,
    },
    pass_result::{LayoutFlushReason, LayoutPassMetrics},
    tree::FrozenLayoutTree,
};

/// A short-lived source view derived from frozen box provenance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutNodeOutput {
    pub principal_box: Option<LayoutOutputBoxId>,
    pub fragments: Vec<LayoutFragmentId>,
    /// Direct generated boxes used only to resolve operations whose target is
    /// a `display: contents` source. They do not manufacture CSSOM rects for
    /// the box-suppressed element itself.
    pub scroll_proxy_boxes: Vec<LayoutOutputBoxId>,
}

/// Document-level dimensions returned from the unified output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutDocumentMetrics {
    pub viewport: LayoutViewport,
    pub viewport_scroll: LayoutPoint,
    pub content_size: LayoutSize,
}

/// CSSOM View and observer metrics for one source element.
///
/// Transformed quads use viewport CSS pixels. Offset and size fields retain
/// the untransformed layout values required by offset/client/scroll APIs.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutElementMetrics<N> {
    pub offset_parent: Option<N>,
    pub offset_position: LayoutPoint,
    pub offset_size: LayoutSize,
    pub content_size: LayoutSize,
    pub client_size: LayoutSize,
    pub client_border: LayoutPoint,
    pub scroll_size: LayoutSize,
    pub scroll_offset: LayoutPoint,
    pub minimum_scroll_offset: LayoutPoint,
    pub maximum_scroll_offset: LayoutPoint,
    pub scrollport: LayoutQuad,
    pub scrollable_overflow: LayoutQuad,
    pub is_scroll_container: bool,
    pub allows_user_scroll_x: bool,
    pub allows_user_scroll_y: bool,
    pub clips_overflow: bool,
    pub visible: bool,
    pub pointer_events: bool,
}

/// One scroll container on a target's layout ancestor chain.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutScrollContainerMetrics<N> {
    pub source: N,
    pub metrics: LayoutElementMetrics<N>,
}

/// Geometry needed to run one `scrollIntoView` operation without retaining
/// layout state or forcing source-dependent follow-up passes.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutScrollIntoViewGeometry<N> {
    pub target_rects: Vec<LayoutQuad>,
    /// Innermost to outermost, including the root scrolling element.
    pub scroll_containers: Vec<LayoutScrollContainerMetrics<N>>,
}

/// Owned inputs for one IntersectionObserver target/root pair.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutIntersectionGeometry {
    pub target_rects: Vec<LayoutQuad>,
    pub root_rect: LayoutQuad,
    pub ancestor_clips: Vec<LayoutQuad>,
    pub target_has_layout: bool,
    pub target_visible: bool,
    pub root_clips_overflow: bool,
    pub root_is_layout_ancestor: bool,
}

/// One request in an explicit same-pass geometry batch.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutQuery<N> {
    DocumentMetrics,
    BoxModel {
        source: N,
    },
    ClientRects {
        source: N,
    },
    ContentQuads {
        source: N,
    },
    TextRangeRects {
        source: N,
        utf16_range: Range<usize>,
    },
    ElementMetrics {
        source: N,
    },
    ScrollIntoViewGeometry {
        source: N,
    },
    IntersectionGeometry {
        target: N,
        root: Option<N>,
    },
    HitTest {
        point: LayoutPoint,
        ignore_pointer_events_none: bool,
    },
    HitTestAll {
        point: LayoutPoint,
        ignore_pointer_events_none: bool,
    },
    CaretPosition {
        point: LayoutPoint,
    },
    EventOffset {
        source: N,
        point: LayoutPoint,
    },
}

/// A high-level batch that must be answered from one full layout pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutQueryBatch<N> {
    pub queries: Vec<LayoutQuery<N>>,
}

impl<N> LayoutQueryBatch<N> {
    pub fn new(queries: Vec<LayoutQuery<N>>) -> Self {
        Self { queries }
    }

    pub fn push(&mut self, query: LayoutQuery<N>) {
        self.queries.push(query);
    }
}

/// One answer corresponding to the same-index query in a batch.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutQueryAnswer<N> {
    DocumentMetrics(LayoutDocumentMetrics),
    BoxModel(Option<LayoutBoxModel>),
    ClientRects(Vec<LayoutQuad>),
    ContentQuads(Vec<LayoutQuad>),
    TextRangeRects(Vec<LayoutQuad>),
    ElementMetrics(Option<LayoutElementMetrics<N>>),
    ScrollIntoViewGeometry(Option<LayoutScrollIntoViewGeometry<N>>),
    IntersectionGeometry(Option<LayoutIntersectionGeometry>),
    HitTest(Option<LayoutHit<N>>),
    HitTestAll(Vec<LayoutHit<N>>),
    CaretPosition(Option<LayoutCaretPosition<N>>),
    EventOffset(Option<LayoutPoint>),
}

/// Minimal owned results derived from one frozen layout tree.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutAnswers<N> {
    pub answers: Vec<LayoutQueryAnswer<N>>,
    pub metrics: LayoutPassMetrics,
}

/// Common real/mock geometry boundary used by renderer consumers.
pub trait GeometryProvider {
    type NodeId: Copy + Debug + Eq + Hash;

    /// Answers a batch from the provider's latest layout state.
    ///
    /// The provider decides whether this requires a fresh pass or can reuse an
    /// already-owned tree; callers must not assume that one call equals one
    /// layout computation.
    fn answer(
        &mut self,
        reason: LayoutFlushReason,
        viewport: LayoutViewport,
        queries: &LayoutQueryBatch<Self::NodeId>,
    ) -> Result<LayoutAnswers<Self::NodeId>, LayoutError>;
}

impl<N> FrozenLayoutTree<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub fn answer_queries(
        &self,
        batch: &LayoutQueryBatch<N>,
        metrics: LayoutPassMetrics,
    ) -> LayoutAnswers<N> {
        let answers = batch
            .queries
            .iter()
            .map(|query| match query {
                LayoutQuery::DocumentMetrics => {
                    LayoutQueryAnswer::DocumentMetrics(LayoutDocumentMetrics {
                        viewport: self.viewport,
                        viewport_scroll: self.viewport_scroll,
                        content_size: self.content_size,
                    })
                }
                LayoutQuery::BoxModel { source } => {
                    LayoutQueryAnswer::BoxModel(self.box_model_for_source(*source))
                }
                LayoutQuery::ClientRects { source } => {
                    LayoutQueryAnswer::ClientRects(self.client_rects_for_source(*source))
                }
                LayoutQuery::ContentQuads { source } => {
                    LayoutQueryAnswer::ContentQuads(self.content_quads_for_source(*source))
                }
                LayoutQuery::TextRangeRects {
                    source,
                    utf16_range,
                } => LayoutQueryAnswer::TextRangeRects(
                    self.text_range_rects(*source, utf16_range.clone()),
                ),
                LayoutQuery::ElementMetrics { source } => {
                    LayoutQueryAnswer::ElementMetrics(self.element_metrics_for_source(*source))
                }
                LayoutQuery::ScrollIntoViewGeometry { source } => {
                    LayoutQueryAnswer::ScrollIntoViewGeometry(
                        self.scroll_into_view_geometry_for_source(*source),
                    )
                }
                LayoutQuery::IntersectionGeometry { target, root } => {
                    LayoutQueryAnswer::IntersectionGeometry(
                        self.intersection_geometry(*target, *root),
                    )
                }
                LayoutQuery::HitTest {
                    point,
                    ignore_pointer_events_none,
                } => LayoutQueryAnswer::HitTest(self.hit_test(*point, *ignore_pointer_events_none)),
                LayoutQuery::HitTestAll {
                    point,
                    ignore_pointer_events_none,
                } => LayoutQueryAnswer::HitTestAll(
                    self.hit_test_all(*point, *ignore_pointer_events_none),
                ),
                LayoutQuery::CaretPosition { point } => {
                    LayoutQueryAnswer::CaretPosition(self.caret_position(*point))
                }
                LayoutQuery::EventOffset { source, point } => {
                    LayoutQueryAnswer::EventOffset(self.event_offset_for_source(*source, *point))
                }
            })
            .collect();
        LayoutAnswers { answers, metrics }
    }
}
