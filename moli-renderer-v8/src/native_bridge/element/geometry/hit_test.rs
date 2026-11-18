use moli_layout::{
    FrozenLayoutTree, LayoutControlSurfaceHit, LayoutError, LayoutHit, LayoutPaintedSurfaceHit,
    LayoutPoint, LayoutTransform2D,
};

#[cfg(test)]
use moli_layout::LayoutScrollbarHit;

use super::provider::{observable_hit_test, observable_hit_test_all};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

const CHILD_FRAME_DEPTH_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InputSurfaceHit {
    pub(crate) target: Option<DomHandle>,
    pub(crate) control: Option<LayoutControlSurfaceHit<DomHandle>>,
}

#[cfg(feature = "devtools")]
pub(crate) fn observable_input_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<DomHandle>, LayoutError> {
    Ok(input_surface_hit_test(runtime, document, point, false, false)?.target)
}

/// Hit tests within an already routed frame. The mouse press can keep that
/// frame even when the root hit test lands in a different Document.
#[cfg(feature = "devtools")]
pub(crate) fn input_target_in_document(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
) -> DomHandle {
    runtime
        .input_frame_transform(document, None)
        .and_then(|transform| {
            runtime.with_latest_layout_tree_for_document(document, |tree| {
                live_hit_in_tree(runtime, tree, transform.map_point(point), false)
                    .map(|(_, target)| target)
            })?
        })
        .or_else(|| {
            runtime
                .dom_host()
                .dom()
                .document_element_handle_for_document(document)
        })
        .unwrap_or(document)
}

#[cfg(test)]
pub(crate) fn observable_scrollbar_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<LayoutScrollbarHit<DomHandle>>, LayoutError> {
    input_surface_hit_test(runtime, document, point, false, true).map(|hit| match hit.control {
        Some(LayoutControlSurfaceHit::Scrollbar(scrollbar)) => Some(scrollbar),
        Some(LayoutControlSurfaceHit::ScrollbarCorner(_)) | None => None,
    })
}

/// Coordinate input shares the public geometry policy and recursive snapshot.
/// Realtime refreshes stale inputs; OnDemand only initializes a cold page.
pub(crate) fn input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    include_scrollbars: bool,
) -> Result<InputSurfaceHit, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        let target =
            input_hit_test_via_documents(runtime, document, point, ignore_pointer_events_none, 0)?;
        return Ok(InputSurfaceHit {
            target,
            control: None,
        });
    }
    runtime.ensure_observable_layout()?;
    runtime
        .with_latest_layout_tree_for_document(document, |tree| {
            input_surface_hit_test_in_tree(
                runtime,
                tree,
                point,
                LayoutTransform2D::IDENTITY,
                ignore_pointer_events_none,
                include_scrollbars,
                0,
            )
        })
        .ok_or(LayoutError::NoLayoutSnapshot)?
}

#[cfg(feature = "devtools")]
pub(crate) fn observable_deep_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<DomHandle>, LayoutError> {
    Ok(input_surface_hit_test(runtime, document, point, ignore_pointer_events_none, false)?.target)
}

fn input_hit_test_via_documents(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    depth: usize,
) -> Result<Option<DomHandle>, LayoutError> {
    let Some((layout_hit, target)) =
        live_hit_in_document(runtime, document, point, ignore_pointer_events_none)?
    else {
        return Ok(None);
    };
    if depth < CHILD_FRAME_DEPTH_LIMIT
        && let Some(child) = runtime.child_browsing_context_document_handle(target)
        && let Some(transform) = layout_hit.embedded_viewport_hit_transform()
    {
        let child_target = input_hit_test_via_documents(
            runtime,
            child,
            transform.map_point(point),
            ignore_pointer_events_none,
            depth + 1,
        )?;
        return Ok(child_target.or(Some(target)));
    }
    Ok(Some(target))
}

fn input_surface_hit_test_in_tree(
    runtime: &PageScriptHost,
    tree: &FrozenLayoutTree<DomHandle>,
    point: LayoutPoint,
    root_to_frame: LayoutTransform2D,
    ignore_pointer_events_none: bool,
    include_scrollbars: bool,
    depth: usize,
) -> Result<InputSurfaceHit, LayoutError> {
    let live_dom_hit = if include_scrollbars {
        let mut live_dom_hit = None;
        for surface in tree.painted_surface_hits(point, ignore_pointer_events_none) {
            match surface {
                LayoutPaintedSurfaceHit::Control(mut control) => {
                    let source = match &control {
                        LayoutControlSurfaceHit::Scrollbar(hit) => hit.source,
                        LayoutControlSurfaceHit::ScrollbarCorner(hit) => hit.source,
                    };
                    if live_interaction_target(runtime, source).is_none() {
                        continue;
                    }
                    match &mut control {
                        LayoutControlSurfaceHit::Scrollbar(hit) => {
                            hit.viewport_to_local =
                                hit.viewport_to_local.concatenate(root_to_frame);
                        }
                        LayoutControlSurfaceHit::ScrollbarCorner(hit) => {
                            hit.viewport_to_local =
                                hit.viewport_to_local.concatenate(root_to_frame);
                        }
                    }
                    return Ok(InputSurfaceHit {
                        target: None,
                        control: Some(control),
                    });
                }
                LayoutPaintedSurfaceHit::Dom(layout_hit) => {
                    let Some(target) = live_interaction_target(runtime, layout_hit.source) else {
                        continue;
                    };
                    live_dom_hit = Some((layout_hit, target));
                    break;
                }
            }
        }
        live_dom_hit
    } else {
        live_hit_in_tree(runtime, tree, point, ignore_pointer_events_none)
    };
    let Some((layout_hit, target)) = live_dom_hit else {
        return Ok(InputSurfaceHit::default());
    };
    let target_hit = InputSurfaceHit {
        target: Some(target),
        control: None,
    };
    if depth >= CHILD_FRAME_DEPTH_LIMIT {
        return Ok(target_hit);
    }
    let Some(child_document) = runtime.child_browsing_context_document_handle(target) else {
        return Ok(target_hit);
    };
    let Some(frame_to_child) = layout_hit.embedded_viewport_hit_transform() else {
        return Ok(target_hit);
    };
    let child_tree = tree
        .embedded_frame_tree(target)
        .ok_or(LayoutError::NoLayoutSnapshot)?;
    if runtime
        .dom_host()
        .dom()
        .document_element_handle_for_document(child_document)
        != Some(child_tree.source_root())
    {
        return Err(LayoutError::NoLayoutSnapshot);
    }
    let child_hit = input_surface_hit_test_in_tree(
        runtime,
        child_tree,
        frame_to_child.map_point(point),
        frame_to_child.concatenate(root_to_frame),
        ignore_pointer_events_none,
        include_scrollbars,
        depth + 1,
    )?;
    if child_hit.target.is_some() || child_hit.control.is_some() {
        Ok(child_hit)
    } else {
        Ok(target_hit)
    }
}

fn live_hit_in_tree(
    runtime: &PageScriptHost,
    tree: &FrozenLayoutTree<DomHandle>,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Option<(LayoutHit<DomHandle>, DomHandle)> {
    let first_hit = tree.hit_test(point, ignore_pointer_events_none);
    let live_first_hit = first_hit
        .and_then(|hit| live_interaction_target(runtime, hit.source).map(|target| (hit, target)));
    if live_first_hit.is_some() || first_hit.is_none() {
        return live_first_hit;
    }
    tree.hit_test_all(point, ignore_pointer_events_none)
        .into_iter()
        .find_map(|hit| live_interaction_target(runtime, hit.source).map(|target| (hit, target)))
}

fn live_hit_in_document(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<(LayoutHit<DomHandle>, DomHandle)>, LayoutError> {
    let (_, first_hit) = observable_hit_test(runtime, document, point, ignore_pointer_events_none)?;
    let live_first_hit = first_hit
        .and_then(|hit| live_interaction_target(runtime, hit.source).map(|target| (hit, target)));
    // A latest-layout snapshot may still contain a text fragment whose DOM
    // node was replaced after publication, and `inert` can change without
    // changing geometry. Walk the sampled paint stack only when the live DOM
    // rejects the foremost candidate. This check neither refreshes nor
    // invalidates layout; the common case retains the single-hit fast path.
    if live_first_hit.is_some() || first_hit.is_none() {
        return Ok(live_first_hit);
    }
    let (_, hits) = observable_hit_test_all(runtime, document, point, ignore_pointer_events_none)?;
    Ok(hits
        .into_iter()
        .find_map(|hit| live_interaction_target(runtime, hit.source).map(|target| (hit, target))))
}

fn element_for_hit_source(runtime: &PageScriptHost, mut source: DomHandle) -> Option<DomHandle> {
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

fn live_interaction_target(runtime: &PageScriptHost, source: DomHandle) -> Option<DomHandle> {
    let target = element_for_hit_source(runtime, source)?;
    (runtime.dom_host().is_connected(target) && !runtime.dom_host().is_inert_in_document(target))
        .then_some(target)
}
