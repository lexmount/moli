/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Animation implementations for text-related types.

use super::{Animate, Procedure, ToAnimatedZero};
use crate::values::computed::LengthPercentage;
use crate::values::{ComputeSquaredDistance, MallocSizeOf, SquaredDistance};

/// A `text-decoration-inset` value for animation: we resolve `auto` to a pair of lengths,
/// but record whether the original value was actually `auto`.
#[derive(Clone, Debug, MallocSizeOf, PartialEq)]
pub struct TextDecorationInset {
    /// The inset at the start of the decoration.
    pub start: LengthPercentage,
    /// The inset at the end of the decoration.
    pub end: LengthPercentage,
    /// Whether this represents a resolved `auto` value.
    pub is_auto: bool,
}

impl Animate for TextDecorationInset {
    fn animate(&self, other: &Self, procedure: Procedure) -> Result<Self, ()> {
        Ok(if self.is_auto && other.is_auto {
            self.clone()
        } else {
            Self {
                start: self.start.animate(&other.start, procedure)?,
                end: self.end.animate(&other.end, procedure)?,
                is_auto: false,
            }
        })
    }
}

impl ComputeSquaredDistance for TextDecorationInset {
    #[inline]
    fn compute_squared_distance(&self, other: &Self) -> Result<SquaredDistance, ()> {
        Ok(if self.is_auto && other.is_auto {
            SquaredDistance::from_sqrt(0.)
        } else {
            self.start.compute_squared_distance(&other.start)?
                + self.end.compute_squared_distance(&other.end)?
        })
    }
}

impl ToAnimatedZero for TextDecorationInset {
    #[inline]
    fn to_animated_zero(&self) -> Result<Self, ()> {
        Ok(Self {
            start: self.start.to_animated_zero()?,
            end: self.end.to_animated_zero()?,
            is_auto: false,
        })
    }
}
