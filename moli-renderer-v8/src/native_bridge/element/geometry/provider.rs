use std::{collections::HashSet, time::Duration};

use moli_layout::{
    LayoutAnswers, LayoutBoxModel, LayoutCaretPosition, LayoutDocumentMetrics,
    LayoutElementMetrics, LayoutError, LayoutFlushReason, LayoutHit, LayoutIntersectionGeometry,
    LayoutPassMetrics, LayoutPoint, LayoutQuad, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch,
    LayoutScrollContainerMetrics, LayoutScrollIntoViewGeometry, LayoutScrollbarHit, LayoutSize,
    LayoutTransform2D, LayoutViewport,
};

use super::layout::{
    ClientRect, compute_mock_client_rect, compute_mock_offset_parent,
    compute_mock_scroll_adjusted_client_rect, mock_hit_test_handle,
    mock_layout_client_rect_for_node, zero_client_rect,
};
use crate::{document_runtime::DomHandle, dom::native::Node, native_bridge::JsContextHost};

const HIT_TEST_CHILD_FRAME_DEPTH_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InputHit {
    pub(crate) handle: DomHandle,
    /// Converts a root-frame input position into the viewport of `handle`'s
    /// owner frame. Keeping the affine map lets boundary and capture events
    /// convert a new root position into a previously targeted child frame.
    pub(crate) root_to_frame: LayoutTransform2D,
}

#[derive(Clone, Copy)]
struct FrameHitTest {
    document: DomHandle,
    viewport: LayoutViewport,
    point: LayoutPoint,
    root_to_frame: LayoutTransform2D,
}

impl FrameHitTest {
    fn root(runtime: &JsContextHost, document: DomHandle, point: LayoutPoint) -> Self {
        Self {
            document,
            viewport: runtime.layout_viewport_for_document(document),
            point,
            root_to_frame: LayoutTransform2D::IDENTITY,
        }
    }

    /// Blink performs the equivalent conversion through LocalFrameView when
    /// an input hit crosses an embedded-content boundary.
    fn child(
        self,
        runtime: &JsContextHost,
        frame: DomHandle,
        hit: LayoutHit<DomHandle>,
    ) -> Option<Self> {
        let document = runtime.child_browsing_context_document_handle(frame)?;
        let content_box = hit.local_content_box?;
        if !content_box.contains(hit.local_point) {
            return None;
        }
        let frame_to_child = LayoutTransform2D::translation(-content_box.x, -content_box.y)
            .concatenate(hit.viewport_to_local);
        Some(Self {
            document,
            viewport: LayoutViewport::new(
                css_viewport_dimension(content_box.width),
                css_viewport_dimension(content_box.height),
                self.viewport.device_pixel_ratio,
            ),
            point: frame_to_child.map_point(self.point),
            root_to_frame: frame_to_child.concatenate(self.root_to_frame),
        })
    }
}

pub(crate) fn observable_geometry_batch(
    runtime: &JsContextHost,
    document: DomHandle,
    reason: LayoutFlushReason,
    batch: &LayoutQueryBatch<DomHandle>,
) -> Result<LayoutAnswers<DomHandle>, LayoutError> {
    if runtime.layout_policy().uses_real_layout() {
        runtime.answer_layout_for_document(document, reason, batch)
    } else {
        Ok(answer_mock_queries(runtime, document, reason, batch))
    }
}

pub(crate) fn observable_client_rects(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: LayoutFlushReason,
) -> Result<Vec<ClientRect>, LayoutError> {
    if !runtime.dom_host().is_connected(source) {
        return Ok(Vec::new());
    }
    let Some(document) = runtime.layout_document_for_source(source) else {
        return Ok(Vec::new());
    };
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![LayoutQuery::ClientRects { source }]),
    )?;
    match answers.answers.into_iter().next() {
        Some(LayoutQueryAnswer::ClientRects(rects)) => {
            Ok(rects.into_iter().map(client_rect_from_quad).collect())
        }
        _ => Err(provider_contract_error("client rects")),
    }
}

pub(crate) fn observable_sources_with_fragments(
    runtime: &JsContextHost,
    document: DomHandle,
    sources: &[DomHandle],
    reason: LayoutFlushReason,
) -> Result<HashSet<DomHandle>, LayoutError> {
    if sources.is_empty() {
        return Ok(HashSet::new());
    }
    let queries = sources
        .iter()
        .copied()
        .map(|source| LayoutQuery::ContentQuads { source })
        .collect();
    let answers =
        observable_geometry_batch(runtime, document, reason, &LayoutQueryBatch::new(queries))?;
    if answers.answers.len() != sources.len() {
        return Err(provider_contract_error("rendered source fragment"));
    }
    let mut rendered = HashSet::new();
    for (source, answer) in sources.iter().copied().zip(answers.answers) {
        match answer {
            LayoutQueryAnswer::ContentQuads(quads) => {
                if !quads.is_empty() {
                    rendered.insert(source);
                }
            }
            _ => return Err(provider_contract_error("rendered source fragment")),
        }
    }
    Ok(rendered)
}

pub(crate) fn observable_bounding_client_rect(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: LayoutFlushReason,
) -> Result<ClientRect, LayoutError> {
    let mut rects = observable_client_rects(runtime, source, reason)?.into_iter();
    let Some(mut bounds) = rects.next() else {
        return Ok(zero_client_rect());
    };
    for rect in rects {
        bounds = union_client_rect(bounds, rect);
    }
    Ok(bounds)
}

pub(crate) fn observable_bounding_client_rects(
    runtime: &JsContextHost,
    sources: &[DomHandle],
    reason: LayoutFlushReason,
) -> Result<Vec<ClientRect>, LayoutError> {
    let Some((&first, rest)) = sources.split_first() else {
        return Ok(Vec::new());
    };
    if !runtime.dom_host().is_connected(first) {
        return Ok(vec![zero_client_rect(); sources.len()]);
    }
    let Some(document) = runtime.layout_document_for_source(first) else {
        return Ok(vec![zero_client_rect(); sources.len()]);
    };
    if rest.iter().any(|source| {
        runtime.layout_document_for_source(*source) != Some(document)
            || !runtime.dom_host().is_connected(*source)
    }) {
        return Err(LayoutError::source_contract(
            "geometry batch",
            "bounding-client-rect sources do not share one connected document",
        ));
    }
    let queries = sources
        .iter()
        .copied()
        .map(|source| LayoutQuery::ClientRects { source })
        .collect();
    let answers =
        observable_geometry_batch(runtime, document, reason, &LayoutQueryBatch::new(queries))?;
    if answers.answers.len() != sources.len() {
        return Err(provider_contract_error("bounding client rects"));
    }
    answers
        .answers
        .into_iter()
        .map(|answer| match answer {
            LayoutQueryAnswer::ClientRects(rects) => Ok(rects
                .into_iter()
                .map(client_rect_from_quad)
                .reduce(union_client_rect)
                .unwrap_or_else(zero_client_rect)),
            _ => Err(provider_contract_error("bounding client rects")),
        })
        .collect()
}

pub(crate) fn observable_box_model(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: LayoutFlushReason,
) -> Result<Option<LayoutBoxModel>, LayoutError> {
    if !runtime.dom_host().is_connected(source) {
        return Ok(None);
    }
    let Some(document) = runtime.layout_document_for_source(source) else {
        return Ok(None);
    };
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![LayoutQuery::BoxModel { source }]),
    )?;
    match answers.answers.into_iter().next() {
        Some(LayoutQueryAnswer::BoxModel(model)) => Ok(model),
        _ => Err(provider_contract_error("box model")),
    }
}

/// Resolve one viewport-relative box for scroll anchoring. Real layout already
/// projects root scrolling into viewport coordinates; the explicit adjustment
/// is retained only inside the legacy Mock provider.
pub(crate) fn observable_scroll_adjusted_client_rect(
    runtime: &JsContextHost,
    source: DomHandle,
    scroll_x: f64,
    scroll_y: f64,
    reason: LayoutFlushReason,
) -> Result<ClientRect, LayoutError> {
    if runtime.layout_policy().uses_real_layout() {
        observable_bounding_client_rect(runtime, source, reason)
    } else {
        Ok(compute_mock_scroll_adjusted_client_rect(
            runtime, source, scroll_x, scroll_y,
        ))
    }
}

pub(crate) fn observable_event_offset(
    runtime: &JsContextHost,
    source: DomHandle,
    point: LayoutPoint,
    reason: LayoutFlushReason,
) -> Result<LayoutPoint, LayoutError> {
    if !runtime.dom_host().is_connected(source) {
        return Ok(point);
    }
    let Some(document) = runtime.layout_document_for_source(source) else {
        return Ok(point);
    };
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![LayoutQuery::EventOffset { source, point }]),
    )?;
    match answers.answers.into_iter().next() {
        Some(LayoutQueryAnswer::EventOffset(offset)) => Ok(offset.unwrap_or(point)),
        _ => Err(provider_contract_error("event offset")),
    }
}

pub(crate) fn observable_element_metrics(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: LayoutFlushReason,
) -> Result<Option<LayoutElementMetrics<DomHandle>>, LayoutError> {
    if !runtime.dom_host().is_connected(source) {
        return Ok(None);
    }
    let Some(document) = runtime.layout_document_for_source(source) else {
        return Ok(None);
    };
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![LayoutQuery::ElementMetrics { source }]),
    )?;
    match answers.answers.into_iter().next() {
        Some(LayoutQueryAnswer::ElementMetrics(metrics)) => Ok(metrics),
        _ => Err(provider_contract_error("element metrics")),
    }
}

pub(crate) fn observable_scroll_into_view_geometry(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: LayoutFlushReason,
) -> Result<Option<LayoutScrollIntoViewGeometry<DomHandle>>, LayoutError> {
    if !runtime.dom_host().is_connected(source) {
        return Ok(None);
    }
    let Some(document) = runtime.layout_document_for_source(source) else {
        return Ok(None);
    };
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![LayoutQuery::ScrollIntoViewGeometry { source }]),
    )?;
    match answers.answers.into_iter().next() {
        Some(LayoutQueryAnswer::ScrollIntoViewGeometry(geometry)) => Ok(geometry),
        _ => Err(provider_contract_error("scroll-into-view geometry")),
    }
}

fn observable_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    reason: LayoutFlushReason,
) -> Result<Option<LayoutHit<DomHandle>>, LayoutError> {
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![LayoutQuery::HitTest {
            point,
            ignore_pointer_events_none,
        }]),
    )?;
    match answers.answers.into_iter().next() {
        Some(LayoutQueryAnswer::HitTest(hit)) => Ok(hit),
        _ => Err(provider_contract_error("hit test")),
    }
}

fn observable_hit_test_in_viewport(
    runtime: &JsContextHost,
    document: DomHandle,
    viewport: LayoutViewport,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<LayoutHit<DomHandle>>, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return observable_hit_test(
            runtime,
            document,
            point,
            ignore_pointer_events_none,
            LayoutFlushReason::HitTest,
        );
    }
    let answers = runtime.answer_layout_at_viewport(
        document,
        LayoutFlushReason::HitTest,
        viewport,
        &LayoutQueryBatch::new(vec![LayoutQuery::HitTest {
            point,
            ignore_pointer_events_none,
        }]),
    )?;
    match answers.answers.into_iter().next() {
        Some(LayoutQueryAnswer::HitTest(hit)) => Ok(hit),
        _ => Err(provider_contract_error("viewport-scoped hit test")),
    }
}

pub(crate) fn observable_caret_position(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    reason: LayoutFlushReason,
) -> Result<Option<LayoutCaretPosition<DomHandle>>, LayoutError> {
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![
            LayoutQuery::DocumentMetrics,
            LayoutQuery::CaretPosition { point },
        ]),
    )?;
    let mut answers = answers.answers.into_iter();
    match (answers.next(), answers.next()) {
        (
            Some(LayoutQueryAnswer::DocumentMetrics(metrics)),
            Some(LayoutQueryAnswer::CaretPosition(position)),
        ) => {
            let inside_viewport = point.x >= 0.0
                && point.y >= 0.0
                && point.x < metrics.viewport.css_width as f32
                && point.y < metrics.viewport.css_height as f32;
            Ok(inside_viewport.then_some(position).flatten())
        }
        _ => Err(provider_contract_error("caret position")),
    }
}

pub(crate) fn observable_input_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<InputHit>, LayoutError> {
    input_hit_test(runtime, document, point, false)
}

pub(crate) fn observable_scrollbar_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<LayoutScrollbarHit<DomHandle>>, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return Ok(None);
    }
    scrollbar_hit_test_in_frame(runtime, FrameHitTest::root(runtime, document, point), 0)
}

pub(crate) fn observable_deep_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<DomHandle>, LayoutError> {
    Ok(input_hit_test(runtime, document, point, ignore_pointer_events_none)?.map(|hit| hit.handle))
}

fn input_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<InputHit>, LayoutError> {
    input_hit_test_in_frame(
        runtime,
        FrameHitTest::root(runtime, document, point),
        ignore_pointer_events_none,
        0,
    )
}

fn input_hit_test_in_frame(
    runtime: &JsContextHost,
    frame: FrameHitTest,
    ignore_pointer_events_none: bool,
    depth: usize,
) -> Result<Option<InputHit>, LayoutError> {
    let Some((layout_hit, target)) = live_hit_in_frame(runtime, frame, ignore_pointer_events_none)?
    else {
        return Ok(None);
    };
    let target_hit = InputHit {
        handle: target,
        root_to_frame: frame.root_to_frame,
    };
    if depth >= HIT_TEST_CHILD_FRAME_DEPTH_LIMIT {
        return Ok(Some(target_hit));
    }
    let Some(child) = frame.child(runtime, target, layout_hit) else {
        return Ok(Some(target_hit));
    };
    Ok(
        input_hit_test_in_frame(runtime, child, ignore_pointer_events_none, depth + 1)?
            .or(Some(target_hit)),
    )
}

fn scrollbar_hit_test_in_frame(
    runtime: &JsContextHost,
    frame: FrameHitTest,
    depth: usize,
) -> Result<Option<LayoutScrollbarHit<DomHandle>>, LayoutError> {
    // Publish a current tree through the same exact-viewport hit-test
    // boundary used for DOM input. Scrollbar geometry intentionally remains a
    // frozen-tree control query because it is user-agent chrome, not a DOM
    // target.
    let live_hit = live_hit_in_frame(runtime, frame, false)?;
    if let Some(mut hit) = runtime
        .with_latest_layout_tree_for_document(frame.document, |tree| {
            tree.scrollbar_hit_test(frame.point)
        })
        .flatten()
    {
        hit.viewport_to_local = hit.viewport_to_local.concatenate(frame.root_to_frame);
        return Ok(Some(hit));
    }
    if depth >= HIT_TEST_CHILD_FRAME_DEPTH_LIMIT {
        return Ok(None);
    }
    let Some((layout_hit, target)) = live_hit else {
        return Ok(None);
    };
    let Some(child) = frame.child(runtime, target, layout_hit) else {
        return Ok(None);
    };
    scrollbar_hit_test_in_frame(runtime, child, depth + 1)
}

fn live_hit_in_frame(
    runtime: &JsContextHost,
    frame: FrameHitTest,
    ignore_pointer_events_none: bool,
) -> Result<Option<(LayoutHit<DomHandle>, DomHandle)>, LayoutError> {
    let first_hit = observable_hit_test_in_viewport(
        runtime,
        frame.document,
        frame.viewport,
        frame.point,
        ignore_pointer_events_none,
    )?;
    let live_first_hit = first_hit.and_then(|hit| {
        let target = element_for_hit_source(runtime, hit.source)?;
        runtime
            .dom_host()
            .is_connected(target)
            .then_some((hit, target))
    });
    // A latest-layout snapshot may still contain a text fragment whose DOM
    // node was replaced after the snapshot was published. Walk the sampled
    // paint stack only on that stale-source path until it identifies a
    // currently connected element. This live DOM check neither refreshes nor
    // invalidates layout; the common case retains the single-hit fast path.
    if live_first_hit.is_some() || first_hit.is_none() {
        return Ok(live_first_hit);
    }
    let hits = observable_hit_test_all_in_viewport(
        runtime,
        frame.document,
        frame.viewport,
        frame.point,
        ignore_pointer_events_none,
    )?;
    Ok(hits.into_iter().find_map(|hit| {
        let target = element_for_hit_source(runtime, hit.source)?;
        runtime
            .dom_host()
            .is_connected(target)
            .then_some((hit, target))
    }))
}

fn element_for_hit_source(runtime: &JsContextHost, mut source: DomHandle) -> Option<DomHandle> {
    loop {
        let node = runtime.dom_host().node(source)?;
        if node.is_element() {
            return Some(source);
        }
        let parent = node.parent_node()?;
        if runtime.dom_host().is_shadow_root(parent) {
            return runtime.dom_host().shadow_root_host(parent);
        }
        source = parent;
    }
}

pub(crate) fn observable_hit_test_all(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    reason: LayoutFlushReason,
) -> Result<(LayoutDocumentMetrics, Vec<LayoutHit<DomHandle>>), LayoutError> {
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![
            LayoutQuery::DocumentMetrics,
            LayoutQuery::HitTestAll {
                point,
                ignore_pointer_events_none,
            },
        ]),
    )?;
    let mut answers = answers.answers.into_iter();
    match (answers.next(), answers.next()) {
        (
            Some(LayoutQueryAnswer::DocumentMetrics(metrics)),
            Some(LayoutQueryAnswer::HitTestAll(hits)),
        ) => Ok((metrics, hits)),
        _ => Err(provider_contract_error("complete hit test")),
    }
}

fn observable_hit_test_all_in_viewport(
    runtime: &JsContextHost,
    document: DomHandle,
    viewport: LayoutViewport,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Vec<LayoutHit<DomHandle>>, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return observable_hit_test_all(
            runtime,
            document,
            point,
            ignore_pointer_events_none,
            LayoutFlushReason::HitTest,
        )
        .map(|(_, hits)| hits);
    }
    let answers = runtime.answer_layout_at_viewport(
        document,
        LayoutFlushReason::HitTest,
        viewport,
        &LayoutQueryBatch::new(vec![LayoutQuery::HitTestAll {
            point,
            ignore_pointer_events_none,
        }]),
    )?;
    match answers.answers.into_iter().next() {
        Some(LayoutQueryAnswer::HitTestAll(hits)) => Ok(hits),
        _ => Err(provider_contract_error("viewport-scoped complete hit test")),
    }
}

fn css_viewport_dimension(value: f32) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(0.0, u32::MAX as f32) as u32
}

fn answer_mock_queries(
    runtime: &JsContextHost,
    document: DomHandle,
    reason: LayoutFlushReason,
    batch: &LayoutQueryBatch<DomHandle>,
) -> LayoutAnswers<DomHandle> {
    let viewport = runtime.layout_viewport_for_document(document);
    let answers = batch
        .queries
        .iter()
        .map(|query| match query {
            LayoutQuery::DocumentMetrics => {
                let viewport_scroll = runtime
                    .dom_host()
                    .dom()
                    .document_element_handle_for_document(document)
                    .and_then(|root| runtime.dom_host().node(root))
                    .and_then(Node::as_element)
                    .map(|element| {
                        LayoutPoint::new(element.scroll_left() as f32, element.scroll_top() as f32)
                    })
                    .unwrap_or(LayoutPoint::ZERO);
                LayoutQueryAnswer::DocumentMetrics(LayoutDocumentMetrics {
                    viewport,
                    viewport_scroll,
                    content_size: LayoutSize::new(
                        viewport.css_width as f32,
                        viewport.css_height as f32,
                    ),
                })
            }
            LayoutQuery::BoxModel { source } => {
                LayoutQueryAnswer::BoxModel(mock_box_model(runtime, *source))
            }
            LayoutQuery::ClientRects { source } => LayoutQueryAnswer::ClientRects(
                mock_layout_client_rect_for_node(runtime, *source)
                    .map(quad_from_client_rect)
                    .into_iter()
                    .collect(),
            ),
            LayoutQuery::ContentQuads { source } => LayoutQueryAnswer::ContentQuads(
                mock_layout_client_rect_for_node(runtime, *source)
                    .map(quad_from_client_rect)
                    .into_iter()
                    .collect(),
            ),
            LayoutQuery::TextRangeRects { source, .. } => LayoutQueryAnswer::TextRangeRects(
                mock_layout_client_rect_for_node(runtime, *source)
                    .map(quad_from_client_rect)
                    .into_iter()
                    .collect(),
            ),
            LayoutQuery::ElementMetrics { source } => {
                LayoutQueryAnswer::ElementMetrics(mock_element_metrics(runtime, *source))
            }
            LayoutQuery::ScrollIntoViewGeometry { source } => {
                LayoutQueryAnswer::ScrollIntoViewGeometry(mock_scroll_into_view_geometry(
                    runtime, document, *source,
                ))
            }
            LayoutQuery::IntersectionGeometry { target, root } => {
                LayoutQueryAnswer::IntersectionGeometry(mock_intersection_geometry(
                    runtime, document, *target, *root,
                ))
            }
            LayoutQuery::HitTest {
                point,
                ignore_pointer_events_none: _,
            } => LayoutQueryAnswer::HitTest(
                mock_hit_test_handle(runtime, document, f64::from(point.x), f64::from(point.y))
                    .map(|source| LayoutHit {
                        source,
                        fragment: None,
                        local_point: *point,
                        is_text: false,
                        local_content_box: None,
                        viewport_to_local: moli_layout::LayoutTransform2D::IDENTITY,
                        box_model: mock_box_model(runtime, source),
                    }),
            ),
            LayoutQuery::HitTestAll {
                point,
                ignore_pointer_events_none: _,
            } => LayoutQueryAnswer::HitTestAll(
                mock_hit_test_handle(runtime, document, f64::from(point.x), f64::from(point.y))
                    .map(|source| {
                        vec![LayoutHit {
                            source,
                            fragment: None,
                            local_point: *point,
                            is_text: false,
                            local_content_box: None,
                            viewport_to_local: moli_layout::LayoutTransform2D::IDENTITY,
                            box_model: mock_box_model(runtime, source),
                        }]
                    })
                    .unwrap_or_default(),
            ),
            LayoutQuery::CaretPosition { point } => LayoutQueryAnswer::CaretPosition(
                mock_hit_test_handle(runtime, document, f64::from(point.x), f64::from(point.y))
                    .map(|source| {
                        let model = mock_box_model(runtime, source);
                        LayoutCaretPosition {
                            source,
                            utf16_offset: None,
                            rect: model
                                .map(|model| model.border)
                                .unwrap_or_else(|| quad_from_client_rect(zero_client_rect())),
                            ancestor_boxes: model
                                .map(|model| vec![(source, model)])
                                .unwrap_or_default(),
                        }
                    }),
            ),
            LayoutQuery::EventOffset { source, point } => {
                let rect = compute_mock_client_rect(runtime, *source);
                LayoutQueryAnswer::EventOffset(Some(LayoutPoint::new(
                    point.x - rect.left as f32,
                    point.y - rect.top as f32,
                )))
            }
        })
        .collect();
    LayoutAnswers {
        answers,
        metrics: LayoutPassMetrics {
            reason,
            elapsed: Duration::ZERO,
            box_count: 0,
            fragment_count: 0,
            paint_operation_count: 0,
            fallback_count: 1,
        },
    }
}

fn mock_box_model(runtime: &JsContextHost, source: DomHandle) -> Option<LayoutBoxModel> {
    let rect = mock_layout_client_rect_for_node(runtime, source)?;
    let quad = quad_from_client_rect(rect);
    Some(LayoutBoxModel {
        content: quad,
        padding: quad,
        border: quad,
        margin: quad,
    })
}

fn mock_element_metrics(
    runtime: &JsContextHost,
    source: DomHandle,
) -> Option<LayoutElementMetrics<DomHandle>> {
    let rect = mock_layout_client_rect_for_node(runtime, source)?;
    let size = LayoutSize::new(rect.width as f32, rect.height as f32);
    let offset = runtime
        .dom_host()
        .node(source)
        .and_then(Node::as_element)
        .map(|element| LayoutPoint::new(element.scroll_left() as f32, element.scroll_top() as f32))
        .unwrap_or(LayoutPoint::ZERO);
    let quad = quad_from_client_rect(rect);
    Some(LayoutElementMetrics {
        offset_parent: compute_mock_offset_parent(runtime, source),
        offset_position: LayoutPoint::new(rect.left as f32, rect.top as f32),
        offset_size: size,
        content_size: size,
        client_size: size,
        client_border: LayoutPoint::ZERO,
        scroll_size: size,
        scroll_offset: offset,
        minimum_scroll_offset: LayoutPoint::ZERO,
        maximum_scroll_offset: LayoutPoint::ZERO,
        scrollport: quad,
        scrollable_overflow: quad,
        is_scroll_container: false,
        allows_user_scroll_x: false,
        allows_user_scroll_y: false,
        clips_overflow: false,
        visible: rect.width > 0.0 && rect.height > 0.0,
        pointer_events: true,
    })
}

fn mock_scroll_into_view_geometry(
    runtime: &JsContextHost,
    document: DomHandle,
    source: DomHandle,
) -> Option<LayoutScrollIntoViewGeometry<DomHandle>> {
    let target_rect = mock_layout_client_rect_for_node(runtime, source)?;
    let root = runtime
        .dom_host()
        .dom()
        .document_element_handle_for_document(document);
    let scroll_containers = root
        .filter(|root| *root != source)
        .and_then(|root| {
            mock_element_metrics(runtime, root).map(|metrics| LayoutScrollContainerMetrics {
                source: root,
                metrics,
            })
        })
        .into_iter()
        .collect();
    Some(LayoutScrollIntoViewGeometry {
        target_rects: vec![quad_from_client_rect(target_rect)],
        scroll_containers,
    })
}

fn mock_intersection_geometry(
    runtime: &JsContextHost,
    document: DomHandle,
    target: DomHandle,
    root: Option<DomHandle>,
) -> Option<LayoutIntersectionGeometry> {
    let target_rect = mock_layout_client_rect_for_node(runtime, target)?;
    let viewport = runtime.layout_viewport_for_document(document);
    let root_rect = root
        .and_then(|root| mock_layout_client_rect_for_node(runtime, root))
        .map(quad_from_client_rect)
        .unwrap_or_else(|| {
            moli_layout::LayoutTransform2D::IDENTITY.map_rect(moli_layout::LayoutRect::new(
                0.0,
                0.0,
                viewport.css_width as f32,
                viewport.css_height as f32,
            ))
        });
    let root_is_layout_ancestor = root.is_none_or(|root| {
        let mut current = Some(target);
        while let Some(candidate) = current {
            if candidate == root {
                return true;
            }
            current = runtime.dom_host().parent_node(candidate);
        }
        false
    });
    Some(LayoutIntersectionGeometry {
        target_rects: vec![quad_from_client_rect(target_rect)],
        root_rect,
        ancestor_clips: Vec::new(),
        target_has_layout: true,
        target_visible: target_rect.width > 0.0 && target_rect.height > 0.0,
        root_clips_overflow: false,
        root_is_layout_ancestor,
    })
}

fn quad_from_client_rect(rect: ClientRect) -> LayoutQuad {
    let left = rect.left as f32;
    let top = rect.top as f32;
    let right = rect.right as f32;
    let bottom = rect.bottom as f32;
    LayoutQuad {
        points: [
            LayoutPoint::new(left, top),
            LayoutPoint::new(right, top),
            LayoutPoint::new(right, bottom),
            LayoutPoint::new(left, bottom),
        ],
    }
}

fn client_rect_from_quad(quad: LayoutQuad) -> ClientRect {
    let rect = quad.bounding_rect();
    ClientRect {
        left: f64::from(rect.x),
        top: f64::from(rect.y),
        right: f64::from(rect.right()),
        bottom: f64::from(rect.bottom()),
        width: f64::from(rect.width),
        height: f64::from(rect.height),
    }
}

fn union_client_rect(left: ClientRect, right: ClientRect) -> ClientRect {
    let min_x = left.left.min(right.left);
    let min_y = left.top.min(right.top);
    let max_x = left.right.max(right.right);
    let max_y = left.bottom.max(right.bottom);
    ClientRect {
        left: min_x,
        top: min_y,
        right: max_x,
        bottom: max_y,
        width: (max_x - min_x).max(0.0),
        height: (max_y - min_y).max(0.0),
    }
}

fn provider_contract_error(answer: &str) -> LayoutError {
    LayoutError::source_contract(
        "renderer geometry provider",
        format!("returned a mismatched {answer} answer"),
    )
}
