// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Optional `bytemuck` trait impls.

#![allow(
    unsafe_code,
    reason = "The `bytemuck` marker traits are `unsafe` and require `unsafe impl`."
)]

use crate::{BidiLevel, GenericFamily, NormalizedCoord};
use bytemuck::{
    Contiguous, NoUninit, Pod, TransparentWrapper, Zeroable, checked::CheckedBitPattern,
};

// Safety: The enum is `repr(u8)` and has only fieldless variants.
unsafe impl NoUninit for GenericFamily {}

// Safety: The enum is `repr(u8)` and `0` is a valid value.
unsafe impl Zeroable for GenericFamily {}

// Safety: The enum is `repr(u8)`.
unsafe impl CheckedBitPattern for GenericFamily {
    type Bits = u8;

    fn is_valid_bit_pattern(bits: &u8) -> bool {
        // Don't need to compare against MIN_VALUE as this is u8 and 0 is the MIN_VALUE.
        *bits <= Self::MAX_VALUE
    }
}

// Safety: The enum is `repr(u8)`. All values are `u8` and fall within
// the min and max values.
unsafe impl Contiguous for GenericFamily {
    type Int = u8;
    const MIN_VALUE: u8 = Self::Serif as u8;
    #[allow(
        clippy::use_self,
        reason = "Using `Self::MAX_VALUE` here would refer to `Contiguous::MAX_VALUE` (self-reference)."
    )]
    const MAX_VALUE: u8 = GenericFamily::MAX_VALUE;
}

// Safety: The struct is `repr(transparent)`, wrapping a `u8`.
//
// While generally BidiLevels have a maximum of 125, no value is unsound.
unsafe impl Pod for BidiLevel {}

// Safety: The struct is `repr(transparent)`, wrapping a `u8`.
unsafe impl Zeroable for BidiLevel {}

// Safety: The struct is `repr(transparent)`, wrapping an `i16`. All bit patterns are valid.
unsafe impl Pod for NormalizedCoord {}

// Safety: The struct is `repr(transparent)`, wrapping an `i16`.
unsafe impl Zeroable for NormalizedCoord {}

// Safety: The struct is `repr(transparent)`, wrapping an `i16`, and has no other fields.
unsafe impl TransparentWrapper<i16> for NormalizedCoord {}

#[cfg(test)]
mod tests {
    use bytemuck::{Contiguous, TransparentWrapper, Zeroable, checked::try_from_bytes};
    use core::ptr;

    use super::{BidiLevel, GenericFamily, NormalizedCoord};

    #[test]
    fn checked_bit_pattern() {
        let valid = bytemuck::bytes_of(&2_u8);
        let invalid = bytemuck::bytes_of(&200_u8);

        assert_eq!(
            Ok(&GenericFamily::Monospace),
            try_from_bytes::<GenericFamily>(valid)
        );

        assert!(try_from_bytes::<GenericFamily>(invalid).is_err());
    }

    #[test]
    fn contiguous() {
        let hd1 = GenericFamily::SansSerif;
        let hd2 = GenericFamily::from_integer(hd1.into_integer());
        assert_eq!(Some(hd1), hd2);

        assert_eq!(None, GenericFamily::from_integer(255));
    }

    #[test]
    fn zeroable() {
        let hd = GenericFamily::zeroed();
        assert_eq!(hd, GenericFamily::Serif);
    }

    /// Tests that the [`Contiguous`] impl for [`GenericFamily`] is not trivially incorrect.
    const _: () = {
        let mut value = 0;
        while value <= GenericFamily::MAX_VALUE {
            // Safety: In a const context, therefore if this makes an invalid GenericFamily, that will be detected.
            let it: GenericFamily = unsafe { ptr::read((&raw const value).cast()) };
            // Evaluate the enum value to ensure it actually has a valid tag.
            if it as u8 != value {
                unreachable!();
            }
            value += 1;
        }
    };

    #[test]
    fn normalized_coord_cast_slice() {
        let coords = [
            NormalizedCoord::from_bits(-16384),
            NormalizedCoord::from_bits(0),
            NormalizedCoord::from_bits(8192),
        ];
        let bits: &[i16] = bytemuck::cast_slice(&coords);
        assert_eq!(bits, [-16384, 0, 8192]);
        let back: &[NormalizedCoord] = bytemuck::cast_slice(bits);
        assert_eq!(back, coords);
    }

    #[test]
    fn normalized_coord_transparent_wrapper() {
        assert_eq!(NormalizedCoord::zeroed(), NormalizedCoord::from_bits(0));
        let coords = [NormalizedCoord::from_bits(1), NormalizedCoord::from_bits(2)];
        assert_eq!(NormalizedCoord::peel_slice(&coords), [1, 2]);
        assert_eq!(
            NormalizedCoord::wrap_slice(&[3, 4]),
            [NormalizedCoord::from_bits(3), NormalizedCoord::from_bits(4)]
        );
    }

    /// Tests that [`NormalizedCoord`] is two bytes.
    ///
    /// That may catch its representation changing, in which case the implementations here
    /// definitely need revisiting.
    const _: () = {
        if size_of::<NormalizedCoord>() != 2 {
            panic!("`NormalizedCoord` is not two bytes");
        }
    };

    /// Tests that [`BidiLevel`] is one byte.
    ///
    /// That may catch its representation changing, in which case the implementations here
    /// definitely need revisiting.
    const _: () = {
        if size_of::<BidiLevel>() != 1 {
            panic!("`BidiLevel` is not one byte");
        }
    };
}

#[cfg(doctest)]
/// Doctests aren't collected under `cfg(test)`; we can use `cfg(doctest)` instead.
mod doctests {
    /// Validates that any new variants in `GenericFamily` has led to a change in the `Contiguous`
    /// impl.
    ///
    /// ```compile_fail,E0080
    /// use bytemuck::Contiguous;
    /// use parlance::GenericFamily;
    /// const {
    ///     let value = GenericFamily::MAX_VALUE + 1;
    ///     // Safety: In a const context, therefore if this makes an invalid GenericFamily, that will be detected.
    ///     // (Indeed, we rely upon that)
    ///     let it: GenericFamily = unsafe { core::ptr::read((&raw const value).cast()) };
    ///     // Evaluate the enum value to ensure it actually has an invalid tag.
    ///     if it as u8 != value {
    ///         unreachable!();
    ///     }
    /// }
    /// ```
    const _GENERIC_FAMILY: () = {};
}
