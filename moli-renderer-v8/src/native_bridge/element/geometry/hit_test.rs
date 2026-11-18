use moli_layout::{
    FrozenLayoutTree, LayoutControlSurfaceHit, LayoutError, LayoutHit, LayoutPaintedSurfaceHit,
    LayoutPoint, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch, LayoutTransform2D,
};

#[cfg(test)]
use moli_layout::LayoutScrollbarHit;

use super::provider::{
    observable_geometry_batch, observable_hit_test_all, provider_contract_error,
};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

const CHILD_FRAME_DEPTH_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InputHit {
    pub(crate) handle: DomHandle,
    /// Converts a root-frame input position into the viewport of `handle`'s
    /// owner frame. Keeping the affine map lets boundary and capture events
    /// convert a new root position into a previously targeted child frame.
    pub(crate) root_to_frame: LayoutTransform2D,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InputSurfaceHit {
    pub(crate) input: Option<InputHit>,
    pub(crate) control: Option<LayoutControlSurfaceHit<DomHandle>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputHitTestMode {
    Dom,
    Mouse,
    Wheel,
}

#[derive(Clone, Copy)]
struct FrameHitTest {
    document: DomHandle,
    point: LayoutPoint,
    root_to_frame: LayoutTransform2D,
}

impl FrameHitTest {
    fn root(document: DomHandle, point: LayoutPoint) -> Self {
        Self {
            document,
            point,
            root_to_frame: LayoutTransform2D::IDENTITY,
        }
    }

    /// Blink performs the equivalent conversion through LocalFrameView when
    /// an input hit crosses an embedded-content boundary.
    fn child(
        self,
        runtime: &PageScriptHost,
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
            point: frame_to_child.map_point(self.point),
            root_to_frame: frame_to_child.concatenate(self.root_to_frame),
        })
    }
}

#[cfg(feature = "devtools")]
pub(crate) fn observable_input_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<InputHit>, LayoutError> {
    input_surface_hit_test(runtime, document, point, false, InputHitTestMode::Dom)
        .map(|hit| hit.input)
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

/// Coordinate input initializes a cold page once, then consumes the same
/// published tree, including embedded frames, until an explicit refresh.
pub(crate) fn input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    mode: InputHitTestMode,
) -> Result<InputSurfaceHit, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return input_hit_test_via_documents(runtime, document, point, ignore_pointer_events_none)
            .map(|input| InputSurfaceHit {
                input,
                control: None,
            });
    }
    runtime.ensure_initial_layout()?;
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
    .input
    .map(|hit| hit.handle))
}

fn input_hit_test_via_documents(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<InputHit>, LayoutError> {
    input_hit_test_in_frame(
        runtime,
        FrameHitTest::root(document, point),
        ignore_pointer_events_none,
        0,
    )
}

fn input_hit_test_in_frame(
    runtime: &PageScriptHost,
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
    if depth >= CHILD_FRAME_DEPTH_LIMIT {
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
                        input: None,
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
        // Wheel scrolling still belongs to this viewport when its DOM is inert.
        return Ok(InputSurfaceHit {
            input: (mode == InputHitTestMode::Wheel).then_some(InputHit {
                handle: tree.source_root(),
                root_to_frame,
            }),
            control: None,
        });
    };
    let target_hit = InputHit {
        handle: target,
        root_to_frame,
    };
    if depth >= CHILD_FRAME_DEPTH_LIMIT {
        return Ok(InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        });
    }
    let Some(child_document) = runtime.child_browsing_context_document_handle(target) else {
        return Ok(InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        });
    };
    let Some(content_box) = layout_hit.local_content_box else {
        return Ok(InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        });
    };
    if !content_box.contains(layout_hit.local_point) {
        return Ok(InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        });
    }
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
    let frame_to_child = LayoutTransform2D::translation(-content_box.x, -content_box.y)
        .concatenate(layout_hit.viewport_to_local);
    let child_hit = input_surface_hit_test_in_tree(
        runtime,
        child_tree,
        frame_to_child.map_point(point),
        frame_to_child.concatenate(root_to_frame),
        ignore_pointer_events_none,
        mode,
        depth + 1,
    )?;
    if child_hit.input.is_some() || child_hit.control.is_some() {
        Ok(child_hit)
    } else {
        Ok(InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        })
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

fn live_hit_in_frame(
    runtime: &PageScriptHost,
    frame: FrameHitTest,
    ignore_pointer_events_none: bool,
) -> Result<Option<(LayoutHit<DomHandle>, DomHandle)>, LayoutError> {
    let (_, first_hit) = observable_hit_test(
        runtime,
        frame.document,
        frame.point,
        ignore_pointer_events_none,
    )?;
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
    let (_, hits) = observable_hit_test_all(
        runtime,
        frame.document,
        frame.point,
        ignore_pointer_events_none,
    )?;
    Ok(hits.into_iter().find_map(resolve_hit))
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
    ignore_pointer_events_none: bool,
) -> Option<DomHandle> {
    // Slotted text can be inside a modal dialog while its DOM parent is inert.
    if !is_live_hit_source(runtime, source, ignore_pointer_events_none) {
        return None;
    }
    element_for_hit_source(runtime, source)
}
