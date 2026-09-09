//! Fieldset decoration geometry, separate from its CSSOM border box.

use std::{fmt::Debug, hash::Hash};

use crate::{
    LayoutBoxId, LayoutRect, PaintPath, PaintPathElement, PaintPoint, PaintShape,
    projection::{OutputProjection, PaintSpace},
};

#[derive(Clone, Copy)]
pub(super) struct FieldsetDecoration {
    pub(super) border_rect: LayoutRect,
    legend_cutout: LayoutRect,
}

impl FieldsetDecoration {
    pub(super) fn for_box<N: Copy + Debug + Eq + Hash>(
        projection: &OutputProjection<'_, N>,
        id: LayoutBoxId,
    ) -> Option<Self> {
        let legend = projection.world.fieldset_children(id)?.legend?;
        let fieldset = &projection.world.boxes[id.index()];
        let layout = fieldset.final_layout;
        let legend = projection.world.boxes[legend.index()].final_layout;
        let mode = fieldset.style.writing_mode();
        // In-flow geometry already excludes relative offsets. CSS transforms
        // and scrolling must not move the hole cut into the fieldset border.
        let legend_origin = legend.in_flow.map_or(legend.location, |flow| flow.location);
        let border = taffy::WritingDirection::new(mode, fieldset.style.taffy.direction)
            .to_logical_box_strut(layout.border)
            .block_start;
        let legend_block = mode.to_logical(legend.size).block_size;
        let inset = ((legend_block - border) / 2.0).max(0.0);
        let mut border_rect = projection.boxes[id.index()].border_box;
        let legend_cutout = if mode.is_horizontal() {
            border_rect.y += inset;
            border_rect.height = (border_rect.height - inset).max(0.0);
            LayoutRect::new(
                legend_origin.x,
                0.0,
                legend.size.width,
                legend_block.max(border),
            )
        } else {
            if !mode.is_block_flow_reversed() {
                border_rect.x += inset;
            }
            border_rect.width = (border_rect.width - inset).max(0.0);
            LayoutRect::new(
                if mode.is_block_flow_reversed() {
                    layout.size.width - legend_block.max(border)
                } else {
                    0.0
                },
                legend_origin.y,
                legend_block.max(border),
                legend.size.height,
            )
        };
        Some(Self {
            border_rect,
            legend_cutout,
        })
    }

    pub(super) fn border_clip(
        self,
        paint_space: PaintSpace,
        outer: LayoutRect,
    ) -> Option<PaintShape> {
        let outer = crate::pixel_snap_paint_rect(paint_space.pre_transform_rect(outer))?;
        let cutout =
            crate::pixel_snap_paint_rect(paint_space.pre_transform_rect(self.legend_cutout))?;
        use PaintPathElement::{Close, LineTo, MoveTo};
        // Opposite winding cuts a hole without an isolated compositing layer
        // or repainting the fieldset background over the legend's position.
        Some(PaintShape::Path(PaintPath {
            bounds: outer.union(cutout),
            elements: vec![
                MoveTo(PaintPoint::new(outer.x, outer.y)),
                LineTo(PaintPoint::new(outer.right(), outer.y)),
                LineTo(PaintPoint::new(outer.right(), outer.bottom())),
                LineTo(PaintPoint::new(outer.x, outer.bottom())),
                Close,
                MoveTo(PaintPoint::new(cutout.x, cutout.y)),
                LineTo(PaintPoint::new(cutout.x, cutout.bottom())),
                LineTo(PaintPoint::new(cutout.right(), cutout.bottom())),
                LineTo(PaintPoint::new(cutout.right(), cutout.y)),
                Close,
            ],
        }))
    }
}
