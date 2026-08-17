use crate::{
    DocumentLayoutServices, LayoutError, LayoutFlushReason, LayoutPassResult, LayoutScrollbarAxis,
    LayoutSource, LayoutStyleResolver, LayoutViewport, PaintCaptureRequest, PaintSnapshot,
    PaintViewport, build_layout_world,
    form::prepare_form_controls,
    inline::prepare_inline_contexts,
    list::prepare_list_markers,
    projection::{finish_layout_pass, overflowing_axes},
    taffy_tree::compute_world_layout,
};
use std::collections::HashMap;
use std::time::Instant;

/// Renderer-owned bridge for one live embedded browsing context.
///
/// The parent numeric layout has already completed when this callback runs, so
/// `viewport` is the iframe's used content-box size rather than an attribute or
/// computed-style estimate. Implementations must return an owned, source-free
/// snapshot and must not run JavaScript, lifecycle work, or an event-loop turn.
/// Recursive child layout is allowed because every nested world remains local
/// to the same synchronous demand.
pub trait EmbeddedFrameRenderer<N> {
    fn render_embedded_frame(
        &mut self,
        frame: N,
        viewport: LayoutViewport,
    ) -> Result<Option<PaintSnapshot>, LayoutError>;
}

struct NoEmbeddedFrames;

impl<N> EmbeddedFrameRenderer<N> for NoEmbeddedFrames {
    fn render_embedded_frame(
        &mut self,
        _frame: N,
        _viewport: LayoutViewport,
    ) -> Result<Option<PaintSnapshot>, LayoutError> {
        Ok(None)
    }
}

/// Inputs for one complete, synchronous layout demand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutPassRequest {
    pub viewport: LayoutViewport,
    pub reason: LayoutFlushReason,
    paint_capture: Option<PaintCaptureRequest>,
}

impl LayoutPassRequest {
    pub const fn new(viewport: LayoutViewport, reason: LayoutFlushReason) -> Self {
        Self {
            viewport,
            reason,
            paint_capture: None,
        }
    }

    /// Creates a demand that also projects the same world into owned paint input.
    pub const fn with_paint(viewport: LayoutViewport, reason: LayoutFlushReason) -> Self {
        Self::with_capture(viewport, reason, PaintCaptureRequest::viewport())
    }

    /// Creates a demand that projects a separate one-shot capture surface.
    pub const fn with_capture(
        viewport: LayoutViewport,
        reason: LayoutFlushReason,
        paint_capture: PaintCaptureRequest,
    ) -> Self {
        Self {
            viewport,
            reason,
            paint_capture: Some(paint_capture),
        }
    }

    /// Whether this demand also needs immutable software-paint input.
    pub const fn requests_paint(self) -> bool {
        self.paint_capture.is_some()
    }

    /// Whether paint snapshots for this demand should include CSS backgrounds.
    /// Layout-only demands return `true` so recursive renderers retain their
    /// normal paint defaults when no capture policy exists.
    pub const fn includes_backgrounds(self) -> bool {
        match self.paint_capture {
            Some(capture) => capture.include_backgrounds,
            None => true,
        }
    }
}

/// Inputs for one on-demand screenshot layout pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenshotLayoutRequest {
    /// Viewport and device scale to lay out and paint.
    pub viewport: PaintViewport,
}

impl ScreenshotLayoutRequest {
    /// Creates a screenshot layout request.
    pub const fn new(viewport: PaintViewport) -> Self {
        Self { viewport }
    }
}

/// Builds, lays out, and paints one synchronous source view into an owned snapshot.
///
/// The working box tree and Taffy caches are dropped before this function
/// returns. This convenience path consumes the frozen tree as well and returns
/// only DOM-neutral paint input.
pub fn build_screenshot_snapshot<S, R>(
    source: &S,
    styles: &mut R,
    services: &mut DocumentLayoutServices,
    request: ScreenshotLayoutRequest,
) -> Result<PaintSnapshot, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    build_layout_pass(
        source,
        styles,
        services,
        LayoutPassRequest::with_paint(request.viewport, LayoutFlushReason::Screenshot),
    )
    .and_then(LayoutPassResult::into_paint_snapshot)
}

/// Builds one complete layout result from a borrowed source view.
///
/// Box construction, inline shaping, Taffy caches, and all style borrows remain
/// local to this call. The returned pass value owns one [`crate::FrozenLayoutTree`]
/// plus pass-only metrics, diagnostics, and an optional DOM-neutral paint
/// snapshot. Consumers may retain the tree, but not the surrounding pass
/// value. Several geometry answers should be batched against one tree instead
/// of triggering a layout per query.
pub fn build_layout_pass<S, R>(
    source: &S,
    styles: &mut R,
    services: &mut DocumentLayoutServices,
    request: LayoutPassRequest,
) -> Result<LayoutPassResult<S::NodeId>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    build_layout_pass_with_embedded_frames(source, styles, services, request, &mut NoEmbeddedFrames)
}

/// Builds one complete layout result and resolves embedded frame pixels after
/// the parent numeric layout has established their exact content viewports.
///
/// This is a one-shot composition seam, not a retained subframe tree. Child
/// snapshots are consumed into the parent snapshot before all layout worlds
/// are dropped.
pub fn build_layout_pass_with_embedded_frames<S, R, F>(
    source: &S,
    styles: &mut R,
    services: &mut DocumentLayoutServices,
    request: LayoutPassRequest,
    frames: &mut F,
) -> Result<LayoutPassResult<S::NodeId>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
    F: EmbeddedFrameRenderer<S::NodeId>,
{
    let started = Instant::now();
    let mut world = build_layout_world(source, styles)?;
    prepare_list_markers(&mut world);
    prepare_form_controls(&mut world);
    prepare_inline_contexts(&mut world, services);
    compute_world_layout_with_scrollbars(&mut world, request.viewport);
    let mut embedded_frames = HashMap::new();
    if request.requests_paint() {
        for index in 0..world.boxes.len() {
            let layout_box = &world.boxes[index];
            if !layout_box.element_semantics().is_some_and(|semantics| {
                semantics.replaced == Some(crate::LayoutReplacedKind::Frame)
            }) {
                continue;
            }
            let Some(source) = layout_box.source() else {
                continue;
            };
            let layout = layout_box.final_layout();
            let width = (layout.size.width
                - layout.border.left
                - layout.border.right
                - layout.padding.left
                - layout.padding.right)
                .max(0.0);
            let height = (layout.size.height
                - layout.border.top
                - layout.border.bottom
                - layout.padding.top
                - layout.padding.bottom)
                .max(0.0);
            if width <= 0.0 || height <= 0.0 {
                continue;
            }
            let viewport = LayoutViewport::new(
                css_viewport_dimension(width),
                css_viewport_dimension(height),
                request.viewport.device_pixel_ratio,
            );
            if let Some(snapshot) = frames.render_embedded_frame(source, viewport)? {
                embedded_frames.insert(crate::LayoutBoxId::from_index(index), snapshot);
            }
        }
    }
    finish_layout_pass(
        &world,
        request.viewport,
        request.reason,
        started,
        request.paint_capture,
        &mut embedded_frames,
    )
}

/// Resolves `overflow:auto` with the same monotonic feedback loop used by
/// classic browser scrollbars: lay out without an automatic gutter, reveal
/// every axis that overflows, then repeat because one gutter can make the
/// perpendicular axis overflow. Each axis changes at most once.
fn compute_world_layout_with_scrollbars<N>(
    world: &mut crate::LayoutWorld<N>,
    viewport: LayoutViewport,
) where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    let root = world.root.index();
    for (index, layout_box) in world.boxes.iter_mut().enumerate() {
        layout_box.style.prepare_scrollbar_layout(index == root);
    }

    loop {
        compute_world_layout(world, viewport);
        let overflow = overflowing_axes(world, viewport);
        let mut changed = false;
        for (index, ((overflow_x, overflow_y), layout_box)) in
            overflow.into_iter().zip(world.boxes.iter_mut()).enumerate()
        {
            let is_root = index == root;
            changed |= layout_box.style.reveal_auto_scrollbar(
                LayoutScrollbarAxis::Horizontal,
                is_root,
                overflow_x,
            );
            changed |= layout_box.style.reveal_auto_scrollbar(
                LayoutScrollbarAxis::Vertical,
                is_root,
                overflow_y,
            );
        }
        if !changed {
            break;
        }
    }
    offset_leading_scrollbar_gutter_children(world);
}

/// Taffy's scalar scrollbar reservation shrinks the available inline size but
/// always leaves its origin at zero. Move direct numeric children into a
/// physical leading gutter (RTL classic bars and `both-edges`) after the final
/// layout, preserving all of Taffy's percentage and flex/grid sizing work.
fn offset_leading_scrollbar_gutter_children<N>(world: &mut crate::LayoutWorld<N>)
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    let root = world.root;
    let offsets = world
        .boxes
        .iter()
        .map(|layout_box| {
            layout_box.layout_parent.map_or(0.0, |parent| {
                if parent == root {
                    // The synthetic initial containing block already places
                    // the root and its descendants after a root leading
                    // gutter.
                    0.0
                } else {
                    world.boxes[parent.index()]
                        .style
                        .scrollbar_leading_gutter_thickness(LayoutScrollbarAxis::Vertical, false)
                }
            })
        })
        .collect::<Vec<_>>();
    for (layout_box, offset) in world.boxes.iter_mut().zip(offsets) {
        layout_box.final_layout.location.x += offset;
    }
}

fn css_viewport_dimension(value: f32) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(0.0, u32::MAX as f32) as u32
}
