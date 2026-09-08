//! Inline box-fragment background and border projection.
//!
//! Blitz currently paints inline background color alongside glyphs but has no
//! general inline-border or image-layer fragment implementation. This adapter
//! consumes the one-shot IFC fragment sidecar, preserves start/end edges across
//! line fragmentation, and emits the same owned background/image/border
//! primitives used by block boxes.

use std::{fmt::Debug, hash::Hash};

use taffy::ResolveOrZero;

use super::{
    PaintSpace,
    background::{project_background_color, project_background_layers},
    geometry::{BoxAreas, inset_radii},
    text::TextClipMaskScope,
};
use crate::{
    LayoutBox, LayoutPoint, LayoutRect, LayoutWorld, PaintEdgeSizes, PaintFragment, PaintSnapshot,
};

pub(super) fn project_inline_box_fragments<N>(
    world: &LayoutWorld<N>,
    owner: &LayoutBox<N>,
    paint_space: PaintSpace,
    include_backgrounds: bool,
    snapshot: &mut PaintSnapshot,
    local_cull: Option<LayoutRect>,
    text_clip_mask: &mut impl FnMut(TextClipMaskScope, &mut PaintSnapshot),
) where
    N: Copy + Debug + Eq + Hash,
{
    let Some(context) = owner.inline_layout.as_ref() else {
        return;
    };
    let owner_layout = owner.final_layout;
    let origin = LayoutPoint::new(
        owner_layout.border.left + owner_layout.padding.left,
        owner_layout.border.top + owner_layout.padding.top,
    );
    let containing_width = (owner_layout.size.width
        - owner_layout.border.left
        - owner_layout.border.right
        - owner_layout.padding.left
        - owner_layout.padding.right)
        .max(0.0);

    for fragment in &context.fragments.boxes {
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
        let box_model = fragment.box_model.translated(origin);
        let rect = box_model.border;
        if rect.width <= 0.0 || rect.height <= 0.0 {
            continue;
        }
        if local_cull.is_some_and(|cull| !super::cull::rects_intersect(rect, cull)) {
            continue;
        }
        let color = style.background_color();
        let radii = style.border_radii(rect.width, rect.height);
        let widths = PaintEdgeSizes::new(
            border.top,
            if has_right_edge { border.right } else { 0.0 },
            border.bottom,
            if has_left_edge { border.left } else { 0.0 },
        );
        let padding_widths = PaintEdgeSizes::new(
            padding.top,
            if has_right_edge { padding.right } else { 0.0 },
            padding.bottom,
            if has_left_edge { padding.left } else { 0.0 },
        );
        let areas = BoxAreas {
            margin_rect: box_model.margin,
            border_rect: rect,
            padding_rect: box_model.padding,
            content_rect: box_model.content,
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
                snapshot,
                &mut project_text_clip_mask,
            );
        }

        let colors = style.border_colors();
        if widths.has_positive_edge() && colors.has_visible_edge() {
            snapshot.push_fragment(PaintFragment::Border {
                rect: paint_space.pre_transform_rect(rect),
                widths,
                colors,
                styles: style.border_styles(),
                radii,
                transform: paint_space.property_transform(),
            });
        }
    }
}
