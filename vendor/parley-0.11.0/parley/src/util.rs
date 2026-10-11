// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Misc helpers.

pub(crate) fn nearly_eq(x: f32, y: f32) -> bool {
    (x - y).abs() < f32::EPSILON
}

pub(crate) fn nearly_zero(x: f32) -> bool {
    nearly_eq(x, 0.)
}

/// A bounding box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoundingBox {
    /// The left edge.
    pub x0: f64,
    /// The top edge.
    pub y0: f64,
    /// The right edge.
    pub x1: f64,
    /// The bottom edge.
    pub y1: f64,
}

impl BoundingBox {
    /// A new bounding box from minimum and maximum coordinates.
    #[inline(always)]
    pub const fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// The width of the bounding box.
    ///
    /// Note: nothing forbids negative width.
    #[inline]
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    /// The height of the bounding box.
    ///
    /// Note: nothing forbids negative height.
    #[inline]
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    /// The smallest bounding box enclosing two bounding boxes.
    ///
    /// Results are valid only if width and height are non-negative.
    #[inline]
    pub fn union(&self, other: Self) -> Self {
        Self::new(
            self.x0.min(other.x0),
            self.y0.min(other.y0),
            self.x1.max(other.x1),
            self.y1.max(other.y1),
        )
    }
}
