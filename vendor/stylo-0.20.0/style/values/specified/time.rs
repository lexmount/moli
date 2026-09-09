/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified time values.

use crate::derives::*;
use crate::parser::{Parse, ParserContext};
use crate::typed_om::{NumericType, NumericValue, ToTyped, TypedValue, UnitValue};
use crate::values::computed::time::Time as ComputedTime;
use crate::values::computed::{Context, ToComputedValue};
use crate::values::specified::calc::{CalcNode, CalcNumeric, Leaf};
use crate::values::tagged_numeric::{NumericUnion, Unpacked};
use crate::values::CSSFloat;
use crate::Zero;
use cssparser::{match_ignore_ascii_case, Parser, Token};
use std::fmt::{self, Write};
use style_traits::values::specified::AllowedNumericType;
use style_traits::{
    CssString, CssWriter, ParseError, SpecifiedValueInfo, StyleParseErrorKind, ToCss,
};
use thin_vec::ThinVec;

/// A time unit.
#[derive(Clone, Copy, Debug, Eq, MallocSizeOf, PartialEq, ToShmem)]
#[repr(u8)]
pub enum TimeUnit {
    /// `s`
    Second,
    /// `ms`
    Millisecond,
}

impl TimeUnit {
    /// Returns the time unit for the given string.
    #[inline]
    pub fn from_str(unit: &str) -> Result<Self, ()> {
        Ok(match_ignore_ascii_case! { unit,
            "s" => Self::Second,
            "ms" => Self::Millisecond,
            _ => return Err(())
        })
    }

    /// Returns this unit as a string.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Second => "s",
            Self::Millisecond => "ms",
        }
    }
}

/// A time value according to CSS-VALUES § 6.2.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, ToShmem)]
#[repr(C)]
pub struct NoCalcTime {
    unit: TimeUnit,
    value: CSSFloat,
}

impl NoCalcTime {
    /// Creates a time with the given unit and value (in that unit).
    #[inline]
    pub fn new(unit: TimeUnit, value: CSSFloat) -> Self {
        Self { unit, value }
    }

    /// Returns a time value that represents `seconds` seconds.
    #[inline]
    pub fn from_seconds(seconds: CSSFloat) -> Self {
        Self::new(TimeUnit::Second, seconds)
    }

    /// Returns the time in fractional seconds.
    #[inline]
    pub fn seconds(&self) -> CSSFloat {
        match self.unit {
            TimeUnit::Second => self.value,
            TimeUnit::Millisecond => self.value / 1000.0,
        }
    }

    /// Returns the unit of the time.
    #[inline]
    pub fn time_unit(&self) -> TimeUnit {
        self.unit
    }

    /// Returns the unit of the time as a string.
    #[inline]
    pub fn unit(&self) -> &'static str {
        self.unit.as_str()
    }

    /// Return the unitless, raw value.
    #[inline]
    pub fn unitless_value(&self) -> CSSFloat {
        self.value
    }

    /// Return the canonical unit for this value.
    pub fn canonical_unit(&self) -> Option<&'static str> {
        Some("s")
    }

    /// Convert this value to the specified unit, if possible.
    pub fn to(&self, unit: &str) -> Result<Self, ()> {
        let target = TimeUnit::from_str(unit)?;
        let value = match target {
            TimeUnit::Second => self.seconds(),
            TimeUnit::Millisecond => self.seconds() * 1000.0,
        };
        Ok(Self::new(target, value))
    }

    /// Parses a time according to CSS-VALUES § 6.2.
    pub fn parse_dimension(value: CSSFloat, unit: &str) -> Result<Self, ()> {
        let unit = TimeUnit::from_str(unit)?;
        Ok(Self::new(unit, value))
    }
}

impl ToCss for NoCalcTime {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        crate::values::serialize_specified_dimension(
            self.unitless_value(),
            self.unit(),
            /* was_calc = */ false,
            dest,
        )
    }
}

impl ToComputedValue for NoCalcTime {
    type ComputedValue = ComputedTime;

    fn to_computed_value(&self, _: &Context) -> Self::ComputedValue {
        ComputedTime::from_seconds(self.seconds())
    }

    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self::from_seconds(computed.seconds())
    }
}

impl ToTyped for NoCalcTime {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        let numeric_value = NumericValue::Unit(UnitValue {
            numeric_type: NumericType::time(),
            value: self.unitless_value(),
            unit: CssString::from(self.unit()),
        });

        // https://drafts.css-houdini.org/css-typed-om-1/#reify-a-math-expression
        dest.push(TypedValue::Numeric(numeric_value));

        Ok(())
    }
}

impl SpecifiedValueInfo for NoCalcTime {}

/// A specified time value, either a plain value or a `calc()` expression.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToShmem)]
pub struct Time(NumericUnion<TimeUnit, f32, CalcNumeric>);

impl ToCss for Time {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcTime::new(unit, value).to_css(dest),
            Unpacked::Boxed(calc) => calc.to_css(dest),
        }
    }
}

impl ToTyped for Time {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcTime::new(unit, value).to_typed(dest),
            Unpacked::Boxed(calc) => calc.to_typed(dest),
        }
    }
}

impl SpecifiedValueInfo for Time {}

impl Time {
    /// Creates a time from a non-calc `NoCalcTime`.
    #[inline]
    pub fn new(time: NoCalcTime) -> Self {
        Self(NumericUnion::inline(time.unit, time.value))
    }

    /// Creates a time from a `calc()` expression.
    #[inline]
    pub fn new_calc(calc: Box<CalcNumeric>) -> Self {
        Self(NumericUnion::boxed(calc))
    }

    /// Returns a time value that represents `seconds` seconds.
    #[inline]
    pub fn from_seconds(seconds: CSSFloat) -> Self {
        Self::new(NoCalcTime::from_seconds(seconds))
    }

    /// Returns true if this is a `calc()` expression.
    #[inline]
    pub fn is_calc(&self) -> bool {
        self.0.is_boxed()
    }

    fn parse_with_clamping_mode<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        clamping_mode: AllowedNumericType,
    ) -> Result<Self, ParseError<'i>> {
        use style_traits::ParsingMode;

        let location = input.current_source_location();
        match *input.next()? {
            // Note that we generally pass ParserContext to is_ok() to check
            // that the ParserMode of the ParserContext allows all numeric
            // values for SMIL regardless of clamping_mode, but in this Time
            // value case, the value does not animate for SMIL at all, so we use
            // ParsingMode::DEFAULT directly.
            Token::Dimension {
                value, ref unit, ..
            } if clamping_mode.is_ok(ParsingMode::DEFAULT, value) => {
                NoCalcTime::parse_dimension(value, unit)
                    .map_err(|()| location.new_custom_error(StyleParseErrorKind::UnspecifiedError))
                    .map(Self::new)
            },
            Token::Function(ref name) => {
                let function = CalcNode::math_function(context, name, location)?;
                CalcNode::parse_time(context, input, clamping_mode, function)
                    .map(Box::new)
                    .map(Self::new_calc)
            },
            ref t => return Err(location.new_unexpected_token_error(t.clone())),
        }
    }

    /// Parses a non-negative time value.
    pub fn parse_non_negative<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_with_clamping_mode(context, input, AllowedNumericType::NonNegative)
    }
}

impl Zero for Time {
    #[inline]
    fn zero() -> Self {
        Self::from_seconds(0.0)
    }

    #[inline]
    fn is_zero(&self) -> bool {
        // The unit doesn't matter, i.e. `s` and `ms` are the same for zero.
        match self.0.unpack() {
            Unpacked::Inline(_, value) => value == 0.0,
            Unpacked::Boxed(_) => false,
        }
    }
}

impl ToComputedValue for Time {
    type ComputedValue = ComputedTime;

    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => {
                NoCalcTime::new(unit, value).to_computed_value(context)
            },
            Unpacked::Boxed(calc) => {
                let value = calc.resolve(context, |result| match result {
                    Ok(Leaf::Time(t)) => t.seconds(),
                    _ => {
                        debug_assert!(false, "Unexpected Time::Calc without resolved time");
                        f32::NAN
                    },
                });
                ComputedTime::from_seconds(
                    crate::values::normalize(value).min(f32::MAX).max(f32::MIN),
                )
            },
        }
    }

    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self::from_seconds(computed.seconds())
    }
}

impl Parse for Time {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_with_clamping_mode(context, input, AllowedNumericType::All)
    }
}
