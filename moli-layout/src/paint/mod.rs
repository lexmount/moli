mod background;
mod clip_path;
mod filters;
mod form_controls;
mod geometry;
mod image;
mod inline_boxes;
mod mask;
mod table;
mod text;

use std::{collections::HashMap, fmt::Debug, hash::Hash};

use background::{project_background_color, project_background_layers};
use clip_path::{ClipPathUnsupported, project_clip_path};
use filters::{expanded_filter_clip, project_filters};
use form_controls::project_form_control_appearance;
use geometry::{BoxAreas, BoxModelBox, canonical_shape, inset_radii};
use image::project_replaced_image;
use inline_boxes::project_inline_box_fragments;
use mask::{CssMaskPlan, inspect_css_mask, project_css_mask};
use style::values::generics::image::GenericImage;
use table::project_collapsed_table_borders;
use text::{TextClipMaskScope, project_text, project_text_clip_mask};

use crate::{
    LayoutBox, LayoutBoxId, LayoutClipChainId, LayoutElementCategory, LayoutFormControlKind,
    LayoutInputControlKind, LayoutReplacedKind, LayoutWorld, PaintColor, PaintDiagnostic,
    PaintDiagnosticSeverity, PaintEdgeSizes, PaintFragment, PaintShape, PaintSnapshot,
    capture::ResolvedPaintCapture,
    projection::{OutputProjection, PaintSpace},
    stacking::PaintOrderEvent,
};

#[derive(Clone, Copy, Debug, Default)]
struct ContextPaintState {
    effect_layers: usize,
    mask_layer: bool,
    clip_path_layer: bool,
}

impl PaintSpace {
    /// Pixel-snap canonical CSS box geometry after ordinary layout offsets
    /// and before property transforms.
    fn pixel_snapped_box_shape(self, shape: PaintShape) -> Option<PaintShape> {
        match shape {
            PaintShape::Rect(rect) => {
                crate::pixel_snap_paint_rect(self.pre_transform_rect(rect)).map(PaintShape::Rect)
            }
            PaintShape::RoundedRect { rect, radii } => {
                crate::pixel_snap_paint_rect(self.pre_transform_rect(rect))
                    .map(|rect| PaintShape::RoundedRect { rect, radii })
            }
            // BoxAreas only constructs rectilinear or rounded box geometry.
            // Arbitrary paths retain their own authored coordinate contract.
            PaintShape::Path(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnavailableReplacedContentPaint {
    OpaquePlaceholder,
    // A newly-created HTML canvas is transparent black. Until its backing
    // store becomes an owned paint resource, preserve that clear state instead
    // of inventing opaque content that CSS filters can amplify.
    TransparentCanvas,
}

pub(crate) fn project_paint_snapshot<N>(
    projection: &OutputProjection<'_, N>,
    capture: ResolvedPaintCapture,
    embedded_frames: &mut HashMap<LayoutBoxId, PaintSnapshot>,
) -> PaintSnapshot
where
    N: Copy + Debug + Eq + Hash,
{
    let root = projection.world.root;
    let (canvas_color, propagated_background) = if capture.include_backgrounds {
        canvas_background(projection.world, root)
    } else {
        (PaintColor::WHITE, None)
    };
    let mut snapshot = PaintSnapshot::new(projection.viewport, canvas_color);
    snapshot.surface = capture.surface;
    snapshot.viewport_to_surface = capture.viewport_to_surface;
    snapshot.content_size = projection.document_content_size();
    snapshot.diagnostics = projection.diagnostics.clone();
    if let Some(background) = propagated_background {
        let transform = capture.viewport_to_surface;
        let paint_space = PaintSpace::ROOT.with_outer_transform(transform);
        let text_clip_mask = |snapshot: &mut PaintSnapshot| {
            project_background_text_clip_mask(
                projection,
                background,
                TextClipMaskScope::AllGlyphs,
                snapshot,
            );
        };
        project_background_layers(
            &projection.world.boxes[background.index()],
            BoxAreas::for_rect(capture.viewport_rect),
            paint_space,
            &mut snapshot,
            &text_clip_mask,
        );
    }

    let mut context_layers = Vec::new();
    for event in &projection.paint_events {
        match *event {
            PaintOrderEvent::BoxOutsetShadow(id) => {
                let clip_count = push_clip_chain(
                    projection,
                    projection.background_clips[id.index()],
                    &mut snapshot,
                );
                project_outset_box_shadows(projection, id, &mut snapshot);
                pop_clips(clip_count, &mut snapshot);
            }
            PaintOrderEvent::PushStackingContext(id) => {
                let layout_box = &projection.world.boxes[id.index()];
                let style = &layout_box.style;
                let geometry = &projection.boxes[id.index()];
                let viewport_transform = projection.coordinate_spaces
                    [geometry.coordinate_space.index()]
                .paint
                .local_transform();
                let transform = capture.viewport_to_surface.concatenate(viewport_transform);
                let areas = BoxAreas::for_box(projection, id);
                let clip_path_layer = match project_clip_path(style, areas) {
                    Ok(Some(shape)) => {
                        snapshot.push_fragment(PaintFragment::PushClip { shape, transform });
                        true
                    }
                    Ok(None) => false,
                    Err(reason) => {
                        let (code, capability) = match reason {
                            ClipPathUnsupported::UrlReference => (
                                "clip-path-url-reference-unsupported",
                                "URL-referenced clip paths",
                            ),
                        };
                        push_diagnostic_once(
                            &mut snapshot,
                            code,
                            format!("{} uses unsupported {capability}", layout_box.source_label),
                        );
                        false
                    }
                };
                let mask_plan = inspect_css_mask(style);
                let mask_layer = match mask_plan {
                    CssMaskPlan::None => false,
                    CssMaskPlan::Supported { luminance_fallback } => {
                        if luminance_fallback {
                            push_diagnostic_once(
                                &mut snapshot,
                                "mask-mode-luminance-fallback",
                                format!(
                                    "{} uses mask-mode:luminance; software paint currently applies the image alpha channel",
                                    layout_box.source_label
                                ),
                            );
                        }
                        snapshot.push_fragment(PaintFragment::PushLayer {
                            // Directly port Blitz's isolation guard. The CPU
                            // backend otherwise folds a fully opaque SrcOver
                            // layer into a clip and DestIn would affect the
                            // backdrop outside this element.
                            opacity: 1.0 - f32::EPSILON,
                            blend_mode: crate::PaintBlendMode::Normal,
                            composite: crate::PaintCompositeMode::SrcOver,
                            clip: areas.shape(BoxModelBox::Border),
                            transform,
                            filter: None,
                        });
                        true
                    }
                    CssMaskPlan::ResourceDeferred => {
                        push_diagnostic_once(
                            &mut snapshot,
                            "mask-image-type-unsupported",
                            format!(
                                "{} uses an unsupported computed mask image type",
                                layout_box.source_label
                            ),
                        );
                        false
                    }
                };
                let filters = project_filters(style);
                if filters.skipped_url {
                    push_diagnostic_once(
                        &mut snapshot,
                        "filter-url-reference-unsupported",
                        format!(
                            "{} uses a URL-referenced filter that software paint cannot resolve",
                            layout_box.source_label
                        ),
                    );
                }
                if filters.has_backdrop_filter {
                    push_diagnostic_once(
                        &mut snapshot,
                        "backdrop-filter-unsupported",
                        format!(
                            "{} uses backdrop-filter, which the CPU backend does not implement",
                            layout_box.source_label
                        ),
                    );
                }
                // A stacking-context effect applies to every pixel produced by
                // the context, including visible descendant overflow, outset
                // shadows, and outlines. Blitz currently clips this layer to
                // the root border box; that loses valid ink. Map the current
                // raster viewport back into this context instead. The raster
                // target remains the natural resource bound, while filter
                // expansion keeps off-edge source pixels available.
                let effect_source_clip = viewport_transform
                    .inverse()
                    .map_or(areas.border_rect, |inverse| {
                        inverse.map_rect(capture.viewport_rect).bounding_rect()
                    });
                let effect_clip = expanded_filter_clip(effect_source_clip, &filters.effects);
                let mut effect_layers = 0usize;
                let needs_effect_layer =
                    style.opacity() < 1.0 || style.blend_mode() != crate::PaintBlendMode::Normal;
                if needs_effect_layer {
                    snapshot.push_fragment(PaintFragment::PushLayer {
                        opacity: style.opacity(),
                        blend_mode: style.blend_mode(),
                        composite: crate::PaintCompositeMode::SrcOver,
                        clip: PaintShape::Rect(effect_clip),
                        transform,
                        filter: None,
                    });
                    effect_layers += 1;
                }
                // CSS applies filter functions in list order. Nest the list in
                // reverse so pop order evaluates f1, f2, ... while every CPU
                // layer contains one backend-supported filter node.
                for filter in filters.effects.iter().rev() {
                    snapshot.push_fragment(PaintFragment::PushLayer {
                        opacity: 1.0,
                        blend_mode: crate::PaintBlendMode::Normal,
                        composite: crate::PaintCompositeMode::SrcOver,
                        clip: PaintShape::Rect(effect_clip),
                        transform,
                        filter: Some(*filter),
                    });
                    effect_layers += 1;
                }
                context_layers.push(ContextPaintState {
                    effect_layers,
                    mask_layer,
                    clip_path_layer,
                });
            }
            PaintOrderEvent::BoxBackground(id) => {
                let clip_count = push_clip_chain(
                    projection,
                    projection.background_clips[id.index()],
                    &mut snapshot,
                );
                project_box_background(
                    projection,
                    id,
                    propagated_background,
                    embedded_frames.contains_key(&id),
                    capture.include_backgrounds,
                    &mut snapshot,
                );
                pop_clips(clip_count, &mut snapshot);
            }
            PaintOrderEvent::BoxContents(id) => {
                let clip_count = push_clip_chain(
                    projection,
                    projection.content_clips[id.index()],
                    &mut snapshot,
                );
                project_box_contents(
                    projection,
                    id,
                    embedded_frames,
                    capture.include_backgrounds,
                    &mut snapshot,
                );
                pop_clips(clip_count, &mut snapshot);
            }
            PaintOrderEvent::TableCollapsedBorders(id) => {
                let clip_count = push_clip_chain(
                    projection,
                    projection.background_clips[id.index()],
                    &mut snapshot,
                );
                project_collapsed_table_borders(projection, id, &mut snapshot);
                pop_clips(clip_count, &mut snapshot);
            }
            PaintOrderEvent::BoxOutline(id) => {
                let clip_count = push_clip_chain(
                    projection,
                    projection.background_clips[id.index()],
                    &mut snapshot,
                );
                project_box_outline(projection, id, &mut snapshot);
                pop_clips(clip_count, &mut snapshot);
            }
            PaintOrderEvent::PopStackingContext(id) => {
                let state = context_layers
                    .pop()
                    .expect("stacking events are structurally balanced");
                for _ in 0..state.effect_layers {
                    snapshot.push_fragment(PaintFragment::PopLayer);
                }
                if state.mask_layer {
                    let layout_box = &projection.world.boxes[id.index()];
                    let geometry = &projection.boxes[id.index()];
                    let paint_space = projection.coordinate_spaces
                        [geometry.coordinate_space.index()]
                    .paint_space(capture.viewport_to_surface);
                    let transform = paint_space.local_transform();
                    let areas = BoxAreas::for_box(projection, id);
                    snapshot.push_fragment(PaintFragment::PushLayer {
                        opacity: 1.0,
                        blend_mode: crate::PaintBlendMode::Normal,
                        composite: crate::PaintCompositeMode::DestIn,
                        clip: areas.shape(BoxModelBox::Border),
                        transform,
                        filter: None,
                    });
                    project_css_mask(layout_box, areas, paint_space, &mut snapshot);
                    snapshot.push_fragment(PaintFragment::PopLayer); // DestIn mask
                    snapshot.push_fragment(PaintFragment::PopLayer); // isolation
                }
                if state.clip_path_layer {
                    snapshot.push_fragment(PaintFragment::PopLayer);
                }
                debug_assert!(
                    projection.world.boxes[id.index()].creates_stacking_context(
                        id == projection.world.root,
                        projection.world.boxes[id.index()]
                            .parent
                            .is_some_and(|parent| {
                                let display =
                                    projection.world.boxes[parent.index()].style.display();
                                display.is_flex_container() || display.is_grid_container()
                            })
                    )
                );
            }
        }
    }
    debug_assert!(context_layers.is_empty());
    snapshot
}

fn push_clip_chain<N>(
    projection: &OutputProjection<'_, N>,
    clip: Option<LayoutClipChainId>,
    snapshot: &mut PaintSnapshot,
) -> usize
where
    N: Copy + Debug + Eq + Hash,
{
    let mut chain = Vec::new();
    let mut current = clip;
    while let Some(id) = current {
        let node = &projection.clip_chain[id.index()];
        chain.push(node);
        current = node.parent;
    }
    chain.reverse();
    chain.retain(|node| node.owner.is_some());
    for node in &chain {
        let transform = snapshot.viewport_to_surface.concatenate(
            projection.coordinate_spaces[node.coordinate_space.index()]
                .paint
                .local_transform(),
        );
        let shape = node.owner.map_or(PaintShape::Rect(node.rect), |owner| {
            let layout_box = &projection.world.boxes[owner.index()];
            let border_radii = layout_box.style.border_radii(
                layout_box.final_layout.size.width,
                layout_box.final_layout.size.height,
            );
            canonical_shape(
                node.rect,
                inset_radii(
                    border_radii,
                    PaintEdgeSizes::new(
                        layout_box.final_layout.border.top,
                        layout_box.final_layout.border.right,
                        layout_box.final_layout.border.bottom,
                        layout_box.final_layout.border.left,
                    ),
                ),
            )
        });
        snapshot.push_fragment(PaintFragment::PushClip { shape, transform });
    }
    chain.len()
}

fn pop_clips(count: usize, snapshot: &mut PaintSnapshot) {
    for _ in 0..count {
        snapshot.push_fragment(PaintFragment::PopLayer);
    }
}

fn canvas_background<N>(
    world: &LayoutWorld<N>,
    root: LayoutBoxId,
) -> (PaintColor, Option<LayoutBoxId>)
where
    N: Copy + Debug + Eq + Hash,
{
    let root_box = &world.boxes[root.index()];
    let is_html_root = root_box
        .element_semantics()
        .is_some_and(|element| element.is_html_element("html"));
    if !is_html_root {
        return (PaintColor::WHITE, None);
    }
    if style_has_canvas_background(&root_box.style) {
        return (root_box.style.background_color(), Some(root));
    }

    let mut stack = root_box.children.iter().rev().copied().collect::<Vec<_>>();
    while let Some(id) = stack.pop() {
        let layout_box = &world.boxes[id.index()];
        if layout_box
            .element_semantics()
            .is_some_and(|element| element.is_html_element("body"))
        {
            if style_has_canvas_background(&layout_box.style) {
                return (layout_box.style.background_color(), Some(id));
            }
            break;
        }
        stack.extend(layout_box.children.iter().rev().copied());
    }
    (PaintColor::WHITE, None)
}

fn style_has_canvas_background(style: &crate::ResolvedLayoutStyle) -> bool {
    if style.background_color().alpha > 0.0 {
        return true;
    }
    style.stylo_computed_values().is_some_and(|computed| {
        computed
            .get_background()
            .background_image
            .0
            .iter()
            .any(|image| !matches!(image, GenericImage::None))
    })
}

fn project_box_background<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    propagated_background: Option<LayoutBoxId>,
    has_embedded_frame: bool,
    include_backgrounds: bool,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if layout_box.inline_flattened || !layout_box.style.is_visible() {
        return;
    }
    let geometry = &projection.boxes[id.index()];
    let paint_space = projection.coordinate_spaces[geometry.coordinate_space.index()]
        .paint_space(snapshot.viewport_to_surface);
    let areas = BoxAreas::for_box(projection, id);
    let rect = areas.border_rect;
    let radii = areas.border_radii;

    let shadows = layout_box.style.box_shadows(
        paint_space.pre_transform_rect(rect),
        radii,
        paint_space.property_transform(),
    );

    if include_backgrounds && propagated_background != Some(id) {
        let mut color = layout_box.style.background_color();
        match unavailable_replaced_content_paint(layout_box, has_embedded_frame) {
            Some(UnavailableReplacedContentPaint::OpaquePlaceholder) => {
                snapshot.push_diagnostic(PaintDiagnostic::new(
                    "replaced-content-placeholder",
                    format!(
                        "{} uses deterministic placeholder paint because replaced content is unavailable",
                        layout_box.source_label
                    ),
                    PaintDiagnosticSeverity::Warning,
                ));
                if color.alpha <= 0.0 {
                    color = PaintColor::new(0.82, 0.84, 0.87, 1.0);
                }
            }
            Some(UnavailableReplacedContentPaint::TransparentCanvas) => {
                snapshot.push_diagnostic(PaintDiagnostic::new(
                    "canvas-content-unavailable",
                    format!(
                        "{} uses a transparent fallback because the canvas backing store is not connected to paint",
                        layout_box.source_label
                    ),
                    PaintDiagnosticSeverity::Warning,
                ));
            }
            None => {}
        }
        let text_clip_mask = |snapshot: &mut PaintSnapshot| {
            project_background_text_clip_mask(
                projection,
                id,
                TextClipMaskScope::AllGlyphs,
                snapshot,
            );
        };
        project_background_color(
            layout_box,
            areas,
            paint_space,
            color,
            snapshot,
            &text_clip_mask,
        );
        project_background_layers(layout_box, areas, paint_space, snapshot, &text_clip_mask);
    }

    let layout = layout_box.final_layout;
    let widths = PaintEdgeSizes::new(
        layout.border.top,
        layout.border.right,
        layout.border.bottom,
        layout.border.left,
    );
    let colors = layout_box.style.border_colors();
    for shadow in shadows.iter().filter(|shadow| shadow.inset).rev() {
        snapshot.push_fragment(PaintFragment::BoxShadow(shadow.clone()));
    }
    if !layout_box.collapsed_table_border_part
        && widths.has_positive_edge()
        && colors.has_visible_edge()
    {
        snapshot.push_fragment(PaintFragment::Border {
            rect: paint_space.pre_transform_rect(rect),
            widths,
            colors,
            styles: layout_box.style.border_styles(),
            radii,
            transform: paint_space.property_transform(),
        });
    }
}

/// Replays only descendant text ink for a `background-clip:text` mask.
///
/// This mirrors Blink's `PaintPhase::kTextClip` traversal without retaining a
/// second layout tree: each box reuses its pass-owned Parley output and exact
/// coordinate space, while ordinary foreground paint remains in paint order.
fn project_background_text_clip_mask<N>(
    projection: &OutputProjection<'_, N>,
    root: LayoutBoxId,
    scope: TextClipMaskScope,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    project_box_text_clip_mask(projection, root, scope, snapshot);

    // A block/background root owns every descendant IFC. A flattened inline
    // instead selects its runs from the shared root IFC, then includes only
    // independent IFCs rooted inside that inline's box subtree.
    let mut stack: Vec<LayoutBoxId> = match scope {
        TextClipMaskScope::AllGlyphs => projection.world.boxes[root.index()]
            .children
            .iter()
            .rev()
            .copied()
            .collect(),
        TextClipMaskScope::InlineBox(target) => projection.world.boxes[target.index()]
            .children
            .iter()
            .rev()
            .copied()
            .collect(),
    };
    while let Some(id) = stack.pop() {
        let layout_box = &projection.world.boxes[id.index()];
        stack.extend(layout_box.children.iter().rev().copied());
        project_box_text_clip_mask(projection, id, TextClipMaskScope::AllGlyphs, snapshot);
    }
}

fn project_box_text_clip_mask<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    scope: TextClipMaskScope,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if !layout_box.style.is_visible() {
        return;
    }
    let geometry = &projection.boxes[id.index()];
    let transform = snapshot.viewport_to_surface.concatenate(
        projection.coordinate_spaces[geometry.coordinate_space.index()]
            .paint
            .local_transform(),
    );
    let clip_count = push_clip_chain(projection, projection.content_clips[id.index()], snapshot);
    project_text_clip_mask(layout_box, transform, snapshot, scope);
    pop_clips(clip_count, snapshot);
}

fn project_outset_box_shadows<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if layout_box.inline_flattened || !layout_box.style.is_visible() {
        return;
    }
    let geometry = &projection.boxes[id.index()];
    let paint_space = projection.coordinate_spaces[geometry.coordinate_space.index()]
        .paint_space(snapshot.viewport_to_surface);
    let areas = BoxAreas::for_box(projection, id);
    for shadow in layout_box
        .style
        .box_shadows(
            paint_space.pre_transform_rect(areas.border_rect),
            areas.border_radii,
            paint_space.property_transform(),
        )
        .into_iter()
        .filter(|shadow| !shadow.inset)
        .rev()
    {
        snapshot.push_fragment(PaintFragment::BoxShadow(shadow));
    }
}

fn push_diagnostic_once(snapshot: &mut PaintSnapshot, code: &'static str, message: String) {
    if snapshot.diagnostics.iter().any(|item| item.code == code) {
        return;
    }
    snapshot.push_diagnostic(PaintDiagnostic::new(
        code,
        message,
        PaintDiagnosticSeverity::Warning,
    ));
}

fn project_box_outline<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if layout_box.inline_flattened || !layout_box.style.is_visible() {
        return;
    }
    let geometry = &projection.boxes[id.index()];
    let paint_space = projection.coordinate_spaces[geometry.coordinate_space.index()]
        .paint_space(snapshot.viewport_to_surface);
    let radii = layout_box
        .style
        .border_radii(geometry.border_box.width, geometry.border_box.height);
    if let Some(outline) = layout_box.style.outline_fragment(
        paint_space.pre_transform_rect(geometry.border_box),
        radii,
        paint_space.property_transform(),
    ) {
        snapshot.push_fragment(outline);
    }
}

fn project_box_contents<N>(
    projection: &OutputProjection<'_, N>,
    id: LayoutBoxId,
    embedded_frames: &mut HashMap<LayoutBoxId, PaintSnapshot>,
    include_backgrounds: bool,
    snapshot: &mut PaintSnapshot,
) where
    N: Copy + Debug + Eq + Hash,
{
    let layout_box = &projection.world.boxes[id.index()];
    if !layout_box.style.is_visible() {
        return;
    }
    let geometry = &projection.boxes[id.index()];
    let paint_space = projection.coordinate_spaces[geometry.coordinate_space.index()]
        .paint_space(snapshot.viewport_to_surface);
    let transform = paint_space.local_transform();
    if let Some(child) = embedded_frames.remove(&id) {
        let content = BoxAreas::for_box(projection, id).content_rect;
        let local_to_surface =
            transform.concatenate(crate::LayoutTransform2D::translation(content.x, content.y));
        snapshot.append_embedded_snapshot(
            child,
            crate::PaintRect::new(0.0, 0.0, content.width, content.height),
            local_to_surface,
        );
    }
    project_replaced_image(projection, id, paint_space, snapshot);
    project_form_control_appearance(layout_box, geometry.border_box, transform, snapshot);
    let text_clip_mask = |scope: TextClipMaskScope, snapshot: &mut PaintSnapshot| {
        project_background_text_clip_mask(projection, id, scope, snapshot);
    };
    project_inline_box_fragments(
        projection.world,
        layout_box,
        paint_space,
        include_backgrounds,
        snapshot,
        &text_clip_mask,
    );
    project_text(layout_box, transform, snapshot);
}

fn unavailable_replaced_content_paint<N>(
    layout_box: &LayoutBox<N>,
    has_embedded_frame: bool,
) -> Option<UnavailableReplacedContentPaint>
where
    N: Copy + Debug + Eq + Hash,
{
    if !layout_box.is_replaced() {
        return None;
    }
    if layout_box
        .replaced_image
        .as_ref()
        .is_some_and(|resource| resource.pixels.is_some() || resource.svg.is_some())
    {
        return None;
    }
    if has_embedded_frame
        && layout_box
            .element_semantics()
            .is_some_and(|semantics| semantics.replaced == Some(LayoutReplacedKind::Frame))
    {
        return None;
    }
    let semantics = layout_box.element_semantics()?;
    match semantics.category {
        LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
            LayoutInputControlKind::Image,
        )) => Some(UnavailableReplacedContentPaint::OpaquePlaceholder),
        LayoutElementCategory::FormControl(_) => None,
        _ => match semantics.replaced {
            Some(LayoutReplacedKind::Canvas) => {
                Some(UnavailableReplacedContentPaint::TransparentCanvas)
            }
            // Ordinary HTML image fallback paint belongs to image projection:
            // it is a content-box outline, not an invented CSS background.
            Some(LayoutReplacedKind::Image) => None,
            Some(_) => Some(UnavailableReplacedContentPaint::OpaquePlaceholder),
            None => None,
        },
    }
}
