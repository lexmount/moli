use moli_layout::{
    LayoutError, LayoutFlushReason, LayoutHit, LayoutPoint, LayoutQuery, LayoutQueryAnswer,
    LayoutQueryBatch, LayoutScrollbarHit, LayoutTransform2D, LayoutViewport,
};

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
    if depth >= CHILD_FRAME_DEPTH_LIMIT {
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
