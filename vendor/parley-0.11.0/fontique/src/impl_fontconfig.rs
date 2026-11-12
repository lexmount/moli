// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fontconfig numeric value conversions.
//!
//! The numeric values used here are based on the `fonts.conf` documentation:
//! <https://www.freedesktop.org/software/fontconfig/fontconfig-user.html>.

use crate::{FontStyle, FontWeight, FontWidth};

/// Conversion from Fontconfig numeric values.
///
/// This is intentionally kept in `fontique` (rather than `parlance`) because it is a
/// Fontconfig-specific mapping.
///
/// The numeric values used by these conversions are based on the `fonts.conf` documentation:
/// <https://www.freedesktop.org/software/fontconfig/fontconfig-user.html>.
pub trait FromFontconfig: Sized {
    /// Creates a value from the corresponding Fontconfig numeric representation.
    ///
    /// The numeric values are determined based on the `fonts.conf` documentation:
    /// <https://www.freedesktop.org/software/fontconfig/fontconfig-user.html>.
    fn from_fontconfig(value: i32) -> Self;
}

impl FromFontconfig for FontWeight {
    /// Creates a new weight attribute with the given value from Fontconfig.
    ///
    /// The values are determined based on the `fonts.conf` documentation:
    /// <https://www.freedesktop.org/software/fontconfig/fontconfig-user.html>.
    fn from_fontconfig(weight: i32) -> Self {
        // A selection of OpenType weights (first) and their corresponding fontconfig value (second)
        // Invariant: The fontconfig values are sorted.
        const MAP: &[(i32, i32)] = &[
            (0, 0),
            (100, 0),
            (200, 40),
            (300, 50),
            (350, 55),
            (380, 75),
            (400, 80),
            (500, 100),
            (600, 180),
            (700, 200),
            (800, 205),
            (900, 210),
            (950, 215),
        ];

        for i in 1..MAP.len() {
            let (ot_b, fc_b) = MAP[i];
            if weight == fc_b {
                return Self::new(ot_b as f32);
            }

            // Linear interpolation if not an exact match.
            if weight < fc_b {
                let weight = weight as f32;
                let fc_b = fc_b as f32;
                let ot_b = ot_b as f32;

                let (ot_a, fc_a) = MAP[i - 1];
                let fc_a = fc_a as f32;
                let ot_a = ot_a as f32;

                let t = (weight - fc_a) / (fc_b - fc_a);
                return Self::new(ot_a + (ot_b - ot_a) * t);
            }
        }

        Self::EXTRA_BLACK
    }
}

impl FromFontconfig for FontWidth {
    /// Creates a new width attribute with the given value from Fontconfig.
    ///
    /// The values are determined based on the `fonts.conf` documentation:
    /// <https://www.freedesktop.org/software/fontconfig/fontconfig-user.html>.
    fn from_fontconfig(width: i32) -> Self {
        match width {
            50 => Self::ULTRA_CONDENSED,
            63 => Self::EXTRA_CONDENSED,
            75 => Self::CONDENSED,
            87 => Self::SEMI_CONDENSED,
            100 => Self::NORMAL,
            113 => Self::SEMI_EXPANDED,
            125 => Self::EXPANDED,
            150 => Self::EXTRA_EXPANDED,
            200 => Self::ULTRA_EXPANDED,
            _ => Self::from_ratio(width as f32 / 100.0),
        }
    }
}

impl FromFontconfig for FontStyle {
    /// Creates a new style attribute with the given value from Fontconfig.
    ///
    /// The values are determined based on the `fonts.conf` documentation:
    /// <https://www.freedesktop.org/software/fontconfig/fontconfig-user.html>.
    fn from_fontconfig(slant: i32) -> Self {
        match slant {
            100 => Self::Italic,
            110 => Self::Oblique(None),
            _ => Self::Normal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FromFontconfig;
    use crate::{FontStyle, FontWeight, FontWidth};
    use alloc::string::ToString;

    #[test]
    fn fontwidth_from_fontconfig() {
        fn check_fc(fc: i32, s: &str) {
            let fs = FontWidth::from_fontconfig(fc);
            assert_eq!(s, fs.to_string());
        }

        check_fc(50, "ultra-condensed");
        check_fc(63, "extra-condensed");
        check_fc(75, "condensed");
        check_fc(87, "semi-condensed");
        check_fc(100, "normal");
        check_fc(113, "semi-expanded");
        check_fc(125, "expanded");
        check_fc(150, "extra-expanded");
        check_fc(200, "ultra-expanded");
    }

    #[test]
    fn fontstyle_from_fontconfig() {
        fn check_fc(fc: i32, s: &str) {
            let fs = FontStyle::from_fontconfig(fc);
            assert_eq!(s, fs.to_string());
        }

        check_fc(0, "normal");
        check_fc(100, "italic");
        check_fc(110, "oblique");
    }

    #[test]
    fn fontweight_from_fontconfig_interpolates_monotonically() {
        let demilight = FontWeight::from_fontconfig(55);
        assert!(demilight > FontWeight::LIGHT);
        assert!(demilight < FontWeight::NORMAL);

        let book = FontWeight::from_fontconfig(75);
        assert!(book > demilight);
        assert!(book < FontWeight::NORMAL);

        let extrablack = FontWeight::from_fontconfig(215);
        assert!(extrablack > FontWeight::BLACK);
    }
}
