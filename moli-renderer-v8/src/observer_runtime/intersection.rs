//! Canonical IntersectionObserver state transitions and owned check batches.

use std::{collections::HashMap, mem};

use crate::{
    dom::native::{DomHost, NativeNodeId},
    native_bridge::{JsContextHost, element::GeometryRead},
};

use super::{
    IntersectionEntryMetrics, IntersectionObserverOptions, LastReportedIntersection, ObserverStore,
    RootlessIntersectionGeometryCache, mock_intersection_entry_metrics, queue_intersection_entry,
    target_is_intersection_observable, threshold_adjusted_is_intersecting, thresholds_crossed,
};

struct IntersectionObserverCheckInput {
    observer_id: u32,
    options: IntersectionObserverOptions,
    targets: Vec<IntersectionTargetCheckInput>,
}

struct IntersectionTargetCheckInput {
    target: NativeNodeId,
    previous: Option<LastReportedIntersection>,
}

pub(super) struct IntersectionCheckBatch {
    observers: Vec<IntersectionObserverCheckInput>,
    geometry_cache: RootlessIntersectionGeometryCache,
}

struct IntersectionCheckResult {
    observer_id: u32,
    target: NativeNodeId,
    metrics: IntersectionEntryMetrics,
    should_queue: bool,
}

pub(super) struct CompletedIntersectionCheckBatch {
    results: Vec<IntersectionCheckResult>,
    geometry_cache: RootlessIntersectionGeometryCache,
}

impl ObserverStore {
    pub(super) fn take_intersection_check_batch(&mut self) -> Option<IntersectionCheckBatch> {
        if self.intersection_observers.is_empty() {
            return None;
        }
        let mut observers = Vec::with_capacity(self.intersection_observers.len());
        for (observer_id, state) in &self.intersection_observers {
            observers.push(IntersectionObserverCheckInput {
                observer_id: *observer_id,
                options: state.options.clone(),
                targets: state
                    .observed_targets
                    .iter()
                    .copied()
                    .map(|target| IntersectionTargetCheckInput {
                        target,
                        previous: state.last_reported_entries.get(&target).copied(),
                    })
                    .collect(),
            });
        }
        Some(IntersectionCheckBatch {
            observers,
            geometry_cache: mem::take(&mut self.rootless_geometry_cache),
        })
    }

    pub(super) fn apply_intersection_check_batch(
        &mut self,
        completed: CompletedIntersectionCheckBatch,
    ) -> bool {
        self.rootless_geometry_cache = completed.geometry_cache;
        let mut queued_any = false;
        for result in completed.results {
            let Some(state) = self.intersection_observers.get_mut(&result.observer_id) else {
                continue;
            };
            if !result.should_queue || !state.observed_targets.contains(&result.target) {
                continue;
            }
            queue_intersection_entry(state, result.target, result.metrics);
            queued_any = true;
        }
        queued_any
    }
}

pub(super) fn compute_intersection_check_batch(
    runtime: &JsContextHost,
    dom_host: &DomHost,
    batch: IntersectionCheckBatch,
    read: GeometryRead,
) -> Result<CompletedIntersectionCheckBatch, moli_layout::LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return Ok(compute_mock_intersection_check_batch(
            runtime, dom_host, batch,
        ));
    }

    struct CheckWork {
        observer_id: u32,
        target: NativeNodeId,
        previous: Option<LastReportedIntersection>,
        options: IntersectionObserverOptions,
        geometry: Option<moli_layout::LayoutIntersectionGeometry>,
    }

    let mut work = Vec::new();
    for observer in batch.observers {
        for target in observer.targets {
            work.push(CheckWork {
                observer_id: observer.observer_id,
                target: target.target,
                previous: target.previous,
                options: observer.options.clone(),
                geometry: None,
            });
        }
    }
    let mut by_document = HashMap::<NativeNodeId, Vec<usize>>::new();
    for (index, check) in work.iter().enumerate() {
        if let Some(document) = runtime.dom_host().owner_document_handle(check.target) {
            by_document.entry(document).or_default().push(index);
        }
    }
    for (document, indices) in by_document {
        let queries = indices
            .iter()
            .map(|index| {
                let check = &work[*index];
                moli_layout::LayoutQuery::IntersectionGeometry {
                    target: check.target,
                    root: check.options.root,
                }
            })
            .collect();
        let queries = moli_layout::LayoutQueryBatch::new(queries);
        let answers = match read {
            GeometryRead::Demand(reason) => {
                crate::native_bridge::element::observable_geometry_batch(
                    runtime, document, reason, &queries,
                )?
                .answers
            }
            // Publication checks consume the completed projection, including
            // absent/hidden frames; they must never start another layout pass.
            GeometryRead::Snapshot => runtime
                .with_latest_layout_tree_for_document(document, |tree| {
                    queries
                        .queries
                        .iter()
                        .map(|query| tree.answer_query(query))
                        .collect()
                })
                .unwrap_or_default(),
        };
        for (index, answer) in indices.into_iter().zip(answers) {
            let moli_layout::LayoutQueryAnswer::IntersectionGeometry(geometry) = answer else {
                return Err(moli_layout::LayoutError::source_contract(
                    "IntersectionObserver geometry",
                    "provider returned a mismatched intersection answer",
                ));
            };
            work[index].geometry = geometry;
        }
    }

    let mut results = Vec::with_capacity(work.len());
    for check in work {
        let metrics = real_intersection_entry_metrics(
            dom_host,
            check.target,
            &check.options,
            check.geometry.as_ref(),
        );
        let should_queue = match check.previous {
            Some(previous) => thresholds_crossed(
                previous,
                LastReportedIntersection {
                    is_intersecting: metrics.is_intersecting,
                    ratio: metrics.ratio,
                },
                &check.options.thresholds,
            ),
            None => target_is_intersection_observable(dom_host, check.target, &check.options),
        };
        results.push(IntersectionCheckResult {
            observer_id: check.observer_id,
            target: check.target,
            metrics,
            should_queue,
        });
    }
    Ok(CompletedIntersectionCheckBatch {
        results,
        geometry_cache: batch.geometry_cache,
    })
}

fn compute_mock_intersection_check_batch(
    runtime: &JsContextHost,
    dom_host: &DomHost,
    mut batch: IntersectionCheckBatch,
) -> CompletedIntersectionCheckBatch {
    let mut results = Vec::new();
    for observer in batch.observers {
        for target in observer.targets {
            let metrics = mock_intersection_entry_metrics(
                runtime,
                dom_host,
                target.target,
                &observer.options,
                &mut batch.geometry_cache,
            );
            let should_queue = match target.previous {
                Some(previous) => thresholds_crossed(
                    previous,
                    LastReportedIntersection {
                        is_intersecting: metrics.is_intersecting,
                        ratio: metrics.ratio,
                    },
                    &observer.options.thresholds,
                ),
                None => {
                    target_is_intersection_observable(dom_host, target.target, &observer.options)
                }
            };
            results.push(IntersectionCheckResult {
                observer_id: observer.observer_id,
                target: target.target,
                metrics,
                should_queue,
            });
        }
    }
    CompletedIntersectionCheckBatch {
        results,
        geometry_cache: batch.geometry_cache,
    }
}

fn real_intersection_entry_metrics(
    dom_host: &DomHost,
    target: NativeNodeId,
    options: &IntersectionObserverOptions,
    geometry: Option<&moli_layout::LayoutIntersectionGeometry>,
) -> IntersectionEntryMetrics {
    let target_rect = geometry
        .and_then(|geometry| rect_from_quads(&geometry.target_rects))
        .unwrap_or_default();
    let mut root_rect = geometry
        .map(|geometry| rect_from_quad(geometry.root_rect))
        .unwrap_or_default();
    root_rect = root_rect.expand_by_margin(super::root_margin_components(
        &options.root_margin,
        root_rect.width,
    ));
    if geometry.is_some_and(|geometry| geometry.root_clips_overflow) {
        root_rect = root_rect.expand_by_margin(super::root_margin_components(
            &options.scroll_margin,
            root_rect.width,
        ));
    }
    let mut clip_bounds = root_rect;
    if let Some(geometry) = geometry {
        for clip in &geometry.ancestor_clips {
            let clip = rect_from_quad(*clip).expand_by_margin(super::root_margin_components(
                &options.scroll_margin,
                rect_from_quad(*clip).width,
            ));
            clip_bounds = clip_bounds.intersection_bounds(clip);
        }
    }
    let observable = target_is_intersection_observable(dom_host, target, options)
        && geometry
            .is_some_and(|geometry| geometry.target_has_layout && geometry.root_is_layout_ancestor);
    // Layout rectangles come from the latest completed snapshot, while tree
    // connectedness is cheap owner state and must be observed live. A removed
    // target may therefore still have sampled geometry, but it no longer has
    // an intersection with the current document.
    let intersection_rect = if observable {
        target_rect.intersection(clip_bounds)
    } else {
        Default::default()
    };
    let raw_is_intersecting = observable && target_rect.intersects_or_touches(clip_bounds);
    let ratio = if target_rect.area() > 0.0 {
        (intersection_rect.area() / target_rect.area()).clamp(0.0, 1.0)
    } else if raw_is_intersecting {
        1.0
    } else {
        0.0
    };
    IntersectionEntryMetrics {
        is_intersecting: threshold_adjusted_is_intersecting(
            raw_is_intersecting,
            ratio,
            &options.thresholds,
        ),
        ratio,
        target_rect,
        root_rect,
        intersection_rect,
    }
}

fn rect_from_quads(quads: &[moli_layout::LayoutQuad]) -> Option<super::IntersectionRectData> {
    quads
        .iter()
        .map(|quad| rect_from_quad(*quad))
        .reduce(|left, right| {
            let x = left.x.min(right.x);
            let y = left.y.min(right.y);
            let right_edge = (left.x + left.width).max(right.x + right.width);
            let bottom = (left.y + left.height).max(right.y + right.height);
            super::IntersectionRectData::new(x, y, right_edge - x, bottom - y)
        })
}

fn rect_from_quad(quad: moli_layout::LayoutQuad) -> super::IntersectionRectData {
    let rect = quad.bounding_rect();
    super::IntersectionRectData::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.width),
        f64::from(rect.height),
    )
}
