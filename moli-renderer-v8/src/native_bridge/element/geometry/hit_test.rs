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
use crate::{document_runtime::DomHandle, dom::native::DomHost, native_bridge::JsContextHost};

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
                    return InputSurfaceHit {
                        input: None,
                        control: Some(control),
                    };
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
    let live_first_hit = first_hit
        .and_then(|hit| live_interaction_target(runtime, hit.source).map(|target| (hit, target)));
    if live_first_hit.is_some() || first_hit.is_none() {
        return live_first_hit;
    }
    tree.hit_test_all(point, ignore_pointer_events_none)
        .into_iter()
        .find_map(|hit| live_interaction_target(runtime, hit.source).map(|target| (hit, target)))
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
    let hits = observable_hit_test_all_in_viewport(
        runtime,
        frame.document,
        frame.viewport,
        frame.point,
        ignore_pointer_events_none,
    )?;
    Ok(hits
        .into_iter()
        .find_map(|hit| live_interaction_target(runtime, hit.source).map(|target| (hit, target))))
}

fn observable_hit_test_in_viewport(
    runtime: &JsContextHost,
    document: DomHandle,
    viewport: LayoutViewport,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<LayoutHit<DomHandle>>, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return observable_hit_test(
            runtime,
            document,
            point,
            ignore_pointer_events_none,
            LayoutFlushReason::HitTest,
        )
        .map(|(_, hit)| hit);
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

/// Resolve the foremost painted hit and the viewport sampled by the same
/// frozen layout pass. Single-point DOM APIs consume this query while
/// penetrating-list APIs use `observable_hit_test_all`.
pub(crate) fn observable_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
    reason: LayoutFlushReason,
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
        reason,
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

fn live_interaction_target(runtime: &JsContextHost, source: DomHandle) -> Option<DomHandle> {
    let target = element_for_hit_source(runtime, source)?;
    (runtime.dom_host().is_connected(target)
        && !element_is_inert_for_hit_testing(runtime.dom_host(), target))
    .then_some(target)
}

/// Returns whether an element is suppressed from hit testing by an explicit
/// `inert` attribute in its flat-tree ancestry.
///
/// A modal dialog escapes inertness inherited from ancestors outside the
/// dialog, while an `inert` attribute on the dialog itself still applies.
pub(crate) fn element_is_inert_for_hit_testing(host: &DomHost, handle: DomHandle) -> bool {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        if let Some(element) = host.node(candidate).and_then(|node| node.as_element()) {
            if element.namespace() == "http://www.w3.org/1999/xhtml"
                && element.has_attribute("inert")
            {
                return true;
            }
            if element.is_html_element("dialog")
                && element.dialog_modal()
                && element.has_attribute("open")
                && host.is_connected(candidate)
            {
                return false;
            }
        }
        current = flat_tree_parent(host, candidate);
    }
    false
}

fn flat_tree_parent(host: &DomHost, handle: DomHandle) -> Option<DomHandle> {
    if let Some(slot) = host.assigned_slot_for_node(handle) {
        return Some(slot);
    }
    let parent = host.parent_node(handle)?;
    if host.is_shadow_root(parent) {
        return host.shadow_root_host(parent);
    }
    Some(parent)
}

fn css_viewport_dimension(value: f32) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(0.0, u32::MAX as f32) as u32
}

#[cfg(test)]
mod tests {
    use super::{DomHost, element_is_inert_for_hit_testing};
    use crate::dom::native::NativeDom;
    use url::Url;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            Url::parse("https://inert-hit-testing.test/").expect("test URL"),
        ))
    }

    #[test]
    fn inert_hit_testing_follows_html_flat_tree_ancestry_and_modal_escape() {
        let mut host = test_host();
        let body = host.create_element("body");
        assert!(host.append_child(host.document_handle(), body));

        let shadow_host = host.create_element("div");
        assert!(host.append_child(body, shadow_host));
        let shadow_root = host
            .attach_shadow_root(shadow_host, "open")
            .expect("shadow root");
        let slot = host.create_element("slot");
        let slotted = host.create_element("button");
        let shadow_child = host.create_element("button");
        assert!(host.append_child(shadow_root, slot));
        assert!(host.append_child(shadow_root, shadow_child));
        assert!(host.append_child(shadow_host, slotted));
        assert_eq!(host.assigned_slot_for_node(slotted), Some(slot));

        assert!(host.set_attribute(slot, "inert", ""));
        assert!(element_is_inert_for_hit_testing(&host, slotted));
        assert!(host.remove_attribute(slot, "inert"));
        assert!(!element_is_inert_for_hit_testing(&host, slotted));

        assert!(host.set_attribute(shadow_host, "inert", ""));
        assert!(element_is_inert_for_hit_testing(&host, shadow_child));

        let svg = host
            .create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
            .expect("SVG element");
        assert!(host.set_attribute(svg, "inert", ""));
        assert!(host.append_child(body, svg));
        assert!(!element_is_inert_for_hit_testing(&host, svg));
        let svg_wrapper = host.create_element("div");
        assert!(host.set_attribute(svg_wrapper, "inert", ""));
        assert!(host.append_child(body, svg_wrapper));
        assert!(host.remove_child(body, svg));
        assert!(host.append_child(svg_wrapper, svg));
        assert!(element_is_inert_for_hit_testing(&host, svg));

        let inert_ancestor = host.create_element("div");
        let dialog = host.create_element("dialog");
        let dialog_child = host.create_element("button");
        assert!(host.set_attribute(inert_ancestor, "inert", ""));
        assert!(host.append_child(body, inert_ancestor));
        assert!(host.append_child(inert_ancestor, dialog));
        assert!(host.append_child(dialog, dialog_child));
        assert!(element_is_inert_for_hit_testing(&host, dialog_child));

        assert!(host.set_attribute(dialog, "open", ""));
        assert!(host.set_dialog_modal(dialog, true));
        assert!(!element_is_inert_for_hit_testing(&host, dialog_child));
        assert!(host.set_attribute(dialog, "inert", ""));
        assert!(element_is_inert_for_hit_testing(&host, dialog_child));
    }
}
