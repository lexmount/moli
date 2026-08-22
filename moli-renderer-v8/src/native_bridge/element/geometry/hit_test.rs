use moli_layout::{
    FrozenLayoutTree, LayoutControlSurfaceHit, LayoutError, LayoutFlushReason, LayoutHit,
    LayoutPaintedSurfaceHit, LayoutPoint, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch,
    LayoutTransform2D, LayoutViewport,
};

#[cfg(test)]
use moli_layout::LayoutScrollbarHit;

use super::provider::{
    observable_geometry_batch, observable_hit_test_all, provider_contract_error,
};
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

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

pub(crate) fn observable_input_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<InputHit>, LayoutError> {
    observable_input_surface_hit_test(runtime, document, point, false, false).map(|hit| hit.input)
}

#[cfg(test)]
pub(crate) fn observable_scrollbar_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<LayoutScrollbarHit<DomHandle>>, LayoutError> {
    observable_input_surface_hit_test(runtime, document, point, false, true).map(|hit| {
        match hit.control {
            Some(LayoutControlSurfaceHit::Scrollbar(scrollbar)) => Some(scrollbar),
            Some(LayoutControlSurfaceHit::ScrollbarCorner(_)) | None => None,
        }
    })
}

pub(crate) fn observable_input_surface_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    include_scrollbars: bool,
) -> Result<InputSurfaceHit, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return input_hit_test_via_documents(runtime, document, point, ignore_pointer_events_none)
            .map(|input| InputSurfaceHit {
                input,
                control: None,
            });
    }

    let viewport = runtime.layout_viewport_for_document(document);
    runtime.ensure_layout_at_viewport(document, LayoutFlushReason::HitTest, viewport)?;
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
        .ok_or(LayoutError::NoLayoutRoot)
}

pub(crate) fn observable_deep_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<DomHandle>, LayoutError> {
    Ok(observable_input_surface_hit_test(
        runtime,
        document,
        point,
        ignore_pointer_events_none,
        false,
    )?
    .input
    .map(|hit| hit.handle))
}

fn input_hit_test_via_documents(
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
    runtime: &JsContextHost,
    tree: &FrozenLayoutTree<DomHandle>,
    point: LayoutPoint,
    root_to_frame: LayoutTransform2D,
    ignore_pointer_events_none: bool,
    include_scrollbars: bool,
    depth: usize,
) -> InputSurfaceHit {
    let live_dom_hit = if include_scrollbars {
        let mut live_dom_hit = None;
        for surface in tree.painted_surface_hits(point, ignore_pointer_events_none) {
            match surface {
                LayoutPaintedSurfaceHit::Control(mut control) => {
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
                    return InputSurfaceHit {
                        input: None,
                        control: Some(control),
                    };
                }
                LayoutPaintedSurfaceHit::Dom(layout_hit) => {
                    let Some(target) = element_for_hit_source(runtime, layout_hit.source) else {
                        continue;
                    };
                    if runtime.dom_host().is_connected(target) {
                        live_dom_hit = Some((layout_hit, target));
                        break;
                    }
                }
            }
        }
        live_dom_hit
    } else {
        live_hit_in_tree(runtime, tree, point, ignore_pointer_events_none)
    };
    let Some((layout_hit, target)) = live_dom_hit else {
        return InputSurfaceHit::default();
    };
    let target_hit = InputHit {
        handle: target,
        root_to_frame,
    };
    if depth >= CHILD_FRAME_DEPTH_LIMIT {
        return InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        };
    }
    let Some(child_tree) = tree.embedded_frame_tree(target) else {
        return InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        };
    };
    if runtime
        .child_browsing_context_document_handle(target)
        .is_none()
    {
        return InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        };
    }
    let Some(content_box) = layout_hit.local_content_box else {
        return InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        };
    };
    if !content_box.contains(layout_hit.local_point) {
        return InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        };
    }
    let frame_to_child = LayoutTransform2D::translation(-content_box.x, -content_box.y)
        .concatenate(layout_hit.viewport_to_local);
    let child_hit = input_surface_hit_test_in_tree(
        runtime,
        child_tree,
        frame_to_child.map_point(point),
        frame_to_child.concatenate(root_to_frame),
        ignore_pointer_events_none,
        include_scrollbars,
        depth + 1,
    );
    if child_hit.input.is_some() || child_hit.control.is_some() {
        child_hit
    } else {
        InputSurfaceHit {
            input: Some(target_hit),
            control: None,
        }
    }
}

fn live_hit_in_tree(
    runtime: &JsContextHost,
    tree: &FrozenLayoutTree<DomHandle>,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Option<(LayoutHit<DomHandle>, DomHandle)> {
    let first_hit = tree.hit_test(point, ignore_pointer_events_none);
    let live_first_hit = first_hit.and_then(|hit| {
        let target = element_for_hit_source(runtime, hit.source)?;
        runtime
            .dom_host()
            .is_connected(target)
            .then_some((hit, target))
    });
    if live_first_hit.is_some() || first_hit.is_none() {
        return live_first_hit;
    }
    tree.hit_test_all(point, ignore_pointer_events_none)
        .into_iter()
        .find_map(|hit| {
            let target = element_for_hit_source(runtime, hit.source)?;
            runtime
                .dom_host()
                .is_connected(target)
                .then_some((hit, target))
        })
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

fn observable_hit_test_in_viewport(
    runtime: &JsContextHost,
    document: DomHandle,
    viewport: LayoutViewport,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<LayoutHit<DomHandle>>, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return observable_hit_test(runtime, document, point, ignore_pointer_events_none);
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

fn observable_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<LayoutHit<DomHandle>>, LayoutError> {
    let answers = observable_geometry_batch(
        runtime,
        document,
        LayoutFlushReason::HitTest,
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

fn css_viewport_dimension(value: f32) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(0.0, u32::MAX as f32) as u32
}
