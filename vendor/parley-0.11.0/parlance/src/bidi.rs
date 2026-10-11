// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// An inline bidirectional text control.
///
/// These map conceptually to Unicode bidi isolate/override controls. They are expressed as style
/// properties rather than literal control characters.
///
/// For background on bidi behavior see UAX #9:
/// <https://www.unicode.org/reports/tr9/>
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BidiControl {
    /// No explicit control.
    #[default]
    None,
    /// Isolate this span with the given direction.
    Isolate(BidiDirection),
    /// Override directional resolution within this span.
    Override(BidiOverride),
}

/// Direction choice used by bidi controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BidiDirection {
    /// Choose direction automatically.
    Auto,
    /// Left-to-right.
    Ltr,
    /// Right-to-left.
    Rtl,
}

/// Direction choice used by bidi overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BidiOverride {
    /// Force left-to-right.
    Ltr,
    /// Force right-to-left.
    Rtl,
}

/// Bidirectional text embedding level.
///
/// These are numbers indicating how deeply bidirectional embeddings are nested in the text, and the
/// default direction of text on that level. Even levels are left-to-right, odd levels are
/// right-to-left. Normally, the minimum level is 0 (left-to-right), and the maximum level,
/// according to [UAX #9 § 3.1.1 BD2][uax-bd2], is 125.
///
/// See [UAX #9 § 3.1][uax-definitions] for more information.
///
/// [uax-definitions]: https://unicode.org/reports/tr9/#Definitions
/// [uax-bd2]: https://unicode.org/reports/tr9/#BD2
///
// NOTICE: If the representation changes, be sure to check the `bytemuck` marker trait
// implementations.
//
// TODO: it would be quite nice for this to implement
// <https://doc.rust-lang.org/stable/core/iter/trait.Step.html>, once stabilized.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(transparent)]
pub struct BidiLevel(u8);

impl BidiLevel {
    /// The maximum bidirectional text embedding level, according to [UAX #9 § 3.1.1 BD2][uax-bd2].
    ///
    /// It is possible for `BidiLevel` to encode greater values; in particular, `unsafe` code **must
    /// not** rely on `BidiLevel` never being greater than this.
    ///
    /// [uax-bd2]: https://unicode.org/reports/tr9/#BD2
    pub const MAX: Self = Self(125);

    /// Construct a new bidi level.
    #[inline(always)]
    pub const fn new(level: u8) -> Self {
        Self(level)
    }

    /// Get the numeric bidi level.
    #[inline(always)]
    pub const fn to_u8(self) -> u8 {
        self.0
    }

    /// Whether this level is left-to-right.
    #[inline(always)]
    pub const fn is_ltr(self) -> bool {
        self.0.is_multiple_of(2)
    }

    /// Whether this level is right-to-left.
    #[inline(always)]
    pub const fn is_rtl(self) -> bool {
        !self.is_ltr()
    }

    /// Get the next odd bidi level.
    ///
    /// When the return value overflows (`self.to_u8() >= 255`) this panics when overflow checks are
    /// enabled. Otherwise, the return value wraps.
    #[inline(always)]
    pub const fn next_odd(self) -> Self {
        Self::new((self.to_u8() + 1) | 1)
    }

    /// Get the next even bidi level.
    ///
    /// When the return value overflows (`self.to_u8() >= 254`) this panics when overflow checks are
    /// enabled. Otherwise, the return value wraps.
    #[inline(always)]
    pub const fn next_even(self) -> Self {
        Self::new((self.to_u8() + 2) & !1)
    }
}
