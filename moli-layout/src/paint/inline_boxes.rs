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
    LayoutBox, LayoutWorld, PaintEdgeSizes, PaintFragment, PaintSnapshot,
    inline::inline_fragment_box_geometry,
};

pub(super) fn project_inline_box_fragments<N>(
    world: &LayoutWorld<N>,
    owner: &LayoutBox<N>,
    paint_space: PaintSpace,
    include_backgrounds: bool,
    snapshot: &mut PaintSnapshot,
    text_clip_mask: &impl Fn(TextClipMaskScope, &mut PaintSnapshot),
) where
    N: Copy + Debug + Eq + Hash,
{
    let Some(context) = owner.inline_layout.as_ref() else {
        return;
    };
    let owner_layout = owner.final_layout;
    let origin_x = owner_layout.border.left + owner_layout.padding.left;
    let origin_y = owner_layout.border.top + owner_layout.padding.top;
    let content_box_size = taffy::Size {
        width: (owner_layout.size.width
            - owner_layout.border.left
            - owner_layout.border.right
            - owner_layout.padding.left
            - owner_layout.padding.right)
            .max(0.0),
        height: (owner_layout.size.height
            - owner_layout.border.top
            - owner_layout.border.bottom
            - owner_layout.padding.top
            - owner_layout.padding.bottom)
            .max(0.0),
    };
    let containing_inline_size = owner
        .style
        .writing_mode()
        .to_logical(content_box_size)
        .inline_size;

    for fragment in &context.fragments.boxes {
        let Some(inline_box) = world.box_by_id(fragment.box_id) else {
            continue;
        };
        if !inline_box.is_visible_for_paint() {
            continue;
        }
        let style = inline_box.style();
        let padding = style.taffy.padding.resolve_or_zero(
            Some(containing_inline_size),
            crate::style::resolve_stylo_calc_value,
        );
        let border = style.taffy.border.resolve_or_zero(
            Some(containing_inline_size),
            crate::style::resolve_stylo_calc_value,
        );
        let margin = style.taffy.margin.resolve_or_zero(
            Some(containing_inline_size),
            crate::style::resolve_stylo_calc_value,
        );
        let mut geometry = inline_fragment_box_geometry(
            fragment,
            style.writing_direction(),
            margin,
            padding,
            border,
        );
        for rect in [
            &mut geometry.margin_rect,
            &mut geometry.border_rect,
            &mut geometry.padding_rect,
            &mut geometry.content_rect,
        ] {
            rect.x += origin_x;
            rect.y += origin_y;
        }
        let rect = geometry.border_rect;
        if rect.width <= 0.0 || rect.height <= 0.0 {
            continue;
        }
        let color = style.background_color();
        let radii = style.border_radii(rect.width, rect.height);
        let widths = geometry.border_widths;
        let padding_widths = geometry.padding_widths;
        let areas = BoxAreas {
            margin_rect: geometry.margin_rect,
            border_rect: rect,
            padding_rect: geometry.padding_rect,
            content_rect: geometry.content_rect,
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
        let project_text_clip_mask = |snapshot: &mut PaintSnapshot| {
            text_clip_mask(TextClipMaskScope::InlineBox(fragment.box_id), snapshot);
        };
        if include_backgrounds {
            project_background_color(
                inline_box,
                areas,
                paint_space,
                color,
                snapshot,
                &project_text_clip_mask,
            );
            project_background_layers(
                inline_box,
                areas,
                paint_space,
                snapshot,
                &project_text_clip_mask,
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
