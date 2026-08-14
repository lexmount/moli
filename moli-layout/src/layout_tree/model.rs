//! Canonical geometry values and node-local data stored by a frozen tree.

use std::ops::Range;

use crate::LayoutPosition;

/// Viewport inputs shared by layout, geometry queries, and paint projection.
///
/// Dimensions are CSS pixels. Device-pixel conversion belongs to the paint
/// backend and never changes the geometry stored in a frozen layout tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutViewport {
    pub css_width: u32,
    pub css_height: u32,
    pub device_pixel_ratio: f32,
}

impl LayoutViewport {
    pub const fn new(css_width: u32, css_height: u32, device_pixel_ratio: f32) -> Self {
        Self {
            css_width,
            css_height,
            device_pixel_ratio,
        }
    }
}

/// A two-dimensional point in CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutPoint {
    pub x: f32,
    pub y: f32,
}

impl LayoutPoint {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A two-dimensional extent in CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutSize {
    pub width: f32,
    pub height: f32,
}

impl LayoutSize {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// An axis-aligned rectangle in one explicit layout coordinate space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LayoutRect {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub fn contains(self, point: LayoutPoint) -> bool {
        self.width >= 0.0
            && self.height >= 0.0
            && point.x >= self.x
            && point.x < self.right()
            && point.y >= self.y
            && point.y < self.bottom()
    }

    pub fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }
}

/// Four corners of a transformed CSS box in top-left, top-right,
/// bottom-right, bottom-left order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutQuad {
    pub points: [LayoutPoint; 4],
}

impl LayoutQuad {
    pub fn bounding_rect(self) -> LayoutRect {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for point in self.points {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
        if ![min_x, min_y, max_x, max_y].into_iter().all(f32::is_finite) {
            return LayoutRect::ZERO;
        }
        LayoutRect::new(
            min_x,
            min_y,
            (max_x - min_x).max(0.0),
            (max_y - min_y).max(0.0),
        )
    }
}

/// A CSS-pixel 2D affine transform.
///
/// Coefficients use the CSS matrix order `[a, b, c, d, e, f]`, where
/// `x' = a*x + c*y + e` and `y' = b*x + d*y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutTransform2D {
    pub coefficients: [f64; 6],
}

impl LayoutTransform2D {
    pub const IDENTITY: Self = Self::new([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub const fn new(coefficients: [f64; 6]) -> Self {
        Self { coefficients }
    }

    pub fn translation(x: f32, y: f32) -> Self {
        Self::new([1.0, 0.0, 0.0, 1.0, f64::from(x), f64::from(y)])
    }

    pub fn scale(x: f64, y: f64) -> Self {
        Self::new([x, 0.0, 0.0, y, 0.0, 0.0])
    }

    pub fn rotation(radians: f64) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self::new([cos, sin, -sin, cos, 0.0, 0.0])
    }

    /// Concatenates a child transform after this parent transform.
    ///
    /// The returned matrix maps a child-local point by `child` first, then by
    /// `self`. This is the operation used while walking coordinate spaces.
    pub fn concatenate(self, child: Self) -> Self {
        let [a, b, c, d, e, f] = self.coefficients;
        let [g, h, i, j, k, l] = child.coefficients;
        Self::new([
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ])
    }

    pub fn inverse(self) -> Option<Self> {
        let [a, b, c, d, e, f] = self.coefficients;
        let determinant = a * d - b * c;
        if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
            return None;
        }
        let inverse = 1.0 / determinant;
        Some(Self::new([
            d * inverse,
            -b * inverse,
            -c * inverse,
            a * inverse,
            (c * f - d * e) * inverse,
            (b * e - a * f) * inverse,
        ]))
    }

    pub fn map_point(self, point: LayoutPoint) -> LayoutPoint {
        let [a, b, c, d, e, f] = self.coefficients;
        let x = f64::from(point.x);
        let y = f64::from(point.y);
        LayoutPoint::new((a * x + c * y + e) as f32, (b * x + d * y + f) as f32)
    }

    pub fn map_rect(self, rect: LayoutRect) -> LayoutQuad {
        LayoutQuad {
            points: [
                self.map_point(LayoutPoint::new(rect.x, rect.y)),
                self.map_point(LayoutPoint::new(rect.right(), rect.y)),
                self.map_point(LayoutPoint::new(rect.right(), rect.bottom())),
                self.map_point(LayoutPoint::new(rect.x, rect.bottom())),
            ],
        }
    }

    pub fn is_finite(self) -> bool {
        self.coefficients.into_iter().all(f64::is_finite)
    }
}

macro_rules! dense_output_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u32);

        impl $name {
            pub(crate) fn from_index(index: usize) -> Self {
                Self(
                    u32::try_from(index).expect("one frozen layout tree exceeded the u32 id limit"),
                )
            }

            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

dense_output_id!(LayoutOutputBoxId);
dense_output_id!(LayoutFragmentId);
dense_output_id!(LayoutCoordinateSpaceId);
dense_output_id!(LayoutClipChainId);

/// One explicit local coordinate system in a frozen layout tree.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutCoordinateSpace {
    pub(crate) id: LayoutCoordinateSpaceId,
    pub(crate) owner: Option<LayoutOutputBoxId>,
    pub(crate) parent: Option<LayoutCoordinateSpaceId>,
    pub(crate) local_to_parent: LayoutTransform2D,
    /// Maps local coordinates to the visual document coordinate system. This
    /// includes element scrolling but excludes the viewport scroll offset.
    pub(crate) local_to_document: LayoutTransform2D,
    /// Maps local coordinates directly to viewport CSS pixels.
    pub(crate) local_to_viewport: LayoutTransform2D,
}

/// Query-facing coordinate data retained for one frozen box-tree node.
#[derive(Clone, Debug, PartialEq)]
pub struct FrozenCoordinateSpace {
    pub owner: Option<LayoutOutputBoxId>,
    pub local_to_viewport: LayoutTransform2D,
}

impl From<LayoutCoordinateSpace> for FrozenCoordinateSpace {
    fn from(space: LayoutCoordinateSpace) -> Self {
        Self {
            owner: space.owner,
            local_to_viewport: space.local_to_viewport,
        }
    }
}

/// One rectangular clip linked to its ancestor clip.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutClipNode {
    pub parent: Option<LayoutClipChainId>,
    pub owner: Option<LayoutOutputBoxId>,
    pub coordinate_space: LayoutCoordinateSpaceId,
    pub rect: LayoutRect,
}

/// Complete physical box model for one tree-local CSS box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutBoxModel {
    pub content: LayoutQuad,
    pub padding: LayoutQuad,
    pub border: LayoutQuad,
    pub margin: LayoutQuad,
}

/// Per-box scroll geometry in the box's own coordinate space.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutScrollExtent {
    pub scrollport: LayoutRect,
    pub scrollable_overflow: LayoutRect,
    pub scroll_size: LayoutSize,
    pub applied_offset: LayoutPoint,
    pub minimum_offset: LayoutPoint,
    pub maximum_offset: LayoutPoint,
    pub is_scroll_container: bool,
    pub allows_user_scroll_x: bool,
    pub allows_user_scroll_y: bool,
    pub clips_overflow: bool,
}

/// Geometry retained for one tree-local box.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutBoxGeometry {
    pub id: LayoutOutputBoxId,
    pub parent: Option<LayoutOutputBoxId>,
    pub layout_parent: Option<LayoutOutputBoxId>,
    pub position: LayoutPosition,
    pub coordinate_space: LayoutCoordinateSpaceId,
    pub clip_chain: Option<LayoutClipChainId>,
    pub content_box: LayoutRect,
    pub padding_box: LayoutRect,
    pub border_box: LayoutRect,
    pub margin_box: LayoutRect,
    pub fragments: Vec<LayoutFragmentId>,
    /// Untransformed border-box origin in document layout coordinates.
    pub layout_origin_in_document: LayoutPoint,
    pub is_body_element: bool,
    pub is_table_offset_parent: bool,
    pub establishes_positioned_containing_block: bool,
    pub establishes_fixed_containing_block: bool,
    pub visible: bool,
    pub pointer_events: bool,
}

/// Physical boxes retained for one box fragment in that fragment's local
/// coordinate space. Inline elements can own several of these across lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutFragmentBoxModel {
    pub content: LayoutRect,
    pub padding: LayoutRect,
    pub border: LayoutRect,
    pub margin: LayoutRect,
}

/// A geometry fragment kind. IDs contained here are valid only in the same
/// [`crate::FrozenLayoutTree`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutFragmentKind {
    Box {
        box_id: LayoutOutputBoxId,
    },
    Line {
        owner: LayoutOutputBoxId,
        line_index: usize,
    },
    InlineBox {
        box_id: LayoutOutputBoxId,
        line_index: usize,
        has_start_edge: bool,
        has_end_edge: bool,
    },
    Text {
        box_id: LayoutOutputBoxId,
        line_index: usize,
        source_utf16_range: Range<usize>,
        rtl: bool,
    },
}

/// One box/line/inline/text fragment in an explicit coordinate space.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutFragment {
    pub id: LayoutFragmentId,
    pub kind: LayoutFragmentKind,
    pub rect: LayoutRect,
    pub box_model: Option<LayoutFragmentBoxModel>,
    pub coordinate_space: LayoutCoordinateSpaceId,
    pub clip_chain: Option<LayoutClipChainId>,
    pub paint_order: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_concatenation_and_inverse_round_trip() {
        let transform = LayoutTransform2D::translation(20.0, 30.0)
            .concatenate(LayoutTransform2D::rotation(std::f64::consts::FRAC_PI_2))
            .concatenate(LayoutTransform2D::scale(2.0, 3.0));
        let point = LayoutPoint::new(4.0, 5.0);
        let mapped = transform.map_point(point);
        let restored = transform.inverse().expect("invertible").map_point(mapped);
        assert!((restored.x - point.x).abs() <= 0.0001);
        assert!((restored.y - point.y).abs() <= 0.0001);
    }

    #[test]
    fn rectangle_contains_uses_half_open_right_and_bottom_edges() {
        let rect = LayoutRect::new(10.0, 20.0, 30.0, 40.0);
        assert!(rect.contains(LayoutPoint::new(10.0, 20.0)));
        assert!(rect.contains(LayoutPoint::new(39.999, 59.999)));
        assert!(!rect.contains(LayoutPoint::new(40.0, 30.0)));
        assert!(!rect.contains(LayoutPoint::new(20.0, 60.0)));
        assert!(!LayoutRect::new(0.0, 0.0, 0.0, 10.0).contains(LayoutPoint::ZERO));
    }
}
