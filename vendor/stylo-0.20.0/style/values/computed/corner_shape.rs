/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Computed type for the `corner-shape` family of properties.

use crate::derives::*;
use crate::values::animated::{Animate, Procedure, ToAnimatedZero};
use crate::values::distance::{ComputeSquaredDistance, SquaredDistance};
use crate::values::generics::border::GenericCornerShapeRect;
use std::fmt::{self, Write};
use style_traits::{CssWriter, ToCss};

/// The computed value for a single corner shape.
///
/// Per the spec, the computed value is always `superellipse(K)` where `K` may
/// be any real value, including +infinity (for `square`) and -infinity
/// (for `notch`).
#[derive(
    Clone, Copy, Debug, MallocSizeOf, PartialEq, ToShmem, ToTyped, ToAnimatedValue, ToResolvedValue,
)]
#[repr(C)]
#[typed(todo_derive_fields)]
pub struct CornerShape {
    /// The K parameter from the `superellipse()` function.
    pub k: f32,
}

impl CornerShape {
    /// `round` -> superellipse(1).
    #[inline]
    pub fn round() -> Self {
        Self { k: 1.0 }
    }
    /// `bevel` -> superellipse(0).
    #[inline]
    pub fn bevel() -> Self {
        Self { k: 0.0 }
    }
    /// `square` -> superellipse(infinity).
    #[inline]
    pub fn square() -> Self {
        Self { k: f32::INFINITY }
    }
    /// `notch` -> superellipse(-infinity).
    #[inline]
    pub fn notch() -> Self {
        Self {
            k: f32::NEG_INFINITY,
        }
    }
    /// `scoop` -> superellipse(-1).
    #[inline]
    pub fn scoop() -> Self {
        Self { k: -1.0 }
    }
    /// `squircle` -> superellipse(2).
    #[inline]
    pub fn squircle() -> Self {
        Self { k: 2.0 }
    }

    /// Whether this corner is the default `round` shape (K == 1).
    #[inline]
    pub fn is_round(&self) -> bool {
        self.k == 1.0
    }
}

impl ToCss for CornerShape {
    fn to_css<W: Write>(&self, dest: &mut CssWriter<W>) -> fmt::Result {
        dest.write_str("superellipse(")?;
        if self.k == f32::INFINITY {
            dest.write_str("infinity")?;
        } else if self.k == f32::NEG_INFINITY {
            dest.write_str("-infinity")?;
        } else {
            self.k.to_css(dest)?;
        }
        dest.write_char(')')
    }
}

/// Compute the "normalized superellipse half corner" for a superellipse
/// parameter `s`, per
/// <https://drafts.csswg.org/css-borders-4/#corner-shape-interpolation>.
fn s_to_interpolation_value(s: f32) -> f32 {
    if s == f32::NEG_INFINITY {
        return 0.0;
    }
    if s == f32::INFINITY {
        return 1.0;
    }
    let k = 0.5f32.powf(s.abs());
    let convex_half_corner = 0.5f32.powf(k);
    if s < 0.0 {
        1.0 - convex_half_corner
    } else {
        convex_half_corner
    }
}

/// Inverse of `s_to_interpolation_value`.
fn interpolation_value_to_s(v: f32) -> f32 {
    if v <= 0.0 {
        return f32::NEG_INFINITY;
    }
    if v >= 1.0 {
        return f32::INFINITY;
    }
    let convex_half_corner = if v < 0.5 { 1.0 - v } else { v };
    let k = 0.5f32.ln() / convex_half_corner.ln();
    let s = k.log2();
    if v < 0.5 {
        -s
    } else {
        s
    }
}

impl Animate for CornerShape {
    fn animate(&self, other: &Self, procedure: Procedure) -> Result<Self, ()> {
        let a = s_to_interpolation_value(self.k);
        let b = s_to_interpolation_value(other.k);
        let interp = a.animate(&b, procedure)?;
        Ok(CornerShape {
            k: interpolation_value_to_s(interp),
        })
    }
}

impl ComputeSquaredDistance for CornerShape {
    fn compute_squared_distance(&self, other: &Self) -> Result<SquaredDistance, ()> {
        let a = s_to_interpolation_value(self.k);
        let b = s_to_interpolation_value(other.k);
        a.compute_squared_distance(&b)
    }
}

impl ToAnimatedZero for CornerShape {
    fn to_animated_zero(&self) -> Result<Self, ()> {
        Ok(CornerShape::round())
    }
}

/// The four computed corner shapes for an element.
pub type CornerShapeRect = GenericCornerShapeRect<CornerShape>;

impl CornerShapeRect {
    /// Initial value: `round` on every corner.
    #[inline]
    pub fn round_all() -> Self {
        Self::all(CornerShape::round())
    }
}
