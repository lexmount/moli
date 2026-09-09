/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! [Length values][length].
//!
//! [length]: https://drafts.csswg.org/css-values/#lengths

use super::{AllowQuirks, Number, ToComputedValue};
use crate::computed_value_flags::ComputedValueFlags;
use crate::derives::*;
use crate::font_metrics::{FontMetrics, FontMetricsOrientation};
#[cfg(feature = "gecko")]
use crate::gecko_bindings::structs::GeckoFontMetrics;
use crate::parser::{Parse, ParserContext};
use crate::typed_om::{NumericType, NumericValue, ToTyped, TypedValue, UnitValue};
use crate::values::computed::{self, CSSPixelLength, Context, FontSize};
use crate::values::generics::length as generics;
use crate::values::generics::length::{
    GenericAnchorSizeFunction, GenericLengthOrNumber, GenericLengthPercentageOrNormal,
    GenericMargin, GenericMaxSize, GenericSize,
};
use crate::values::generics::NonNegative;
use crate::values::specified::calc::{
    AllowAnchorPositioningFunctions, CalcLengthPercentage, CalcNode,
};
use crate::values::specified::font::QueryFontMetricsFlags;
use crate::values::specified::percentage::NoCalcPercentage;
use crate::values::specified::NonNegativeNumber;
use crate::values::tagged_numeric::{Extracted, NumericUnion, Unpacked};
use crate::values::CSSFloat;
use crate::{Zero, ZeroNoPercent};
use app_units::AU_PER_PX;
use cssparser::{match_ignore_ascii_case, Parser, Token};
use std::cmp;
use std::fmt::{self, Write};
use style_traits::values::specified::AllowedNumericType;
use style_traits::{
    CssString, CssWriter, ParseError, ParsingMode, SpecifiedValueInfo, StyleParseErrorKind, ToCss,
};
use thin_vec::ThinVec;

pub use super::image::Image;
pub use super::image::{EndingShape as GradientEndingShape, Gradient};

/// Number of pixels per inch
pub const PX_PER_IN: CSSFloat = 96.;
/// Number of pixels per centimeter
pub const PX_PER_CM: CSSFloat = PX_PER_IN / 2.54;
/// Number of pixels per millimeter
pub const PX_PER_MM: CSSFloat = PX_PER_IN / 25.4;
/// Number of pixels per quarter
pub const PX_PER_Q: CSSFloat = PX_PER_MM / 4.;
/// Number of pixels per point
pub const PX_PER_PT: CSSFloat = PX_PER_IN / 72.;
/// Number of pixels per pica
pub const PX_PER_PC: CSSFloat = PX_PER_PT * 12.;

/// The unit of a `<length>` value. Note that if any new font-relative value is
/// added here, `custom_properties::NonCustomReferences::from_unit`
/// must also be updated. Consult the comment in that function as to why.
///
/// The variants are grouped (absolute, font-relative, viewport, container,
/// servo-internal) so that `is_*` predicates can be implemented with simple
/// range checks.
#[derive(Clone, Copy, Debug, Eq, MallocSizeOf, PartialEq, PartialOrd, ToShmem)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum LengthUnit {
    // Absolute lengths.
    Px,
    In,
    Cm,
    Mm,
    Q,
    Pt,
    Pc,
    // Font-relative lengths.
    Em,
    Ex,
    Rex,
    Ch,
    Rch,
    Cap,
    Rcap,
    Ic,
    Ric,
    Rem,
    Lh,
    Rlh,
    // Viewport-percentage lengths.
    Vw,
    Svw,
    Lvw,
    Dvw,
    Vh,
    Svh,
    Lvh,
    Dvh,
    Vmin,
    Svmin,
    Lvmin,
    Dvmin,
    Vmax,
    Svmax,
    Lvmax,
    Dvmax,
    Vb,
    Svb,
    Lvb,
    Dvb,
    Vi,
    Svi,
    Lvi,
    Dvi,
    // Container-relative lengths.
    Cqw,
    Cqh,
    Cqi,
    Cqb,
    Cqmin,
    Cqmax,
    /// HTML5 "character width", as defined in HTML5 § 14.5.4. Internal-only.
    ServoCharacterWidth,
}

impl LengthUnit {
    /// Returns the length unit for the given string.
    #[inline]
    pub fn from_str(unit: &str) -> Result<Self, ()> {
        Self::from_str_with_flags(ParsingMode::DEFAULT, /* in_page_rule = */ false, unit)
    }

    /// Returns the length unit for the given flags and string.
    #[inline]
    pub fn from_str_with_flags(
        parsing_mode: ParsingMode,
        in_page_rule: bool,
        unit: &str,
    ) -> Result<Self, ()> {
        let allows_computational_dependence = parsing_mode.allows_computational_dependence();

        Ok(match_ignore_ascii_case! { unit,
            "px" => Self::Px,
            "in" => Self::In,
            "cm" => Self::Cm,
            "mm" => Self::Mm,
            "q" => Self::Q,
            "pt" => Self::Pt,
            "pc" => Self::Pc,
            // font-relative
            "em" if allows_computational_dependence => Self::Em,
            "ex" if allows_computational_dependence => Self::Ex,
            "rex" if allows_computational_dependence => Self::Rex,
            "ch" if allows_computational_dependence => Self::Ch,
            "rch" if allows_computational_dependence => Self::Rch,
            "cap" if allows_computational_dependence => Self::Cap,
            "rcap" if allows_computational_dependence => Self::Rcap,
            "ic" if allows_computational_dependence => Self::Ic,
            "ric" if allows_computational_dependence => Self::Ric,
            "rem" if allows_computational_dependence => Self::Rem,
            "lh" if allows_computational_dependence => Self::Lh,
            "rlh" if allows_computational_dependence => Self::Rlh,
            // viewport percentages
            "vw" if !in_page_rule => Self::Vw,
            "svw" if !in_page_rule => Self::Svw,
            "lvw" if !in_page_rule => Self::Lvw,
            "dvw" if !in_page_rule => Self::Dvw,
            "vh" if !in_page_rule => Self::Vh,
            "svh" if !in_page_rule => Self::Svh,
            "lvh" if !in_page_rule => Self::Lvh,
            "dvh" if !in_page_rule => Self::Dvh,
            "vmin" if !in_page_rule => Self::Vmin,
            "svmin" if !in_page_rule => Self::Svmin,
            "lvmin" if !in_page_rule => Self::Lvmin,
            "dvmin" if !in_page_rule => Self::Dvmin,
            "vmax" if !in_page_rule => Self::Vmax,
            "svmax" if !in_page_rule => Self::Svmax,
            "lvmax" if !in_page_rule => Self::Lvmax,
            "dvmax" if !in_page_rule => Self::Dvmax,
            "vb" if !in_page_rule => Self::Vb,
            "svb" if !in_page_rule => Self::Svb,
            "lvb" if !in_page_rule => Self::Lvb,
            "dvb" if !in_page_rule => Self::Dvb,
            "vi" if !in_page_rule => Self::Vi,
            "svi" if !in_page_rule => Self::Svi,
            "lvi" if !in_page_rule => Self::Lvi,
            "dvi" if !in_page_rule => Self::Dvi,
            // Container query lengths. Inherit the limitation from viewport units since
            // we may fall back to them.
            "cqw" if !in_page_rule => Self::Cqw,
            "cqh" if !in_page_rule => Self::Cqh,
            "cqi" if !in_page_rule => Self::Cqi,
            "cqb" if !in_page_rule => Self::Cqb,
            "cqmin" if !in_page_rule => Self::Cqmin,
            "cqmax" if !in_page_rule => Self::Cqmax,
            _ => return Err(()),
        })
    }

    /// Returns this unit as a string.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Px => "px",
            Self::In => "in",
            Self::Cm => "cm",
            Self::Mm => "mm",
            Self::Q => "q",
            Self::Pt => "pt",
            Self::Pc => "pc",
            Self::Em => NoCalcLength::EM,
            Self::Ex => NoCalcLength::EX,
            Self::Rex => NoCalcLength::REX,
            Self::Ch => NoCalcLength::CH,
            Self::Rch => NoCalcLength::RCH,
            Self::Cap => NoCalcLength::CAP,
            Self::Rcap => NoCalcLength::RCAP,
            Self::Ic => NoCalcLength::IC,
            Self::Ric => NoCalcLength::RIC,
            Self::Rem => NoCalcLength::REM,
            Self::Lh => NoCalcLength::LH,
            Self::Rlh => NoCalcLength::RLH,
            Self::Vw => "vw",
            Self::Svw => "svw",
            Self::Lvw => "lvw",
            Self::Dvw => "dvw",
            Self::Vh => "vh",
            Self::Svh => "svh",
            Self::Lvh => "lvh",
            Self::Dvh => "dvh",
            Self::Vmin => "vmin",
            Self::Svmin => "svmin",
            Self::Lvmin => "lvmin",
            Self::Dvmin => "dvmin",
            Self::Vmax => "vmax",
            Self::Svmax => "svmax",
            Self::Lvmax => "lvmax",
            Self::Dvmax => "dvmax",
            Self::Vb => "vb",
            Self::Svb => "svb",
            Self::Lvb => "lvb",
            Self::Dvb => "dvb",
            Self::Vi => "vi",
            Self::Svi => "svi",
            Self::Lvi => "lvi",
            Self::Dvi => "dvi",
            Self::Cqw => "cqw",
            Self::Cqh => "cqh",
            Self::Cqi => "cqi",
            Self::Cqb => "cqb",
            Self::Cqmin => "cqmin",
            Self::Cqmax => "cqmax",
            Self::ServoCharacterWidth => "",
        }
    }

    /// Whether this is an absolute length unit (px, in, cm, mm, q, pt, pc).
    #[inline]
    pub fn is_absolute(self) -> bool {
        matches!(
            self,
            Self::Px | Self::In | Self::Cm | Self::Mm | Self::Q | Self::Pt | Self::Pc
        )
    }

    /// Whether this is a font-relative unit.
    #[inline]
    pub fn is_font_relative(self) -> bool {
        matches!(
            self,
            Self::Em
                | Self::Ex
                | Self::Rex
                | Self::Ch
                | Self::Rch
                | Self::Cap
                | Self::Rcap
                | Self::Ic
                | Self::Ric
                | Self::Rem
                | Self::Lh
                | Self::Rlh
        )
    }

    /// Whether this is a viewport-percentage unit.
    #[inline]
    pub fn is_viewport_percentage(self) -> bool {
        matches!(
            self,
            Self::Vw
                | Self::Svw
                | Self::Lvw
                | Self::Dvw
                | Self::Vh
                | Self::Svh
                | Self::Lvh
                | Self::Dvh
                | Self::Vmin
                | Self::Svmin
                | Self::Lvmin
                | Self::Dvmin
                | Self::Vmax
                | Self::Svmax
                | Self::Lvmax
                | Self::Dvmax
                | Self::Vb
                | Self::Svb
                | Self::Lvb
                | Self::Dvb
                | Self::Vi
                | Self::Svi
                | Self::Lvi
                | Self::Dvi
        )
    }

    /// Whether this is a container-relative unit.
    #[inline]
    pub fn is_container_relative(self) -> bool {
        matches!(
            self,
            Self::Cqw | Self::Cqh | Self::Cqi | Self::Cqb | Self::Cqmin | Self::Cqmax
        )
    }

    /// Returns the sort key for this unit. Must not be called for the internal
    /// `ServoCharacterWidth` unit.
    fn sort_key(self) -> crate::values::generics::calc::SortKey {
        use crate::values::generics::calc::SortKey;
        match self {
            Self::Px | Self::In | Self::Cm | Self::Mm | Self::Q | Self::Pt | Self::Pc => {
                SortKey::Px
            },
            Self::Em => SortKey::Em,
            Self::Ex => SortKey::Ex,
            Self::Rex => SortKey::Rex,
            Self::Ch => SortKey::Ch,
            Self::Rch => SortKey::Rch,
            Self::Cap => SortKey::Cap,
            Self::Rcap => SortKey::Rcap,
            Self::Ic => SortKey::Ic,
            Self::Ric => SortKey::Ric,
            Self::Rem => SortKey::Rem,
            Self::Lh => SortKey::Lh,
            Self::Rlh => SortKey::Rlh,
            Self::Vw => SortKey::Vw,
            Self::Svw => SortKey::Svw,
            Self::Lvw => SortKey::Lvw,
            Self::Dvw => SortKey::Dvw,
            Self::Vh => SortKey::Vh,
            Self::Svh => SortKey::Svh,
            Self::Lvh => SortKey::Lvh,
            Self::Dvh => SortKey::Dvh,
            Self::Vmin => SortKey::Vmin,
            Self::Svmin => SortKey::Svmin,
            Self::Lvmin => SortKey::Lvmin,
            Self::Dvmin => SortKey::Dvmin,
            Self::Vmax => SortKey::Vmax,
            Self::Svmax => SortKey::Svmax,
            Self::Lvmax => SortKey::Lvmax,
            Self::Dvmax => SortKey::Dvmax,
            Self::Vb => SortKey::Vb,
            Self::Svb => SortKey::Svb,
            Self::Lvb => SortKey::Lvb,
            Self::Dvb => SortKey::Dvb,
            Self::Vi => SortKey::Vi,
            Self::Svi => SortKey::Svi,
            Self::Lvi => SortKey::Lvi,
            Self::Dvi => SortKey::Dvi,
            Self::Cqw => SortKey::Cqw,
            Self::Cqh => SortKey::Cqh,
            Self::Cqi => SortKey::Cqi,
            Self::Cqb => SortKey::Cqb,
            Self::Cqmin => SortKey::Cqmin,
            Self::Cqmax => SortKey::Cqmax,
            Self::ServoCharacterWidth => unreachable!(),
        }
    }
}

/// A source to resolve font-relative units against
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontBaseSize {
    /// Use the font-size of the current element.
    CurrentStyle,
    /// Use the inherited font-size.
    InheritedStyle,
}

/// A source to resolve font-relative line-height units against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeightBase {
    /// Use the line-height of the current element.
    CurrentStyle,
    /// Use the inherited line-height.
    InheritedStyle,
}

impl FontBaseSize {
    /// Calculate the actual size for a given context
    pub fn resolve(&self, context: &Context) -> computed::FontSize {
        let style = context.style();
        match *self {
            Self::CurrentStyle => style.get_font().clone_font_size(),
            Self::InheritedStyle => {
                // If we're using the size from our inherited style, we still need to apply our
                // own zoom.
                let zoom = style.effective_zoom_for_inheritance;
                style.get_parent_font().clone_font_size().zoom(zoom)
            },
        }
    }
}

/// https://drafts.csswg.org/css-values/#viewport-variants
pub enum ViewportVariant {
    /// https://drafts.csswg.org/css-values/#ua-default-viewport-size
    UADefault,
    /// https://drafts.csswg.org/css-values/#small-viewport-percentage-units
    Small,
    /// https://drafts.csswg.org/css-values/#large-viewport-percentage-units
    Large,
    /// https://drafts.csswg.org/css-values/#dynamic-viewport-percentage-units
    Dynamic,
}

/// https://drafts.csswg.org/css-values/#viewport-relative-units
#[derive(PartialEq)]
enum ViewportUnit {
    /// *vw units.
    Vw,
    /// *vh units.
    Vh,
    /// *vmin units.
    Vmin,
    /// *vmax units.
    Vmax,
    /// *vb units.
    Vb,
    /// *vi units.
    Vi,
}

/// A `<length>` without taking `calc` expressions into account
///
/// <https://drafts.csswg.org/css-values/#lengths>
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, ToShmem)]
#[repr(C)]
pub struct NoCalcLength {
    unit: LengthUnit,
    value: CSSFloat,
}

impl NoCalcLength {
    /// Unit identifier for `em`.
    pub const EM: &'static str = "em";
    /// Unit identifier for `ex`.
    pub const EX: &'static str = "ex";
    /// Unit identifier for `rex`.
    pub const REX: &'static str = "rex";
    /// Unit identifier for `ch`.
    pub const CH: &'static str = "ch";
    /// Unit identifier for `rch`.
    pub const RCH: &'static str = "rch";
    /// Unit identifier for `cap`.
    pub const CAP: &'static str = "cap";
    /// Unit identifier for `rcap`.
    pub const RCAP: &'static str = "rcap";
    /// Unit identifier for `ic`.
    pub const IC: &'static str = "ic";
    /// Unit identifier for `ric`.
    pub const RIC: &'static str = "ric";
    /// Unit identifier for `rem`.
    pub const REM: &'static str = "rem";
    /// Unit identifier for `lh`.
    pub const LH: &'static str = "lh";
    /// Unit identifier for `rlh`.
    pub const RLH: &'static str = "rlh";

    /// Creates a length with the given unit and value.
    #[inline]
    pub fn new(unit: LengthUnit, value: CSSFloat) -> Self {
        Self { unit, value }
    }

    /// Returns the unit of this length.
    #[inline]
    pub fn length_unit(&self) -> LengthUnit {
        self.unit
    }

    /// Return the unitless, raw value.
    #[inline]
    pub fn unitless_value(&self) -> CSSFloat {
        self.value
    }

    /// Return the unit, as a string.
    #[inline]
    pub fn unit(&self) -> &'static str {
        self.unit.as_str()
    }

    /// Return the canonical unit for this value, if one exists.
    pub fn canonical_unit(&self) -> Option<&'static str> {
        if self.unit.is_absolute() {
            Some("px")
        } else {
            None
        }
    }

    /// Convert this value to the specified unit, if possible.
    pub fn to(&self, unit: &str) -> Result<Self, ()> {
        let px = self.to_px_if_absolute().ok_or(())?;
        let (target, divisor) = match_ignore_ascii_case! { unit,
            "px" => (LengthUnit::Px, 1.0),
            "in" => (LengthUnit::In, PX_PER_IN),
            "cm" => (LengthUnit::Cm, PX_PER_CM),
            "mm" => (LengthUnit::Mm, PX_PER_MM),
            "q" => (LengthUnit::Q, PX_PER_Q),
            "pt" => (LengthUnit::Pt, PX_PER_PT),
            "pc" => (LengthUnit::Pc, PX_PER_PC),
             _ => return Err(()),
        };
        Ok(Self::new(target, px / divisor))
    }

    /// Returns whether the value of this length without unit is less than zero.
    pub fn is_negative(&self) -> bool {
        self.value.is_sign_negative()
    }

    /// Returns whether the value of this length without unit is equal to zero.
    pub fn is_zero(&self) -> bool {
        self.value == 0.0
    }

    /// Returns whether the value of this length without unit is infinite.
    pub fn is_infinite(&self) -> bool {
        self.value.is_infinite()
    }

    /// Returns whether the value of this length without unit is NaN.
    pub fn is_nan(&self) -> bool {
        self.value.is_nan()
    }

    /// Whether text-only zoom should be applied to this length.
    ///
    /// Generally, font-dependent/relative units don't get text-only-zoomed,
    /// because the font they're relative to should be zoomed already.
    pub fn should_zoom_text(&self) -> bool {
        !self.unit.is_font_relative() && self.unit != LengthUnit::ServoCharacterWidth
    }

    /// Returns the SortKey for this length. Must not be called on the internal
    /// `ServoCharacterWidth` unit.
    pub(crate) fn sort_key(&self) -> crate::values::generics::calc::SortKey {
        self.unit.sort_key()
    }

    /// Parse a given absolute or relative dimension.
    pub fn parse_dimension_with_flags(
        parsing_mode: ParsingMode,
        in_page_rule: bool,
        value: CSSFloat,
        unit: &str,
    ) -> Result<Self, ()> {
        let length_unit = LengthUnit::from_str_with_flags(parsing_mode, in_page_rule, unit)?;
        Ok(Self::new(length_unit, value))
    }

    /// Parse a given absolute or relative dimension.
    pub fn parse_dimension_with_context(
        context: &ParserContext,
        value: CSSFloat,
        unit: &str,
    ) -> Result<Self, ()> {
        Self::parse_dimension_with_flags(context.parsing_mode, context.in_page_rule(), value, unit)
    }

    pub(crate) fn try_op<O>(&self, other: &Self, op: O) -> Result<Self, ()>
    where
        O: Fn(f32, f32) -> f32,
    {
        // For absolute lengths, normalize both to px and produce a px result.
        if let (Some(a), Some(b)) = (self.to_px_if_absolute(), other.to_px_if_absolute()) {
            return Ok(Self::new(LengthUnit::Px, op(a, b)));
        }
        if self.unit != other.unit {
            return Err(());
        }
        Ok(Self::new(self.unit, op(self.value, other.value)))
    }

    pub(crate) fn map(&self, mut op: impl FnMut(f32) -> f32) -> Self {
        // For absolute lengths, normalize to px.
        if let Some(px) = self.to_px_if_absolute() {
            return Self::new(LengthUnit::Px, op(px));
        }
        Self::new(self.unit, op(self.value))
    }

    /// Get a px value without context (so only absolute units can be handled).
    #[inline]
    pub fn to_computed_pixel_length_without_context(&self) -> Result<CSSFloat, ()> {
        self.to_px_if_absolute().ok_or(())
    }

    /// Get a px value without a full style context; this can handle either
    /// absolute or (if a font metrics getter is provided) font-relative units.
    #[cfg(feature = "gecko")]
    #[inline]
    pub fn to_computed_pixel_length_with_font_metrics(
        &self,
        get_font_metrics: Option<impl Fn() -> GeckoFontMetrics>,
    ) -> Result<CSSFloat, ()> {
        if let Some(px) = self.to_px_if_absolute() {
            return Ok(CSSPixelLength::new(px).finite().px());
        }
        if !self.unit.is_font_relative() {
            return Err(());
        }
        let getter = match get_font_metrics {
            Some(g) => g,
            None => return Err(()),
        };
        let metrics = getter();
        Ok(match self.unit {
            LengthUnit::Em => self.value * metrics.mComputedEmSize.px(),
            LengthUnit::Ex => self.value * metrics.mXSize.px(),
            LengthUnit::Ch => self.value * metrics.mChSize.px(),
            LengthUnit::Cap => self.value * metrics.mCapHeight.px(),
            LengthUnit::Ic => self.value * metrics.mIcWidth.px(),
            // `lh`, `rlh` are unsupported as we have no line-height context
            // `rem`, `rex`, `rch`, `rcap`, and `ric` are unsupported as we have no root font context.
            _ => return Err(()),
        })
    }

    /// Get an absolute length from a px value.
    #[inline]
    pub fn from_px(px_value: CSSFloat) -> NoCalcLength {
        Self::new(LengthUnit::Px, px_value)
    }

    /// Returns the value as a px-canonical length (absolute lengths only).
    #[inline]
    pub fn to_px_if_absolute(&self) -> Option<CSSFloat> {
        let factor = match self.unit {
            LengthUnit::Px => 1.0,
            LengthUnit::In => PX_PER_IN,
            LengthUnit::Cm => PX_PER_CM,
            LengthUnit::Mm => PX_PER_MM,
            LengthUnit::Q => PX_PER_Q,
            LengthUnit::Pt => PX_PER_PT,
            LengthUnit::Pc => PX_PER_PC,
            _ => return None,
        };
        Some(self.value * factor)
    }

    /// Construct a font-relative em value.
    #[inline]
    pub fn from_em(value: CSSFloat) -> Self {
        Self::new(LengthUnit::Em, value)
    }

    /// Construct an internal ServoCharacterWidth length from an i32 column count.
    #[inline]
    pub fn from_servo_character_width(value: i32) -> Self {
        Self::new(LengthUnit::ServoCharacterWidth, value as CSSFloat)
    }

    /// Compute a font-relative length against the given base sizes. Must only
    /// be called on a font-relative unit.
    fn font_relative_to_computed_value(
        &self,
        context: &Context,
        base_size: FontBaseSize,
        line_height_base: LineHeightBase,
    ) -> computed::Length {
        let (reference_size, length) =
            self.reference_font_size_and_length(context, base_size, line_height_base);
        (reference_size * length).finite()
    }

    fn reference_font_size_and_length(
        &self,
        context: &Context,
        base_size: FontBaseSize,
        line_height_base: LineHeightBase,
    ) -> (computed::Length, CSSFloat) {
        fn query_font_metrics(
            context: &Context,
            base_size: FontBaseSize,
            orientation: FontMetricsOrientation,
            flags: QueryFontMetricsFlags,
        ) -> FontMetrics {
            context.query_font_metrics(base_size, orientation, flags)
        }

        fn ex_size(
            context: &Context,
            base_size: FontBaseSize,
            reference_font_size: &FontSize,
        ) -> computed::Length {
            let metrics = query_font_metrics(
                context,
                base_size,
                FontMetricsOrientation::Horizontal,
                QueryFontMetricsFlags::empty(),
            );
            metrics.x_height_or_default(reference_font_size.used_size())
        }

        fn ch_size(
            context: &Context,
            base_size: FontBaseSize,
            reference_font_size: &FontSize,
        ) -> computed::Length {
            let metrics = query_font_metrics(
                context,
                base_size,
                FontMetricsOrientation::MatchContextPreferHorizontal,
                QueryFontMetricsFlags::NEEDS_CH,
            );
            metrics.zero_advance_measure_or_default(
                reference_font_size.used_size(),
                context.style().writing_mode.is_upright(),
            )
        }

        fn cap_size(context: &Context, base_size: FontBaseSize) -> computed::Length {
            let metrics = query_font_metrics(
                context,
                base_size,
                FontMetricsOrientation::Horizontal,
                QueryFontMetricsFlags::empty(),
            );
            metrics.cap_height_or_default()
        }

        fn ic_size(
            context: &Context,
            base_size: FontBaseSize,
            reference_font_size: &FontSize,
        ) -> computed::Length {
            let metrics = query_font_metrics(
                context,
                base_size,
                FontMetricsOrientation::MatchContextPreferVertical,
                QueryFontMetricsFlags::NEEDS_IC,
            );
            metrics.ic_width_or_default(reference_font_size.used_size())
        }

        context
            .builder
            .add_flags(ComputedValueFlags::USES_FONT_RELATIVE_UNITS);

        let reference_font_size = base_size.resolve(context);
        let length = self.value;
        match self.unit {
            LengthUnit::Em => {
                if context.for_non_inherited_property && base_size == FontBaseSize::CurrentStyle {
                    context
                        .rule_cache_conditions
                        .borrow_mut()
                        .set_font_size_dependency(reference_font_size.computed_size);
                }

                (reference_font_size.computed_size(), length)
            },
            LengthUnit::Lh => {
                let reference_size = if context.in_media_query {
                    context
                        .device()
                        .calc_line_height(
                            &context.default_style().get_font(),
                            context.style().writing_mode,
                            None,
                        )
                        .0
                } else {
                    let line_height = context.builder.calc_line_height(
                        context.device(),
                        line_height_base,
                        context.style().writing_mode,
                    );
                    if context.for_non_inherited_property
                        && line_height_base == LineHeightBase::CurrentStyle
                    {
                        context
                            .rule_cache_conditions
                            .borrow_mut()
                            .set_line_height_dependency(line_height)
                    }
                    line_height.0
                };
                (reference_size, length)
            },
            LengthUnit::Ex => (ex_size(context, base_size, &reference_font_size), length),
            LengthUnit::Ch => (ch_size(context, base_size, &reference_font_size), length),
            LengthUnit::Cap => (cap_size(context, base_size), length),
            LengthUnit::Ic => (ic_size(context, base_size, &reference_font_size), length),
            LengthUnit::Rex => {
                let reference_size = if context.builder.is_root_element || context.in_media_query {
                    ex_size(context, base_size, &reference_font_size)
                } else {
                    context
                        .device()
                        .root_font_metrics_ex()
                        .zoom(context.builder.effective_zoom)
                };
                (reference_size, length)
            },
            LengthUnit::Rch => {
                let reference_size = if context.builder.is_root_element || context.in_media_query {
                    ch_size(context, base_size, &reference_font_size)
                } else {
                    context
                        .device()
                        .root_font_metrics_ch()
                        .zoom(context.builder.effective_zoom)
                };
                (reference_size, length)
            },
            LengthUnit::Rcap => {
                let reference_size = if context.builder.is_root_element || context.in_media_query {
                    cap_size(context, base_size)
                } else {
                    context
                        .device()
                        .root_font_metrics_cap()
                        .zoom(context.builder.effective_zoom)
                };
                (reference_size, length)
            },
            LengthUnit::Ric => {
                let reference_size = if context.builder.is_root_element || context.in_media_query {
                    ic_size(context, base_size, &reference_font_size)
                } else {
                    context
                        .device()
                        .root_font_metrics_ic()
                        .zoom(context.builder.effective_zoom)
                };
                (reference_size, length)
            },
            LengthUnit::Rem => {
                let reference_size = if context.builder.is_root_element || context.in_media_query {
                    reference_font_size.computed_size()
                } else {
                    context
                        .device()
                        .root_font_size()
                        .zoom(context.builder.effective_zoom)
                };
                (reference_size, length)
            },
            LengthUnit::Rlh => {
                let reference_size = if context.builder.is_root_element {
                    context
                        .builder
                        .calc_line_height(
                            context.device(),
                            line_height_base,
                            context.style().writing_mode,
                        )
                        .0
                } else if context.in_media_query {
                    context
                        .device()
                        .calc_line_height(
                            &context.default_style().get_font(),
                            context.style().writing_mode,
                            None,
                        )
                        .0
                } else {
                    context.device().root_line_height()
                };
                let reference_size = reference_size.zoom(context.builder.effective_zoom);
                (reference_size, length)
            },
            _ => unreachable!("reference_font_size_and_length: not a font-relative unit"),
        }
    }

    /// Compute the viewport-percentage length. Must only be called on a
    /// viewport-relative unit.
    fn viewport_percentage_to_computed_value(&self, context: &Context) -> CSSPixelLength {
        let (variant, unit) = match self.unit {
            LengthUnit::Vw => (ViewportVariant::UADefault, ViewportUnit::Vw),
            LengthUnit::Svw => (ViewportVariant::Small, ViewportUnit::Vw),
            LengthUnit::Lvw => (ViewportVariant::Large, ViewportUnit::Vw),
            LengthUnit::Dvw => (ViewportVariant::Dynamic, ViewportUnit::Vw),
            LengthUnit::Vh => (ViewportVariant::UADefault, ViewportUnit::Vh),
            LengthUnit::Svh => (ViewportVariant::Small, ViewportUnit::Vh),
            LengthUnit::Lvh => (ViewportVariant::Large, ViewportUnit::Vh),
            LengthUnit::Dvh => (ViewportVariant::Dynamic, ViewportUnit::Vh),
            LengthUnit::Vmin => (ViewportVariant::UADefault, ViewportUnit::Vmin),
            LengthUnit::Svmin => (ViewportVariant::Small, ViewportUnit::Vmin),
            LengthUnit::Lvmin => (ViewportVariant::Large, ViewportUnit::Vmin),
            LengthUnit::Dvmin => (ViewportVariant::Dynamic, ViewportUnit::Vmin),
            LengthUnit::Vmax => (ViewportVariant::UADefault, ViewportUnit::Vmax),
            LengthUnit::Svmax => (ViewportVariant::Small, ViewportUnit::Vmax),
            LengthUnit::Lvmax => (ViewportVariant::Large, ViewportUnit::Vmax),
            LengthUnit::Dvmax => (ViewportVariant::Dynamic, ViewportUnit::Vmax),
            LengthUnit::Vb => (ViewportVariant::UADefault, ViewportUnit::Vb),
            LengthUnit::Svb => (ViewportVariant::Small, ViewportUnit::Vb),
            LengthUnit::Lvb => (ViewportVariant::Large, ViewportUnit::Vb),
            LengthUnit::Dvb => (ViewportVariant::Dynamic, ViewportUnit::Vb),
            LengthUnit::Vi => (ViewportVariant::UADefault, ViewportUnit::Vi),
            LengthUnit::Svi => (ViewportVariant::Small, ViewportUnit::Vi),
            LengthUnit::Lvi => (ViewportVariant::Large, ViewportUnit::Vi),
            LengthUnit::Dvi => (ViewportVariant::Dynamic, ViewportUnit::Vi),
            _ => {
                unreachable!("viewport_percentage_to_computed_value: not a viewport-relative unit")
            },
        };
        let factor = self.value;
        let size = context.viewport_size_for_viewport_unit_resolution(variant);
        let length: app_units::Au = match unit {
            ViewportUnit::Vw => size.width,
            ViewportUnit::Vh => size.height,
            ViewportUnit::Vmin => cmp::min(size.width, size.height),
            ViewportUnit::Vmax => cmp::max(size.width, size.height),
            ViewportUnit::Vi | ViewportUnit::Vb => {
                context
                    .rule_cache_conditions
                    .borrow_mut()
                    .set_writing_mode_dependency(context.builder.writing_mode);
                if (unit == ViewportUnit::Vb) == context.style().writing_mode.is_vertical() {
                    size.width
                } else {
                    size.height
                }
            },
        };
        let length = context.builder.effective_zoom.zoom(length.0 as f32);

        let trunc_scaled =
            ((length as f64 * factor as f64 / 100.).trunc() / AU_PER_PX as f64) as f32;
        CSSPixelLength::new(crate::values::normalize(trunc_scaled))
    }

    /// Compute the container-relative length. Must only be called on a
    /// container-relative unit.
    fn container_relative_to_computed_value(&self, context: &Context) -> CSSPixelLength {
        if context.for_non_inherited_property {
            context.rule_cache_conditions.borrow_mut().set_uncacheable();
        }
        context
            .builder
            .add_flags(ComputedValueFlags::USES_CONTAINER_UNITS);

        let size = context.get_container_size_query();
        let factor = self.value;
        let container_length = match self.unit {
            LengthUnit::Cqw => size.get_container_width(context),
            LengthUnit::Cqh => size.get_container_height(context),
            LengthUnit::Cqi => size.get_container_inline_size(context),
            LengthUnit::Cqb => size.get_container_block_size(context),
            LengthUnit::Cqmin => cmp::min(
                size.get_container_inline_size(context),
                size.get_container_block_size(context),
            ),
            LengthUnit::Cqmax => cmp::max(
                size.get_container_inline_size(context),
                size.get_container_block_size(context),
            ),
            _ => {
                unreachable!("container_relative_to_computed_value: not a container-relative unit")
            },
        };
        CSSPixelLength::new((container_length.to_f64_px() * factor as f64 / 100.0) as f32).finite()
    }

    /// Computes a ServoCharacterWidth length against a reference font size.
    fn servo_character_width_to_computed_value(
        &self,
        reference_font_size: computed::Length,
    ) -> computed::Length {
        debug_assert_eq!(self.unit, LengthUnit::ServoCharacterWidth);
        let cols = self.value as i32 as CSSFloat;
        // This applies the *converting a character width to pixels* algorithm
        // as specified in HTML5 § 14.5.4.
        let average_advance = reference_font_size * 0.5;
        let max_advance = reference_font_size;
        (average_advance * (cols - 1.0) + max_advance).finite()
    }

    /// Computes a length with a given font-relative base size.
    pub fn to_computed_value_with_base_size(
        &self,
        context: &Context,
        base_size: FontBaseSize,
        line_height_base: LineHeightBase,
    ) -> CSSPixelLength {
        if let Some(px) = self.to_px_if_absolute() {
            return CSSPixelLength::new(px)
                .zoom(context.builder.effective_zoom)
                .finite();
        }
        let unit = self.length_unit();
        if unit.is_font_relative() {
            return self.font_relative_to_computed_value(context, base_size, line_height_base);
        }
        if unit.is_viewport_percentage() {
            return self.viewport_percentage_to_computed_value(context);
        }
        if unit.is_container_relative() {
            return self.container_relative_to_computed_value(context);
        }
        debug_assert_eq!(unit, LengthUnit::ServoCharacterWidth);
        self.servo_character_width_to_computed_value(
            context.style().get_font().clone_font_size().computed_size(),
        )
    }
}

impl ToComputedValue for NoCalcLength {
    type ComputedValue = computed::Length;

    #[inline]
    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        self.to_computed_value_with_base_size(
            context,
            FontBaseSize::CurrentStyle,
            LineHeightBase::CurrentStyle,
        )
    }

    #[inline]
    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self::from_px(computed.px())
    }
}

impl ToCss for NoCalcLength {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        crate::values::serialize_specified_dimension(
            self.unitless_value(),
            self.unit(),
            false,
            dest,
        )
    }
}

impl ToTyped for NoCalcLength {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        let numeric_type = NumericType::length();
        let value = self.unitless_value();
        let unit = CssString::from(self.unit());
        dest.push(TypedValue::Numeric(NumericValue::Unit(UnitValue {
            numeric_type,
            value,
            unit,
        })));
        Ok(())
    }
}

impl SpecifiedValueInfo for NoCalcLength {}

impl PartialOrd for NoCalcLength {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        // For absolute units, compare in px.
        if let (Some(a), Some(b)) = (self.to_px_if_absolute(), other.to_px_if_absolute()) {
            return a.partial_cmp(&b);
        }
        if self.unit != other.unit {
            return None;
        }
        self.value.partial_cmp(&other.value)
    }
}

impl Zero for NoCalcLength {
    fn zero() -> Self {
        Self::from_px(0.)
    }

    fn is_zero(&self) -> bool {
        NoCalcLength::is_zero(self)
    }
}

/// An extension to `NoCalcLength` to parse `calc` expressions.
/// This is commonly used for the `<length>` values.
///
/// Either stored inline as length + unit without calc or as a boxed calc node.
///
/// <https://drafts.csswg.org/css-values/#lengths>
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToShmem)]
pub struct Length(NumericUnion<LengthUnit, f32, CalcLengthPercentage>);

impl ToCss for Length {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcLength::new(unit, value).to_css(dest),
            Unpacked::Boxed(calc) => calc.to_css(dest),
        }
    }
}

impl ToTyped for Length {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcLength::new(unit, value).to_typed(dest),
            Unpacked::Boxed(calc) => calc.to_typed(dest),
        }
    }
}

impl SpecifiedValueInfo for Length {}

impl From<NoCalcLength> for Length {
    #[inline]
    fn from(len: NoCalcLength) -> Self {
        Self::new(len)
    }
}

impl Length {
    /// Creates a length from a non-calc `NoCalcLength`.
    #[inline]
    pub fn new(len: NoCalcLength) -> Self {
        Self(NumericUnion::inline(len.unit, len.value))
    }

    /// Creates a length from a `calc()` expression.
    #[inline]
    pub fn new_calc(calc: Box<CalcLengthPercentage>) -> Self {
        Self(NumericUnion::boxed(calc))
    }

    /// Returns true if this is a `calc()` expression.
    #[inline]
    pub fn is_calc(&self) -> bool {
        self.0.is_boxed()
    }

    #[inline]
    fn parse_internal<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        num_context: AllowedNumericType,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();
        let token = input.next()?;
        match *token {
            Token::Dimension {
                value, ref unit, ..
            } if num_context.is_ok(context.parsing_mode, value) => {
                NoCalcLength::parse_dimension_with_context(context, value, unit)
                    .map(Self::new)
                    .map_err(|()| location.new_unexpected_token_error(token.clone()))
            },
            Token::Number { value, .. } if num_context.is_ok(context.parsing_mode, value) => {
                let allowed = context.parsing_mode.allows_unitless_lengths()
                    || allow_quirks.allowed(context.quirks_mode)
                    || (value == 0. && context.parsing_mode.allows_unitless_zero_lengths());

                if !allowed {
                    return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                }

                Ok(Self::new(NoCalcLength::from_px(value)))
            },
            Token::Function(ref name) => {
                let function = CalcNode::math_function(context, name, location)?;
                let calc = CalcNode::parse_length(context, input, num_context, function)?;
                Ok(Self::new_calc(Box::new(calc)))
            },
            ref token => return Err(location.new_unexpected_token_error(token.clone())),
        }
    }

    /// Parse a non-negative length
    #[inline]
    pub fn parse_non_negative<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_non_negative_quirky(context, input, AllowQuirks::No)
    }

    /// Parse a non-negative length, allowing quirks.
    #[inline]
    pub fn parse_non_negative_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(
            context,
            input,
            AllowedNumericType::NonNegative,
            allow_quirks,
        )
    }

    /// Get an absolute length from a px value.
    #[inline]
    pub fn from_px(px_value: CSSFloat) -> Length {
        Self::new(NoCalcLength::from_px(px_value))
    }

    /// Get a px value without context.
    pub fn to_computed_pixel_length_without_context(&self) -> Result<CSSFloat, ()> {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => {
                NoCalcLength::new(unit, value).to_computed_pixel_length_without_context()
            },
            Unpacked::Boxed(calc) => calc.to_computed_pixel_length_without_context(),
        }
    }

    /// Get a px value, with an optional GeckoFontMetrics getter to resolve font-relative units.
    #[cfg(feature = "gecko")]
    pub fn to_computed_pixel_length_with_font_metrics(
        &self,
        get_font_metrics: Option<impl Fn() -> GeckoFontMetrics>,
    ) -> Result<CSSFloat, ()> {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcLength::new(unit, value)
                .to_computed_pixel_length_with_font_metrics(get_font_metrics),
            Unpacked::Boxed(calc) => {
                calc.to_computed_pixel_length_with_font_metrics(get_font_metrics)
            },
        }
    }
}

impl Parse for Length {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_quirky(context, input, AllowQuirks::No)
    }
}

impl ToComputedValue for Length {
    type ComputedValue = computed::Length;

    #[inline]
    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => {
                NoCalcLength::new(unit, value).to_computed_value(context)
            },
            Unpacked::Boxed(calc) => {
                let result = calc.to_computed_value(context);
                debug_assert!(
                    result.to_length().is_some(),
                    "{:?} didn't resolve to a length: {:?}",
                    calc,
                    result,
                );
                result.to_length().unwrap_or_else(computed::Length::zero)
            },
        }
    }

    #[inline]
    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self::new(NoCalcLength::from_computed_value(computed))
    }
}

impl Zero for Length {
    fn zero() -> Self {
        Self::new(NoCalcLength::zero())
    }

    fn is_zero(&self) -> bool {
        // FIXME(emilio): Seems a bit weird to treat calc() unconditionally as
        // non-zero here?
        match self.0.unpack() {
            Unpacked::Inline(_, value) => value == 0.0,
            Unpacked::Boxed(_) => false,
        }
    }
}

impl Length {
    /// Parses a length, with quirks.
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(context, input, AllowedNumericType::All, allow_quirks)
    }
}

/// A wrapper of Length, whose value must be >= 0.
pub type NonNegativeLength = NonNegative<Length>;

impl Parse for NonNegativeLength {
    #[inline]
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Ok(NonNegative(Length::parse_non_negative(context, input)?))
    }
}

impl From<NoCalcLength> for NonNegativeLength {
    #[inline]
    fn from(len: NoCalcLength) -> Self {
        NonNegative(Length::new(len))
    }
}

impl From<Length> for NonNegativeLength {
    #[inline]
    fn from(len: Length) -> Self {
        NonNegative(len)
    }
}

impl NonNegativeLength {
    /// Get an absolute length from a px value.
    #[inline]
    pub fn from_px(px_value: CSSFloat) -> Self {
        Length::from_px(px_value.max(0.)).into()
    }

    /// Parses a non-negative length, optionally with quirks.
    #[inline]
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Ok(NonNegative(Length::parse_non_negative_quirky(
            context,
            input,
            allow_quirks,
        )?))
    }
}

/// A `<length-percentage>` value. This can be either a `<length>`, a
/// `<percentage>`, or a combination of both via `calc()`.
///
/// https://drafts.csswg.org/css-values-4/#typedef-length-percentage
#[allow(missing_docs)]
#[derive(Clone, Debug, MallocSizeOf, PartialEq, SpecifiedValueInfo, ToCss, ToShmem, ToTyped)]
pub enum LengthPercentage {
    Length(NoCalcLength),
    Percentage(NoCalcPercentage),
    Calc(Box<CalcLengthPercentage>),
}

impl From<Length> for LengthPercentage {
    fn from(len: Length) -> LengthPercentage {
        match len.0.extract() {
            Extracted::Inline(unit, value) => {
                LengthPercentage::Length(NoCalcLength::new(unit, value))
            },
            Extracted::Boxed(calc) => LengthPercentage::Calc(calc),
        }
    }
}

impl From<NoCalcLength> for LengthPercentage {
    #[inline]
    fn from(len: NoCalcLength) -> Self {
        LengthPercentage::Length(len)
    }
}

impl From<computed::Percentage> for LengthPercentage {
    #[inline]
    fn from(pc: computed::Percentage) -> Self {
        LengthPercentage::Percentage(NoCalcPercentage::new(pc.0))
    }
}

impl Parse for LengthPercentage {
    #[inline]
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_quirky(context, input, AllowQuirks::No)
    }
}

impl LengthPercentage {
    #[inline]
    /// Returns a `0%` value.
    pub fn zero_percent() -> LengthPercentage {
        LengthPercentage::Percentage(NoCalcPercentage::zero())
    }

    #[inline]
    /// Returns a `100%` value.
    pub fn hundred_percent() -> LengthPercentage {
        LengthPercentage::Percentage(NoCalcPercentage::hundred())
    }

    fn parse_internal<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        num_context: AllowedNumericType,
        allow_quirks: AllowQuirks,
        allow_anchor: AllowAnchorPositioningFunctions,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();
        let token = input.next()?;
        match *token {
            Token::Dimension {
                value, ref unit, ..
            } if num_context.is_ok(context.parsing_mode, value) => {
                return NoCalcLength::parse_dimension_with_context(context, value, unit)
                    .map(LengthPercentage::Length)
                    .map_err(|()| location.new_unexpected_token_error(token.clone()));
            },
            Token::Percentage { unit_value, .. }
                if num_context.is_ok(context.parsing_mode, unit_value) =>
            {
                return Ok(LengthPercentage::Percentage(NoCalcPercentage::new(
                    unit_value,
                )));
            },
            Token::Number { value, .. } if num_context.is_ok(context.parsing_mode, value) => {
                let allowed = context.parsing_mode.allows_unitless_lengths()
                    || allow_quirks.allowed(context.quirks_mode)
                    || (value == 0. && context.parsing_mode.allows_unitless_zero_lengths());

                if !allowed {
                    return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                }

                Ok(LengthPercentage::Length(NoCalcLength::from_px(value)))
            },
            Token::Function(ref name) => {
                let function = CalcNode::math_function(context, name, location)?;
                let calc = CalcNode::parse_length_or_percentage(
                    context,
                    input,
                    num_context,
                    function,
                    allow_anchor,
                )?;
                Ok(LengthPercentage::Calc(Box::new(calc)))
            },
            _ => return Err(location.new_unexpected_token_error(token.clone())),
        }
    }

    /// Parses allowing the unitless length quirk.
    /// <https://quirks.spec.whatwg.org/#the-unitless-length-quirk>
    #[inline]
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(
            context,
            input,
            AllowedNumericType::All,
            allow_quirks,
            AllowAnchorPositioningFunctions::No,
        )
    }

    /// Parses allowing the unitless length quirk, as well as allowing
    /// anchor-positioning related function, `anchor-size()`.
    #[inline]
    fn parse_quirky_with_anchor_size_function<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(
            context,
            input,
            AllowedNumericType::All,
            allow_quirks,
            AllowAnchorPositioningFunctions::AllowAnchorSize,
        )
    }

    /// Parses allowing the unitless length quirk, as well as allowing
    /// anchor-positioning related functions, `anchor()` and `anchor-size()`.
    #[inline]
    pub fn parse_quirky_with_anchor_functions<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(
            context,
            input,
            AllowedNumericType::All,
            allow_quirks,
            AllowAnchorPositioningFunctions::AllowAnchorAndAnchorSize,
        )
    }

    /// Parses non-negative length, allowing the unitless length quirk,
    /// as well as allowing `anchor-size()`.
    pub fn parse_non_negative_with_anchor_size<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(
            context,
            input,
            AllowedNumericType::NonNegative,
            allow_quirks,
            AllowAnchorPositioningFunctions::AllowAnchorSize,
        )
    }

    /// Parse a non-negative length.
    ///
    /// FIXME(emilio): This should be not public and we should use
    /// NonNegativeLengthPercentage instead.
    #[inline]
    pub fn parse_non_negative<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_non_negative_quirky(context, input, AllowQuirks::No)
    }

    /// Parse a non-negative length, with quirks.
    #[inline]
    pub fn parse_non_negative_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(
            context,
            input,
            AllowedNumericType::NonNegative,
            allow_quirks,
            AllowAnchorPositioningFunctions::No,
        )
    }

    /// Computes this specified value without style context. This fails for calc and non-px units.
    pub fn compute_without_context(&self) -> Option<computed::LengthPercentage> {
        use crate::values::normalize;
        match self {
            Self::Length(ref length) => length
                .to_computed_pixel_length_without_context()
                .map(|v| computed::LengthPercentage::new_length(computed::Length::new(v)))
                .ok(),
            Self::Percentage(ref pc) => Some(computed::LengthPercentage::new_percent(
                computed::Percentage(normalize(pc.get())),
            )),
            _ => None,
        }
    }
}

impl Zero for LengthPercentage {
    fn zero() -> Self {
        LengthPercentage::Length(NoCalcLength::zero())
    }

    fn is_zero(&self) -> bool {
        match *self {
            LengthPercentage::Length(l) => l.is_zero(),
            LengthPercentage::Percentage(p) => p.get() == 0.0,
            LengthPercentage::Calc(_) => false,
        }
    }
}

impl ZeroNoPercent for LengthPercentage {
    fn is_zero_no_percent(&self) -> bool {
        match *self {
            LengthPercentage::Percentage(_) => false,
            _ => self.is_zero(),
        }
    }
}

/// Check if this equal to a specific percentage.
pub trait EqualsPercentage {
    /// Returns true if this is a specific percentage value. This should exclude calc() even if it
    /// only contains percentage component.
    fn equals_percentage(&self, v: CSSFloat) -> bool;
}

impl EqualsPercentage for LengthPercentage {
    fn equals_percentage(&self, v: CSSFloat) -> bool {
        match *self {
            LengthPercentage::Percentage(p) => p.get() == v,
            _ => false,
        }
    }
}

/// A specified type for `<length-percentage> | auto`.
pub type LengthPercentageOrAuto = generics::LengthPercentageOrAuto<LengthPercentage>;

impl LengthPercentageOrAuto {
    /// Returns a value representing `0%`.
    #[inline]
    pub fn zero_percent() -> Self {
        generics::LengthPercentageOrAuto::LengthPercentage(LengthPercentage::zero_percent())
    }

    /// Parses a length or a percentage, allowing the unitless length quirk.
    /// <https://quirks.spec.whatwg.org/#the-unitless-length-quirk>
    #[inline]
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_with(context, input, |context, input| {
            LengthPercentage::parse_quirky(context, input, allow_quirks)
        })
    }
}

/// A wrapper of LengthPercentageOrAuto, whose value must be >= 0.
pub type NonNegativeLengthPercentageOrAuto =
    generics::LengthPercentageOrAuto<NonNegativeLengthPercentage>;

impl NonNegativeLengthPercentageOrAuto {
    /// Returns a value representing `0%`.
    #[inline]
    pub fn zero_percent() -> Self {
        generics::LengthPercentageOrAuto::LengthPercentage(
            NonNegativeLengthPercentage::zero_percent(),
        )
    }

    /// Parses a non-negative length-percentage, allowing the unitless length
    /// quirk.
    #[inline]
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_with(context, input, |context, input| {
            NonNegativeLengthPercentage::parse_quirky(context, input, allow_quirks)
        })
    }
}

/// A wrapper of LengthPercentage, whose value must be >= 0.
pub type NonNegativeLengthPercentage = NonNegative<LengthPercentage>;

/// Either a NonNegativeLengthPercentage or the `normal` keyword.
pub type NonNegativeLengthPercentageOrNormal =
    GenericLengthPercentageOrNormal<NonNegativeLengthPercentage>;

impl From<NoCalcLength> for NonNegativeLengthPercentage {
    #[inline]
    fn from(len: NoCalcLength) -> Self {
        NonNegative(LengthPercentage::from(len))
    }
}

impl Parse for NonNegativeLengthPercentage {
    #[inline]
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_quirky(context, input, AllowQuirks::No)
    }
}

impl NonNegativeLengthPercentage {
    #[inline]
    /// Returns a `0%` value.
    pub fn zero_percent() -> Self {
        NonNegative(LengthPercentage::zero_percent())
    }

    /// Parses a length or a percentage, allowing the unitless length quirk.
    /// <https://quirks.spec.whatwg.org/#the-unitless-length-quirk>
    #[inline]
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        LengthPercentage::parse_non_negative_quirky(context, input, allow_quirks).map(NonNegative)
    }

    /// Parses a length or a percentage, allowing the unitless length quirk,
    /// as well as allowing `anchor-size()`.
    #[inline]
    pub fn parse_non_negative_with_anchor_size<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        LengthPercentage::parse_non_negative_with_anchor_size(context, input, allow_quirks)
            .map(NonNegative)
    }
}

/// Either a `<length>` or the `auto` keyword.
///
/// Note that we use LengthPercentage just for convenience, since it pretty much
/// is everything we care about, but we could just add a similar LengthOrAuto
/// instead if we think getting rid of this weirdness is worth it.
pub type LengthOrAuto = generics::LengthPercentageOrAuto<Length>;

impl LengthOrAuto {
    /// Parses a length, allowing the unitless length quirk.
    /// <https://quirks.spec.whatwg.org/#the-unitless-length-quirk>
    #[inline]
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_with(context, input, |context, input| {
            Length::parse_quirky(context, input, allow_quirks)
        })
    }
}

/// Either a non-negative `<length>` or the `auto` keyword.
pub type NonNegativeLengthOrAuto = generics::LengthPercentageOrAuto<NonNegativeLength>;

/// Either a `<length>` or a `<number>`.
pub type LengthOrNumber = GenericLengthOrNumber<Length, Number>;

/// A specified value for `min-width`, `min-height`, `width` or `height` property.
pub type Size = GenericSize<NonNegativeLengthPercentage>;

impl Parse for Size {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Size::parse_quirky(context, input, AllowQuirks::No)
    }
}

macro_rules! parse_size_non_length {
    ($size:ident, $input:expr, $allow_webkit_fill_available:expr,
     $auto_or_none:expr => $auto_or_none_ident:ident) => {{
        let size = $input.try_parse(|input| {
            Ok(try_match_ident_ignore_ascii_case! { input,
                "min-content" | "-moz-min-content" => $size::MinContent,
                "max-content" | "-moz-max-content" => $size::MaxContent,
                "fit-content" | "-moz-fit-content" => $size::FitContent,
                #[cfg(feature = "gecko")]
                "-moz-available" => $size::MozAvailable,
                "-webkit-fill-available" if $allow_webkit_fill_available => $size::WebkitFillAvailable,
                "stretch" if is_stretch_enabled() => $size::Stretch,
                $auto_or_none => $size::$auto_or_none_ident,
            })
        });
        if size.is_ok() {
            return size;
        }
    }};
}

fn is_webkit_fill_available_enabled_in_width_and_height() -> bool {
    static_prefs::pref!("layout.css.webkit-fill-available.enabled")
}

fn is_webkit_fill_available_enabled_in_all_size_properties() -> bool {
    // For convenience at the callsites, we check both prefs here,
    // since both must be 'true' in order for the keyword to be
    // enabled in all size properties.
    static_prefs::pref!("layout.css.webkit-fill-available.enabled")
        && static_prefs::pref!("layout.css.webkit-fill-available.all-size-properties.enabled")
}

fn is_stretch_enabled() -> bool {
    static_prefs::pref!("layout.css.stretch-size-keyword.enabled")
}

fn is_fit_content_function_enabled() -> bool {
    static_prefs::pref!("layout.css.fit-content-function.enabled")
}

macro_rules! parse_fit_content_function {
    ($size:ident, $input:expr, $context:expr, $allow_quirks:expr) => {
        if is_fit_content_function_enabled() {
            if let Ok(length) = $input.try_parse(|input| {
                input.expect_function_matching("fit-content")?;
                input.parse_nested_block(|i| {
                    NonNegativeLengthPercentage::parse_quirky($context, i, $allow_quirks)
                })
            }) {
                return Ok($size::FitContentFunction(length));
            }
        }
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParseAnchorFunctions {
    Yes,
    No,
}

impl Size {
    /// Parses, with quirks.
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        let allow_webkit_fill_available = is_webkit_fill_available_enabled_in_all_size_properties();
        Self::parse_quirky_internal(
            context,
            input,
            allow_quirks,
            allow_webkit_fill_available,
            ParseAnchorFunctions::Yes,
        )
    }

    /// Parses for flex-basis: <width>
    pub fn parse_size_for_flex_basis_width<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_quirky_internal(
            context,
            input,
            AllowQuirks::No,
            true,
            ParseAnchorFunctions::No,
        )
    }

    /// Parses, with quirks and configurable support for
    /// whether the '-webkit-fill-available' keyword is allowed.
    /// TODO(dholbert) Fold this function into callsites in bug 1989073 when
    /// removing 'layout.css.webkit-fill-available.all-size-properties.enabled'.
    fn parse_quirky_internal<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
        allow_webkit_fill_available: bool,
        allow_anchor_functions: ParseAnchorFunctions,
    ) -> Result<Self, ParseError<'i>> {
        parse_size_non_length!(Size, input, allow_webkit_fill_available,
                               "auto" => Auto);
        parse_fit_content_function!(Size, input, context, allow_quirks);

        let allow_anchor = allow_anchor_functions == ParseAnchorFunctions::Yes
            && static_prefs::pref!("layout.css.anchor-positioning.enabled");
        match input
            .try_parse(|i| NonNegativeLengthPercentage::parse_quirky(context, i, allow_quirks))
        {
            Ok(length) => return Ok(GenericSize::LengthPercentage(length)),
            Err(e) if !allow_anchor => return Err(e.into()),
            Err(_) => (),
        };
        if let Ok(length) = input.try_parse(|i| {
            NonNegativeLengthPercentage::parse_non_negative_with_anchor_size(
                context,
                i,
                allow_quirks,
            )
        }) {
            return Ok(GenericSize::AnchorContainingCalcFunction(length));
        }
        Ok(Self::AnchorSizeFunction(Box::new(
            GenericAnchorSizeFunction::parse(context, input)?,
        )))
    }

    /// Parse a size for width or height, where -webkit-fill-available
    /// support is only controlled by one pref (vs. other properties where
    /// there's an additional pref check):
    /// TODO(dholbert) Remove this custom parse func in bug 1989073, along with
    /// 'layout.css.webkit-fill-available.all-size-properties.enabled'.
    pub fn parse_size_for_width_or_height_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        let allow_webkit_fill_available = is_webkit_fill_available_enabled_in_width_and_height();
        Self::parse_quirky_internal(
            context,
            input,
            allow_quirks,
            allow_webkit_fill_available,
            ParseAnchorFunctions::Yes,
        )
    }

    /// Parse a size for width or height, where -webkit-fill-available
    /// support is only controlled by one pref (vs. other properties where
    /// there's an additional pref check):
    /// TODO(dholbert) Remove this custom parse func in bug 1989073, along with
    /// 'layout.css.webkit-fill-available.all-size-properties.enabled'.
    pub fn parse_size_for_width_or_height<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let allow_webkit_fill_available = is_webkit_fill_available_enabled_in_width_and_height();
        Self::parse_quirky_internal(
            context,
            input,
            AllowQuirks::No,
            allow_webkit_fill_available,
            ParseAnchorFunctions::Yes,
        )
    }

    /// Returns `0%`.
    #[inline]
    pub fn zero_percent() -> Self {
        GenericSize::LengthPercentage(NonNegativeLengthPercentage::zero_percent())
    }
}

/// A specified value for `max-width` or `max-height` property.
pub type MaxSize = GenericMaxSize<NonNegativeLengthPercentage>;

impl Parse for MaxSize {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        MaxSize::parse_quirky(context, input, AllowQuirks::No)
    }
}

impl MaxSize {
    /// Parses, with quirks.
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        let allow_webkit_fill_available = is_webkit_fill_available_enabled_in_all_size_properties();
        parse_size_non_length!(MaxSize, input, allow_webkit_fill_available,
                               "none" => None);
        parse_fit_content_function!(MaxSize, input, context, allow_quirks);

        match input
            .try_parse(|i| NonNegativeLengthPercentage::parse_quirky(context, i, allow_quirks))
        {
            Ok(length) => return Ok(GenericMaxSize::LengthPercentage(length)),
            Err(e) if !static_prefs::pref!("layout.css.anchor-positioning.enabled") => {
                return Err(e.into())
            },
            Err(_) => (),
        };
        if let Ok(length) = input.try_parse(|i| {
            NonNegativeLengthPercentage::parse_non_negative_with_anchor_size(
                context,
                i,
                allow_quirks,
            )
        }) {
            return Ok(GenericMaxSize::AnchorContainingCalcFunction(length));
        }
        Ok(Self::AnchorSizeFunction(Box::new(
            GenericAnchorSizeFunction::parse(context, input)?,
        )))
    }
}

/// A specified non-negative `<length>` | `<number>`.
pub type NonNegativeLengthOrNumber = GenericLengthOrNumber<NonNegativeLength, NonNegativeNumber>;

/// A specified value for `margin` properties.
pub type Margin = GenericMargin<LengthPercentage>;

impl Margin {
    /// Parses a margin type, allowing the unitless length quirk.
    /// <https://quirks.spec.whatwg.org/#the-unitless-length-quirk>
    #[inline]
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        if let Ok(l) = input.try_parse(|i| LengthPercentage::parse_quirky(context, i, allow_quirks))
        {
            return Ok(Self::LengthPercentage(l));
        }
        match input.try_parse(|i| i.expect_ident_matching("auto")) {
            Ok(_) => return Ok(Self::Auto),
            Err(e) if !static_prefs::pref!("layout.css.anchor-positioning.enabled") => {
                return Err(e.into())
            },
            Err(_) => (),
        };
        if let Ok(l) = input.try_parse(|i| {
            LengthPercentage::parse_quirky_with_anchor_size_function(context, i, allow_quirks)
        }) {
            return Ok(Self::AnchorContainingCalcFunction(l));
        }
        let inner = GenericAnchorSizeFunction::<Margin>::parse(context, input)?;
        Ok(Self::AnchorSizeFunction(Box::new(inner)))
    }
}

impl Parse for Margin {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_quirky(context, input, AllowQuirks::No)
    }
}
