//! Builds output from completed numeric layout and clamped scroll geometry.
//!
//! Numeric locations remain in parent layout CSS pixels. Positioning adds only
//! the node's local delta; ancestor translations arrive through the parent space.
//! Layout metrics separately accumulate those positioning deltas without authored
//! CSS transforms. Visual document mappings include element scrolling but exclude
//! viewport scrolling; viewport mappings include both. Paint derives from the same
//! placement, with its own snapping and property-space boundaries.
//!
//! Overflow feedback stays independent of this output pass. Projection publishes
//! a FrozenLayoutTree and can retain the inputs for an active screen paint
//! consumer. Clip inheritance and paint order remain separate relations.

mod coordinates;
mod paint_space;
mod paint_view;
mod scroll;
mod sticky;

use coordinates::{BoxPlacement, resolve_coordinate_spaces};
pub(crate) use paint_space::PaintSpace;
pub(crate) use paint_view::PaintProjection;
use paint_view::{BoxPaintMetadata, InlineSvgPaint, PaintMetadata};
use scroll::clamp_scroll_offset;
pub(crate) use scroll::{ScrollProjection, ScrollTranslation};

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
    time::Instant,
};

use taffy::ResolveOrZero;

use crate::layout_tree::{CssSizing, CssSizingBox, FrozenInlineSvgInputs, LayoutCoordinateSpace};
use crate::overflow::{OverflowProjection, inset_rect, offset_rect, outset_rect};
use crate::stacking::{PaintOrderEvent, build_paint_order};
use crate::style::ResolvedLayoutTransform;
use crate::{
    FrozenCoordinateSpace, FrozenLayoutBox, FrozenLayoutTree, LayoutAnonymousReason,
    LayoutBoxGeometry, LayoutBoxId, LayoutClipChainId, LayoutClipKind, LayoutClipNode,
    LayoutCoordinateSpaceId, LayoutError, LayoutFlushReason, LayoutFragment,
    LayoutFragmentBoxModel, LayoutFragmentId, LayoutFragmentKind, LayoutOutputBoxId,
    LayoutPassMetrics, LayoutPassResult, LayoutPoint, LayoutRect, LayoutScrollExtent,
    LayoutScrollbarAxis, LayoutScrollbarGeometry, LayoutSize, LayoutTransform2D, LayoutViewport,
    LayoutWorld, PaintCaptureRequest, PaintDiagnostic, PaintDiagnosticSeverity, PaintPixelGrid,
    PaintSnapshot, RetainedLayoutWorld,
};

pub(crate) struct LayoutPassPhaseMetrics {
    pub(crate) box_tree_elapsed: std::time::Duration,
    pub(crate) list_marker_elapsed: std::time::Duration,
    pub(crate) form_control_elapsed: std::time::Duration,
    pub(crate) inline_preparation_elapsed: std::time::Duration,
    pub(crate) numeric_layout_elapsed: std::time::Duration,
    pub(crate) numeric_first_pass_elapsed: std::time::Duration,
    pub(crate) numeric_followup_passes_elapsed: std::time::Duration,
    pub(crate) overflow_detection_elapsed: std::time::Duration,
    pub(crate) scrollbar_feedback_elapsed: std::time::Duration,
    pub(crate) numeric_layout_pass_count: usize,
    pub(crate) numeric_feedback_invalidated_node_count: usize,
    pub(crate) numeric_feedback_overflow_recomputed_node_count: usize,
    pub(crate) retained_box_count: usize,
    pub(crate) pruned_source_count: usize,
    pub(crate) retained_numeric_layout_hits: usize,
    pub(crate) retained_numeric_tree: bool,
    pub(crate) reused_inline_contexts: usize,
    pub(crate) reused_overflow_projection: bool,
}

pub(crate) fn finish_layout_pass<N, S, F>(
    mut world: LayoutWorld<N>,
    source: &S,
    viewport: LayoutViewport,
    reason: LayoutFlushReason,
    started: Instant,
    paint_capture: Option<PaintCaptureRequest>,
    paint_cull_rect: Option<LayoutRect>,
    retain_scroll_projection: bool,
    retain_screen_paint: bool,
    retain_layout_world: bool,
    frozen_inline_svg: Option<Box<FrozenInlineSvgInputs<N>>>,
    svg_image_lookup: Option<&crate::InlineSvgImageLookup<'_, N>>,
    mut embedded_viewports: HashMap<LayoutBoxId, (N, LayoutViewport)>,
    frames: &mut F,
    overflow_projection: Option<OverflowProjection>,
    phase_metrics: LayoutPassPhaseMetrics,
) -> Result<LayoutPassResult<N>, LayoutError>
where
    N: Copy + Debug + Eq + Hash,
    S: crate::LayoutSource<NodeId = N>,
    F: crate::EmbeddedFrameRenderer<N>,
{
    let projection_started = Instant::now();
    // The final scrollbar iteration used these exact rounded box dimensions.
    // A pass with hidden scrollbars builds overflow here for the first time.
    let overflow = overflow_projection.unwrap_or_else(|| OverflowProjection::new(&world, viewport));
    let mut projection = OutputProjection::new(&world, viewport);
    projection.build_local_box_geometry(&overflow);
    projection.build_scroll_extents(&overflow);
    let scroll_projection =
        projection.build_coordinate_spaces(paint_capture.is_some(), retain_scroll_projection)?;
    projection.build_fragments();
    projection.assign_clip_and_paint_order();

    let content_size = projection.document_content_size();
    let mut embedded_frames = HashMap::new();
    let mut embedded_frame_elapsed = std::time::Duration::ZERO;
    let inline_svg_images = RefCell::new(HashMap::new());
    let mut paint_projection_metrics = crate::paint::PaintProjectionMetrics::default();
    let mut paint_snapshot = if let Some(request) = paint_capture {
        let capture = request.resolve(viewport, projection.viewport_scroll, content_size)?;
        let css_images = |url: &str| source.css_image_state(url);
        let inline_svg = svg_image_lookup.map(|lookup| InlineSvgPaint {
            root: source.root(),
            frozen: frozen_inline_svg.as_deref(),
            lookup,
            images: &inline_svg_images,
        });
        let paint_projection =
            PaintProjection::from_layout_pass(&projection, &css_images, inline_svg);
        let mut paint_frame = |id, cull_rect| {
            let Some((source, child_viewport)) = embedded_viewports.remove(&id) else {
                return Ok(None);
            };
            let started = Instant::now();
            let child = frames.render_embedded_frame(
                source,
                child_viewport,
                Some(crate::EmbeddedFramePaintRequest {
                    include_backgrounds: capture.include_backgrounds,
                    cull_rect,
                }),
            )?;
            embedded_frame_elapsed += started.elapsed();
            Ok(child.and_then(|mut child| {
                let paint = child.paint.take();
                embedded_frames.insert(id, (source, child));
                paint
            }))
        };
        let output = crate::paint::project_paint_snapshot(
            &paint_projection,
            capture,
            content_size,
            (reason == LayoutFlushReason::Print)
                .then(|| crate::paint::LinkProjection::new(&paint_projection, source)),
            paint_cull_rect,
            &mut paint_frame,
        )?;
        paint_projection_metrics = output.metrics;
        Some(output.snapshot)
    } else {
        None
    };

    // A clipped frame still publishes its geometry and resource discovery.
    let frames_started = Instant::now();
    for (id, (source, viewport)) in embedded_viewports {
        if let Some(child) = frames.render_embedded_frame(source, viewport, None)? {
            embedded_frames.insert(id, (source, child));
        }
    }
    embedded_frame_elapsed += frames_started.elapsed();
    let mut css_image_references = world.css_image_references.clone();
    for (_, child) in embedded_frames.values_mut() {
        css_image_references.append(&mut child.css_image_references);
    }
    let mut diagnostics = paint_snapshot
        .as_ref()
        .map(|snapshot| snapshot.diagnostics.clone())
        .unwrap_or_default();
    for diagnostic in &projection.diagnostics {
        if !diagnostics.contains(diagnostic) {
            diagnostics.push(diagnostic.clone());
        }
    }
    if let Some(snapshot) = paint_snapshot.as_mut() {
        snapshot.content_size = content_size;
        for diagnostic in &projection.diagnostics {
            if !snapshot.diagnostics.contains(diagnostic) {
                snapshot.push_diagnostic(diagnostic.clone());
            }
        }
    }
    let metrics = LayoutPassMetrics {
        reason,
        elapsed: started.elapsed(),
        box_tree_elapsed: phase_metrics.box_tree_elapsed,
        list_marker_elapsed: phase_metrics.list_marker_elapsed,
        form_control_elapsed: phase_metrics.form_control_elapsed,
        inline_preparation_elapsed: phase_metrics.inline_preparation_elapsed,
        numeric_layout_elapsed: phase_metrics.numeric_layout_elapsed,
        numeric_first_pass_elapsed: phase_metrics.numeric_first_pass_elapsed,
        numeric_followup_passes_elapsed: phase_metrics.numeric_followup_passes_elapsed,
        overflow_detection_elapsed: phase_metrics.overflow_detection_elapsed,
        scrollbar_feedback_elapsed: phase_metrics.scrollbar_feedback_elapsed,
        embedded_frame_elapsed,
        projection_elapsed: projection_started
            .elapsed()
            .saturating_sub(embedded_frame_elapsed),
        numeric_layout_pass_count: phase_metrics.numeric_layout_pass_count,
        numeric_feedback_invalidated_node_count: phase_metrics
            .numeric_feedback_invalidated_node_count,
        numeric_feedback_overflow_recomputed_node_count: phase_metrics
            .numeric_feedback_overflow_recomputed_node_count,
        box_count: projection.boxes.len(),
        retained_box_count: phase_metrics.retained_box_count,
        pruned_source_count: phase_metrics.pruned_source_count,
        retained_embedded_box_count: embedded_frames
            .values()
            .map(|(_, snapshot)| snapshot.retained_box_count)
            .sum(),
        retained_numeric_layout_hits: phase_metrics.retained_numeric_layout_hits,
        retained_numeric_tree: phase_metrics.retained_numeric_tree,
        reused_inline_contexts: phase_metrics.reused_inline_contexts,
        reused_overflow_projection: phase_metrics.reused_overflow_projection,
        fragment_count: projection.fragments.len(),
        paint_event_count: paint_projection_metrics.event_count,
        paint_culled_event_count: paint_projection_metrics.culled_event_count,
        paint_text_line_count: paint_projection_metrics.text_line_count,
        paint_culled_text_line_count: paint_projection_metrics.culled_text_line_count,
        paint_operation_count: paint_snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.fragments.len()),
        fallback_count: diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == PaintDiagnosticSeverity::Warning)
            .count(),
    };
    let mut screen_frames = HashMap::new();
    let embedded_frames = embedded_frames
        .into_iter()
        .map(|(id, (source, snapshot))| {
            if let Some(retained) = snapshot.retained_layout {
                screen_frames.insert(source, (id, retained));
            }
            crate::FrozenEmbeddedFrame {
                source,
                tree: Box::new(snapshot.tree),
            }
        })
        .collect();
    let paint = retain_screen_paint.then(|| {
        Box::new(RetainedPaintData {
            paint_metadata: std::mem::take(&mut projection.paint_metadata),
            diagnostics: projection.diagnostics.clone(),
            inline_svg_images,
        })
    });
    let mut tree = projection.into_frozen_tree(
        source.root(),
        content_size,
        embedded_frames,
        scroll_projection,
    );
    tree.frozen_inline_svg = frozen_inline_svg;
    let reusable_layout = retain_layout_world && crate::builder::can_retain_root(&world);
    let retained_layout = (reusable_layout || paint.is_some()).then(|| {
        world.native_grid_details = HashMap::new();
        world.resolved_grid_tracks = HashMap::new();
        if !reusable_layout || world.numeric_tree.is_none() {
            world.table_structures = HashMap::new();
        }
        if !reusable_layout {
            world.release_numeric_cache_storage();
        } else if let Some(tree) = &mut world.numeric_tree {
            tree.overflow = Some(Box::new(overflow));
        }
        if paint.is_none() {
            world.release_paint_storage();
        } else {
            // Replaced images with live inputs are sampled during paint.
            // Inline SVG scenes belong to the paint resource cache.
            for layout_box in world.boxes.values_mut() {
                if layout_box.source.is_some()
                    && layout_box
                        .element_semantics()
                        .and_then(|element| element.replaced)
                        .is_some_and(crate::LayoutReplacedKind::is_live_paint_image)
                {
                    layout_box.replaced_image = None;
                }
            }
        }
        Box::new(RetainedLayoutWorld {
            world,
            viewport,
            reusable_layout,
            frames: screen_frames,
            paint,
        })
    });
    Ok(LayoutPassResult::new(
        tree,
        diagnostics,
        metrics,
        paint_snapshot,
        css_image_references,
        retained_layout,
    ))
}

/// Additional inputs for painting one published screen layout.
pub(crate) struct RetainedPaintData {
    diagnostics: Vec<PaintDiagnostic>,
    paint_metadata: PaintMetadata,
    inline_svg_images: RefCell<HashMap<LayoutBoxId, Option<crate::LayoutImageResource>>>,
}

impl<N: Copy + Debug + Eq + Hash> RetainedLayoutWorld<N> {
    pub fn clear_inline_svg_images(&mut self) {
        if let Some(paint) = &mut self.paint {
            paint.inline_svg_images.get_mut().clear();
        }
        for (_, frame) in self.frames.values_mut() {
            frame.clear_inline_svg_images();
        }
    }

    /// Apply canonical REPAINT damage while retaining the published geometry.
    /// Prepare all replacements before changing the world, so an unsupported
    /// box or brush can fall back to the regular pass.
    pub fn update_repaint_styles(
        &mut self,
        styles: Vec<(N, style::servo_arc::Arc<style::properties::ComputedValues>)>,
    ) -> bool {
        self.update_published_styles(styles, |_| Some(())).is_some()
    }

    /// Refresh coordinates, overflow, clips, and paint order over the same
    /// numeric layout. Fall back if an automatic gutter or layout role would
    /// change the known numeric geometry.
    pub fn project_screen_styles(
        &mut self,
        old: &FrozenLayoutTree<N>,
        styles: Vec<(N, style::servo_arc::Arc<style::properties::ComputedValues>)>,
        scroll_offset: impl Fn(N) -> LayoutPoint,
    ) -> Option<FrozenLayoutTree<N>> {
        if old.embedded_frames().next().is_some()
            || self
                .world
                .boxes
                .values()
                .any(|layout_box| layout_box.style.display().is_grid_container())
        {
            return None;
        }
        let include_paint = self.paint.is_some();
        let viewport = self.viewport;
        let (tree, diagnostics, metadata) = self.update_published_styles(styles, |world| {
            let overflow = OverflowProjection::new(world, viewport);
            if !projection_preserves_scrollbar_layout(world, &overflow) {
                return None;
            }
            let mut projection = OutputProjection::new(world, viewport);
            projection.build_local_box_geometry(&overflow);
            projection.build_scroll_extents(&overflow);
            let scroll = projection.build_coordinate_spaces(false, true).ok()?;
            projection.build_fragments();
            projection.assign_clip_and_paint_order();
            let size = projection.document_content_size();
            let metadata = include_paint.then(|| std::mem::take(&mut projection.paint_metadata));
            let diagnostics = include_paint.then(|| projection.diagnostics.clone());
            let mut tree = projection.into_frozen_tree(old.source_root(), size, Vec::new(), scroll);
            tree.svg_viewports.clone_from(&old.svg_viewports);
            tree.clear_inline_svg_geometry();
            tree.refresh_scroll_projection(scroll_offset).ok()?;
            Some((tree, diagnostics, metadata))
        })?;
        if let Some(paint) = &mut self.paint {
            paint.diagnostics = diagnostics.expect("existing paint diagnostics");
            paint.paint_metadata = metadata.expect("existing paint metadata");
        }
        Some(tree)
    }

    fn update_published_styles<T>(
        &mut self,
        styles: Vec<(N, style::servo_arc::Arc<style::properties::ComputedValues>)>,
        project: impl FnOnce(&LayoutWorld<N>) -> Option<T>,
    ) -> Option<T> {
        let world = &mut self.world;
        let mut replacements = HashMap::new();
        for (source, computed) in styles {
            let &id = world.source_mapping.get(&source)?;
            let old = &world.boxes[id];
            let next = crate::ResolvedLayoutStyle::from_stylo(computed);
            if old.element_semantics.as_ref().is_none_or(|element| {
                element.namespace != crate::LayoutNamespace::Html
                    || element.category != crate::LayoutElementCategory::Generic
            }) || crate::table::is_table_root(old.kind)
                || old.kind.is_internal_table_box()
                || old.kind == crate::LayoutBoxKind::TableCaption
                || !old.style.can_repaint_with(&next)
            {
                return None;
            }
            let containing_role = |style: &crate::ResolvedLayoutStyle| {
                let root = world.is_document_element(id);
                let css_box = old.is_css_box();
                let eligible = old.is_eligible_for_paint_or_layout_containment();
                (
                    style.establishes_positioned_containing_block(root, css_box, eligible),
                    style.establishes_fixed_containing_block(root, css_box, eligible),
                )
            };
            if containing_role(&old.style) != containing_role(&next) {
                return None;
            }
            replacements.insert(id, next);
        }

        let mut brush_updates = Vec::new();
        let mut text_styles = Vec::new();
        for (index, layout_box) in world.boxes.ordered().enumerate() {
            if layout_box.kind == crate::LayoutBoxKind::Text
                && let Some(parent) = layout_box.structural_parent
                && let Some(style) = replacements.get(&parent)
            {
                text_styles.push((index, crate::ResolvedLayoutStyle::text_leaf_from(style)));
            }
            if layout_box.source.is_none()
                && layout_box.kind != crate::LayoutBoxKind::Text
                && layout_box
                    .owner
                    .and_then(|node| world.source_mapping.get(&node))
                    .is_some_and(|owner| replacements.contains_key(owner))
            {
                return None;
            }
            let Some(inline) = &layout_box.inline_layout else {
                continue;
            };
            let Some(shaped) = inline
                .laid_out
                .as_ref()
                .or(inline.measurement_layout.as_ref())
            else {
                continue;
            };
            let mut brushes = inline
                .repaint_brushes
                .as_deref()
                .unwrap_or_default()
                .to_vec();
            let mut changed = false;
            for (slot, (shaped, parent)) in shaped
                .styles()
                .iter()
                .zip(&inline.style_parents)
                .enumerate()
            {
                let Some(style) = replacements.get(parent) else {
                    continue;
                };
                let original = inline.paint_brush(slot, &shaped.brush);
                let mut current = style.parley_text_style().brush;
                current.paint = original.paint;
                if &current == original {
                    continue;
                }
                let mut colors_only = original.clone();
                colors_only.color = current.color;
                colors_only.decoration.color = current.decoration.color;
                if colors_only != current || inline.selection.is_some() {
                    return None;
                }
                match brushes.binary_search_by_key(&slot, |(index, _)| *index) {
                    Ok(index) => brushes[index].1 = current,
                    Err(index) => brushes.insert(index, (slot, current)),
                }
                changed = true;
            }
            if changed {
                brush_updates.push((index, brushes.into_boxed_slice()));
            }
        }

        for (&id, replacement) in &mut replacements {
            std::mem::swap(&mut world.boxes[id].style, replacement);
        }
        let Some(result) = project(world) else {
            for (id, original) in replacements {
                world.boxes[id].style = original;
            }
            return None;
        };
        for (id, _previous) in replacements {
            world.clear_layout_caches(id);
            if let Some(numeric) = &mut world.inline_numeric_reuse {
                numeric.discard_owner(id);
            }
        }
        for (index, replacement) in text_styles {
            world.boxes[index].style = replacement;
        }
        for (index, brushes) in brush_updates {
            world.boxes[index]
                .inline_layout
                .as_mut()
                .expect("validated inline context")
                .repaint_brushes = Some(brushes);
        }
        Some(result)
    }

    /// Projects the viewport of the paired tree, using its latest scroll offsets.
    pub fn paint(
        &self,
        tree: &FrozenLayoutTree<N>,
        request: PaintCaptureRequest,
        cull_rect: Option<LayoutRect>,
        css_images: &dyn Fn(N, &str) -> crate::LayoutCssImageState,
        live_replaced_images: &crate::source::LiveReplacedImageLookup<'_, N>,
        svg_image_lookup: &crate::InlineSvgImageLookup<'_, N>,
    ) -> Result<PaintSnapshot, LayoutError> {
        let paint = self
            .paint
            .as_deref()
            .ok_or(LayoutError::PaintProjectionNotRequested)?;
        let scroll = tree.scroll_projection.as_deref().ok_or_else(|| {
            LayoutError::source_contract("screen paint", "missing published box placements")
        })?;
        let world = &self.world;
        if tree.viewport != self.viewport || tree.boxes.len() != world.boxes.len() {
            return Err(LayoutError::source_contract(
                "screen paint",
                "paint inputs and published box placements differ",
            ));
        }
        let coordinates = resolve_coordinate_spaces(
            world.output_id(world.root),
            tree.viewport_scroll,
            tree.boxes.len(),
            Some(PaintPixelGrid::new(tree.viewport.device_pixel_ratio)),
            |index| Ok(scroll.placement(index)),
            |id| {
                let extent = &tree.boxes[id.index()].scroll_extent;
                extent.is_scroll_container.then_some(extent.applied_offset)
            },
        )?;
        let document_images = |url: &str| css_images(tree.source_root(), url);
        let projection = PaintProjection::from_publication(
            world,
            paint,
            tree,
            scroll,
            &coordinates.spaces,
            &document_images,
            live_replaced_images,
            InlineSvgPaint {
                root: tree.source_root(),
                frozen: tree.frozen_inline_svg.as_deref(),
                lookup: svg_image_lookup,
                images: &paint.inline_svg_images,
            },
        );

        let mut frames = HashMap::new();
        for frame in tree.embedded_frames() {
            if frame.tree.viewport.css_width == 0 || frame.tree.viewport.css_height == 0 {
                continue;
            }
            let (id, retained) = self.frames.get(&frame.source).ok_or_else(|| {
                LayoutError::source_contract("screen paint", "missing embedded paint inputs")
            })?;
            frames.insert(*id, (&frame.tree, retained));
        }
        let child_request = PaintCaptureRequest {
            include_backgrounds: request.include_backgrounds,
            base_background_color: crate::PaintColor::TRANSPARENT,
            ..PaintCaptureRequest::viewport()
        };
        let mut paint_frame = |id, child_cull_rect| {
            frames
                .remove(&id)
                .map(|(tree, world)| {
                    world.paint(
                        tree,
                        child_request,
                        child_cull_rect,
                        css_images,
                        live_replaced_images,
                        svg_image_lookup,
                    )
                })
                .transpose()
        };

        let capture = request.resolve(tree.viewport, tree.viewport_scroll, tree.content_size)?;
        crate::paint::project_paint_snapshot(
            &projection,
            capture,
            tree.content_size,
            None,
            cull_rect,
            &mut paint_frame,
        )
        .map(|output| output.snapshot)
    }
}

fn projection_preserves_scrollbar_layout<N: Copy + Debug + Eq + Hash>(
    world: &LayoutWorld<N>,
    overflow: &OverflowProjection,
) -> bool {
    if world.environment.scrollbars_hidden {
        return true;
    }
    let policy = world.viewport_scroll_policy;
    for (index, layout_box) in world.boxes.ordered().enumerate() {
        let id = world.boxes.id_at(index);
        if world.is_viewport_defining_body(id) {
            continue;
        }
        let (horizontal, vertical) = overflow.overflowing_axes(world, id);
        for (axis, overflowing) in [
            (crate::LayoutScrollbarAxis::Horizontal, horizontal),
            (crate::LayoutScrollbarAxis::Vertical, vertical),
        ] {
            use crate::style::LayoutOverflowMode::Auto;
            let auto_gutter = if id == world.root {
                policy.overflow_mode(axis) == Auto && policy.has_scrollbar(axis, overflowing)
            } else {
                layout_box.style.overflow_mode(axis) == Auto
                    && layout_box.style.has_scrollbar(axis, false, overflowing)
            };
            if auto_gutter {
                // A cold numeric pass starts with no auto gutter. A
                // previously admitted or newly required gutter needs that pass.
                return false;
            }
        }
    }
    true
}

/// One coordinate space while output and paint projection are both live.
///
/// The public layout space retains exact CSS geometry for queries. `paint`
/// may deliberately diverge at transform boundaries because Blink rounds the
/// property translation and carries only the transform-safe subpixel residue.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProjectedCoordinateSpace {
    pub(crate) layout: LayoutCoordinateSpace,
    pub(crate) paint: PaintSpace,
}

impl ProjectedCoordinateSpace {
    fn into_layout_space(self) -> LayoutCoordinateSpace {
        self.layout
    }

    pub(crate) fn paint_space(&self, outer: LayoutTransform2D) -> PaintSpace {
        self.paint.with_outer_transform(outer)
    }
}

struct OutputProjection<'a, N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) world: &'a LayoutWorld<N>,
    pub(crate) viewport: LayoutViewport,
    viewport_scroll: LayoutPoint,
    pub(crate) boxes: Vec<LayoutBoxGeometry>,
    box_sources: Vec<Option<N>>,
    principal_sources: Vec<Option<N>>,
    hit_sources: Vec<Option<N>>,
    pub(crate) fragments: Vec<LayoutFragment>,
    scroll_proxy_links: Vec<(N, LayoutOutputBoxId)>,
    pub(crate) scroll_extents: Vec<LayoutScrollExtent>,
    pub(crate) coordinate_spaces: Vec<ProjectedCoordinateSpace>,
    pub(crate) clip_chain: Vec<LayoutClipNode>,
    paint_order_count: usize,
    control_paint_orders: Vec<Option<u32>>,
    pub(crate) diagnostics: Vec<PaintDiagnostic>,
    resolved_transforms: Vec<ResolvedLayoutTransform>,
    scrollable_overflow: Vec<LayoutRect>,
    paint_metadata: PaintMetadata,
}

impl<'a, N> OutputProjection<'a, N>
where
    N: Copy + Debug + Eq + Hash,
{
    fn new(world: &'a LayoutWorld<N>, viewport: LayoutViewport) -> Self {
        let count = world.boxes.len();
        let mut principal_sources = vec![None; count];
        for (source, box_id) in &world.source_mapping {
            principal_sources[world.boxes.index_of(box_id)] = Some(*source);
        }
        let scroll_proxy_links = world
            .display_contents_mapping
            .iter()
            .flat_map(|(source, boxes)| {
                boxes.iter().map(|box_id| {
                    (
                        *source,
                        LayoutOutputBoxId::from_index(world.boxes.index_of(box_id)),
                    )
                })
            })
            .collect();
        Self {
            world,
            viewport,
            viewport_scroll: LayoutPoint::ZERO,
            boxes: Vec::with_capacity(count),
            box_sources: Vec::with_capacity(count),
            principal_sources,
            hit_sources: Vec::with_capacity(count),
            fragments: Vec::new(),
            scroll_proxy_links,
            scroll_extents: Vec::with_capacity(count),
            coordinate_spaces: Vec::new(),
            clip_chain: Vec::new(),
            paint_order_count: 0,
            control_paint_orders: vec![None; count],
            diagnostics: Vec::new(),
            resolved_transforms: vec![ResolvedLayoutTransform::IDENTITY; count],
            scrollable_overflow: vec![LayoutRect::ZERO; count],
            paint_metadata: PaintMetadata {
                boxes: (0..count).map(|_| BoxPaintMetadata::default()).collect(),
                events: Vec::new(),
            },
        }
    }

    fn scrollbar_control_thickness(&self, id: LayoutBoxId) -> f32 {
        if self.world.environment.scrollbars_hidden {
            return 0.0;
        }
        if id == self.world.root {
            self.world
                .viewport_scroll_policy
                .scrollbar_control_thickness()
        } else if self.world.is_viewport_defining_body(id) {
            0.0
        } else {
            self.world.boxes[id].style.scrollbar_control_thickness()
        }
    }

    fn places_vertical_scrollbar_on_left(&self, id: LayoutBoxId) -> bool {
        id != self.world.root
            && !self.world.is_viewport_defining_body(id)
            && self.world.boxes[id]
                .style
                .places_vertical_scrollbar_on_left(false)
    }

    fn establishes_scroll_container(&self, id: LayoutBoxId) -> bool {
        if id == self.world.root {
            self.world
                .viewport_scroll_policy
                .establishes_scroll_container()
        } else if self.world.is_viewport_defining_body(id) {
            false
        } else {
            self.world.boxes[id].style.establishes_scroll_container()
        }
    }

    fn allows_user_scroll(&self, id: LayoutBoxId, axis: LayoutScrollbarAxis) -> bool {
        if id == self.world.root {
            self.world.viewport_scroll_policy.allows_user_scroll(axis)
        } else if self.world.is_viewport_defining_body(id) {
            false
        } else {
            match axis {
                LayoutScrollbarAxis::Horizontal => {
                    self.world.boxes[id].style.allows_user_scroll_x()
                }
                LayoutScrollbarAxis::Vertical => self.world.boxes[id].style.allows_user_scroll_y(),
            }
        }
    }

    fn clips_overflow(&self, id: LayoutBoxId) -> bool {
        if id == self.world.root {
            self.world.viewport_scroll_policy.clips_overflow()
        } else if self.world.is_viewport_defining_body(id) {
            false
        } else {
            self.world.boxes[id].style.clips_overflow()
        }
    }

    fn has_scrollbar(&self, id: LayoutBoxId, axis: LayoutScrollbarAxis, overflowing: bool) -> bool {
        if self.world.environment.scrollbars_hidden {
            return false;
        }
        if id == self.world.root {
            self.world
                .viewport_scroll_policy
                .has_scrollbar(axis, overflowing)
        } else if self.world.is_viewport_defining_body(id) {
            false
        } else {
            self.world.boxes[id]
                .style
                .has_scrollbar(axis, false, overflowing)
        }
    }

    fn scrollbar_colors(&self, id: LayoutBoxId) -> Option<crate::LayoutScrollbarColors> {
        if id == self.world.root {
            self.world.viewport_scroll_policy.scrollbar_colors()
        } else if self.world.is_viewport_defining_body(id) {
            None
        } else {
            self.world.boxes[id].style.scrollbar_colors()
        }
    }

    fn build_local_box_geometry(&mut self, overflow: &OverflowProjection) {
        assert_eq!(overflow.len(), self.world.boxes.len());
        let layout_origins = self.world.global_layout_origins();
        for (index, layout_box) in self.world.boxes.ordered().enumerate() {
            for diagnostic in layout_box.capability_diagnostics() {
                self.diagnostics.push(PaintDiagnostic::new(
                    diagnostic.code(),
                    format!(
                        "{} uses a conservative layout/paint fallback for {}",
                        layout_box.source_label,
                        diagnostic.code()
                    ),
                    PaintDiagnosticSeverity::Warning,
                ));
            }
            let box_id = self.world.boxes.id_at(index);
            let id = LayoutOutputBoxId::from_index(index);
            let (geometry, scrollable_overflow) = overflow.geometry_and_overflow(box_id);
            let resolved = geometry.resolved_transform;
            self.resolved_transforms[index] = resolved;
            if resolved.has_unsupported_3d {
                self.diagnostics.push(PaintDiagnostic::new(
                    "transform-3d-geometry-fallback",
                    format!(
                        "{} contains a 3D transform or perspective that cannot be represented by the Phase 5 2D coordinate space",
                        layout_box.source_label
                    ),
                    PaintDiagnosticSeverity::Warning,
                ));
            }
            self.scrollable_overflow[index] = scrollable_overflow;
            let source = layout_box.source.or_else(|| {
                (layout_box.anonymous_reason
                    == Some(LayoutAnonymousReason::InlineSplitContinuation))
                .then_some(layout_box.owner)
                .flatten()
            });
            self.box_sources.push(source);
            self.hit_sources.push(layout_box.source.or_else(|| {
                (layout_box.pseudo.is_some()
                    || layout_box.anonymous_reason
                        == Some(LayoutAnonymousReason::InlineSplitContinuation))
                .then_some(layout_box.owner)
                .flatten()
            }));
            let semantics = layout_box.element_semantics();
            self.boxes.push(LayoutBoxGeometry {
                id,
                effective_zoom: layout_box.style.effective_zoom(),
                structural_parent: layout_box
                    .structural_parent
                    .map(|parent| LayoutOutputBoxId::from_index(self.world.boxes.index_of(parent))),
                parent: layout_box
                    .parent
                    .map(|parent| LayoutOutputBoxId::from_index(self.world.boxes.index_of(parent))),
                layout_parent: layout_box
                    .layout_parent
                    .map(|parent| LayoutOutputBoxId::from_index(self.world.boxes.index_of(parent))),
                position: layout_box.style.position(),
                coordinate_space: LayoutCoordinateSpaceId::from_index(index + 1),
                clip_chain: None,
                content_box: geometry.content_box,
                padding_box: geometry.padding_box,
                border_box: geometry.border_box,
                margin_box: geometry.margin_box,
                fragments: Vec::new(),
                layout_origin_in_document: layout_origins[index],
                is_body_element: semantics.is_some_and(|element| element.is_html_element("body")),
                is_table_offset_parent: semantics.is_some_and(|element| {
                    element.is_html_element("table")
                        || element.is_html_element("td")
                        || element.is_html_element("th")
                }),
                establishes_positioned_containing_block: self
                    .world
                    .establishes_positioned_containing_block(self.world.boxes.id_at(index)),
                establishes_fixed_containing_block: self
                    .world
                    .establishes_fixed_containing_block(self.world.boxes.id_at(index)),
                visible: layout_box.style.is_visible(),
                pointer_events: layout_box.style.accepts_pointer_events(),
            });
        }
    }

    fn build_scroll_extents(&mut self, overflow_projection: &OverflowProjection) {
        for index in 0..self.world.boxes.len() {
            let id = self.world.boxes.id_at(index);
            let (geometry, overflow) = overflow_projection.geometry_and_overflow(id);
            let is_root = id == self.world.root;
            let scrollbar_thickness = self.scrollbar_control_thickness(id);
            let vertical_scrollbar_on_left = self.places_vertical_scrollbar_on_left(id);
            let layout_scrollport = geometry.local_scrollport;
            let mut scrollport = layout_scrollport;
            let mut scroll_size = overflow_projection.scroll_size(self.world, id);
            if is_root {
                scrollport.x = geometry.vertical_leading_gutter;
                scrollport.y = geometry.horizontal_leading_gutter;
                // Overflow is measured in the CSS layout. The mobile frame
                // can scroll over that layout even when its contents fit it.
                if let Some((width, height)) = self.viewport.mobile_frame_size() {
                    scrollport.width = width as f32;
                    scrollport.height = height as f32;
                    scroll_size.width = scroll_size.width.max(scrollport.width);
                    scroll_size.height = scroll_size.height.max(scrollport.height);
                }
            }
            let horizontal_range = (scroll_size.width - scrollport.width).max(0.0);
            let vertical_range = (scroll_size.height - scrollport.height).max(0.0);
            let is_scroll_container = self.establishes_scroll_container(id);
            let requested = self.world.boxes[index].scroll_offset;
            let (minimum_offset, maximum_offset) = if is_scroll_container {
                let reversed = self.world.boxes[index].style.reversed_scroll_axes();
                // A reversed axis can still pan toward the far edge of the
                // CSS viewport when the mobile frame is smaller.
                let maximum = LayoutPoint::new(
                    if reversed.x {
                        (layout_scrollport.width - scrollport.width).max(0.0)
                    } else {
                        horizontal_range
                    },
                    if reversed.y {
                        (layout_scrollport.height - scrollport.height).max(0.0)
                    } else {
                        vertical_range
                    },
                );
                (
                    LayoutPoint::new(maximum.x - horizontal_range, maximum.y - vertical_range),
                    maximum,
                )
            } else {
                (LayoutPoint::ZERO, LayoutPoint::ZERO)
            };
            let applied = if is_scroll_container {
                clamp_scroll_offset(requested, minimum_offset, maximum_offset)
            } else {
                LayoutPoint::ZERO
            };
            let horizontal_scrollbar = self
                .has_scrollbar(
                    id,
                    LayoutScrollbarAxis::Horizontal,
                    horizontal_range > f32::EPSILON,
                )
                .then(|| {
                    let frame = LayoutRect::new(
                        scrollport.x,
                        scrollport.bottom(),
                        scrollport.width,
                        scrollbar_thickness,
                    );
                    LayoutScrollbarGeometry::new(
                        LayoutScrollbarAxis::Horizontal,
                        frame,
                        minimum_offset.x,
                        maximum_offset.x,
                        applied.x,
                        scrollport.width,
                        scroll_size.width,
                    )
                })
                .filter(|bar| bar.frame.width > 0.0 && bar.frame.height > 0.0);
            let vertical_scrollbar = self
                .has_scrollbar(
                    id,
                    LayoutScrollbarAxis::Vertical,
                    vertical_range > f32::EPSILON,
                )
                .then(|| {
                    let x = if vertical_scrollbar_on_left {
                        scrollport.x - scrollbar_thickness
                    } else {
                        scrollport.right()
                    };
                    let frame =
                        LayoutRect::new(x, scrollport.y, scrollbar_thickness, scrollport.height);
                    LayoutScrollbarGeometry::new(
                        LayoutScrollbarAxis::Vertical,
                        frame,
                        minimum_offset.y,
                        maximum_offset.y,
                        applied.y,
                        scrollport.height,
                        scroll_size.height,
                    )
                })
                .filter(|bar| bar.frame.width > 0.0 && bar.frame.height > 0.0);
            let scrollbar_corner =
                (geometry.vertical_gutter > 0.0 && geometry.horizontal_gutter > 0.0).then(|| {
                    LayoutRect::new(
                        if vertical_scrollbar_on_left {
                            scrollport.x - scrollbar_thickness
                        } else {
                            scrollport.right()
                        },
                        scrollport.bottom(),
                        scrollbar_thickness,
                        scrollbar_thickness,
                    )
                });
            self.scroll_extents.push(LayoutScrollExtent {
                scrollport,
                scrollable_overflow: overflow,
                scroll_size,
                applied_offset: applied,
                minimum_offset,
                maximum_offset,
                is_scroll_container,
                allows_user_scroll_x: self.allows_user_scroll(id, LayoutScrollbarAxis::Horizontal),
                allows_user_scroll_y: self.allows_user_scroll(id, LayoutScrollbarAxis::Vertical),
                clips_overflow: self.clips_overflow(id),
                horizontal_scrollbar,
                vertical_scrollbar,
                scrollbar_corner,
                scrollbar_colors: self.scrollbar_colors(id),
            });
        }
        self.viewport_scroll =
            self.scroll_extents[self.world.boxes.index_of(self.world.root)].applied_offset;
    }

    fn build_coordinate_spaces(
        &mut self,
        paint: bool,
        retain: bool,
    ) -> Result<Option<Box<ScrollProjection<N>>>, LayoutError> {
        let world = self.world;
        let scroll_extents = &self.scroll_extents;
        let placements = |index: usize| {
            let node = &world.boxes[index];
            let location = node.final_layout.location;
            Ok((
                BoxPlacement {
                    parent: node.layout_parent.map(|id| world.output_id(id)),
                    location: LayoutPoint::new(location.x, location.y),
                    transform: self.resolved_transforms[index],
                    fixed: node.style.is_fixed_positioned(),
                },
                sticky::prepare_constraints(index, world, scroll_extents)?,
            ))
        };
        let retained = retain
            .then(|| ScrollProjection::new(world, &self.boxes, scroll_extents, placements))
            .transpose()?;
        let resolved = resolve_coordinate_spaces(
            world.output_id(world.root),
            self.viewport_scroll,
            world.boxes.len(),
            paint.then(|| PaintPixelGrid::new(self.viewport.device_pixel_ratio)),
            |index| match &retained {
                Some(retained) => Ok(retained.placement(index)),
                None => placements(index),
            },
            |id| {
                let extent = &scroll_extents[id.index()];
                extent.is_scroll_container.then_some(extent.applied_offset)
            },
        )?;
        for (geometry, offset) in self.boxes.iter_mut().zip(resolved.position_adjustments) {
            geometry.layout_origin_in_document.x += offset.x;
            geometry.layout_origin_in_document.y += offset.y;
        }
        self.coordinate_spaces = resolved.spaces;
        Ok(retained.map(Box::new))
    }

    fn build_fragments(&mut self) {
        // All fragment counts are known after inline layout. Allocate once so
        // growing the output does not repeatedly move its existing fragments.
        let fragment_count = self
            .world
            .boxes
            .values()
            .map(|layout_box| {
                usize::from(!layout_box.inline_flattened)
                    + layout_box.inline_layout.as_ref().map_or(0, |context| {
                        let fragments = &context.fragments;
                        fragments.lines.len() + fragments.boxes.len() + fragments.text.len()
                    })
            })
            .sum();
        self.fragments.reserve_exact(fragment_count);

        for index in 0..self.world.boxes.len() {
            let layout_box = &self.world.boxes[index];
            let output_box = LayoutOutputBoxId::from_index(index);
            let coordinate_space = LayoutCoordinateSpaceId::from_index(index + 1);
            if !layout_box.inline_flattened {
                let fragment = self.push_fragment(LayoutFragment {
                    id: LayoutFragmentId::from_index(0),
                    kind: LayoutFragmentKind::Box { box_id: output_box },
                    rect: self.boxes[index].border_box,
                    box_model: Some(LayoutFragmentBoxModel {
                        content: self.boxes[index].content_box,
                        padding: self.boxes[index].padding_box,
                        border: self.boxes[index].border_box,
                        margin: self.boxes[index].margin_box,
                    }),
                    coordinate_space,
                    clip_chain: None,
                    paint_order: None,
                });
                self.paint_metadata.boxes[index].direct_fragment = Some(fragment);
                self.register_box_fragment(index, fragment);
            }

            let Some(context) = layout_box.inline_layout.as_ref() else {
                continue;
            };
            let layout = layout_box.final_layout;
            let box_id = self.world.boxes.id_at(index);
            let vertical_leading_gutter =
                if box_id == self.world.root || self.world.is_viewport_defining_body(box_id) {
                    0.0
                } else {
                    layout_box
                        .style
                        .scrollbar_leading_gutter_thickness(LayoutScrollbarAxis::Vertical, false)
                };
            let horizontal_leading_gutter =
                if box_id == self.world.root || self.world.is_viewport_defining_body(box_id) {
                    0.0
                } else {
                    layout_box
                        .style
                        .scrollbar_leading_gutter_thickness(LayoutScrollbarAxis::Horizontal, false)
                };
            let content_origin = LayoutPoint::new(
                layout.border.left + layout.padding.left + vertical_leading_gutter,
                layout.border.top + layout.padding.top + horizontal_leading_gutter,
            );
            for line in &context.fragments.lines {
                let fragment = self.push_fragment(LayoutFragment {
                    id: LayoutFragmentId::from_index(0),
                    kind: LayoutFragmentKind::Line {
                        owner: output_box,
                        line_index: line.line_index,
                    },
                    rect: offset_rect(line.rect, content_origin),
                    box_model: None,
                    coordinate_space,
                    clip_chain: None,
                    paint_order: None,
                });
                self.boxes[index].fragments.push(fragment);
            }
            let inline_start = self.fragments.len();
            for inline in &context.fragments.boxes {
                let target = self.world.boxes.index_of(inline.box_id);
                let box_model = inline_fragment_box_model(self.world, index, inline);
                let fragment = self.push_fragment(LayoutFragment {
                    id: LayoutFragmentId::from_index(0),
                    kind: LayoutFragmentKind::InlineBox {
                        box_id: LayoutOutputBoxId::from_index(target),
                        line_index: inline.line_index,
                        has_start_edge: inline.has_start_edge,
                        has_end_edge: inline.has_end_edge,
                    },
                    rect: box_model.border,
                    box_model: Some(box_model),
                    coordinate_space,
                    clip_chain: None,
                    paint_order: None,
                });
                self.register_box_fragment(target, fragment);
            }
            for text in &context.fragments.text {
                let target = self.world.boxes.index_of(text.box_id);
                let fragment = self.push_fragment(LayoutFragment {
                    id: LayoutFragmentId::from_index(0),
                    kind: LayoutFragmentKind::Text {
                        box_id: LayoutOutputBoxId::from_index(target),
                        line_index: text.line_index,
                        source_utf16_range: text.source_utf16_range.clone(),
                        rtl: text.rtl,
                    },
                    rect: offset_rect(text.rect, content_origin),
                    box_model: None,
                    coordinate_space,
                    clip_chain: None,
                    paint_order: None,
                });
                self.register_box_fragment(target, fragment);
            }
            self.paint_metadata.boxes[index].inline_fragments = inline_start..self.fragments.len();
        }
    }

    fn register_box_fragment(&mut self, box_index: usize, fragment: LayoutFragmentId) {
        self.boxes[box_index].fragments.push(fragment);
    }

    fn push_fragment(&mut self, mut fragment: LayoutFragment) -> LayoutFragmentId {
        let id = LayoutFragmentId::from_index(self.fragments.len());
        fragment.id = id;
        self.fragments.push(fragment);
        id
    }

    fn assign_clip_and_paint_order(&mut self) {
        let viewport_space = LayoutCoordinateSpaceId::from_index(0);
        let viewport_clip = self.push_clip(
            None,
            LayoutClipKind::Viewport,
            viewport_space,
            self.scroll_extents[self.world.boxes.index_of(self.world.root)].scrollport,
        );
        self.assign_box_clip_metadata(
            self.world.root,
            Some(viewport_clip),
            viewport_clip,
            &mut Vec::new(),
        );
        self.paint_metadata.events = build_paint_order(self.world);
        let mut text_owners = HashMap::new();
        for event_index in 0..self.paint_metadata.events.len() {
            let event = self.paint_metadata.events[event_index];
            match event {
                PaintOrderEvent::BoxBackground(id) => {
                    let index = self.world.boxes.index_of(id);
                    if let Some(fragment) = self.paint_metadata.boxes[index].direct_fragment {
                        self.assign_fragment_paint_metadata(fragment, self.boxes[index].clip_chain);
                    }
                }
                PaintOrderEvent::InlineContents(id) => {
                    self.assign_owner_paint_metadata(id, false);
                }
                PaintOrderEvent::BoxText(id) => self.assign_owner_paint_metadata(id, true),
                PaintOrderEvent::InlineBox { owner, fragment } => {
                    let metadata = &self.paint_metadata.boxes[self.world.boxes.index_of(owner)];
                    let fragment = self.fragments[metadata.inline_fragments.clone()][fragment].id;
                    self.assign_fragment_paint_metadata(fragment, metadata.content_clip);
                }
                PaintOrderEvent::InlineText { owner, line, item } => {
                    // Index the shaped line and its hit fragments once. Repeated
                    // runs must not scan the paragraph or all its text nodes.
                    let lines = text_owners.entry(owner).or_insert_with(|| {
                        let context = self.world.boxes[owner].inline_layout.as_ref().unwrap();
                        let layout = context.laid_out.as_ref().unwrap();
                        let mut lines = layout
                            .lines()
                            .map(|line| {
                                let ranges = line
                                    .items()
                                    .map(|item| match item {
                                        parley::PositionedLayoutItem::GlyphRun(run) => {
                                            Some(run.run().text_range())
                                        }
                                        parley::PositionedLayoutItem::InlineBox(_) => None,
                                    })
                                    .collect::<Vec<_>>();
                                (ranges, Vec::new())
                            })
                            .collect::<Vec<_>>();
                        let range = self.paint_metadata.boxes[self.world.boxes.index_of(owner)]
                            .inline_fragments
                            .clone();
                        let text_fragments =
                            &self.fragments[range][context.fragments.boxes.len()..];
                        for (index, text) in context.fragments.text.iter().enumerate() {
                            lines[text.line_index]
                                .1
                                .push((text.output_range.clone(), text_fragments[index].id));
                        }
                        for (_, fragments) in &mut lines {
                            fragments.sort_by_key(|(range, _)| (range.start, range.end));
                        }
                        lines
                    });
                    let (ranges, fragments) = &lines[line];
                    let range = ranges[item].as_ref().unwrap();
                    let start =
                        fragments.partition_point(|(fragment, _)| fragment.end <= range.start);
                    for (_, fragment) in fragments[start..]
                        .iter()
                        .take_while(|(fragment, _)| fragment.start < range.end)
                    {
                        self.assign_fragment_paint_metadata(
                            *fragment,
                            self.paint_metadata.boxes[self.world.boxes.index_of(owner)]
                                .content_clip,
                        );
                    }
                }
                PaintOrderEvent::BoxOutline(id) => {
                    let index = self.world.boxes.index_of(id);
                    let extent = &self.scroll_extents[index];
                    if extent.horizontal_scrollbar.is_some()
                        || extent.vertical_scrollbar.is_some()
                        || extent.scrollbar_corner.is_some()
                    {
                        self.control_paint_orders[index] = Some(self.next_paint_order());
                    }
                }
                PaintOrderEvent::BoxOutsetShadow(_)
                | PaintOrderEvent::BoxContents(_)
                | PaintOrderEvent::BoxCaret(_)
                | PaintOrderEvent::InlineSelection { .. }
                | PaintOrderEvent::PushStackingContext(_)
                | PaintOrderEvent::TableCollapsedBorders(_)
                | PaintOrderEvent::PopStackingContext(_) => {}
            }
        }
    }

    fn assign_owner_paint_metadata(&mut self, id: LayoutBoxId, text: bool) {
        let index = self.world.boxes.index_of(id);
        let Some(context) = self.world.boxes[index].inline_layout.as_ref() else {
            return;
        };
        let visible_lines = context.visible_lines(context.fragments.lines.len());
        let metadata = &self.paint_metadata.boxes[index];
        let clip = metadata.content_clip;
        for fragment_index in metadata.inline_fragments.clone() {
            let line_index = match self.fragments[fragment_index].kind {
                LayoutFragmentKind::Text { line_index, .. } if text => line_index,
                LayoutFragmentKind::InlineBox { line_index, .. } if !text => line_index,
                _ => continue,
            };
            if line_index < visible_lines {
                self.assign_fragment_paint_metadata(
                    LayoutFragmentId::from_index(fragment_index),
                    clip,
                );
            } else {
                // Keep the geometry and clip chain for CSSOM. Hit testing only
                // considers fragments that participate in paint order.
                self.fragments[fragment_index].clip_chain = clip;
            }
        }
    }

    fn assign_box_clip_metadata(
        &mut self,
        id: LayoutBoxId,
        ancestor_clip: Option<LayoutClipChainId>,
        viewport_clip: LayoutClipChainId,
        legacy_clips: &mut Vec<LayoutClipChainId>,
    ) {
        let index = self.world.boxes.index_of(id);
        // CSS overflow clips follow the containing-block chain, not the box
        // parent chain. In particular, an absolute/fixed descendant skips any
        // overflow clips between itself and its containing block. A
        // viewport-fixed descendant therefore starts from the viewport clip,
        // while a transformed/contained fixed descendant starts from the clip
        // accumulated by that fixed containing block. Blink maintains the
        // same separate in-flow, absolute-position and fixed-position clip
        // contexts in its paint property tree builder.
        let mut box_clip = if self.world.boxes[index].style.is_absolute_positioned()
            || self.world.boxes[index].style.is_fixed_positioned()
        {
            let mut positioned_clip = self.world.boxes[index].positioned_containing_block.map_or(
                Some(viewport_clip),
                |containing_block| {
                    self.paint_metadata.boxes[self.world.boxes.index_of(containing_block)]
                        .content_clip
                },
            );
            // CSS clip applies to the entire subtree, including descendants
            // that skip an intermediate overflow clip.
            if positioned_clip != ancestor_clip && !legacy_clips.is_empty() {
                let mut retained = HashSet::new();
                let mut ancestor = positioned_clip;
                while let Some(id) = ancestor {
                    let clip = &self.clip_chain[id.index()];
                    if let LayoutClipKind::CssRect(owner) = clip.kind {
                        retained.insert(owner);
                    }
                    ancestor = clip.parent;
                }
                for &legacy_clip in legacy_clips.iter() {
                    let legacy = &self.clip_chain[legacy_clip.index()];
                    if let LayoutClipKind::CssRect(owner) = legacy.kind
                        && !retained.contains(&owner)
                    {
                        positioned_clip = Some(self.push_clip(
                            positioned_clip,
                            legacy.kind,
                            legacy.coordinate_space,
                            legacy.rect,
                        ));
                    }
                }
            }
            positioned_clip
        } else {
            ancestor_clip
        };
        let legacy_rect = self.world.boxes[index]
            .style
            .legacy_clip_rect(self.boxes[index].border_box);
        let owner = LayoutOutputBoxId::from_index(index);
        if let Some(rect) = legacy_rect {
            let clip = self.push_clip(
                box_clip,
                LayoutClipKind::CssRect(owner),
                self.boxes[index].coordinate_space,
                rect,
            );
            box_clip = Some(clip);
            legacy_clips.push(clip);
        }
        self.boxes[index].clip_chain = box_clip;

        let child_clip = if id != self.world.root
            && !self.world.is_viewport_defining_body(id)
            && self.world.boxes[index].clips_descendant_paint()
        {
            Some(self.push_clip(
                box_clip,
                LayoutClipKind::Overflow(owner),
                self.boxes[index].coordinate_space,
                self.scroll_extents[index].scrollport,
            ))
        } else {
            box_clip
        };
        self.paint_metadata.boxes[index].content_clip = child_clip;
        for child in self.world.boxes[index].children.clone() {
            self.assign_box_clip_metadata(child, child_clip, viewport_clip, legacy_clips);
        }
        if legacy_rect.is_some() {
            legacy_clips.pop();
        }
    }

    fn assign_fragment_paint_metadata(
        &mut self,
        fragment_id: LayoutFragmentId,
        clip_chain: Option<LayoutClipChainId>,
    ) {
        match self.fragments[fragment_id.index()].kind {
            LayoutFragmentKind::Box { .. }
            | LayoutFragmentKind::InlineBox { .. }
            | LayoutFragmentKind::Text { .. } => {}
            LayoutFragmentKind::Line { .. } => return,
        }
        let order = self.next_paint_order();
        let fragment = &mut self.fragments[fragment_id.index()];
        fragment.clip_chain = clip_chain;
        fragment.paint_order = Some(order);
    }

    fn next_paint_order(&mut self) -> u32 {
        let order = u32::try_from(self.paint_order_count)
            .expect("one frozen layout tree exceeded the u32 paint-order limit");
        self.paint_order_count = self.paint_order_count.saturating_add(1);
        order
    }

    fn push_clip(
        &mut self,
        parent: Option<LayoutClipChainId>,
        kind: LayoutClipKind,
        coordinate_space: LayoutCoordinateSpaceId,
        rect: LayoutRect,
    ) -> LayoutClipChainId {
        let id = LayoutClipChainId::from_index(self.clip_chain.len());
        self.clip_chain.push(LayoutClipNode {
            parent,
            kind,
            coordinate_space,
            rect,
        });
        id
    }

    pub(crate) fn document_content_size(&self) -> LayoutSize {
        let root = self.world.boxes.index_of(self.world.root);
        let root_layout = self.world.boxes[root].final_layout.location;
        let layout_translation = LayoutTransform2D::translation(root_layout.x, root_layout.y);
        let overflow = layout_translation
            .concatenate(self.resolved_transforms[root].transform)
            .map_rect(self.scrollable_overflow[root])
            .bounding_rect()
            .union(
                layout_translation
                    .map_rect(self.boxes[root].margin_box)
                    .bounding_rect(),
            );
        LayoutSize::new(
            (self.viewport.css_width as f32).max(overflow.right().max(0.0)),
            (self.viewport.css_height as f32).max(overflow.bottom().max(0.0)),
        )
    }

    fn into_frozen_tree(
        self,
        source_root: N,
        content_size: LayoutSize,
        embedded_frames: Vec<crate::FrozenEmbeddedFrame<N>>,
        scroll_projection: Option<Box<ScrollProjection<N>>>,
    ) -> FrozenLayoutTree<N> {
        let root_box = LayoutOutputBoxId::from_index(self.world.boxes.index_of(self.world.root));
        let mut coordinate_spaces = self
            .coordinate_spaces
            .into_iter()
            .map(ProjectedCoordinateSpace::into_layout_space);
        let viewport_coordinate_space = FrozenCoordinateSpace::from(
            coordinate_spaces
                .next()
                .expect("a frozen layout tree always owns the viewport coordinate space"),
        );
        let world = self.world;
        let boxes = self
            .boxes
            .into_iter()
            .zip(self.box_sources)
            .zip(self.principal_sources)
            .zip(self.hit_sources)
            .zip(self.scroll_extents)
            .zip(self.control_paint_orders)
            .zip(coordinate_spaces.map(FrozenCoordinateSpace::from))
            .map(
                |(
                    (
                        (
                            (((geometry, geometry_source), principal_source), hit_source),
                            scroll_extent,
                        ),
                        control_paint_order,
                    ),
                    coordinate_space,
                )| {
                    let layout_box = &world.boxes[geometry.id.index()];
                    let display = layout_box.style.display();
                    let css_sizing = (principal_source.is_some()
                        && !matches!(
                            display,
                            crate::LayoutDisplay::None | crate::LayoutDisplay::Contents
                        )
                        && (!display.is_inline_flow() || layout_box.is_replaced()))
                    .then_some(CssSizing {
                        box_kind: match layout_box.style.taffy.box_sizing {
                            taffy::BoxSizing::ContentBox => CssSizingBox::Content,
                            taffy::BoxSizing::BorderBox => CssSizingBox::Border,
                        },
                        horizontal: layout_box.style.uses_horizontal_writing_mode(),
                    });
                    let resolved_grid_tracks = world
                        .resolved_grid_tracks
                        .get(&self.world.boxes.id_at(geometry.id.index()))
                        .cloned();
                    FrozenLayoutBox {
                        display,
                        list_marker_position: layout_box.style.list_marker_position(),
                        geometry,
                        scroll_extent,
                        coordinate_space,
                        geometry_source,
                        principal_source,
                        hit_source,
                        #[cfg(feature = "molio")]
                        pseudo: layout_box.pseudo,
                        css_sizing,
                        resolved_grid_tracks,
                        control_paint_order,
                    }
                },
            )
            .collect();
        let mut tree = FrozenLayoutTree::new(
            source_root,
            self.viewport,
            self.viewport_scroll,
            content_size,
            root_box,
            boxes,
            self.fragments,
            self.scroll_proxy_links,
            viewport_coordinate_space,
            self.clip_chain,
            embedded_frames,
        );
        tree.scroll_projection = scroll_projection;
        tree
    }
}

fn inline_fragment_box_model<N>(
    world: &LayoutWorld<N>,
    owner_index: usize,
    fragment: &crate::inline::InlineBoxFragment,
) -> LayoutFragmentBoxModel
where
    N: Copy + Debug + Eq + Hash,
{
    let owner = &world.boxes[owner_index];
    let owner_id = world.boxes.id_at(owner_index);
    let owner_layout = owner.final_layout;
    let vertical_leading_gutter =
        if owner_id == world.root || world.is_viewport_defining_body(owner_id) {
            0.0
        } else {
            owner
                .style
                .scrollbar_leading_gutter_thickness(LayoutScrollbarAxis::Vertical, false)
        };
    let horizontal_leading_gutter =
        if owner_id == world.root || world.is_viewport_defining_body(owner_id) {
            0.0
        } else {
            owner
                .style
                .scrollbar_leading_gutter_thickness(LayoutScrollbarAxis::Horizontal, false)
        };
    let content_origin = LayoutPoint::new(
        owner_layout.border.left + owner_layout.padding.left + vertical_leading_gutter,
        owner_layout.border.top + owner_layout.padding.top + horizontal_leading_gutter,
    );
    let containing_width = (owner_layout.size.width
        - owner_layout.border.left
        - owner_layout.border.right
        - owner_layout.padding.left
        - owner_layout.padding.right)
        - if owner_id == world.root || world.is_viewport_defining_body(owner_id) {
            0.0
        } else {
            owner
                .style
                .scrollbar_gutter_thickness(LayoutScrollbarAxis::Vertical)
        };
    let containing_width = containing_width.max(0.0);
    let inline_box = &world.boxes[fragment.box_id];
    let style = &inline_box.style;
    let padding = style.taffy.padding.resolve_or_zero(
        Some(containing_width),
        crate::style::resolve_stylo_calc_value,
    );
    let border = style.taffy.border.resolve_or_zero(
        Some(containing_width),
        crate::style::resolve_stylo_calc_value,
    );
    let margin = style.taffy.margin.resolve_or_zero(
        Some(containing_width),
        crate::style::resolve_stylo_calc_value,
    );
    let ltr = style.direction() == crate::style::InlineDirection::Ltr;
    let has_left_edge = if ltr {
        fragment.has_start_edge
    } else {
        fragment.has_end_edge
    };
    let has_right_edge = if ltr {
        fragment.has_end_edge
    } else {
        fragment.has_start_edge
    };
    let left_margin = if has_left_edge {
        margin.left.max(0.0)
    } else {
        0.0
    };
    let right_margin = if has_right_edge {
        margin.right.max(0.0)
    } else {
        0.0
    };
    let left_padding = if has_left_edge { padding.left } else { 0.0 };
    let right_padding = if has_right_edge { padding.right } else { 0.0 };
    let left_border = if has_left_edge { border.left } else { 0.0 };
    let right_border = if has_right_edge { border.right } else { 0.0 };
    let border_box = LayoutRect::new(
        content_origin.x + fragment.rect.x + left_margin,
        content_origin.y + fragment.rect.y - padding.top - border.top,
        (fragment.rect.width - left_margin - right_margin).max(0.0),
        fragment.rect.height + padding.top + padding.bottom + border.top + border.bottom,
    );
    let padding_box = inset_rect(
        border_box,
        border.top,
        right_border,
        border.bottom,
        left_border,
    );
    let content_box = inset_rect(
        padding_box,
        padding.top,
        right_padding,
        padding.bottom,
        left_padding,
    );
    let margin_box = outset_rect(
        border_box,
        margin.top,
        right_margin,
        margin.bottom,
        left_margin,
    );
    LayoutFragmentBoxModel {
        content: content_box,
        padding: padding_box,
        border: border_box,
        margin: margin_box,
    }
}

#[cfg(test)]
mod dependency_tests {
    use super::*;

    #[test]
    fn sticky_resolves_a_scroll_space_outside_its_layout_parent_chain() {
        fn box_for(id: usize) -> crate::world::LayoutBox<usize> {
            LayoutWorld::new_box(
                Some(id),
                None,
                None,
                id.to_string(),
                None,
                None,
                None,
                crate::LayoutBoxKind::PrincipalBlock,
                crate::ResolvedLayoutStyle::synthetic(
                    crate::LayoutDisplay::Block,
                    taffy::Style::default(),
                    crate::PaintColor::TRANSPARENT,
                ),
                None,
            )
        }
        let mut world = LayoutWorld::new(box_for(0), false);
        world.boxes.extend([box_for(1), box_for(2)]);
        for layout_box in world.boxes.values_mut() {
            layout_box.final_layout.size = taffy::Size {
                width: 1000.0,
                height: 1000.0,
            };
        }
        let root = LayoutBoxId::from_index(0);
        let scroller = LayoutBoxId::from_index(2);
        world.boxes[1].parent = Some(scroller);
        world.boxes[1].layout_parent = Some(root);
        world.boxes[1].style = crate::ResolvedLayoutStyle::synthetic(
            crate::LayoutDisplay::Block,
            taffy::Style {
                inset: taffy::Rect {
                    top: taffy::style_helpers::length(10.0),
                    ..taffy::Rect::auto()
                },
                ..taffy::Style::default()
            },
            crate::PaintColor::TRANSPARENT,
        )
        .with_position(crate::LayoutPosition::Sticky);
        world.boxes[1].final_layout.size = taffy::Size {
            width: 20.0,
            height: 20.0,
        };
        world.boxes[2].layout_parent = Some(root);
        world.boxes[2].parent = Some(root);
        world.boxes[2].final_layout.location.y = 40.0;
        let mut projection = OutputProjection::new(&world, LayoutViewport::new(100, 100, 1.0));
        projection.scroll_extents = (0..3)
            .map(|index| LayoutScrollExtent {
                scrollport: LayoutRect::new(0.0, 0.0, 100.0, 100.0),
                scrollable_overflow: LayoutRect::ZERO,
                scroll_size: LayoutSize::new(100.0, 100.0),
                applied_offset: LayoutPoint::ZERO,
                minimum_offset: LayoutPoint::ZERO,
                maximum_offset: LayoutPoint::ZERO,
                is_scroll_container: index == 2,
                allows_user_scroll_x: false,
                allows_user_scroll_y: false,
                clips_overflow: false,
                horizontal_scrollbar: None,
                vertical_scrollbar: None,
                scrollbar_corner: None,
                scrollbar_colors: None,
            })
            .collect();
        projection.build_coordinate_spaces(false, false).unwrap();
        let sticky = &projection.coordinate_spaces[2].layout;
        assert_eq!(
            sticky.local_to_viewport.map_point(LayoutPoint::ZERO).y,
            50.0
        );
        // The output pass borrows the numeric world and never patches locations.
        assert_eq!(world.boxes[1].final_layout.location.y, 0.0);
    }
}
