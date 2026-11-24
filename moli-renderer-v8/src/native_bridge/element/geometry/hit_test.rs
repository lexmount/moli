use moli_layout::{
    FrozenLayoutTree, LayoutControlSurfaceHit, LayoutError, LayoutHit, LayoutPaintedSurfaceHit,
    LayoutPoint, LayoutTransform2D,
};

#[cfg(test)]
use moli_layout::LayoutScrollbarHit;

use super::mock::{MockHitTestOptions, mock_input_hit_test};
use super::provider::{is_live_hit_source, live_interaction_target};
#[cfg(feature = "devtools")]
use crate::dom::native::{DomHost, Node};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};
#[cfg(feature = "devtools")]
use moli_layout::LayoutFragmentKind;
#[cfg(feature = "devtools")]
use std::collections::HashSet;

pub(super) const CHILD_FRAME_DEPTH_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InputHit {
    pub(crate) handle: DomHandle,
    pub(crate) root_to_frame: LayoutTransform2D,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InputSurfaceHit {
    pub(crate) input: Option<InputHit>,
    pub(crate) control: Option<LayoutControlSurfaceHit<DomHandle>>,
    viewport_document: Option<InputHit>,
    /// The painted source before mapping it to an element for DOM events.
    #[cfg(feature = "molio")]
    pub(crate) painted_source: Option<PaintedInputSource>,
}

impl InputSurfaceHit {
    pub(crate) fn target(handle: DomHandle, root_to_frame: LayoutTransform2D) -> Self {
        Self {
            input: Some(InputHit {
                handle,
                root_to_frame,
            }),
            ..Self::default()
        }
    }

    fn empty_viewport(handle: DomHandle, root_to_frame: LayoutTransform2D) -> Self {
        Self {
            viewport_document: Some(InputHit {
                handle,
                root_to_frame,
            }),
            ..Self::default()
        }
    }

    pub(crate) fn document_or_element(self) -> Option<InputHit> {
        self.input.or(self.viewport_document)
    }

    pub(crate) fn hover_source(&self) -> Option<DomHandle> {
        #[cfg(feature = "molio")]
        if let Some(source) = self.painted_source {
            return Some(source.node);
        }
        self.input.map(|hit| hit.handle)
    }
}

#[cfg(feature = "molio")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaintedInputSource {
    pub(crate) node: DomHandle,
    pub(crate) pseudo: Option<moli_layout::LayoutPseudo>,
    pub(crate) is_text: bool,
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
) -> Result<Option<InputHit>, LayoutError> {
    Ok(input_surface_hit_test(
        runtime,
        document,
        point,
        InputHitTestPolicy::INTERACTIVE,
        InputHitTestMode::Dom,
    )?
    .input)
}

/// A pressed pointer keeps hit testing in its initiating frame until release.
#[cfg(feature = "devtools")]
pub(crate) fn captured_frame_input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    root_point: LayoutPoint,
    previous_root_to_frame: LayoutTransform2D,
    include_scrollbars: bool,
) -> Result<InputSurfaceHit, LayoutError> {
    let mode = if include_scrollbars {
        InputHitTestMode::Mouse
    } else {
        InputHitTestMode::Dom
    };
    let (mut hit, root_to_frame) = frame_input_surface_hit_test(
        runtime,
        document,
        root_point,
        previous_root_to_frame,
        mode,
        false,
    )?;
    if hit.input.is_none() && hit.control.is_none() {
        let handle = runtime
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
            .unwrap_or(document);
        hit = InputSurfaceHit::target(handle, root_to_frame);
    }
    Ok(hit)
}

/// A drag starts from the rendered branch at the press point before child
/// frame delegation.
#[cfg(feature = "devtools")]
pub(crate) fn drag_start_frame_input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    root_point: LayoutPoint,
    previous_root_to_frame: LayoutTransform2D,
) -> Result<InputSurfaceHit, LayoutError> {
    frame_input_surface_hit_test(
        runtime,
        document,
        root_point,
        previous_root_to_frame,
        InputHitTestMode::Mouse,
        false,
    )
    .map(|(hit, _)| hit)
}

#[cfg(feature = "devtools")]
fn frame_input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    root_point: LayoutPoint,
    previous_root_to_frame: LayoutTransform2D,
    mode: InputHitTestMode,
    descend_child_frames: bool,
) -> Result<(InputSurfaceHit, LayoutTransform2D), LayoutError> {
    let policy = InputHitTestPolicy::INTERACTIVE;
    if !runtime.layout_policy().uses_real_layout() {
        let hit = mock_input_surface_hit_test(
            runtime,
            document,
            root_point,
            previous_root_to_frame,
            policy,
            descend_child_frames,
        );
        return Ok((hit, previous_root_to_frame));
    }
    runtime.ensure_observable_layout()?;
    let Some(root_to_frame) = runtime.input_frame_transform(document, None) else {
        return Ok((InputSurfaceHit::default(), LayoutTransform2D::IDENTITY));
    };
    let hit = runtime.with_latest_layout_tree_for_document(document, |tree| {
        input_surface_hit_test_in_tree(
            runtime,
            tree,
            root_to_frame.map_point(root_point),
            FrameHitTest {
                document,
                root_to_frame,
                depth: 0,
                descend_child_frames,
            },
            policy,
            mode,
        )
    });
    let hit = match hit {
        Some(hit) => hit?,
        None if document == runtime.document_handle() => return Err(LayoutError::NoLayoutSnapshot),
        None => InputSurfaceHit::default(),
    };
    Ok((hit, root_to_frame))
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
/// Realtime refreshes stale inputs; OnDemand only initializes a cold page.
pub(crate) fn input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
    policy: InputHitTestPolicy,
    mode: InputHitTestMode,
) -> Result<InputSurfaceHit, LayoutError> {
    if !runtime.layout_policy().uses_real_layout() {
        return Ok(mock_input_surface_hit_test(
            runtime,
            document,
            point,
            LayoutTransform2D::IDENTITY,
            policy,
            true,
        ));
    }
    runtime.ensure_observable_layout()?;
    runtime
        .with_latest_layout_tree_for_document(document, |tree| {
            input_surface_hit_test_in_tree(
                runtime,
                tree,
                point,
                FrameHitTest {
                    document,
                    root_to_frame: LayoutTransform2D::IDENTITY,
                    depth: 0,
                    descend_child_frames: true,
                },
                policy,
                mode,
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
    Ok(
        input_surface_hit_test(runtime, document, point, policy, InputHitTestMode::Dom)?
            .document_or_element()
            .map(|hit| hit.handle),
    )
}

fn mock_input_surface_hit_test(
    runtime: &PageScriptHost,
    document: DomHandle,
    root_point: LayoutPoint,
    root_to_frame: LayoutTransform2D,
    policy: InputHitTestPolicy,
    descend_child_frames: bool,
) -> InputSurfaceHit {
    let Some((handle, local_to_hit_frame)) = mock_input_hit_test(
        runtime,
        document,
        root_to_frame.map_point(root_point),
        MockHitTestOptions {
            ignore_inert: policy.ignore_inert,
            clip_to_viewport: true,
            use_viewport_fallback: policy.use_viewport_fallback,
            include_empty_viewport: true,
            stay_in_document: !descend_child_frames,
        },
    ) else {
        return InputSurfaceHit::default();
    };
    let transform = local_to_hit_frame.concatenate(root_to_frame);
    if runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_document())
    {
        InputSurfaceHit::empty_viewport(handle, transform)
    } else {
        InputSurfaceHit::target(handle, transform)
    }
}

#[derive(Clone, Copy)]
struct FrameHitTest {
    document: DomHandle,
    root_to_frame: LayoutTransform2D,
    depth: usize,
    descend_child_frames: bool,
}

fn input_surface_hit_test_in_tree(
    runtime: &PageScriptHost,
    tree: &FrozenLayoutTree<DomHandle>,
    point: LayoutPoint,
    frame: FrameHitTest,
    policy: InputHitTestPolicy,
    mode: InputHitTestMode,
) -> Result<InputSurfaceHit, LayoutError> {
    let FrameHitTest {
        document,
        root_to_frame,
        depth,
        descend_child_frames,
    } = frame;
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
                        control: Some(control),
                        ..InputSurfaceHit::default()
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
        if !use_viewport_target {
            return Ok(InputSurfaceHit::default());
        }
        let root = runtime
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
            .filter(|root| runtime.dom_host().is_connected(*root))
            .filter(|root| {
                tree.boxes
                    .get(tree.root_box.index())
                    .is_some_and(|layout_box| layout_box.principal_source == Some(*root))
            });
        return Ok(root.map_or_else(
            || InputSurfaceHit::empty_viewport(document, root_to_frame),
            |root| InputSurfaceHit::target(root, root_to_frame),
        ));
    };
    let target_hit = InputSurfaceHit {
        #[cfg(feature = "molio")]
        painted_source: Some(PaintedInputSource {
            node: layout_hit.source,
            pseudo: layout_hit
                .fragment
                .and_then(|fragment| tree.fragment_pseudo(fragment)),
            is_text: layout_hit.is_text,
        }),
        ..InputSurfaceHit::target(target, root_to_frame)
    };
    if depth >= CHILD_FRAME_DEPTH_LIMIT || !descend_child_frames {
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
    if child_root.is_none() && runtime.layout_policy().is_realtime() {
        // The frame can keep its outer box while its document has no layout root.
        return Ok(target_hit);
    }
    let child_tree = tree
        .embedded_frame_tree(target)
        .ok_or(LayoutError::NoLayoutSnapshot)?;
    if child_root.unwrap_or(child_document) != child_tree.source_root() {
        return Err(LayoutError::NoLayoutSnapshot);
    }
    input_surface_hit_test_in_tree(
        runtime,
        child_tree,
        frame_to_child.map_point(point),
        FrameHitTest {
            document: child_document,
            root_to_frame: frame_to_child.concatenate(root_to_frame),
            depth: depth + 1,
            descend_child_frames: true,
        },
        policy,
        mode,
    )
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

/// Resolve the drag origin from the published rendered ancestry, rather than
/// dispatching at the last mouse-press target. A mousedown handler can move or
/// replace that node; the geometry and ancestry must come from the same pass.
#[cfg(feature = "devtools")]
pub(crate) fn draggable_source_at_point(
    runtime: &PageScriptHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<DomHandle>, LayoutError> {
    let host = runtime.dom_host();
    // A pointer listener can remove the document's root before drag startup.
    // There is then no rendered source to query, even if a press is pending.
    if host
        .dom()
        .document_element_handle_for_document(document)
        .is_none()
    {
        return Ok(None);
    }
    let eligible = |handle| {
        host.is_connected(handle)
            && host.owner_document_handle(handle) == Some(document)
            && host
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.is_draggable())
    };
    if runtime.layout_policy().uses_real_layout() {
        runtime.ensure_initial_layout()?;
        return runtime
            .with_latest_layout_tree_for_document(document, |tree| {
                let (hit, _) =
                    live_hit_in_tree(runtime, tree, point, InputHitTestPolicy::INTERACTIVE)?;
                let mut candidate = hit
                    .fragment
                    .and_then(|id| tree.fragments.get(id.index()))
                    .map(|fragment| match fragment.kind {
                        LayoutFragmentKind::Box { box_id }
                        | LayoutFragmentKind::InlineBox { box_id, .. }
                        | LayoutFragmentKind::Text { box_id, .. } => box_id,
                        LayoutFragmentKind::Line { owner, .. } => owner,
                    });
                let mut rendered_path = Vec::new();
                while let Some(id) = candidate {
                    let layout_box = tree.boxes.get(id.index())?;
                    if let Some(handle) = layout_box.principal_source {
                        rendered_path.push(handle);
                    }
                    candidate = layout_box.structural_parent;
                }
                // A moved descendant no longer occupies its old rendered
                // branch. Resolve the origin at the surviving ancestor in
                // the published path without refreshing layout during input.
                let mut origin = hit.source;
                let mut ancestors = flat_tree_ancestors(host, origin);
                let mut start = 0;
                for (index, handle) in rendered_path.iter().copied().enumerate() {
                    if !ancestors.contains(&handle) {
                        origin = handle;
                        ancestors = flat_tree_ancestors(host, origin);
                        start = index;
                    }
                }
                let source = rendered_path[start..]
                    .iter()
                    .copied()
                    .find(|handle| eligible(*handle))?;
                // Slot distribution supplies rendered ancestry, but does not
                // make a light child a DOM descendant of the shadow element.
                host.shadow_including_contains(source, origin)
                    .then_some(source)
            })
            .ok_or(LayoutError::NoLayoutSnapshot);
    }
    let origin = mock_input_hit_test(
        runtime,
        document,
        point,
        MockHitTestOptions {
            stay_in_document: true,
            ..MockHitTestOptions::default()
        },
    )
    .map(|(handle, _)| handle);
    let mut candidate = origin;
    while let Some(handle) = candidate {
        if eligible(handle) {
            return Ok(origin
                .filter(|origin| host.shadow_including_contains(handle, *origin))
                .map(|_| handle));
        }
        candidate = host.flat_tree_parent(handle);
    }
    Ok(None)
}

#[cfg(feature = "devtools")]
fn flat_tree_ancestors(host: &DomHost, handle: DomHandle) -> HashSet<DomHandle> {
    let mut ancestors = HashSet::new();
    let mut current = Some(handle);
    while let Some(handle) = current {
        if !ancestors.insert(handle) {
            break;
        }
        current = host.flat_tree_parent(handle);
    }
    ancestors
}
