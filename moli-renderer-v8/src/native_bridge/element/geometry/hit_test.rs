use std::{cell::RefCell, rc::Rc};

use moli_layout::{
    FrozenLayoutTree, LayoutControlSurfaceHit, LayoutError, LayoutHit, LayoutPaintedSurfaceHit,
    LayoutPoint, LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch, LayoutTransform2D,
};

#[cfg(test)]
use moli_layout::LayoutScrollbarHit;

use super::provider::{
    observable_geometry_batch, observable_geometry_query, observable_hit_test_all,
    provider_contract_error,
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

/// Reuses hit-test transforms and retains the published snapshot for receivers
/// in other frames. Sampling does not walk layout boxes or enumerate frames.
pub(crate) struct InputFrameTransforms {
    root_document: DomHandle,
    tree: Option<Rc<FrozenLayoutTree<DomHandle>>>,
    root_to_document: RefCell<smallvec::SmallVec<[(DomHandle, LayoutTransform2D); 1]>>,
}

impl InputFrameTransforms {
    pub(crate) fn sample(
        runtime: &JsContextHost,
        hits: impl IntoIterator<Item = InputHit>,
    ) -> Self {
        let root_document = runtime.document_handle();
        let transforms = hits
            .into_iter()
            .filter_map(|hit| {
                let document = runtime.dom_host().owner_document_handle(hit.handle)?;
                (document != root_document).then_some((document, hit.root_to_frame))
            })
            .collect();
        Self {
            root_document,
            tree: runtime.retain_latest_layout_tree(),
            root_to_document: RefCell::new(transforms),
        }
    }

    pub(crate) fn for_target(
        &self,
        runtime: &JsContextHost,
        target: DomHandle,
    ) -> LayoutTransform2D {
        let Some(document) = runtime.dom_host().owner_document_handle(target) else {
            return LayoutTransform2D::IDENTITY;
        };
        if document == self.root_document {
            return LayoutTransform2D::IDENTITY;
        }
        if let Some((_, transform)) = self
            .root_to_document
            .borrow()
            .iter()
            .find(|(known, _)| *known == document)
        {
            return *transform;
        }
        // Only a receiver outside the hit-tested frames needs another geometry
        // lookup. Query the retained snapshot, never live layout after callbacks.
        let transform = if runtime.layout_policy().uses_real_layout() {
            self.tree.as_ref().and_then(|tree| {
                let root = runtime
                    .dom_host()
                    .dom()
                    .document_element_handle_for_document(document)?;
                Self::for_root_in_tree(tree, root, 0)
            })
        } else {
            self.for_mock_document(runtime, document, 0)
        }
        // An unrendered or newly created frame has no sampled viewport. Root
        // coordinates let capture/touch cleanup finish without a layout error.
        .unwrap_or(LayoutTransform2D::IDENTITY);
        self.root_to_document
            .borrow_mut()
            .push((document, transform));
        transform
    }

    fn for_root_in_tree(
        tree: &FrozenLayoutTree<DomHandle>,
        root: DomHandle,
        depth: usize,
    ) -> Option<LayoutTransform2D> {
        if tree.source_root() == root {
            return Some(LayoutTransform2D::IDENTITY);
        }
        if depth >= CHILD_FRAME_DEPTH_LIMIT {
            return None;
        }
        for frame in tree.embedded_frames() {
            let Some(child_to_target) = Self::for_root_in_tree(&frame.tree, root, depth + 1) else {
                continue;
            };
            // Inspect boxes only along the requested frame's ancestor chain.
            let geometry = &tree
                .boxes
                .iter()
                .rev()
                .find(|layout_box| layout_box.principal_source == Some(frame.source))?
                .geometry;
            let viewport_to_local = tree
                .coordinate_space(geometry.coordinate_space)?
                .local_to_viewport
                .inverse()?;
            let parent_to_child =
                LayoutTransform2D::translation(-geometry.content_box.x, -geometry.content_box.y)
                    .concatenate(viewport_to_local);
            return Some(child_to_target.concatenate(parent_to_child));
        }
        None
    }

    fn for_mock_document(
        &self,
        runtime: &JsContextHost,
        document: DomHandle,
        depth: usize,
    ) -> Option<LayoutTransform2D> {
        if document == self.root_document {
            return Some(LayoutTransform2D::IDENTITY);
        }
        if depth >= CHILD_FRAME_DEPTH_LIMIT {
            return None;
        }
        let frame = runtime.child_browsing_context_host_for_document_handle(document)?;
        let parent = runtime.dom_host().owner_document_handle(frame)?;
        let LayoutQueryAnswer::BoxModel(Some(model)) =
            observable_geometry_query(runtime, parent, &LayoutQuery::BoxModel { source: frame })
                .ok()?
        else {
            return None;
        };
        let origin = model.content.points[0];
        Some(
            LayoutTransform2D::translation(-origin.x, -origin.y)
                .concatenate(self.for_mock_document(runtime, parent, depth + 1)?),
        )
    }
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
    input_surface_hit_test(runtime, document, point, false, false).map(|hit| hit.input)
}

#[cfg(test)]
pub(crate) fn observable_scrollbar_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
) -> Result<Option<LayoutScrollbarHit<DomHandle>>, LayoutError> {
    input_surface_hit_test(runtime, document, point, false, true).map(|hit| match hit.control {
        Some(LayoutControlSurfaceHit::Scrollbar(scrollbar)) => Some(scrollbar),
        Some(LayoutControlSurfaceHit::ScrollbarCorner(_)) | None => None,
    })
}

/// Coordinate input initializes a cold page once, then consumes the same
/// published tree, including embedded frames, until an explicit refresh.
pub(crate) fn input_surface_hit_test(
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
    runtime.ensure_initial_layout()?;
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

pub(crate) fn observable_deep_hit_test(
    runtime: &JsContextHost,
    document: DomHandle,
    point: LayoutPoint,
    ignore_pointer_events_none: bool,
) -> Result<Option<DomHandle>, LayoutError> {
    Ok(
        input_surface_hit_test(runtime, document, point, ignore_pointer_events_none, false)?
            .input
            .map(|hit| hit.handle),
    )
}

fn input_hit_test_via_documents(
    runtime: &JsContextHost,
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
) -> Result<InputSurfaceHit, LayoutError> {
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
                    return Ok(InputSurfaceHit {
                        input: None,
                        control: Some(control),
                    });
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
        return Ok(InputSurfaceHit::default());
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
        include_scrollbars,
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
    let (_, first_hit) = observable_hit_test(
        runtime,
        frame.document,
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
    let (_, hits) = observable_hit_test_all(
        runtime,
        frame.document,
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

/// Resolve the foremost painted hit and the viewport sampled by the same
/// frozen layout pass. Single-point DOM APIs consume this query while
/// penetrating-list APIs use `observable_hit_test_all`.
pub(crate) fn observable_hit_test(
    runtime: &JsContextHost,
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
