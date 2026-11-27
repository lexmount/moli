//! Shared CSS box geometry for paint projection.

use std::{fmt::Debug, hash::Hash};

use style::properties::generated::longhands::{
    background_clip::single_value::computed_value::T as StyloBackgroundClip,
    background_origin::single_value::computed_value::T as StyloBackgroundOrigin,
    mask_origin::single_value::computed_value::T as StyloMaskOrigin,
};

use crate::{
    LayoutBoxId, LayoutRect, PaintCornerRadii, PaintEdgeSizes, PaintShape,
    projection::PaintProjection,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BoxModelBox {
    Border,
    Padding,
    Content,
    BorderArea,
    Text,
}

impl From<StyloBackgroundClip> for BoxModelBox {
    fn from(value: StyloBackgroundClip) -> Self {
        match value {
            StyloBackgroundClip::BorderBox => Self::Border,
            StyloBackgroundClip::PaddingBox => Self::Padding,
            StyloBackgroundClip::ContentBox => Self::Content,
            StyloBackgroundClip::BorderArea => Self::BorderArea,
            StyloBackgroundClip::Text => Self::Text,
        }
    }
}

impl From<StyloBackgroundOrigin> for BoxModelBox {
    fn from(value: StyloBackgroundOrigin) -> Self {
        match value {
            StyloBackgroundOrigin::BorderBox => Self::Border,
            StyloBackgroundOrigin::PaddingBox => Self::Padding,
            StyloBackgroundOrigin::ContentBox => Self::Content,
        }
    }
}

impl From<StyloMaskOrigin> for BoxModelBox {
    fn from(value: StyloMaskOrigin) -> Self {
        match value {
            StyloMaskOrigin::BorderBox => Self::Border,
            StyloMaskOrigin::PaddingBox => Self::Padding,
            StyloMaskOrigin::ContentBox => Self::Content,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct BoxAreas {
    pub(super) margin_rect: LayoutRect,
    pub(super) border_rect: LayoutRect,
    pub(super) padding_rect: LayoutRect,
    pub(super) content_rect: LayoutRect,
    pub(super) border_radii: PaintCornerRadii,
    pub(super) padding_radii: PaintCornerRadii,
    pub(super) content_radii: PaintCornerRadii,
}

impl BoxAreas {
    pub(super) fn for_box<N>(projection: &PaintProjection<'_, N>, id: LayoutBoxId) -> Self
    where
        N: Copy + Debug + Eq + Hash,
    {
        let layout_box = &projection.world.boxes[id];
        let geometry = projection.geometry(projection.world.boxes.index_of(id));
        let layout = layout_box.final_layout;
        // CSS ignores border-radius on boxes participating in the collapsed
        // table border model. Their shared grid edges are painted by the table
        // owner, and table-part backgrounds remain rectangular.
        let border_radii = if layout_box.collapsed_table_border_part {
            PaintCornerRadii::ZERO
        } else {
            layout_box
                .style
                .border_radii(geometry.border_box.width, geometry.border_box.height)
        };
        let border = PaintEdgeSizes::new(
            layout.border.top,
            layout.border.right,
            layout.border.bottom,
            layout.border.left,
        );
        let padding = PaintEdgeSizes::new(
            layout.padding.top,
            layout.padding.right,
            layout.padding.bottom,
            layout.padding.left,
        );
        Self {
            margin_rect: geometry.margin_box,
            border_rect: geometry.border_box,
            padding_rect: geometry.padding_box,
            content_rect: geometry.content_box,
            border_radii,
            padding_radii: inset_radii(border_radii, border),
            content_radii: inset_radii(
                border_radii,
                PaintEdgeSizes::new(
                    border.top + padding.top,
                    border.right + padding.right,
                    border.bottom + padding.bottom,
                    border.left + padding.left,
                ),
            ),
        }
    }

    pub(super) fn for_rect(rect: LayoutRect) -> Self {
        Self {
            margin_rect: rect,
            border_rect: rect,
            padding_rect: rect,
            content_rect: rect,
            border_radii: PaintCornerRadii::ZERO,
            padding_radii: PaintCornerRadii::ZERO,
            content_radii: PaintCornerRadii::ZERO,
        }
    }

    pub(super) const fn rect(self, area: BoxModelBox) -> LayoutRect {
        match area {
            BoxModelBox::Border => self.border_rect,
            BoxModelBox::Padding => self.padding_rect,
            BoxModelBox::Content | BoxModelBox::Text => self.content_rect,
            BoxModelBox::BorderArea => self.border_rect,
        }
    }

    pub(super) fn shape(self, area: BoxModelBox) -> PaintShape {
        let (rect, radii) = match area {
            BoxModelBox::Border | BoxModelBox::BorderArea => (self.border_rect, self.border_radii),
            BoxModelBox::Padding => (self.padding_rect, self.padding_radii),
            BoxModelBox::Content | BoxModelBox::Text => (self.content_rect, self.content_radii),
        };
        canonical_shape(rect, radii)
    }
}

pub(super) fn inset_radii(radii: PaintCornerRadii, widths: PaintEdgeSizes) -> PaintCornerRadii {
    let inset = |radius: crate::PaintCornerRadius, horizontal: f32, vertical: f32| {
        crate::PaintCornerRadius::new(
            (radius.x - horizontal.max(0.0)).max(0.0),
            (radius.y - vertical.max(0.0)).max(0.0),
        )
    };
    PaintCornerRadii {
        top_left: inset(radii.top_left, widths.left, widths.top),
        top_right: inset(radii.top_right, widths.right, widths.top),
        bottom_right: inset(radii.bottom_right, widths.right, widths.bottom),
        bottom_left: inset(radii.bottom_left, widths.left, widths.bottom),
    }
}

pub(super) fn canonical_shape(rect: LayoutRect, radii: PaintCornerRadii) -> PaintShape {
    if radii.is_zero() {
        PaintShape::Rect(rect)
    } else {
        PaintShape::RoundedRect { rect, radii }
    }
}
