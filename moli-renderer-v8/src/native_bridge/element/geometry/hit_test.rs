use moli_layout::{
    FrozenLayoutTree, LayoutControlSurfaceHit, LayoutError, LayoutHit, LayoutPaintedSurfaceHit,
    LayoutPoint, LayoutTransform2D,
};

#[cfg(test)]
use moli_layout::LayoutScrollbarHit;

use super::mock::{MockHitTestOptions, mock_hit_test_handle};
use super::provider::{is_live_hit_source, live_interaction_target};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

const CHILD_FRAME_DEPTH_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InputSurfaceHit {
    pub(crate) target: Option<DomHandle>,
    pub(crate) control: Option<LayoutControlSurfaceHit<DomHandle>>,
    /// The painted source before mapping it to an element for DOM events.
    #[cfg(feature = "molio")]
    pub(crate) painted_source: Option<PaintedInputSource>,
}

#[cfg(feature = "molio")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaintedInputSource {
    pub(crate) node: DomHandle,
    pub(crate) pseudo: Option<moli_layout::LayoutPseudo>,
    pub(crate) is_text: bool,
}

#[cfg(feature = "molio")]
impl InputSurfaceHit {
    pub(crate) fn hover_source(&self) -> Option<DomHandle> {
        self.painted_source
            .map(|source| source.node)
            .or(self.target)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputHitTestMode {
    #[cfg(feature = "devtools")]
    Dom,
    Mouse,
    Wheel,
}

#[derive(Clone, Copy)]
pub(crate) struct InputHitTestPolicy {
    pub(crate) ignore_pointer_events_none: bool,
    pub(crate) ignore_inert: bool,
    pub(crate) use_viewport_fallback: bool,
}

impl InputHitTestPolicy {
    pub(crate) const INTERACTIVE: Self = Self {
        ignore_pointer_events_none: false,
        ignore_inert: false,
        use_viewport_fallback: true,
    };
}

#[cfg(feature = "devtools")]
pub(crate) fn observable_input_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<DomHandle>, LayoutError> {
    observable_deep_hit_test(runtime, document, point, InputHitTestPolicy::INTERACTIVE)
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
                live_hit_in_tree(
                    runtime,
                    tree,
                    transform.map_point(point),
                    InputHitTestPolicy::INTERACTIVE,
                )
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
    input_surface_hit_test(
        runtime,
        document,
        point,
        InputHitTestPolicy::INTERACTIVE,
        InputHitTestMode::Mouse,
    )
    .map(|hit| match hit.control {
        Some(LayoutControlSurfaceHit::Scrollbar(scrollbar)) => Some(scrollbar),
        Some(LayoutControlSurfaceHit::ScrollbarCorner(_)) | None => None,
    })
}

/// Coordinate input shares the public geometry policy and recursive snapshot.
/// Realtime refreshes stale inputs; Lazy only initializes a cold page.
pub(crate) fn input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    policy: InputHitTestPolicy,
    mode: InputHitTestMode,
) -> Result<InputSurfaceHit, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return Ok(InputSurfaceHit {
            target: mock_hit_test_handle(
                runtime,
                document,
                point,
                MockHitTestOptions {
                    ignore_inert: policy.ignore_inert,
                    clip_to_viewport: true,
                    use_viewport_fallback: policy.use_viewport_fallback,
                },
            ),
            control: None,
            #[cfg(feature = "molio")]
            painted_source: None,
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
                policy,
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
    policy: InputHitTestPolicy,
) -> Result<Option<DomHandle>, LayoutError> {
    Ok(input_surface_hit_test(runtime, document, point, policy, InputHitTestMode::Dom)?.target)
}

fn input_surface_hit_test_in_tree(
    runtime: &PageScriptHost,
    tree: &FrozenLayoutTree<DomHandle>,
    point: LayoutPoint,
    root_to_frame: LayoutTransform2D,
    policy: InputHitTestPolicy,
    mode: InputHitTestMode,
    depth: usize,
) -> Result<InputSurfaceHit, LayoutError> {
    let live_dom_hit = if mode == InputHitTestMode::Mouse {
        let mut live_dom_hit = None;
        for surface in tree.painted_surface_hits(point, policy.ignore_pointer_events_none) {
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
                        #[cfg(feature = "molio")]
                        painted_source: None,
                    });
                }
                LayoutPaintedSurfaceHit::Dom(layout_hit) => {
                    let Some(target) =
                        live_interaction_target(runtime, layout_hit.source, policy.ignore_inert)
                    else {
                        continue;
                    };
                    live_dom_hit = Some((layout_hit, target));
                    break;
                }
            }
        }
        live_dom_hit
    } else {
        live_hit_in_tree(runtime, tree, point, policy)
    };
    let Some((layout_hit, target)) = live_dom_hit else {
        // The viewport still belongs to this document when all its painted
        // candidates are inert.
        let use_viewport_target = policy.use_viewport_fallback
            && (mode == InputHitTestMode::Wheel || tree.viewport.contains(point));
        return Ok(InputSurfaceHit {
            target: use_viewport_target.then_some(tree.source_root()),
            control: None,
            #[cfg(feature = "molio")]
            painted_source: None,
        });
    };
    let target_hit = InputSurfaceHit {
        target: Some(target),
        control: None,
        #[cfg(feature = "molio")]
        painted_source: Some(PaintedInputSource {
            node: layout_hit.source,
            pseudo: layout_hit
                .fragment
                .and_then(|fragment| tree.fragment_pseudo(fragment)),
            is_text: layout_hit.is_text,
        }),
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
    let child_root = runtime
        .dom_host()
        .dom()
        .document_element_handle_for_document(child_document);
    if runtime.layout_policy().is_realtime() && child_root.is_none() {
        // A frame can have an outer box while its new Document has no
        // layout root. Keep the hit on the frame until it has contents.
        return Ok(target_hit);
    }
    let child_tree = tree
        .embedded_frame_tree(target)
        .ok_or(LayoutError::NoLayoutSnapshot)?;
    if child_root != Some(child_tree.source_root()) {
        return Err(LayoutError::NoLayoutSnapshot);
    }
    let child_hit = input_surface_hit_test_in_tree(
        runtime,
        child_tree,
        frame_to_child.map_point(point),
        frame_to_child.concatenate(root_to_frame),
        policy,
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
    policy: InputHitTestPolicy,
) -> Option<(LayoutHit<DomHandle>, DomHandle)> {
    let first_hit = tree.hit_test(point, policy.ignore_pointer_events_none);
    let resolve_hit = |hit: LayoutHit<DomHandle>| {
        live_interaction_target(runtime, hit.source, policy.ignore_inert)
            .map(|target| (hit, target))
    };
    let live_first_hit = first_hit.and_then(resolve_hit);
    if live_first_hit.is_some() || first_hit.is_none() {
        return live_first_hit;
    }
    tree.hit_test_all(point, policy.ignore_pointer_events_none)
        .into_iter()
        .find_map(resolve_hit)
}
