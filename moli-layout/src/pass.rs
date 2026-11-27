use crate::{
    DocumentLayoutServices, FrozenLayoutTree, LayoutError, LayoutFlushReason, LayoutPassResult,
    LayoutScrollbarAxis, LayoutSource, LayoutStyleResolver, LayoutViewport, LayoutWorld,
    PaintCaptureRequest, PaintSnapshot, PaintViewport, build_layout_world,
    form::prepare_form_controls,
    inline::prepare_inline_contexts,
    layout_tree::FrozenInlineSvgInputs,
    list::prepare_list_markers,
    overflow::OverflowProjection,
    projection::{RetainedPaintData, finish_layout_pass},
    taffy_tree::{compute_prepared_world_layout, prepare_world_layout},
};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Owned products for one embedded browsing context in the same synchronous
/// layout demand as its parent.
pub struct EmbeddedFrameSnapshot<N>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    pub tree: FrozenLayoutTree<N>,
    pub paint: Option<PaintSnapshot>,
    pub css_image_references: Vec<crate::LayoutCssImageReference<N>>,
    pub(crate) retained_layout: Option<Box<RetainedLayoutWorld<N>>>,
    pub(crate) retained_box_count: usize,
}

impl<N> EmbeddedFrameSnapshot<N>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    #[must_use]
    pub const fn new(
        tree: FrozenLayoutTree<N>,
        paint: Option<PaintSnapshot>,
        css_image_references: Vec<crate::LayoutCssImageReference<N>>,
    ) -> Self {
        Self {
            tree,
            paint,
            css_image_references,
            retained_layout: None,
            retained_box_count: 0,
        }
    }
}

/// Paint demand for a child viewport sampled by its parent's capture.
#[derive(Clone, Copy)]
pub struct EmbeddedFramePaintRequest {
    pub include_backgrounds: bool,
    /// The parent capture's bounds in child viewport coordinates.
    pub cull_rect: Option<crate::LayoutRect>,
}

/// Renderer-owned bridge for one live embedded browsing context.
///
/// The parent numeric layout has already completed when this callback runs, so
/// `viewport` is the iframe's used content-box size rather than an attribute or
/// computed-style estimate. Implementations must return an owned, source-free
/// snapshot and must not run JavaScript, lifecycle work, or an event-loop turn.
/// Recursive child layout is allowed because every nested world remains local
/// to the same synchronous demand. The parent owns the child tree and any
/// retained work from the child's publication.
pub trait EmbeddedFrameRenderer<N>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    fn render_embedded_frame(
        &mut self,
        frame: N,
        viewport: LayoutViewport,
        paint: Option<EmbeddedFramePaintRequest>,
    ) -> Result<Option<EmbeddedFrameSnapshot<N>>, LayoutError>;
}

struct NoEmbeddedFrames;

impl<N> EmbeddedFrameRenderer<N> for NoEmbeddedFrames
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    fn render_embedded_frame(
        &mut self,
        _frame: N,
        _viewport: LayoutViewport,
        _paint: Option<EmbeddedFramePaintRequest>,
    ) -> Result<Option<EmbeddedFrameSnapshot<N>>, LayoutError> {
        Ok(None)
    }
}

/// Which embedded Documents a pass must compute when their frame has a box.
/// Hidden frames without a layout box remain absent in both cases.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LayoutFrameCoverage {
    #[default]
    PositiveViewport,
    /// Includes zero-sized viewports whose descendants still have geometry.
    AllLayoutBoxes,
}

/// Inputs for one complete, synchronous layout demand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutPassRequest {
    pub viewport: LayoutViewport,
    pub reason: LayoutFlushReason,
    environment: crate::LayoutEnvironment,
    paint_capture: Option<PaintCaptureRequest>,
    paint_cull_rect: Option<crate::LayoutRect>,
    frame_coverage: LayoutFrameCoverage,
    retain_scroll_projection: bool,
    retain_screen_paint: bool,
    retain_layout_world: bool,
}

impl LayoutPassRequest {
    pub const fn new(viewport: LayoutViewport, reason: LayoutFlushReason) -> Self {
        Self {
            viewport,
            reason,
            environment: crate::LayoutEnvironment {
                scrollbars_hidden: false,
            },
            paint_capture: None,
            paint_cull_rect: None,
            frame_coverage: LayoutFrameCoverage::PositiveViewport,
            retain_scroll_projection: false,
            retain_screen_paint: false,
            retain_layout_world: false,
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
        let mut request = Self::new(viewport, reason);
        request.paint_capture = Some(paint_capture);
        request
    }

    /// Sets document-wide runtime policy without changing authored CSS styles.
    pub const fn with_environment(mut self, environment: crate::LayoutEnvironment) -> Self {
        self.environment = environment;
        self
    }

    /// Narrows embedded paint to the part exposed by the parent capture.
    pub const fn with_paint_cull_rect(mut self, rect: Option<crate::LayoutRect>) -> Self {
        self.paint_cull_rect = rect;
        self
    }

    /// Selects recursive frame geometry independently of paint demand.
    pub const fn with_frame_coverage(mut self, coverage: LayoutFrameCoverage) -> Self {
        self.frame_coverage = coverage;
        self
    }

    pub const fn frame_coverage(self) -> LayoutFrameCoverage {
        self.frame_coverage
    }

    /// Retains box placement inputs needed to update screen geometry after scrolling.
    pub const fn with_scroll_projection(mut self, retain: bool) -> Self {
        self.retain_scroll_projection = retain;
        self
    }

    pub const fn retains_scroll_projection(self) -> bool {
        self.retain_scroll_projection || self.retain_screen_paint
    }

    /// Keep the layout inputs needed to paint this screen publication again.
    pub const fn with_screen_paint(mut self, retain: bool) -> Self {
        self.retain_screen_paint = retain;
        self
    }

    pub const fn retains_screen_paint(self) -> bool {
        self.retain_screen_paint
    }

    /// Keep the working tree and numeric caches for a subsequent screen pass.
    pub const fn with_retained_layout_world(mut self) -> Self {
        self.retain_layout_world = true;
        self
    }

    pub const fn retains_layout_world(self) -> bool {
        self.retain_layout_world
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
/// Style borrows and Taffy caches end with the pass. The result owns a frozen
/// tree, metrics, diagnostics and any requested paint products. A screen
/// publication can take the owned inputs for another paint projection.
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

/// Builds one complete layout result and resolves embedded frame geometry after
/// the parent numeric layout has established their exact content viewports.
///
/// Child trees belong to the parent snapshot. An immediate paint request
/// composes the child snapshots into the parent. A retained screen paint
/// request keeps the child paint inputs alongside the parent inputs.
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
    finish_prepared_layout_pass(
        prepare_layout_pass(source, styles, request)?,
        source,
        services,
        frames,
    )
}

/// Owned box construction and control preparation for one layout demand.
/// The style resolver can be released before shaping, geometry, and child
/// document layout consume this value.
pub struct PreparedLayoutPass<N>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    world: LayoutWorld<N>,
    request: LayoutPassRequest,
    started: Instant,
    box_tree_elapsed: Duration,
    list_marker_elapsed: Duration,
    form_control_elapsed: Duration,
    retained_box_count: usize,
    pruned_source_count: usize,
    reuse_numeric_layout: bool,
}

/// A complete retained box update includes every changed native and style
/// input since the world was last published. A subtree change requires
/// rechecking its descendants even when the root's primary style is equal.
#[derive(Clone, Copy)]
pub enum LayoutSourceChange<N> {
    Node(N),
    Subtree(N),
}

/// Document-owned working tree with optional paint inputs for its publication.
pub struct RetainedLayoutWorld<N>
where
    N: Copy + std::fmt::Debug + Eq + Hash,
{
    pub(crate) world: LayoutWorld<N>,
    pub(crate) viewport: LayoutViewport,
    pub(crate) reusable_layout: bool,
    pub(crate) frames: HashMap<N, (crate::LayoutBoxId, Box<RetainedLayoutWorld<N>>)>,
    pub(crate) paint: Option<Box<RetainedPaintData>>,
}

impl<N: Copy + std::fmt::Debug + Eq + Hash> RetainedLayoutWorld<N> {
    /// Whether the working tree was retained for another numeric layout pass.
    pub const fn can_reuse_layout(&self) -> bool {
        self.reusable_layout
    }

    pub const fn has_paint(&self) -> bool {
        self.paint.is_some()
    }

    /// Release the old publication's paint inputs while retaining layout caches.
    pub fn discard_paint(&mut self) {
        if self.paint.take().is_some() {
            self.world.release_paint_storage();
            self.frames.retain(|_, (_, frame)| {
                frame.discard_paint();
                frame.can_reuse_layout()
            });
        }
    }

    /// The parent refresh moves each frame's existing work into its next pass.
    pub fn take_embedded_layouts(&mut self) -> HashMap<N, (crate::LayoutBoxId, Box<Self>)> {
        std::mem::take(&mut self.frames)
    }

    fn into_layout_world(mut self) -> LayoutWorld<N> {
        self.discard_paint();
        self.world
    }
}

/// Pass-local numeric layout, before embedded frames and paint are projected.
pub struct MeasuredLayoutPass<N>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    prepared: PreparedLayoutPass<N>,
    inline_preparation_elapsed: Duration,
    reused_inline_contexts: usize,
    numeric_layout_elapsed: Duration,
    numeric_metrics: NumericLayoutMetrics,
    overflow_projection: Option<OverflowProjection>,
}

#[derive(Clone, Copy)]
pub(crate) struct InlineSvgViewport {
    pub(crate) owner: crate::LayoutOutputBoxId,
    pub(crate) size: crate::LayoutSize,
    pub(crate) prepared: bool,
}

pub(crate) fn svg_viewport_size<N: Copy>(layout_box: &crate::LayoutBox<N>) -> crate::LayoutSize {
    let layout = layout_box.final_layout;
    let zoom = layout_box.style.effective_zoom();
    crate::LayoutSize::new(
        ((layout.size.width
            - layout.border.left
            - layout.border.right
            - layout.padding.left
            - layout.padding.right
            - layout.scrollbar_size.width)
            / zoom)
            .max(0.0),
        ((layout.size.height
            - layout.border.top
            - layout.border.bottom
            - layout.padding.top
            - layout.padding.bottom
            - layout.scrollbar_size.height)
            / zoom)
            .max(0.0),
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ScrollbarOwner<N> {
    Viewport,
    Element(N),
    Pseudo(N, crate::LayoutPseudo),
}

impl<N: Copy> ScrollbarOwner<N> {
    fn for_box(layout_box: &crate::LayoutBox<N>) -> Option<Self> {
        if let Some(source) = layout_box.source() {
            return Some(Self::Element(source));
        }
        let (owner, pseudo) = layout_box.owner().zip(layout_box.pseudo())?;
        Some(Self::Pseudo(owner, pseudo))
    }
}

const SCROLLBAR_AXES: [LayoutScrollbarAxis; 2] = [
    LayoutScrollbarAxis::Horizontal,
    LayoutScrollbarAxis::Vertical,
];

/// Revealed automatic scrollbars, keyed by stable sources across box-tree rebuilds.
#[derive(PartialEq, Eq)]
pub struct AutoScrollbars<N: Eq + Hash>(HashSet<(ScrollbarOwner<N>, LayoutScrollbarAxis)>);

impl<N: Eq + Hash> Default for AutoScrollbars<N> {
    fn default() -> Self {
        Self(HashSet::new())
    }
}

impl<N: Copy + Eq + Hash> AutoScrollbars<N> {
    /// Record every axis whose visibility differs between two measurements.
    pub fn extend_changed_axes(&mut self, first: &Self, second: &Self) {
        self.0
            .extend(first.0.symmetric_difference(&second.0).copied());
    }

    /// Add the axes observed in a confirmed cycle to the frozen gutters.
    pub fn freeze_axes(&mut self, changed: Self) -> bool {
        let previous_count = self.0.len();
        self.0.extend(changed.0);
        self.0.len() != previous_count
    }
}

impl<N> MeasuredLayoutPass<N>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    pub fn box_count(&self) -> usize {
        self.prepared.world.len()
    }

    /// Rebuild after style feedback, keeping the cost of the discarded layout.
    pub fn remeasure(
        self,
        services: &mut DocumentLayoutServices,
        frozen_scrollbars: &AutoScrollbars<N>,
        prepare: impl FnOnce() -> Result<PreparedLayoutPass<N>, LayoutError>,
    ) -> Result<Self, LayoutError> {
        let Self {
            prepared,
            inline_preparation_elapsed,
            reused_inline_contexts,
            numeric_layout_elapsed,
            numeric_metrics,
            overflow_projection,
        } = self;
        let PreparedLayoutPass {
            world,
            started,
            box_tree_elapsed,
            list_marker_elapsed,
            form_control_elapsed,
            ..
        } = prepared;
        drop(world);
        drop(overflow_projection);
        let mut next =
            measure_prepared_layout_pass_with_scrollbars(prepare()?, services, frozen_scrollbars)?;
        next.prepared.started = started;
        next.prepared.box_tree_elapsed += box_tree_elapsed;
        next.prepared.list_marker_elapsed += list_marker_elapsed;
        next.prepared.form_control_elapsed += form_control_elapsed;
        next.inline_preparation_elapsed += inline_preparation_elapsed;
        next.reused_inline_contexts += reused_inline_contexts;
        next.numeric_layout_elapsed += numeric_layout_elapsed;
        next.numeric_metrics.accumulate(numeric_metrics);
        Ok(next)
    }

    pub fn auto_scrollbars(&self) -> AutoScrollbars<N> {
        let world = &self.prepared.world;
        let mut scrollbars = AutoScrollbars::default();
        if world.environment.scrollbars_hidden {
            return scrollbars;
        }
        for axis in SCROLLBAR_AXES {
            if world.viewport_scroll_policy.auto_scrollbar_revealed(axis) {
                scrollbars.0.insert((ScrollbarOwner::Viewport, axis));
            }
        }
        for layout_box in world.boxes.values() {
            for axis in SCROLLBAR_AXES {
                if layout_box.style.auto_scrollbar_revealed(axis)
                    && let Some(owner) = ScrollbarOwner::for_box(layout_box)
                {
                    scrollbars.0.insert((owner, axis));
                }
            }
        }
        scrollbars
    }

    /// Untransformed content-box dimensions on the queryable physical axes.
    pub fn query_container_sizes(&self) -> impl Iterator<Item = (N, [Option<f32>; 2])> + '_ {
        let world = &self.prepared.world;
        world
            .boxes
            .iter()
            .enumerate()
            .filter_map(|(index, layout_box)| {
                let source = layout_box.source()?;
                let axes = layout_box.size_query_axes()?;
                let size = crate::overflow::query_content_box_size(world, world.boxes.id_at(index));
                Some((
                    source,
                    [
                        axes.width.then_some(size.width),
                        axes.height.then_some(size.height),
                    ],
                ))
            })
    }
}

pub fn prepare_layout_pass<S, R>(
    source: &S,
    styles: &mut R,
    request: LayoutPassRequest,
) -> Result<PreparedLayoutPass<S::NodeId>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    prepare_layout_pass_reusing(source, styles, request, None, None)
}

pub fn prepare_layout_pass_reusing<S, R>(
    source: &S,
    styles: &mut R,
    request: LayoutPassRequest,
    retained: Option<Box<RetainedLayoutWorld<S::NodeId>>>,
    changes: Option<&[LayoutSourceChange<S::NodeId>]>,
) -> Result<PreparedLayoutPass<S::NodeId>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    let started = Instant::now();
    let phase_started = Instant::now();
    let reusable = retained.filter(|retained| {
        request.retains_layout_world()
            && retained.can_reuse_layout()
            && retained.world.environment == request.environment
    });
    let reused = if let Some(retained) = reusable {
        let viewport_changed = retained.viewport != request.viewport;
        // Paint metadata indexes the previous publication. Retire it before
        // updating the world, even when the update later requires a cold pass.
        let mut world = retained.into_layout_world();
        // DOM damage does not describe media-query or viewport-unit changes.
        // Revalidate every style while preserving boxes whose inputs still match.
        let changes = if viewport_changed { None } else { changes };
        if viewport_changed {
            // Shaped IFC inputs can survive; measured sizes and line placement
            // must be checked against the new viewport constraints.
            world.inline_numeric_reuse = None;
        }
        crate::builder::update_layout_world(source, styles, &mut world, changes)?.map(
            |(count, pruned, reuse_numeric_layout)| {
                (
                    world,
                    count,
                    pruned,
                    reuse_numeric_layout && !viewport_changed,
                )
            },
        )
    } else {
        None
    };
    let (mut world, retained_box_count, pruned_source_count, reuse_numeric_layout) = match reused {
        Some(reused) => reused,
        None => (build_layout_world(source, styles)?, 0, 0, false),
    };
    world.environment = request.environment;
    if request.viewport.mobile_frame_size().is_some() {
        // Mobile viewport scrollbars overlay content. Classic scrollbar
        // gutters would resize the CSS layout in response to frame overflow.
        world.viewport_scroll_policy.suppress_classic_scrollbars();
    }
    let box_tree_elapsed = phase_started.elapsed();
    debug_assert!(
        world
            .viewport_scroll_policy
            .defining_body()
            .is_none_or(|body| world.box_by_id(body).is_some()),
        "viewport policy must retain only a live pass-local body box identity"
    );
    let phase_started = Instant::now();
    prepare_list_markers(&mut world)?;
    let list_marker_elapsed = phase_started.elapsed();
    let phase_started = Instant::now();
    prepare_form_controls(&mut world, styles)?;
    let form_control_elapsed = phase_started.elapsed();
    Ok(PreparedLayoutPass {
        world,
        request,
        started,
        box_tree_elapsed,
        list_marker_elapsed,
        form_control_elapsed,
        retained_box_count,
        pruned_source_count,
        reuse_numeric_layout,
    })
}

pub fn finish_prepared_layout_pass<S, F>(
    prepared: PreparedLayoutPass<S::NodeId>,
    source: &S,
    services: &mut DocumentLayoutServices,
    frames: &mut F,
) -> Result<LayoutPassResult<S::NodeId>, LayoutError>
where
    S: LayoutSource,
    F: EmbeddedFrameRenderer<S::NodeId>,
{
    finish_measured_layout_pass(
        measure_prepared_layout_pass(prepared, services)?,
        source,
        frames,
        crate::InlineSvgConsumers::default(),
    )
}

pub fn measure_prepared_layout_pass<N>(
    prepared: PreparedLayoutPass<N>,
    services: &mut DocumentLayoutServices,
) -> Result<MeasuredLayoutPass<N>, LayoutError>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    measure_prepared_layout_pass_with_scrollbars(prepared, services, &AutoScrollbars::default())
}

fn measure_prepared_layout_pass_with_scrollbars<N>(
    mut prepared: PreparedLayoutPass<N>,
    services: &mut DocumentLayoutServices,
    frozen_scrollbars: &AutoScrollbars<N>,
) -> Result<MeasuredLayoutPass<N>, LayoutError>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    let world = &mut prepared.world;
    let retain_inline = prepared.request.retains_layout_world();
    if retain_inline {
        let fonts = services.font_inputs();
        if world.retained_inline_fonts.as_deref() != Some(&fonts) {
            for layout_box in world.boxes.iter_mut() {
                layout_box.inline_layout = None;
            }
            world.inline_numeric_reuse = None;
            world.retained_inline_fonts = Some(Box::new(fonts));
            prepared.reuse_numeric_layout = false;
        }
        if let Some(numeric) = &mut world.inline_numeric_reuse {
            numeric.begin_pass();
        } else {
            world.inline_numeric_reuse = Some(Box::new(crate::inline::InlineNumericReuse::new()));
        }
    }
    let mut invalidated = Vec::new();
    if prepared.reuse_numeric_layout {
        // Revisit cached Taffy branches that cover IFC fragments. Their
        // inline results are restored or rebuilt under the current inputs.
        invalidated.extend(world.boxes.keyed().filter_map(|(id, layout_box)| {
            layout_box.inline_context_owner.is_some().then_some(id)
        }));
    }
    world.validate_invariants()?;
    let phase_started = Instant::now();
    let reused_inline_contexts = if retain_inline {
        crate::inline::prepare_inline_contexts_with_retained_inputs(world, services, true)
    } else {
        prepare_inline_contexts(world, services);
        0
    };
    let inline_preparation_elapsed = phase_started.elapsed();
    let phase_started = Instant::now();
    let (numeric_metrics, overflow_projection) = compute_world_layout_with_scrollbars(
        world,
        prepared.request.viewport,
        frozen_scrollbars,
        prepared.reuse_numeric_layout,
        invalidated,
    )?;
    let numeric_layout_elapsed = phase_started.elapsed();
    if moli_trace::cpu_profile_enabled() {
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "layout_numeric_cache",
            box_count = world.len(),
            cache_count = world.box_caches.len(),
            cache_capacity = world.box_caches.capacity(),
            box_index_slots = world.len(),
            // These fields belong to the box allocation, not a separate index.
            embedded_index_field_bytes = world.boxes.capacity()
                * std::mem::size_of::<Option<std::num::NonZeroU32>>(),
            estimated_cache_bytes = world.box_caches.capacity()
                * std::mem::size_of::<taffy::Cache>(),
            retained_inline_layout_hits = world.inline_numeric_reuse.as_deref()
                .map_or(0, crate::inline::InlineNumericReuse::hits),
            retained_inline_measurement_hits = world.inline_numeric_reuse.as_deref()
                .map_or(0, crate::inline::InlineNumericReuse::measurement_hits),
        );
    }
    Ok(MeasuredLayoutPass {
        prepared,
        inline_preparation_elapsed,
        reused_inline_contexts,
        numeric_layout_elapsed,
        numeric_metrics,
        overflow_projection,
    })
}

pub fn finish_measured_layout_pass<S, F>(
    measured: MeasuredLayoutPass<S::NodeId>,
    source: &S,
    frames: &mut F,
    svg: crate::InlineSvgConsumers<'_, S::NodeId>,
) -> Result<LayoutPassResult<S::NodeId>, LayoutError>
where
    S: LayoutSource,
    F: EmbeddedFrameRenderer<S::NodeId>,
{
    let MeasuredLayoutPass {
        prepared:
            PreparedLayoutPass {
                world,
                request,
                started,
                box_tree_elapsed,
                list_marker_elapsed,
                form_control_elapsed,
                retained_box_count,
                pruned_source_count,
                ..
            },
        inline_preparation_elapsed,
        reused_inline_contexts,
        numeric_layout_elapsed,
        numeric_metrics,
        overflow_projection,
    } = measured;
    let mut embedded_viewports = HashMap::new();
    let mut svg_viewports = HashMap::new();
    let mut frozen_inline_svg = None;
    for (index, layout_box) in world.boxes.iter().enumerate() {
        let Some(kind) = layout_box
            .element_semantics()
            .and_then(|element| element.replaced)
        else {
            continue;
        };
        let Some(source) = layout_box.source() else {
            continue;
        };
        if kind == crate::LayoutReplacedKind::Svg {
            let size = svg_viewport_size(layout_box);
            if size.width > 0.0
                && size.height > 0.0
                && let std::collections::hash_map::Entry::Vacant(entry) =
                    svg_viewports.entry(source)
            {
                entry.insert(InlineSvgViewport {
                    owner: crate::LayoutOutputBoxId::from_index(index),
                    size,
                    prepared: false,
                });
                if let Some(input) = svg
                    .freeze
                    .and_then(|freeze| freeze(source, &layout_box.style, size))
                {
                    frozen_inline_svg
                        .get_or_insert_with(|| Box::new(FrozenInlineSvgInputs::new()))
                        .insert(source, input);
                }
            }
        }
        if kind != crate::LayoutReplacedKind::Frame {
            continue;
        }
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
        if request.frame_coverage == LayoutFrameCoverage::PositiveViewport
            && (width <= 0.0 || height <= 0.0)
        {
            continue;
        }
        let viewport = LayoutViewport::new(
            css_viewport_dimension(width),
            css_viewport_dimension(height),
            request.viewport.device_pixel_ratio,
        );
        embedded_viewports.insert(world.boxes.id_at(index), (source, viewport));
    }
    let mut result = finish_layout_pass(
        world,
        source,
        request.viewport,
        request.reason,
        started,
        request.paint_capture,
        request.paint_cull_rect,
        request.retains_scroll_projection(),
        request.retains_screen_paint(),
        request.retains_layout_world(),
        frozen_inline_svg,
        svg.paint,
        embedded_viewports,
        frames,
        overflow_projection,
        crate::projection::LayoutPassPhaseMetrics {
            box_tree_elapsed,
            list_marker_elapsed,
            form_control_elapsed,
            inline_preparation_elapsed,
            reused_inline_contexts,
            numeric_layout_elapsed,
            numeric_first_pass_elapsed: numeric_metrics.first_pass_elapsed,
            numeric_followup_passes_elapsed: numeric_metrics.followup_passes_elapsed,
            overflow_detection_elapsed: numeric_metrics.overflow_detection_elapsed,
            scrollbar_feedback_elapsed: numeric_metrics.scrollbar_feedback_elapsed,
            numeric_layout_pass_count: numeric_metrics.pass_count,
            numeric_feedback_invalidated_node_count: numeric_metrics
                .feedback_invalidated_node_count,
            numeric_feedback_overflow_recomputed_node_count: numeric_metrics
                .feedback_overflow_recomputed_node_count,
            retained_box_count,
            pruned_source_count,
            retained_numeric_layout_hits: numeric_metrics.retained_layout_hits,
            retained_numeric_tree: numeric_metrics.reused_tree,
            reused_overflow_projection: numeric_metrics.reused_overflow_projection,
        },
    )?;
    result.tree.svg_viewports = svg_viewports;
    Ok(result)
}

/// Resolves `overflow:auto` with a monotonic browser-style feedback loop.
///
/// The first iteration lays out without an automatic gutter. If overflow
/// reveals a scrollbar, its gutter changes the available space and the
/// affected box must be laid out again; that gutter can in turn reveal the
/// perpendicular scrollbar. An axis is only ever revealed, never hidden, so
/// each box/axis changes at most once and the loop converges.
///
/// Every iteration enters Taffy through the synthetic viewport root, but that
/// is a scheduling entry point rather than an unconditional full-tree layout.
/// `invalidate_scrollbar_feedback` clears only changed boxes and their numeric
/// ancestors. All other subtrees keep valid Taffy cache entries and return
/// immediately. This is the same important boundary as Blink's corrective
/// scrollbar relayout: pay a follow-up pass only after state changes, and only
/// recompute the paths whose available space can have changed.
#[derive(Default)]
struct NumericLayoutMetrics {
    pass_count: usize,
    first_pass_elapsed: Duration,
    followup_passes_elapsed: Duration,
    overflow_detection_elapsed: Duration,
    scrollbar_feedback_elapsed: Duration,
    feedback_invalidated_node_count: usize,
    feedback_overflow_recomputed_node_count: usize,
    retained_layout_hits: usize,
    reused_tree: bool,
    reused_overflow_projection: bool,
}

impl NumericLayoutMetrics {
    fn accumulate(&mut self, previous: Self) {
        self.pass_count += previous.pass_count;
        self.first_pass_elapsed += previous.first_pass_elapsed;
        self.followup_passes_elapsed += previous.followup_passes_elapsed;
        self.overflow_detection_elapsed += previous.overflow_detection_elapsed;
        self.scrollbar_feedback_elapsed += previous.scrollbar_feedback_elapsed;
        self.feedback_invalidated_node_count += previous.feedback_invalidated_node_count;
        self.feedback_overflow_recomputed_node_count +=
            previous.feedback_overflow_recomputed_node_count;
        self.retained_layout_hits += previous.retained_layout_hits;
    }
}

fn compute_world_layout_with_scrollbars<N>(
    world: &mut crate::LayoutWorld<N>,
    viewport: LayoutViewport,
    frozen_scrollbars: &AutoScrollbars<N>,
    reuse_numeric_layout: bool,
    mut invalidated: Vec<crate::LayoutBoxId>,
) -> Result<(NumericLayoutMetrics, Option<OverflowProjection>), LayoutError>
where
    N: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    let root_id = world.root;
    let defining_body = world.viewport_scroll_policy.defining_body();
    let scrollbars_hidden = world.environment.scrollbars_hidden;
    world
        .viewport_scroll_policy
        .prepare_scrollbar_layout(scrollbars_hidden);
    for axis in SCROLLBAR_AXES {
        if !scrollbars_hidden
            && frozen_scrollbars
                .0
                .contains(&(ScrollbarOwner::Viewport, axis))
        {
            world
                .viewport_scroll_policy
                .reveal_auto_scrollbar(axis, true);
        }
    }
    for (id, layout_box) in world.boxes.keyed_mut() {
        if reuse_numeric_layout
            && (layout_box.inline_formatting_context
                || layout_box.inline_context_owner.is_some()
                // Grid track details and table used geometry belong to this
                // numeric pass even when box construction was retained.
                || layout_box.style.display().is_grid_container()
                || crate::table::is_table_root(layout_box.kind)
                || SCROLLBAR_AXES
                    .into_iter()
                    .any(|axis| layout_box.style.auto_scrollbar_revealed(axis)))
        {
            invalidated.push(id);
        }
        if id == root_id {
            layout_box.style.prepare_viewport_root_layout();
        } else if defining_body == Some(id) {
            layout_box.style.prepare_viewport_defining_body_layout();
        } else {
            layout_box
                .style
                .prepare_scrollbar_layout(false, scrollbars_hidden);
            if !scrollbars_hidden
                && !frozen_scrollbars.0.is_empty()
                && let Some(owner) = ScrollbarOwner::for_box(layout_box)
            {
                for axis in SCROLLBAR_AXES {
                    if frozen_scrollbars.0.contains(&(owner, axis)) {
                        layout_box.style.reveal_auto_scrollbar(axis, false, true);
                    }
                }
            }
        }
    }

    let mut metrics = NumericLayoutMetrics::default();
    let phase_started = Instant::now();
    let mut prepared = prepare_world_layout(world, viewport, reuse_numeric_layout)?;
    if reuse_numeric_layout {
        world.track_retained_cache_reads();
        // Preserve local geometry for cache hits. Recompute the root under
        // the viewport's initial gutter state, which can differ between passes.
        prepared.invalidate_cached_layout_paths(world, &invalidated, true);
    }
    metrics.reused_tree = prepared.reused_tree;
    let preparation_elapsed = phase_started.elapsed();
    let mut overflow_projection = prepared
        .tree
        .overflow
        .take()
        .filter(|_| reuse_numeric_layout)
        .map(|projection| *projection);
    let mut pending_feedback_seeds: Vec<_> = std::mem::take(&mut prepared.tree.overflow_dirty)
        .into_iter()
        .collect();
    if overflow_projection.is_some() {
        // The viewport and prior auto-gutter boxes were reset before this
        // iteration. Their local geometry may change even on a cache hit.
        pending_feedback_seeds.push(root_id);
        pending_feedback_seeds.extend(invalidated);
    }
    metrics.reused_overflow_projection = overflow_projection.is_some();
    loop {
        metrics.pass_count = metrics.pass_count.saturating_add(1);
        let phase_started = Instant::now();
        let mut touched = compute_prepared_world_layout(world, viewport, &mut prepared);
        let elapsed = phase_started.elapsed();
        if metrics.pass_count == 1 {
            metrics.first_pass_elapsed = preparation_elapsed + elapsed;
        } else {
            metrics.followup_passes_elapsed += elapsed;
        }
        // Hidden controls cannot introduce an automatic gutter. Final output
        // projection still computes scroll extents, so this demand needs no
        // scrollbar-feedback overflow projection or corrective numeric pass.
        if scrollbars_hidden {
            if let Some(projection) = &mut overflow_projection {
                let phase_started = Instant::now();
                touched.append(&mut pending_feedback_seeds);
                let affected = projection.refresh(world, viewport, &touched);
                metrics.feedback_overflow_recomputed_node_count += affected.len();
                metrics.overflow_detection_elapsed += phase_started.elapsed();
            }
            break;
        }
        let phase_started = Instant::now();
        touched.append(&mut pending_feedback_seeds);
        let candidates = if let Some(projection) = overflow_projection.as_mut() {
            let affected = projection.refresh(world, viewport, &touched);
            metrics.feedback_overflow_recomputed_node_count = metrics
                .feedback_overflow_recomputed_node_count
                .saturating_add(affected.len());
            affected
        } else {
            let projection = OverflowProjection::new(world, viewport);
            let candidates = (0..projection.len())
                .map(|index| world.boxes.id_at(index))
                .collect();
            overflow_projection = Some(projection);
            candidates
        };
        let projection = overflow_projection
            .as_ref()
            .expect("overflow projection is initialized above");
        let candidates = candidates
            .into_iter()
            .map(|id| (id, projection.overflowing_axes(world, id)))
            .collect::<Vec<_>>();
        metrics.overflow_detection_elapsed += phase_started.elapsed();
        let phase_started = Instant::now();
        let mut viewport_changed = false;
        let mut changed_boxes = Vec::new();
        for (id, (overflow_x, overflow_y)) in candidates {
            if id == root_id {
                viewport_changed |= world
                    .viewport_scroll_policy
                    .reveal_auto_scrollbar(LayoutScrollbarAxis::Horizontal, overflow_x);
                viewport_changed |= world
                    .viewport_scroll_policy
                    .reveal_auto_scrollbar(LayoutScrollbarAxis::Vertical, overflow_y);
                continue;
            }
            if defining_body == Some(id) {
                continue;
            }
            let layout_box = &mut world.boxes[id];
            let mut box_changed = layout_box.style.reveal_auto_scrollbar(
                LayoutScrollbarAxis::Horizontal,
                false,
                overflow_x,
            );
            box_changed |= layout_box.style.reveal_auto_scrollbar(
                LayoutScrollbarAxis::Vertical,
                false,
                overflow_y,
            );
            if box_changed {
                changed_boxes.push(id);
            }
        }
        let changed = viewport_changed || !changed_boxes.is_empty();
        if changed {
            // Do not clear the whole Taffy tree here. Although the next call to
            // `compute_prepared_world_layout` starts at the root, untouched
            // branches remain cache hits; only these boxes and their ancestor
            // paths participate in the corrective numeric layout.
            metrics.feedback_invalidated_node_count = metrics
                .feedback_invalidated_node_count
                .saturating_add(prepared.invalidate_cached_layout_paths(
                    world,
                    &changed_boxes,
                    viewport_changed,
                ));
            pending_feedback_seeds.extend(changed_boxes.iter().copied());
            if viewport_changed {
                pending_feedback_seeds.push(root_id);
            }
        }
        metrics.scrollbar_feedback_elapsed += phase_started.elapsed();
        if !changed {
            break;
        }
    }
    if world.inline_numeric_reuse.is_some() {
        world.numeric_tree = Some(prepared.tree);
    }
    metrics.retained_layout_hits = world.finish_retained_cache_reads();
    Ok((metrics, overflow_projection))
}

fn css_viewport_dimension(value: f32) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(0.0, u32::MAX as f32) as u32
}
