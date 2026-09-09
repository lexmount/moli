/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Resolution values:
//!
//! https://drafts.csswg.org/css-values/#resolution

use crate::derives::*;
use crate::parser::{Parse, ParserContext};
use crate::values::computed::resolution::Resolution as ComputedResolution;
use crate::values::computed::{Context, ToComputedValue};
use crate::values::specified::calc::{CalcNode, CalcNumeric, Leaf};
use crate::values::tagged_numeric::{NumericUnion, Unpacked};
use crate::values::CSSFloat;
use cssparser::{match_ignore_ascii_case, Parser, Token};
use std::fmt::{self, Write};
use style_traits::{CssWriter, ParseError, SpecifiedValueInfo, StyleParseErrorKind, ToCss};

/// The unit of a `<resolution>` value.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, SpecifiedValueInfo, ToCss, ToShmem)]
#[repr(u8)]
pub enum ResolutionUnit {
    /// Dots per inch.
    Dpi,
    /// An alias unit for dots per pixel.
    X,
    /// Dots per pixel.
    Dppx,
    /// Dots per centimeter.
    Dpcm,
}

impl ResolutionUnit {
    /// Returns the resolution unit for the given string.
    #[inline]
    pub fn from_str(unit: &str) -> Result<Self, ()> {
        Ok(match_ignore_ascii_case! { &unit,
            "dpi" => Self::Dpi,
            "dppx" => Self::Dppx,
            "dpcm" => Self::Dpcm,
            "x" => Self::X,
            _ => return Err(())
        })
    }

    /// Returns this unit as a string.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dpi => "dpi",
            Self::X => "x",
            Self::Dppx => "dppx",
            Self::Dpcm => "dpcm",
        }
    }
}

/// A non-calc `<resolution>` value.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, ToShmem)]
#[repr(C)]
pub struct NoCalcResolution {
    unit: ResolutionUnit,
    value: CSSFloat,
}

impl NoCalcResolution {
    /// Creates a resolution with the given unit and value.
    #[inline]
    pub fn new(unit: ResolutionUnit, value: CSSFloat) -> Self {
        Self { unit, value }
    }

    /// Returns a resolution value from dppx units.
    #[inline]
    pub fn from_dppx(value: CSSFloat) -> Self {
        Self::new(ResolutionUnit::Dppx, value)
    }

    /// Returns a resolution value from x units.
    #[inline]
    pub fn from_x(value: CSSFloat) -> Self {
        Self::new(ResolutionUnit::X, value)
    }

    /// Convert this resolution value to dppx units.
    pub fn dppx(&self) -> CSSFloat {
        match self.unit {
            ResolutionUnit::X | ResolutionUnit::Dppx => self.value,
            _ => self.dpi() / 96.0,
        }
    }

    /// Convert this resolution value to dpi units.
    pub fn dpi(&self) -> CSSFloat {
        match self.unit {
            ResolutionUnit::Dpi => self.value,
            ResolutionUnit::X | ResolutionUnit::Dppx => self.value * 96.0,
            ResolutionUnit::Dpcm => self.value * 2.54,
        }
    }

    /// Returns the unit of the resolution.
    #[inline]
    pub fn resolution_unit(&self) -> ResolutionUnit {
        self.unit
    }

    /// Parse a resolution given a value and unit.
    pub fn parse_dimension(value: CSSFloat, unit: &str) -> Result<Self, ()> {
        let unit = ResolutionUnit::from_str(unit)?;
        Ok(Self::new(unit, value))
    }
}

impl ToCss for NoCalcResolution {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        crate::values::serialize_specified_dimension(
            self.value,
            self.unit.as_str(),
            /* was_calc = */ false,
            dest,
        )
    }
}

impl SpecifiedValueInfo for NoCalcResolution {}

/// A specified resolution value, either a plain value or a `calc()` expression.
///
/// https://drafts.csswg.org/css-values/#resolution-value
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToShmem)]
pub struct Resolution(NumericUnion<ResolutionUnit, f32, CalcNumeric>);

impl ToCss for Resolution {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcResolution::new(unit, value).to_css(dest),
            Unpacked::Boxed(calc) => calc.to_css(dest),
        }
    }
}

impl SpecifiedValueInfo for Resolution {}

impl Resolution {
    /// Creates a resolution from a non-calc `NoCalcResolution`.
    #[inline]
    pub fn new(resolution: NoCalcResolution) -> Self {
        Self(NumericUnion::inline(resolution.unit, resolution.value))
    }

    /// Creates a resolution from a `calc()` expression.
    #[inline]
    pub fn new_calc(calc: Box<CalcNumeric>) -> Self {
        Self(NumericUnion::boxed(calc))
    }

    /// Returns a resolution value from dppx units.
    #[inline]
    pub fn from_dppx(value: CSSFloat) -> Self {
        Self::new(NoCalcResolution::from_dppx(value))
    }

    /// Returns a resolution value from x units.
    #[inline]
    pub fn from_x(value: CSSFloat) -> Self {
        Self::new(NoCalcResolution::from_x(value))
    }

    /// Returns true if this is a `calc()` expression.
    #[inline]
    pub fn is_calc(&self) -> bool {
        self.0.is_boxed()
    }

    /// Parse a resolution given a value and unit.
    pub fn parse_dimension(value: CSSFloat, unit: &str) -> Result<Self, ()> {
        NoCalcResolution::parse_dimension(value, unit).map(Self::new)
    }
}

impl ToComputedValue for Resolution {
    type ComputedValue = ComputedResolution;

    #[inline]
    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        let dppx = match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcResolution::new(unit, value).dppx().max(0.0),
            Unpacked::Boxed(calc) => calc.resolve(context, |result| match result {
                Ok(Leaf::Resolution(r)) => r.dppx().max(0.0),
                _ => {
                    debug_assert!(
                        false,
                        "Unexpected Resolution::Calc without resolved resolution"
                    );
                    0.0
                },
            }),
        };
        ComputedResolution::from_dppx(crate::values::normalize(dppx))
    }

    #[inline]
    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self::from_dppx(computed.dppx())
    }
}

impl Parse for Resolution {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();
        match *input.next()? {
            Token::Dimension {
                value, ref unit, ..
            } if value >= 0. => Self::parse_dimension(value, unit)
                .map_err(|()| location.new_custom_error(StyleParseErrorKind::UnspecifiedError)),
            Token::Function(ref name) => {
                let function = CalcNode::math_function(context, name, location)?;
                CalcNode::parse_resolution(context, input, function)
                    .map(Box::new)
                    .map(Self::new_calc)
            },
            ref t => return Err(location.new_unexpected_token_error(t.clone())),
        }
    }
}
