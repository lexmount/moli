use std::time::Duration;

use moli_layout::{
    LayoutAnswers, LayoutBoxModel, LayoutCaretPosition, LayoutDocumentMetrics,
    LayoutElementMetrics, LayoutFlushReason, LayoutHit, LayoutIntersectionGeometry,
    LayoutPassMetrics, LayoutPoint, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch,
    LayoutScrollContainerMetrics, LayoutScrollIntoViewGeometry, LayoutSize,
};

use super::super::client_rect::{quad_from_client_rect, zero_client_rect};
use super::layout::{
    compute_mock_client_rect, compute_mock_offset_parent, mock_hit_test_handle,
    mock_layout_client_rect_for_node,
};
use crate::{document_runtime::DomHandle, dom::native::Node, native_bridge::JsContextHost};

pub(crate) fn answer_queries(
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
                        paint_order: None,
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
                            paint_order: None,
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
