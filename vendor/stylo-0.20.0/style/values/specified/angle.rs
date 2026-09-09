/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified angles.

use crate::derives::*;
use crate::parser::{Parse, ParserContext};
use crate::typed_om::{NumericType, NumericValue, ToTyped, TypedValue, UnitValue};
use crate::values::computed::angle::Angle as ComputedAngle;
use crate::values::computed::{Context, ToComputedValue};
use crate::values::specified::calc::{CalcNode, CalcNumeric, Leaf};
use crate::values::tagged_numeric::{Extracted, NumericUnion, Unpacked};
use crate::values::CSSFloat;
use crate::Zero;
use cssparser::{match_ignore_ascii_case, Parser, Token};
use std::f32::consts::PI;
use std::fmt::{self, Write};
use std::ops::Neg;
use style_traits::{CssString, CssWriter, ParseError, SpecifiedValueInfo, ToCss};
use thin_vec::ThinVec;

/// Number of degrees per radian.
const DEG_PER_RAD: f32 = 180.0 / PI;
/// Number of degrees per turn.
const DEG_PER_TURN: f32 = 360.0;
/// Number of degrees per gradian.
const DEG_PER_GRAD: f32 = 180.0 / 200.0;

/// The unit of a `<angle>` value.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, PartialOrd, ToShmem)]
#[repr(u8)]
pub enum AngleUnit {
    /// `deg`
    Deg,
    /// `grad`
    Grad,
    /// `rad`
    Rad,
    /// `turn`
    Turn,
}

impl AngleUnit {
    /// Returns the angle unit for the given string.
    #[inline]
    pub fn from_str(unit: &str) -> Result<Self, ()> {
        Ok(match_ignore_ascii_case! { unit,
            "deg" => Self::Deg,
            "grad" => Self::Grad,
            "turn" => Self::Turn,
            "rad" => Self::Rad,
             _ => return Err(())
        })
    }

    /// Returns this unit as a string.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deg => "deg",
            Self::Grad => "grad",
            Self::Rad => "rad",
            Self::Turn => "turn",
        }
    }
}

/// A non-calc `<angle>` value.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, PartialOrd, ToShmem)]
#[repr(C)]
pub struct NoCalcAngle {
    unit: AngleUnit,
    value: CSSFloat,
}

impl Zero for NoCalcAngle {
    fn zero() -> Self {
        Self::from_degrees(0.)
    }

    fn is_zero(&self) -> bool {
        self.value == 0.0
    }
}

impl ToCss for NoCalcAngle {
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

impl ToTyped for NoCalcAngle {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        let numeric_type = NumericType::angle();
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

impl SpecifiedValueInfo for NoCalcAngle {}

impl NoCalcAngle {
    /// Creates an angle with the given unit and value.
    #[inline]
    pub fn new(unit: AngleUnit, value: CSSFloat) -> Self {
        Self { unit, value }
    }

    /// Creates an angle with the given value in degrees.
    #[inline]
    pub fn from_degrees(value: CSSFloat) -> Self {
        Self::new(AngleUnit::Deg, value)
    }

    /// Creates an angle with the given value in radians.
    #[inline]
    pub fn from_radians(value: CSSFloat) -> Self {
        Self::new(AngleUnit::Rad, value)
    }

    /// Return `0deg`.
    pub fn zero() -> Self {
        Self::from_degrees(0.0)
    }

    /// Returns the value of the angle in degrees.
    #[inline]
    pub fn degrees(&self) -> CSSFloat {
        match self.unit {
            AngleUnit::Deg => self.value,
            AngleUnit::Rad => self.value * DEG_PER_RAD,
            AngleUnit::Turn => self.value * DEG_PER_TURN,
            AngleUnit::Grad => self.value * DEG_PER_GRAD,
        }
    }

    /// Returns the value of the angle in radians.
    #[inline]
    pub fn radians(&self) -> CSSFloat {
        const RAD_PER_DEG: f32 = PI / 180.0;
        self.degrees() * RAD_PER_DEG
    }

    /// Returns the unit of the angle.
    #[inline]
    pub fn angle_unit(&self) -> AngleUnit {
        self.unit
    }

    /// Returns the unitless, raw value.
    #[inline]
    pub fn unitless_value(&self) -> CSSFloat {
        self.value
    }

    /// Returns the unit of the angle as a string.
    #[inline]
    pub fn unit(&self) -> &'static str {
        self.unit.as_str()
    }

    /// Return the canonical unit for this value.
    pub fn canonical_unit(&self) -> Option<&'static str> {
        Some("deg")
    }

    /// Convert this value to the specified unit, if possible.
    pub fn to(&self, unit: &str) -> Result<Self, ()> {
        let degrees = self.degrees();
        let unit = AngleUnit::from_str(unit)?;
        let divisor = match unit {
            AngleUnit::Deg => 1.0,
            AngleUnit::Grad => DEG_PER_GRAD,
            AngleUnit::Turn => DEG_PER_TURN,
            AngleUnit::Rad => DEG_PER_RAD,
        };
        Ok(Self::new(unit, degrees / divisor))
    }

    /// Parse an `<angle>` value given a value and a unit.
    pub fn parse_dimension(value: CSSFloat, unit: &str) -> Result<Self, ()> {
        let unit = AngleUnit::from_str(unit)?;
        Ok(Self::new(unit, value))
    }
}

impl Neg for NoCalcAngle {
    type Output = NoCalcAngle;

    #[inline]
    fn neg(self) -> NoCalcAngle {
        Self::new(self.unit, -self.value)
    }
}

/// A specified `<angle>` value, either a plain value or a `calc()` expression.
///
/// https://drafts.csswg.org/css-values/#angle-value
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToShmem)]
pub struct Angle(NumericUnion<AngleUnit, f32, CalcNumeric>);

impl ToCss for Angle {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcAngle::new(unit, value).to_css(dest),
            Unpacked::Boxed(calc) => calc.to_css(dest),
        }
    }
}

impl ToTyped for Angle {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcAngle::new(unit, value).to_typed(dest),
            Unpacked::Boxed(calc) => calc.to_typed(dest),
        }
    }
}

impl SpecifiedValueInfo for Angle {}

/// Whether to allow parsing an unitless zero as a valid angle.
///
/// This should always be `No`, except for exceptions like:
///
///   https://github.com/w3c/fxtf-drafts/issues/228
///
/// See also: https://github.com/w3c/csswg-drafts/issues/1162.
#[allow(missing_docs)]
pub enum AllowUnitlessZeroAngle {
    Yes,
    No,
}

impl Parse for Angle {
    /// Parses an angle according to CSS-VALUES § 6.1.
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(context, input, AllowUnitlessZeroAngle::No)
    }
}

impl Zero for Angle {
    fn zero() -> Self {
        Self::new(NoCalcAngle::zero())
    }

    fn is_zero(&self) -> bool {
        match self.0.unpack() {
            Unpacked::Inline(_, v) => v == 0.0,
            Unpacked::Boxed(_) => false,
        }
    }
}

impl ToComputedValue for Angle {
    type ComputedValue = ComputedAngle;

    #[inline]
    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        let degrees = match self.0.unpack() {
            Unpacked::Inline(unit, value) => NoCalcAngle::new(unit, value).degrees(),
            Unpacked::Boxed(ref calc) => calc.resolve(context, |result| match result {
                Ok(Leaf::Angle(a)) => a.degrees(),
                _ => {
                    debug_assert!(false, "Unexpected Angle::Calc without resolved angle");
                    f32::NAN
                },
            }),
        };

        // NaN and +-infinity should degenerate to 0: https://github.com/w3c/csswg-drafts/issues/6105
        ComputedAngle::from_degrees(if degrees.is_finite() { degrees } else { 0.0 })
    }

    #[inline]
    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self::new(NoCalcAngle::from_degrees(computed.degrees()))
    }
}

impl Angle {
    /// Creates an angle from a non-calc `NoCalcAngle`.
    #[inline]
    pub fn new(angle: NoCalcAngle) -> Self {
        Self(NumericUnion::inline(angle.unit, angle.value))
    }

    /// Creates an angle from a calc() expression.
    #[inline]
    pub fn new_calc(calc: Box<CalcNumeric>) -> Self {
        Self(NumericUnion::boxed(calc))
    }

    /// Creates an angle with the given value in degrees.
    #[inline]
    pub fn from_degrees(value: CSSFloat) -> Self {
        Self::new(NoCalcAngle::from_degrees(value))
    }

    /// Return `0deg`.
    pub fn zero() -> Self {
        Self::new(NoCalcAngle::zero())
    }

    /// Returns true if this is a `calc()` expression.
    #[inline]
    pub fn is_calc(&self) -> bool {
        self.0.is_boxed()
    }

    /// Returns the inner non-calc angle, if this isn't a calc expression.
    #[inline]
    pub fn as_no_calc(&self) -> Option<NoCalcAngle> {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => Some(NoCalcAngle::new(unit, value)),
            Unpacked::Boxed(_) => None,
        }
    }

    /// Returns the angle in degrees if it can be resolved at parse time, or None for calc
    /// expressions that require computed context. Prefer `to_computed_value(context).degrees()`
    /// when an element context is available.
    #[inline]
    pub fn degrees(&self) -> Option<CSSFloat> {
        match self.0.unpack() {
            Unpacked::Inline(unit, value) => Some(NoCalcAngle::new(unit, value).degrees()),
            Unpacked::Boxed(ref calc) => calc
                .as_angle()
                .map(|a| calc.clamping_mode.clamp(a.degrees())),
        }
    }

    /// Parse an `<angle>` allowing unitless zero to represent a zero angle.
    ///
    /// See the comment in `AllowUnitlessZeroAngle` for why.
    #[inline]
    pub fn parse_with_unitless<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(context, input, AllowUnitlessZeroAngle::Yes)
    }

    pub(super) fn parse_internal<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_unitless_zero: AllowUnitlessZeroAngle,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();
        let t = input.next()?;
        let allow_unitless_zero = matches!(allow_unitless_zero, AllowUnitlessZeroAngle::Yes);
        match *t {
            Token::Dimension {
                value, ref unit, ..
            } => match NoCalcAngle::parse_dimension(value, unit) {
                Ok(angle) => Ok(Self::new(angle)),
                Err(()) => {
                    let t = t.clone();
                    Err(input.new_unexpected_token_error(t))
                },
            },
            Token::Function(ref name) => {
                let function = CalcNode::math_function(context, name, location)?;
                CalcNode::parse_angle(context, input, function)
                    .map(Box::new)
                    .map(Self::new_calc)
            },
            Token::Number { value, .. } if value == 0. && allow_unitless_zero => Ok(Angle::zero()),
            ref t => {
                let t = t.clone();
                Err(input.new_unexpected_token_error(t))
            },
        }
    }
}

impl Neg for Angle {
    type Output = Angle;

    #[inline]
    fn neg(self) -> Angle {
        match self.0.extract() {
            Extracted::Inline(unit, value) => Self::new(NoCalcAngle::new(unit, -value)),
            Extracted::Boxed(mut c) => {
                c.node.negate();
                Self::new_calc(c)
            },
        }
    }
}
