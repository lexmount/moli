use moli_layout::{
    FrozenLayoutTree, LayoutControlSurfaceHit, LayoutError, LayoutHit, LayoutPaintedSurfaceHit,
    LayoutPoint, LayoutTransform2D,
};

#[cfg(test)]
use moli_layout::LayoutScrollbarHit;

use super::provider::{
    is_live_hit_source, live_interaction_target, observable_hit_test, observable_hit_test_all,
};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

const CHILD_FRAME_DEPTH_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InputSurfaceHit {
    pub(crate) target: Option<DomHandle>,
    pub(crate) control: Option<LayoutControlSurfaceHit<DomHandle>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputHitTestMode {
    #[cfg(feature = "devtools")]
    Dom,
    Mouse,
    Wheel,
}

#[cfg(feature = "devtools")]
pub(crate) fn observable_input_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<DomHandle>, LayoutError> {
    Ok(input_surface_hit_test(runtime, document, point, false, InputHitTestMode::Dom)?.target)
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
    input_surface_hit_test(runtime, document, point, false, InputHitTestMode::Mouse).map(|hit| {
        match hit.control {
            Some(LayoutControlSurfaceHit::Scrollbar(scrollbar)) => Some(scrollbar),
            Some(LayoutControlSurfaceHit::ScrollbarCorner(_)) | None => None,
        }
    })
}

/// Coordinate input shares the public geometry policy and recursive snapshot.
/// Realtime refreshes stale inputs; OnDemand only initializes a cold page.
pub(crate) fn input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    mode: InputHitTestMode,
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
                mode,
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
    Ok(input_surface_hit_test(
        runtime,
        document,
        point,
        ignore_pointer_events_none,
        InputHitTestMode::Dom,
    )?
    .target)
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
    mode: InputHitTestMode,
    depth: usize,
) -> Result<InputSurfaceHit, LayoutError> {
    let live_dom_hit = if mode == InputHitTestMode::Mouse {
        let mut live_dom_hit = None;
        for surface in tree.painted_surface_hits(point, ignore_pointer_events_none) {
            match surface {
                LayoutPaintedSurfaceHit::Control(mut control) => {
                    let source = match &control {
                        LayoutControlSurfaceHit::Scrollbar(hit) => hit.source,
                        LayoutControlSurfaceHit::ScrollbarCorner(hit) => hit.source,
                    };
                    // Viewport scrollbars remain interactive when the document root is inert.
                    let is_viewport_control = source == tree.source_root();
                    if !is_live_hit_source(runtime, source, is_viewport_control) {
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
                    let Some(target) = live_interaction_target(
                        runtime,
                        layout_hit.source,
                        ignore_pointer_events_none,
                    ) else {
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
        // A mouse hit inside a frame still belongs to its viewport when all
        // of its painted DOM candidates are inert.
        let use_viewport_target = mode == InputHitTestMode::Wheel
            || (mode == InputHitTestMode::Mouse && tree.viewport.contains(point));
        return Ok(InputSurfaceHit {
            target: use_viewport_target.then_some(tree.source_root()),
            control: None,
        });
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
        mode,
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
    let resolve_hit = |hit: LayoutHit<DomHandle>| {
        live_interaction_target(runtime, hit.source, ignore_pointer_events_none)
            .map(|target| (hit, target))
    };
    let live_first_hit = first_hit.and_then(resolve_hit);
    if live_first_hit.is_some() || first_hit.is_none() {
        return live_first_hit;
    }
    tree.hit_test_all(point, ignore_pointer_events_none)
        .into_iter()
        .find_map(resolve_hit)
}

fn live_hit_in_document(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<(LayoutHit<DomHandle>, DomHandle)>, LayoutError> {
    let (_, first_hit) = observable_hit_test(runtime, document, point, ignore_pointer_events_none)?;
    let resolve_hit = |hit: LayoutHit<DomHandle>| {
        live_interaction_target(runtime, hit.source, ignore_pointer_events_none)
            .map(|target| (hit, target))
    };
    let live_first_hit = first_hit.and_then(resolve_hit);
    // A latest-layout snapshot may still contain a text fragment whose DOM
    // node was replaced after publication, and `inert` can change without
    // changing geometry. Walk the sampled paint stack only when the live DOM
    // rejects the foremost candidate. This check neither refreshes nor
    // invalidates layout; the common case retains the single-hit fast path.
    if live_first_hit.is_some() || first_hit.is_none() {
        return Ok(live_first_hit);
    }
    let (_, hits) = observable_hit_test_all(runtime, document, point, ignore_pointer_events_none)?;
    Ok(hits.into_iter().find_map(resolve_hit))
}
