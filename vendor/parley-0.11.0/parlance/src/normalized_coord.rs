// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// A normalized font coordinate.
///
/// This is a 16-bit fixed-point number with a 14-bit fractional part. For font coordinates, its
/// useful values are in the range -1.0..=1.0.
//
// NOTICE: If the representation changes, be sure to check the `bytemuck` marker trait
// implementations.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NormalizedCoord(i16);

impl NormalizedCoord {
    /// Create a fixed-point value from the bit representation.
    ///
    /// The [`i16`] is interpreted as `bits / 16384`.
    #[inline(always)]
    pub fn from_bits(bits: i16) -> Self {
        Self(bits)
    }

    /// Create a fixed-point value from the bit representation.
    ///
    /// The `i16` can be interpreted as `bits / 16384`.
    #[inline(always)]
    pub fn to_bits(self) -> i16 {
        self.0
    }

    /// The value of the normalized coordinate, represented as a floating-point number.
    ///
    /// This will usually be in the inclusive range `-1.0..=1.0`, and is guaranteed to be in the
    /// half-open range `-2.0..2.0`.
    #[inline]
    pub fn to_f32(self) -> f32 {
        f32::from(self.0) / (1 << 14) as f32
    }
}
