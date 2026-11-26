use std::collections::HashSet;

use moli_layout::{
    LayoutAnswers, LayoutBoxModel, LayoutCaretPosition, LayoutDocumentMetrics,
    LayoutElementMetrics, LayoutError, LayoutFlushReason, LayoutHit, LayoutPoint, LayoutQuery,
    LayoutQueryAnswer, LayoutQueryBatch, LayoutResolvedGridTracks, LayoutScrollIntoViewGeometry,
};

use super::client_rect::{ClientRect, union_client_rect, zero_client_rect};
use super::mock::{answer_queries as answer_mock_queries, answer_query as answer_mock_query};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

fn query_source(
    runtime: &PageScriptHost,
    source: DomHandle,
    query: LayoutQuery<DomHandle>,
) -> Result<Option<LayoutQueryAnswer<DomHandle>>, LayoutError> {
    if !runtime.dom_host().is_connected(source) {
        return Ok(None);
    }
    let Some(document) = runtime.layout_document_for_source(source) else {
        return Ok(None);
    };
    match observable_geometry_query(runtime, document, &query) {
        Ok(answer) => Ok(Some(answer)),
        Err(LayoutError::NoLayoutSnapshot) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn observable_geometry_query(
    runtime: &PageScriptHost,
    document: DomHandle,
    query: &LayoutQuery<DomHandle>,
) -> Result<LayoutQueryAnswer<DomHandle>, LayoutError> {
    ensure_geometry_layout(runtime, document, std::slice::from_ref(query))?;
    published_geometry_query(runtime, document, query)
}

/// Style sampling never initializes layout as a side effect.
fn published_geometry_query(
    runtime: &PageScriptHost,
    document: DomHandle,
    query: &LayoutQuery<DomHandle>,
) -> Result<LayoutQueryAnswer<DomHandle>, LayoutError> {
    if runtime.layout_policy().uses_real_layout() {
        runtime.answer_layout_query_for_document(document, query)
    } else {
        Ok(answer_mock_query(runtime, document, query))
    }
}

pub(crate) fn observable_geometry_batch(
    runtime: &PageScriptHost,
    document: DomHandle,
    batch: &LayoutQueryBatch<DomHandle>,
) -> Result<LayoutAnswers<DomHandle>, LayoutError> {
    ensure_geometry_layout(runtime, document, &batch.queries)?;
    published_geometry_batch(runtime, document, batch)
}

fn ensure_geometry_layout(
    runtime: &PageScriptHost,
    document: DomHandle,
    queries: &[LayoutQuery<DomHandle>],
) -> Result<(), LayoutError> {
    runtime.ensure_observable_layout()?;
    if runtime.layout_policy().is_realtime() {
        runtime
            .prepare_svg_query_geometry(document, queries.iter().filter_map(svg_query_source))?;
    }
    Ok(())
}

fn svg_query_source(query: &LayoutQuery<DomHandle>) -> Option<DomHandle> {
    match *query {
        LayoutQuery::ClientRects { source }
        | LayoutQuery::ContentQuads { source }
        | LayoutQuery::ScrollIntoViewGeometry { source } => Some(source),
        LayoutQuery::IntersectionGeometry { target, .. } => Some(target),
        _ => None,
    }
}

/// Style and observer sampling never initializes layout as a side effect.
pub(crate) fn published_geometry_batch(
    runtime: &PageScriptHost,
    document: DomHandle,
    batch: &LayoutQueryBatch<DomHandle>,
) -> Result<LayoutAnswers<DomHandle>, LayoutError> {
    if runtime.layout_policy().uses_real_layout() {
        runtime.answer_layout_for_document(document, batch)
    } else {
        Ok(answer_mock_queries(
            runtime,
            document,
            LayoutFlushReason::SynchronousGeometry,
            batch,
        ))
    }
}

pub(crate) fn observable_document_metrics(
    runtime: &PageScriptHost,
    document: DomHandle,
) -> Result<LayoutDocumentMetrics, LayoutError> {
    match observable_geometry_query(runtime, document, &LayoutQuery::DocumentMetrics)? {
        LayoutQueryAnswer::DocumentMetrics(metrics) => Ok(metrics),
        _ => Err(provider_contract_error("document metrics")),
    }
}

pub(crate) fn read_client_rects(
    runtime: &PageScriptHost,
    source: DomHandle,
) -> Result<Vec<ClientRect>, LayoutError> {
    match query_source(runtime, source, LayoutQuery::ClientRects { source })? {
        Some(LayoutQueryAnswer::ClientRects(rects)) => {
            Ok(rects.into_iter().map(ClientRect::from).collect())
        }
        None => Ok(Vec::new()),
        _ => Err(provider_contract_error("client rects")),
    }
}

pub(crate) fn observable_sources_with_fragments(
    runtime: &PageScriptHost,
    document: DomHandle,
    sources: &[DomHandle],
) -> Result<HashSet<DomHandle>, LayoutError> {
    if sources.is_empty() {
        return Ok(HashSet::new());
    }
    let queries = sources
        .iter()
        .copied()
        .map(|source| LayoutQuery::ContentQuads { source })
        .collect();
    let answers = observable_geometry_batch(runtime, document, &LayoutQueryBatch::new(queries))?;
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
    runtime: &PageScriptHost,
    source: DomHandle,
) -> Result<ClientRect, LayoutError> {
    let mut rects = read_client_rects(runtime, source)?.into_iter();
    let Some(mut bounds) = rects.next() else {
        return Ok(zero_client_rect());
    };
    for rect in rects {
        bounds = union_client_rect(bounds, rect);
    }
    Ok(bounds)
}

pub(crate) fn observable_bounding_client_rects(
    runtime: &PageScriptHost,
    sources: &[DomHandle],
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
        match observable_geometry_batch(runtime, document, &LayoutQueryBatch::new(queries)) {
            Ok(answers) => answers,
            Err(LayoutError::NoLayoutSnapshot) => {
                return Ok(vec![zero_client_rect(); sources.len()]);
            }
            Err(error) => return Err(error),
        };
    if answers.answers.len() != sources.len() {
        return Err(provider_contract_error("bounding client rects"));
    }
    answers
        .answers
        .into_iter()
        .map(|answer| match answer {
            LayoutQueryAnswer::ClientRects(rects) => Ok(rects
                .into_iter()
                .map(ClientRect::from)
                .reduce(union_client_rect)
                .unwrap_or_else(zero_client_rect)),
            _ => Err(provider_contract_error("bounding client rects")),
        })
        .collect()
}

pub(crate) fn read_box_model(
    runtime: &PageScriptHost,
    source: DomHandle,
) -> Result<Option<LayoutBoxModel>, LayoutError> {
    match query_source(runtime, source, LayoutQuery::BoxModel { source })? {
        Some(LayoutQueryAnswer::BoxModel(model)) => Ok(model),
        None => Ok(None),
        _ => Err(provider_contract_error("box model")),
    }
}

pub(crate) fn observable_event_offset(
    runtime: &PageScriptHost,
    source: DomHandle,
    point: LayoutPoint,
) -> Result<LayoutPoint, LayoutError> {
    match query_source(runtime, source, LayoutQuery::EventOffset { source, point })? {
        Some(LayoutQueryAnswer::EventOffset(offset)) => Ok(offset.unwrap_or(point)),
        None => Ok(point),
        _ => Err(provider_contract_error("event offset")),
    }
}

pub(crate) fn read_element_metrics(
    runtime: &PageScriptHost,
    source: DomHandle,
) -> Result<Option<LayoutElementMetrics<DomHandle>>, LayoutError> {
    match query_source(runtime, source, LayoutQuery::ElementMetrics { source })? {
        Some(LayoutQueryAnswer::ElementMetrics(metrics)) => Ok(metrics),
        None => Ok(None),
        _ => Err(provider_contract_error("element metrics")),
    }
}

pub(crate) fn observable_used_grid_tracks(
    runtime: &PageScriptHost,
    source: DomHandle,
) -> Result<Option<LayoutResolvedGridTracks>, LayoutError> {
    if !runtime.dom_host().is_connected(source) {
        return Ok(None);
    }
    let Some(document) = runtime.layout_document_for_source(source) else {
        return Ok(None);
    };
    let answer = match published_geometry_query(
        runtime,
        document,
        &LayoutQuery::UsedGridTracks { source },
    ) {
        Ok(answer) => answer,
        Err(LayoutError::NoLayoutSnapshot) => return Ok(None),
        Err(error) => return Err(error),
    };
    match answer {
        LayoutQueryAnswer::UsedGridTracks(tracks) => Ok(tracks),
        _ => Err(provider_contract_error("used Grid tracks")),
    }
}

pub(crate) fn read_scroll_into_view_geometry(
    runtime: &PageScriptHost,
    source: DomHandle,
) -> Result<Option<LayoutScrollIntoViewGeometry<DomHandle>>, LayoutError> {
    match query_source(
        runtime,
        source,
        LayoutQuery::ScrollIntoViewGeometry { source },
    )? {
        Some(LayoutQueryAnswer::ScrollIntoViewGeometry(geometry)) => Ok(geometry),
        None => Ok(None),
        _ => Err(provider_contract_error("scroll-into-view geometry")),
    }
}

pub(crate) fn observable_caret_position(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<LayoutCaretPosition<DomHandle>>, LayoutError> {
    let answers = observable_geometry_batch(
        runtime,
        document,
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
            let inside_viewport = metrics.viewport.contains(point);
            Ok(inside_viewport.then_some(position).flatten())
        }
        _ => Err(provider_contract_error("caret position")),
    }
}

pub(crate) fn observable_hit_test_all(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<(LayoutDocumentMetrics, Vec<LayoutHit<DomHandle>>), LayoutError> {
    let answers = observable_geometry_batch(
        runtime,
        document,
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

/// Resolve the foremost painted hit and the viewport sampled by the same
/// frozen layout pass. Single-point DOM APIs consume this query while
/// penetrating-list APIs use `observable_hit_test_all`.
pub(crate) fn observable_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<
    (
        moli_layout::LayoutDocumentMetrics,
        Option<LayoutHit<DomHandle>>,
    ),
    LayoutError,
> {
    let answers = observable_geometry_batch(
        runtime,
        document,
        &LayoutQueryBatch::new(vec![
            LayoutQuery::DocumentMetrics,
            LayoutQuery::HitTest {
                point,
                ignore_pointer_events_none,
            },
        ]),
    )?;
    let mut answers = answers.answers.into_iter();
    match (answers.next(), answers.next()) {
        (
            Some(LayoutQueryAnswer::DocumentMetrics(metrics)),
            Some(LayoutQueryAnswer::HitTest(hit)),
        ) => Ok((metrics, hit)),
        _ => Err(provider_contract_error("hit test")),
    }
}

pub(in crate::native_bridge) fn element_for_hit_source(
    runtime: &PageScriptHost,
    mut source: DomHandle,
) -> Option<DomHandle> {
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

pub(super) fn is_live_hit_source(
    runtime: &PageScriptHost,
    source: DomHandle,
    ignore_inert: bool,
) -> bool {
    let dom = runtime.dom_host();
    dom.is_connected(source) && (ignore_inert || !dom.is_inert_in_document(source))
}

pub(in crate::native_bridge) fn live_interaction_target(
    runtime: &PageScriptHost,
    source: DomHandle,
    ignore_inert: bool,
) -> Option<DomHandle> {
    // Slotted text can be inside a modal dialog while its DOM parent is inert.
    if !is_live_hit_source(runtime, source, ignore_inert) {
        return None;
    }
    element_for_hit_source(runtime, source)
}
