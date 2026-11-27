//! Inline box-fragment background and border projection.
//!
//! Blitz currently paints inline background color alongside glyphs but has no
//! general inline-border or image-layer fragment implementation. This adapter
//! consumes the one-shot IFC fragment sidecar, preserves start/end edges across
//! line fragmentation, and emits the same owned background/image/border
//! primitives used by block boxes.

use std::{collections::HashMap, fmt::Debug, hash::Hash};

use taffy::ResolveOrZero;

use super::{
    PaintSpace,
    background::{project_background_color, project_background_layers},
    border::{
        BorderImageGeometry, border_image_outsets, outset_border_image_rect, project_box_border,
    },
    geometry::{BoxAreas, inset_radii},
    text::TextClipMaskScope,
};
use crate::{
    LayoutBoxId, LayoutFragment, LayoutFragmentId, LayoutFragmentKind, LayoutOutputBoxId,
    LayoutRect, PaintEdgeSizes, PaintSnapshot, projection::PaintProjection, style::InlineDirection,
};

pub(super) type InlineBorderImages = HashMap<LayoutFragmentId, BorderImageGeometry>;

pub(super) fn inline_border_images<N>(projection: &PaintProjection<'_, N>) -> InlineBorderImages
where
    N: Copy + Debug + Eq + Hash,
{
    fn inline_box(fragment: &LayoutFragment) -> Option<LayoutOutputBoxId> {
        match fragment.kind {
            LayoutFragmentKind::InlineBox { box_id, .. } => Some(box_id),
            _ => None,
        }
    }

    let mut images = HashMap::new();
    // Each structural inline's projected boxes are adjacent and in line
    // order. Compute their cumulative widths once even when paint order
    // visits the fragments individually around atomic inlines.
    for fragments in projection
        .fragments
        .chunk_by(|left, right| inline_box(left).is_some_and(|id| inline_box(right) == Some(id)))
    {
        let Some(id) = inline_box(&fragments[0]) else {
            continue;
        };
        let style = &projection.world.boxes[id.index()].style;
        let widths = style
            .taffy
            .border
            .resolve_or_zero(None, crate::style::resolve_stylo_calc_value);
        let widths = PaintEdgeSizes::new(widths.top, widths.right, widths.bottom, widths.left);
        let Some(outsets) = border_image_outsets(style, widths) else {
            continue;
        };
        let strip_width: f32 = fragments.iter().map(|fragment| fragment.rect.width).sum();
        let ltr = style.direction() == InlineDirection::Ltr;
        let mut advance = 0.0;
        for fragment in fragments {
            let LayoutFragmentKind::InlineBox {
                has_start_edge,
                has_end_edge,
                ..
            } = fragment.kind
            else {
                unreachable!("grouped by inline box");
            };
            let rect = fragment.rect;
            let offset = if ltr {
                advance
            } else {
                strip_width - advance - rect.width
            };
            advance += rect.width;
            let strip = LayoutRect::new(rect.x - offset, rect.y, strip_width, rect.height);
            let fragment_clip = (fragments.len() > 1).then(|| {
                let (has_left, has_right) = if ltr {
                    (has_start_edge, has_end_edge)
                } else {
                    (has_end_edge, has_start_edge)
                };
                let mut clip_outsets = outsets;
                if !has_left {
                    clip_outsets.left = 0.0;
                }
                if !has_right {
                    clip_outsets.right = 0.0;
                }
                outset_border_image_rect(rect, clip_outsets)
            });
            images.insert(
                fragment.id,
                BorderImageGeometry {
                    area: outset_border_image_rect(strip, outsets),
                    widths,
                    fragment_clip,
                },
            );
        }
    }
    images
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_inline_box_fragments<N>(
    projection: &PaintProjection<'_, N>,
    owner_id: LayoutBoxId,
    selected_fragment: Option<usize>,
    paint_space: PaintSpace,
    include_backgrounds: bool,
    border_images: &InlineBorderImages,
    snapshot: &mut PaintSnapshot,
    local_cull: Option<LayoutRect>,
    text_clip_mask: &mut impl FnMut(TextClipMaskScope, &mut PaintSnapshot),
) where
    N: Copy + Debug + Eq + Hash,
{
    let world = projection.world;
    let owner = &world.boxes[owner_id];
    let Some(context) = owner.inline_layout.as_ref() else {
        return;
    };
    let owner_layout = owner.final_layout;
    let origin_x = owner_layout.border.left + owner_layout.padding.left;
    let origin_y = owner_layout.border.top + owner_layout.padding.top;
    let containing_width = (owner_layout.size.width
        - owner_layout.border.left
        - owner_layout.border.right
        - owner_layout.padding.left
        - owner_layout.padding.right)
        .max(0.0);

    let fragments =
        selected_fragment.map_or(0..context.fragments.boxes.len(), |index| index..index + 1);
    let projected_fragments = projection.inline_fragments(owner_id);
    let visible_lines = context.visible_lines(context.fragments.lines.len());
    let pixel_grid = snapshot.pixel_grid();
    let paint_cull = local_cull.map(|rect| paint_space.pre_transform_rect(rect));
    for index in fragments {
        let fragment = &context.fragments.boxes[index];
        if fragment.line_index >= visible_lines {
            continue;
        }
        let Some(inline_box) = world.box_by_id(fragment.box_id) else {
            continue;
        };
        if !inline_box.style.is_visible() {
            continue;
        }
        let style = inline_box.style();
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
        let ltr = style.direction() == InlineDirection::Ltr;
        let has_start = fragment.has_start_edge && inline_box.has_inline_decoration_edge(true);
        let has_end = fragment.has_end_edge && inline_box.has_inline_decoration_edge(false);
        let has_left_edge = if ltr { has_start } else { has_end };
        let has_right_edge = if ltr { has_end } else { has_start };
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
        let rect = LayoutRect::new(
            origin_x + fragment.rect.x + left_margin,
            origin_y + fragment.rect.y - padding.top - border.top,
            (fragment.rect.width - left_margin - right_margin).max(0.0),
            fragment.rect.height + padding.top + padding.bottom + border.top + border.bottom,
        );
        if rect.width <= 0.0 || rect.height <= 0.0 {
            continue;
        }
        let widths = PaintEdgeSizes::new(
            border.top,
            if has_right_edge { border.right } else { 0.0 },
            border.bottom,
            if has_left_edge { border.left } else { 0.0 },
        );
        let border_image = border_images.get(&projected_fragments[index].id).copied();
        if paint_cull.is_some_and(|cull| {
            let snap = |rect| paint_space.pixel_snapped_rect(rect, pixel_grid);
            let mut ink = snap(rect).unwrap_or_else(|| paint_space.pre_transform_rect(rect));
            if let Some(border_ink) = border_image.and_then(|image| snap(image.ink_bounds())) {
                ink = ink.union(border_ink);
            }
            !super::cull::rects_intersect(ink, cull)
        }) {
            continue;
        }
        let color = style.background_color();
        let radii = style.border_radii(rect.width, rect.height);
        let padding_widths = PaintEdgeSizes::new(
            padding.top,
            if has_right_edge { padding.right } else { 0.0 },
            padding.bottom,
            if has_left_edge { padding.left } else { 0.0 },
        );
        let padding_rect = inset_rect(rect, widths);
        let content_rect = inset_rect(padding_rect, padding_widths);
        let areas = BoxAreas {
            margin_rect: LayoutRect::new(
                rect.x - left_margin,
                rect.y - margin.top,
                (rect.width + left_margin + right_margin).max(0.0),
                (rect.height + margin.top + margin.bottom).max(0.0),
            ),
            border_rect: rect,
            padding_rect,
            content_rect,
            border_radii: radii,
            padding_radii: inset_radii(radii, widths),
            content_radii: inset_radii(
                radii,
                PaintEdgeSizes::new(
                    widths.top + padding_widths.top,
                    widths.right + padding_widths.right,
                    widths.bottom + padding_widths.bottom,
                    widths.left + padding_widths.left,
                ),
            ),
        };
        // Flattened inline descendants share the owner's Parley output. Give
        // the callback an explicit target so text.rs selects this structural
        // inline's glyph runs before background.rs applies the fragment clip.
        let mut project_text_clip_mask = |snapshot: &mut PaintSnapshot| {
            text_clip_mask(TextClipMaskScope::InlineBox(fragment.box_id), snapshot);
        };
        if include_backgrounds {
            project_background_color(
                inline_box,
                areas,
                paint_space,
                color,
                snapshot,
                &mut project_text_clip_mask,
            );
            project_background_layers(
                inline_box,
                areas,
                paint_space,
                projection.css_images,
                snapshot,
                &mut project_text_clip_mask,
            );
        }

        project_box_border(
            inline_box,
            rect,
            widths,
            radii,
            border_image,
            paint_space,
            projection.css_images,
            || local_cull,
            snapshot,
        );
    }
}

fn inset_rect(rect: LayoutRect, widths: PaintEdgeSizes) -> LayoutRect {
    let top = widths.top.max(0.0);
    let right = widths.right.max(0.0);
    let bottom = widths.bottom.max(0.0);
    let left = widths.left.max(0.0);
    LayoutRect::new(
        rect.x + left,
        rect.y + top,
        (rect.width - left - right).max(0.0),
        (rect.height - top - bottom).max(0.0),
    )
}
