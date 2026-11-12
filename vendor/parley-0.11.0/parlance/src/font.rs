// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

/// Visual weight class of a font, typically on a scale from 1.0 to 1000.0.
///
/// The default value is [`FontWeight::NORMAL`] or `400.0`.
///
/// In variable fonts, this can be controlled with the `wght` axis. This uses an `f32` so that it
/// can represent the full range of values possible with variable fonts.
///
/// See <https://fonts.google.com/knowledge/glossary/weight>.
///
/// In CSS, this corresponds to the `font-weight` property:
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop>.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct FontWeight(f32);

impl FontWeight {
    /// Weight value of 100.
    pub const THIN: Self = Self(100.0);

    /// Weight value of 200.
    pub const EXTRA_LIGHT: Self = Self(200.0);

    /// Weight value of 300.
    pub const LIGHT: Self = Self(300.0);

    /// Weight value of 350.
    pub const SEMI_LIGHT: Self = Self(350.0);

    /// Weight value of 400. This is the default value.
    pub const NORMAL: Self = Self(400.0);

    /// Weight value of 500.
    pub const MEDIUM: Self = Self(500.0);

    /// Weight value of 600.
    pub const SEMI_BOLD: Self = Self(600.0);

    /// Weight value of 700.
    pub const BOLD: Self = Self(700.0);

    /// Weight value of 800.
    pub const EXTRA_BOLD: Self = Self(800.0);

    /// Weight value of 900.
    pub const BLACK: Self = Self(900.0);

    /// Weight value of 950.
    pub const EXTRA_BLACK: Self = Self(950.0);

    /// Creates a new weight value.
    #[inline(always)]
    pub const fn new(weight: f32) -> Self {
        Self(weight)
    }

    /// Returns the underlying weight value.
    #[inline(always)]
    pub const fn value(self) -> f32 {
        self.0
    }

    /// Parses a CSS `font-weight` value.
    ///
    /// Supported syntax (after trimming ASCII whitespace):
    /// - `normal` → `FontWeight::NORMAL`
    /// - `bold` → `FontWeight::BOLD`
    /// - a number → `FontWeight::new(value)`
    ///
    /// This parser is case-sensitive and does not clamp the numeric range.
    ///
    /// ```
    /// use parlance::FontWeight;
    ///
    /// assert_eq!(FontWeight::parse_css("normal"), Some(FontWeight::NORMAL));
    /// assert_eq!(FontWeight::parse_css("bold"), Some(FontWeight::BOLD));
    /// assert_eq!(FontWeight::parse_css("850"), Some(FontWeight::new(850.0)));
    /// assert_eq!(FontWeight::parse_css("invalid"), None);
    /// ```
    pub fn parse_css(s: &str) -> Option<Self> {
        let s = s.trim();
        Some(match s {
            "normal" => Self::NORMAL,
            "bold" => Self::BOLD,
            _ => Self(s.parse::<f32>().ok()?),
        })
    }
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl fmt::Display for FontWeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display only valid CSS values.
        if *self == Self::NORMAL {
            return f.write_str("normal");
        }
        if *self == Self::BOLD {
            return f.write_str("bold");
        }

        #[allow(
            clippy::cast_possible_truncation,
            reason = "Truncation is only used when the cast is lossless (checked)."
        )]
        let int_value = self.0 as i32;
        if self.0 == int_value as f32 {
            write!(f, "{int_value}")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

/// Visual width of a font — a relative change from the normal aspect ratio, typically in the
/// range `0.5` to `2.0`.
///
/// The default value is [`FontWidth::NORMAL`] or `1.0`.
///
/// In variable fonts, this can be controlled with the `wdth` axis. This uses an `f32` so that it
/// can represent the same range of values as the `wdth` axis.
///
/// In Open Type, the `u16` `usWidthClass` field has 9 values, from 1-9, which doesn't allow for
/// the wide range of values possible with variable fonts.
/// See <https://learn.microsoft.com/en-us/typography/opentype/spec/os2#uswidthclass>.
///
/// See <https://fonts.google.com/knowledge/glossary/width>.
///
/// In CSS, this corresponds to the `font-width` property:
/// <https://www.w3.org/TR/css-fonts-4/#font-width-prop>.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct FontWidth(f32);

impl FontWidth {
    /// Width that is 50% of normal.
    pub const ULTRA_CONDENSED: Self = Self(0.5);

    /// Width that is 62.5% of normal.
    pub const EXTRA_CONDENSED: Self = Self(0.625);

    /// Width that is 75% of normal.
    pub const CONDENSED: Self = Self(0.75);

    /// Width that is 87.5% of normal.
    pub const SEMI_CONDENSED: Self = Self(0.875);

    /// Width that is 100% of normal. This is the default value.
    pub const NORMAL: Self = Self(1.0);

    /// Width that is 112.5% of normal.
    pub const SEMI_EXPANDED: Self = Self(1.125);

    /// Width that is 125% of normal.
    pub const EXPANDED: Self = Self(1.25);

    /// Width that is 150% of normal.
    pub const EXTRA_EXPANDED: Self = Self(1.5);

    /// Width that is 200% of normal.
    pub const ULTRA_EXPANDED: Self = Self(2.0);

    /// Creates a new width value with the given ratio.
    #[inline(always)]
    pub const fn from_ratio(ratio: f32) -> Self {
        Self(ratio)
    }

    /// Creates a width value from a percentage.
    #[inline(always)]
    pub const fn from_percentage(percentage: f32) -> Self {
        Self(percentage / 100.0)
    }

    /// Returns the width value as a ratio, with `1.0` being normal width.
    #[inline(always)]
    pub const fn ratio(self) -> f32 {
        self.0
    }

    /// Returns the width value as a percentage.
    #[inline(always)]
    pub const fn percentage(self) -> f32 {
        self.0 * 100.0
    }

    /// Returns `true` if the width is normal.
    #[inline(always)]
    pub const fn is_normal(self) -> bool {
        self.0 == Self::NORMAL.0
    }

    /// Returns `true` if the width is condensed (less than normal).
    #[inline(always)]
    pub const fn is_condensed(self) -> bool {
        self.0 < Self::NORMAL.0
    }

    /// Returns `true` if the width is expanded (greater than normal).
    #[inline(always)]
    pub const fn is_expanded(self) -> bool {
        self.0 > Self::NORMAL.0
    }

    /// Parses a CSS `font-width` / `font-stretch` value.
    ///
    /// Supported syntax (after trimming ASCII whitespace):
    /// - keywords: `ultra-condensed`, `extra-condensed`, `condensed`, `semi-condensed`, `normal`,
    ///   `semi-expanded`, `expanded`, `extra-expanded`, `ultra-expanded`
    /// - a percentage: e.g. `87.5%` → `FontWidth::from_percentage(87.5)`
    ///
    /// This parser is case-sensitive.
    ///
    /// ```
    /// use parlance::FontWidth;
    ///
    /// assert_eq!(
    ///     FontWidth::parse_css("semi-condensed"),
    ///     Some(FontWidth::SEMI_CONDENSED)
    /// );
    /// assert_eq!(
    ///     FontWidth::parse_css("80%"),
    ///     Some(FontWidth::from_percentage(80.0))
    /// );
    /// assert_eq!(FontWidth::parse_css("wideload"), None);
    /// ```
    pub fn parse_css(s: &str) -> Option<Self> {
        let s = s.trim();
        Some(match s {
            "ultra-condensed" => Self::ULTRA_CONDENSED,
            "extra-condensed" => Self::EXTRA_CONDENSED,
            "condensed" => Self::CONDENSED,
            "semi-condensed" => Self::SEMI_CONDENSED,
            "normal" => Self::NORMAL,
            "semi-expanded" => Self::SEMI_EXPANDED,
            "expanded" => Self::EXPANDED,
            "extra-expanded" => Self::EXTRA_EXPANDED,
            "ultra-expanded" => Self::ULTRA_EXPANDED,
            _ => {
                if s.ends_with('%') {
                    let p = s.get(..s.len() - 1)?.parse::<f32>().ok()?;
                    return Some(Self::from_percentage(p));
                }
                return None;
            }
        })
    }
}

impl Default for FontWidth {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl fmt::Display for FontWidth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.0 * 1000.0;

        #[allow(
            clippy::cast_possible_truncation,
            reason = "The integer keyword mapping is only used when the cast is lossless (checked)."
        )]
        let int_value = value as i32;

        if value == int_value as f32 {
            let keyword = match int_value {
                500 => "ultra-condensed",
                625 => "extra-condensed",
                750 => "condensed",
                875 => "semi-condensed",
                1000 => "normal",
                1125 => "semi-expanded",
                1250 => "expanded",
                1500 => "extra-expanded",
                2000 => "ultra-expanded",
                _ => return write!(f, "{}%", self.percentage()),
            };
            f.write_str(keyword)
        } else {
            write!(f, "{}%", self.percentage())
        }
    }
}

/// Visual style or “slope” of a font.
///
/// In variable fonts, this can be controlled with the `ital` and `slnt` axes for italic and
/// oblique styles, respectively.
///
/// In CSS, this corresponds to the `font-style` property:
/// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum FontStyle {
    /// An upright or "roman" style.
    #[default]
    Normal,
    /// Generally a slanted style, originally based on semi-cursive forms.
    ///
    /// This often has a different structure from the normal style.
    Italic,
    /// `oblique` with an optional angle in degrees.
    ///
    /// If `None`, the engine-specific default oblique angle is used.
    Oblique(Option<f32>),
}

impl FontStyle {
    /// Parses a CSS `font-style` value.
    ///
    /// Supported syntax (after trimming ASCII whitespace):
    /// - `normal` → `FontStyle::Normal`
    /// - `italic` → `FontStyle::Italic`
    /// - `oblique` → `FontStyle::Oblique(Some(14.0))`
    /// - `oblique <angle>` where `<angle>` is one of:
    ///   - `<number>deg`
    ///   - `<number>grad` (gradians, converted to degrees)
    ///   - `<number>rad` (radians, converted to degrees)
    ///   - `<number>turn` (turns, converted to degrees)
    ///
    /// If an `oblique <angle>` form is present but the angle cannot be parsed, this returns
    /// `Some(FontStyle::Oblique(None))`.
    ///
    /// This parser is case-sensitive.
    ///
    /// ```
    /// use parlance::FontStyle;
    ///
    /// assert_eq!(FontStyle::parse_css("normal"), Some(FontStyle::Normal));
    /// assert_eq!(FontStyle::parse_css("italic"), Some(FontStyle::Italic));
    /// assert_eq!(
    ///     FontStyle::parse_css("oblique"),
    ///     Some(FontStyle::Oblique(Some(14.0)))
    /// );
    /// assert_eq!(
    ///     FontStyle::parse_css("oblique 30deg"),
    ///     Some(FontStyle::Oblique(Some(30.0)))
    /// );
    /// assert_eq!(
    ///     FontStyle::parse_css("oblique banana"),
    ///     Some(FontStyle::Oblique(None))
    /// );
    /// assert_eq!(FontStyle::parse_css("banana"), None);
    /// ```
    pub fn parse_css(mut s: &str) -> Option<Self> {
        s = s.trim();
        Some(match s {
            "normal" => Self::Normal,
            "italic" => Self::Italic,
            "oblique" => Self::Oblique(Some(14.0)),
            _ => {
                if s.starts_with("oblique ") {
                    s = s.get(8..)?;
                    if s.ends_with("deg") {
                        s = s.get(..s.len() - 3)?;
                        if let Ok(degrees) = s.trim().parse::<f32>() {
                            return Some(Self::Oblique(Some(degrees)));
                        }
                    } else if s.ends_with("grad") {
                        s = s.get(..s.len() - 4)?;
                        if let Ok(gradians) = s.trim().parse::<f32>() {
                            return Some(Self::Oblique(Some(gradians / 400.0 * 360.0)));
                        }
                    } else if s.ends_with("rad") {
                        s = s.get(..s.len() - 3)?;
                        if let Ok(radians) = s.trim().parse::<f32>() {
                            return Some(Self::Oblique(Some(radians.to_degrees())));
                        }
                    } else if s.ends_with("turn") {
                        s = s.get(..s.len() - 4)?;
                        if let Ok(turns) = s.trim().parse::<f32>() {
                            return Some(Self::Oblique(Some(turns * 360.0)));
                        }
                    }
                    return Some(Self::Oblique(None));
                }
                return None;
            }
        })
    }
}

impl fmt::Display for FontStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => f.write_str("normal"),
            Self::Italic => f.write_str("italic"),
            Self::Oblique(None) => f.write_str("oblique"),
            Self::Oblique(Some(degrees)) if *degrees == 14.0 => f.write_str("oblique"),
            Self::Oblique(Some(degrees)) => write!(f, "oblique {degrees}deg"),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::FontWidth;
    use crate::{FontStyle, FontWeight};

    #[test]
    fn fontwidth_parse_includes_expanded() {
        assert_eq!(FontWidth::parse_css("expanded"), Some(FontWidth::EXPANDED));
    }

    #[test]
    fn fontwidth_parse_keywords() {
        assert_eq!(FontWidth::parse_css("normal"), Some(FontWidth::NORMAL));
        assert_eq!(
            FontWidth::parse_css("ultra-condensed"),
            Some(FontWidth::ULTRA_CONDENSED)
        );
        assert_eq!(
            FontWidth::parse_css("extra-expanded"),
            Some(FontWidth::EXTRA_EXPANDED)
        );
        assert_eq!(
            FontWidth::parse_css("  condensed "),
            Some(FontWidth::CONDENSED)
        );
    }

    #[test]
    fn fontwidth_parse_percentage() {
        assert_eq!(
            FontWidth::parse_css("87.5%"),
            Some(FontWidth::from_percentage(87.5))
        );
        assert_eq!(
            FontWidth::parse_css(" 80% "),
            Some(FontWidth::from_percentage(80.0))
        );
        assert_eq!(FontWidth::parse_css("80"), None);
        assert_eq!(FontWidth::parse_css("%"), None);
        assert_eq!(FontWidth::parse_css("80%%"), None);
    }

    #[test]
    fn fontweight_parse_keywords_and_numbers() {
        assert_eq!(FontWeight::parse_css("normal"), Some(FontWeight::NORMAL));
        assert_eq!(FontWeight::parse_css("bold"), Some(FontWeight::BOLD));
        assert_eq!(FontWeight::parse_css(" 850 "), Some(FontWeight::new(850.0)));
        assert_eq!(FontWeight::parse_css("invalid"), None);
    }

    #[test]
    fn fontstyle_parse_keywords() {
        assert_eq!(FontStyle::parse_css("normal"), Some(FontStyle::Normal));
        assert_eq!(FontStyle::parse_css("italic"), Some(FontStyle::Italic));
        assert_eq!(
            FontStyle::parse_css("oblique"),
            Some(FontStyle::Oblique(Some(14.0)))
        );
        assert_eq!(
            FontStyle::parse_css(" oblique "),
            Some(FontStyle::Oblique(Some(14.0)))
        );
    }

    #[test]
    fn fontstyle_parse_oblique_angles() {
        assert_eq!(
            FontStyle::parse_css("oblique 30deg"),
            Some(FontStyle::Oblique(Some(30.0)))
        );
        assert_eq!(
            FontStyle::parse_css("oblique 0.5turn"),
            Some(FontStyle::Oblique(Some(180.0)))
        );
        assert_eq!(
            FontStyle::parse_css("oblique 200grad"),
            Some(FontStyle::Oblique(Some(180.0)))
        );
        assert_eq!(
            FontStyle::parse_css("oblique 3.1415927rad"),
            Some(FontStyle::Oblique(Some(180.0)))
        );

        // Present but unparsable angle yields `Oblique(None)`.
        assert_eq!(
            FontStyle::parse_css("oblique banana"),
            Some(FontStyle::Oblique(None))
        );
        assert_eq!(
            FontStyle::parse_css("oblique 12"),
            Some(FontStyle::Oblique(None))
        );
        assert_eq!(
            FontStyle::parse_css("oblique 12foo"),
            Some(FontStyle::Oblique(None))
        );
    }

    #[test]
    fn fontstyle_parse_invalid() {
        assert_eq!(FontStyle::parse_css("banana"), None);
        assert_eq!(FontStyle::parse_css("oblique12deg"), None);
        assert_eq!(FontStyle::parse_css("Oblique"), None);
    }
}
