use std::{collections::HashSet, time::Duration};

use moli_layout::{
    LayoutAnswers, LayoutBoxModel, LayoutCaretPosition, LayoutDocumentMetrics,
    LayoutElementMetrics, LayoutError, LayoutFlushReason, LayoutHit, LayoutIntersectionGeometry,
    LayoutPassMetrics, LayoutPoint, LayoutQuad, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch,
    LayoutScrollContainerMetrics, LayoutScrollIntoViewGeometry, LayoutSize,
};

use super::layout::{
    ClientRect, compute_mock_client_rect, compute_mock_offset_parent,
    compute_mock_scroll_adjusted_client_rect, mock_hit_test_handle,
    mock_layout_client_rect_for_node, zero_client_rect,
};
use crate::{document_runtime::DomHandle, dom::native::Node, native_bridge::JsContextHost};

const HIT_TEST_CHILD_FRAME_DEPTH_LIMIT: usize = 16;

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
) -> Result<Option<DomHandle>, LayoutError> {
    observable_deep_hit_test(runtime, document, point, false)
}

pub(crate) fn observable_deep_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<DomHandle>, LayoutError> {
    observable_deep_hit_test_inner(runtime, document, point, ignore_pointer_events_none, 0)
}

fn observable_deep_hit_test_inner(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    depth: usize,
) -> Result<Option<DomHandle>, LayoutError> {
    let first_hit = observable_hit_test(
        runtime,
        document,
        point,
        ignore_pointer_events_none,
        LayoutFlushReason::HitTest,
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
    let fallback_hit = if live_first_hit.is_none() && first_hit.is_some() {
        let (_, hits) = observable_hit_test_all(
            runtime,
            document,
            point,
            ignore_pointer_events_none,
            LayoutFlushReason::HitTest,
        )?;
        hits.into_iter().find_map(|hit| {
            let target = element_for_hit_source(runtime, hit.source)?;
            runtime
                .dom_host()
                .is_connected(target)
                .then_some((hit, target))
        })
    } else {
        None
    };
    let Some((hit, target)) = live_first_hit.or(fallback_hit) else {
        return Ok(None);
    };
    if depth >= HIT_TEST_CHILD_FRAME_DEPTH_LIMIT {
        return Ok(Some(target));
    }
    let Some(child_document) = runtime.child_browsing_context_document_handle(target) else {
        return Ok(Some(target));
    };
    let Some(content_quad) = hit.box_model.map(|model| model.content) else {
        return Ok(Some(target));
    };
    let child_viewport = runtime.layout_viewport_for_document(child_document);
    let Some(child_point) = map_point_into_quad(
        point,
        content_quad,
        LayoutSize::new(
            child_viewport.css_width as f32,
            child_viewport.css_height as f32,
        ),
    ) else {
        return Ok(Some(target));
    };
    Ok(observable_deep_hit_test_inner(
        runtime,
        child_document,
        child_point,
        ignore_pointer_events_none,
        depth + 1,
    )?
    .or(Some(target)))
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

fn map_point_into_quad(
    point: LayoutPoint,
    quad: LayoutQuad,
    target_size: LayoutSize,
) -> Option<LayoutPoint> {
    let [origin, x_corner, _, y_corner] = quad.points;
    let basis_x = (
        f64::from(x_corner.x - origin.x),
        f64::from(x_corner.y - origin.y),
    );
    let basis_y = (
        f64::from(y_corner.x - origin.x),
        f64::from(y_corner.y - origin.y),
    );
    let relative = (f64::from(point.x - origin.x), f64::from(point.y - origin.y));
    let determinant = basis_x.0 * basis_y.1 - basis_x.1 * basis_y.0;
    if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
        return None;
    }
    let u = (relative.0 * basis_y.1 - relative.1 * basis_y.0) / determinant;
    let v = (basis_x.0 * relative.1 - basis_x.1 * relative.0) / determinant;
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }
    Some(LayoutPoint::new(
        (u * f64::from(target_size.width)) as f32,
        (v * f64::from(target_size.height)) as f32,
    ))
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
