use std::collections::HashSet;

use moli_layout::{
    LayoutAnswers, LayoutBoxModel, LayoutCaretPosition, LayoutDocumentMetrics,
    LayoutElementMetrics, LayoutError, LayoutFlushReason, LayoutHit, LayoutPoint, LayoutQuery,
    LayoutQueryAnswer, LayoutQueryBatch, LayoutResolvedGridTracks, LayoutScrollIntoViewGeometry,
};

use super::client_rect::{ClientRect, client_rect_from_quad, union_client_rect, zero_client_rect};
use super::mock::{
    answer_queries as answer_mock_queries, compute_mock_scroll_adjusted_client_rect,
};
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

/// Native input reads the rendered snapshot; explicit geometry APIs may
/// prepare layout. Both paths share the query and result conversion below.
#[derive(Clone, Copy)]
pub(crate) enum GeometryRead {
    Demand(LayoutFlushReason),
    Snapshot,
}

impl From<LayoutFlushReason> for GeometryRead {
    fn from(reason: LayoutFlushReason) -> Self {
        Self::Demand(reason)
    }
}

impl GeometryRead {
    pub(crate) fn for_default_action(runtime: &JsContextHost) -> Self {
        if runtime
            .current_input_event()
            .is_some_and(|event| event.is_mouse())
        {
            Self::Snapshot
        } else {
            Self::Demand(LayoutFlushReason::SynchronousGeometry)
        }
    }
}

fn query_source(
    runtime: &JsContextHost,
    source: DomHandle,
    read: impl Into<GeometryRead>,
    query: LayoutQuery<DomHandle>,
) -> Result<Option<LayoutQueryAnswer<DomHandle>>, LayoutError> {
    if !runtime.dom_host().is_connected(source) {
        return Ok(None);
    }
    let Some(document) = runtime.layout_document_for_source(source) else {
        return Ok(None);
    };
    let reason = match read.into() {
        GeometryRead::Snapshot if runtime.layout_policy().uses_real_layout() => {
            return Ok(runtime
                .with_latest_layout_tree_for_document(document, |tree| tree.answer_query(&query)));
        }
        GeometryRead::Snapshot => LayoutFlushReason::SynchronousGeometry,
        GeometryRead::Demand(reason) => reason,
    };
    let answers = observable_geometry_batch(
        runtime,
        document,
        reason,
        &LayoutQueryBatch::new(vec![query]),
    )?;
    answers
        .answers
        .into_iter()
        .next()
        .map(Some)
        .ok_or_else(|| provider_contract_error("source geometry"))
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

pub(crate) fn read_client_rects(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: impl Into<GeometryRead>,
) -> Result<Vec<ClientRect>, LayoutError> {
    match query_source(runtime, source, reason, LayoutQuery::ClientRects { source })? {
        Some(LayoutQueryAnswer::ClientRects(rects)) => {
            Ok(rects.into_iter().map(client_rect_from_quad).collect())
        }
        None => Ok(Vec::new()),
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

pub(crate) fn read_bounding_client_rect(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: impl Into<GeometryRead>,
) -> Result<ClientRect, LayoutError> {
    let mut rects = read_client_rects(runtime, source, reason)?.into_iter();
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

pub(crate) fn read_box_model(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: impl Into<GeometryRead>,
) -> Result<Option<LayoutBoxModel>, LayoutError> {
    match query_source(runtime, source, reason, LayoutQuery::BoxModel { source })? {
        Some(LayoutQueryAnswer::BoxModel(model)) => Ok(model),
        None => Ok(None),
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
        read_bounding_client_rect(runtime, source, reason)
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
    match query_source(
        runtime,
        source,
        reason,
        LayoutQuery::EventOffset { source, point },
    )? {
        Some(LayoutQueryAnswer::EventOffset(offset)) => Ok(offset.unwrap_or(point)),
        None => Ok(point),
        _ => Err(provider_contract_error("event offset")),
    }
}

pub(crate) fn read_element_metrics(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: impl Into<GeometryRead>,
) -> Result<Option<LayoutElementMetrics<DomHandle>>, LayoutError> {
    match query_source(
        runtime,
        source,
        reason,
        LayoutQuery::ElementMetrics { source },
    )? {
        Some(LayoutQueryAnswer::ElementMetrics(metrics)) => Ok(metrics),
        None => Ok(None),
        _ => Err(provider_contract_error("element metrics")),
    }
}

pub(crate) fn observable_used_grid_tracks(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: LayoutFlushReason,
) -> Result<Option<LayoutResolvedGridTracks>, LayoutError> {
    match query_source(
        runtime,
        source,
        reason,
        LayoutQuery::UsedGridTracks { source },
    )? {
        Some(LayoutQueryAnswer::UsedGridTracks(tracks)) => Ok(tracks),
        None => Ok(None),
        _ => Err(provider_contract_error("used Grid tracks")),
    }
}

pub(crate) fn read_scroll_into_view_geometry(
    runtime: &JsContextHost,
    source: DomHandle,
    reason: impl Into<GeometryRead>,
) -> Result<Option<LayoutScrollIntoViewGeometry<DomHandle>>, LayoutError> {
    match query_source(
        runtime,
        source,
        reason,
        LayoutQuery::ScrollIntoViewGeometry { source },
    )? {
        Some(LayoutQueryAnswer::ScrollIntoViewGeometry(geometry)) => Ok(geometry),
        None => Ok(None),
        _ => Err(provider_contract_error("scroll-into-view geometry")),
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

pub(super) fn provider_contract_error(answer: &str) -> LayoutError {
    LayoutError::source_contract(
        "renderer geometry provider",
        format!("returned a mismatched {answer} answer"),
    )
}
