/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! [Calc expressions][calc].
//!
//! [calc]: https://drafts.csswg.org/css-values/#calc-notation

use crate::color::parsing::ChannelKeyword;
use crate::color::AbsoluteColor;
use crate::derives::*;
use crate::parser::{Parse, ParserContext};
use crate::typed_om::{ToTyped, TypedValue};
use crate::values::computed::{self, ToComputedValue};
use crate::values::generics::calc::{
    self as generic, CalcNodeLeaf, CalcUnits, GenericAnchorFunctionFallback, MinMaxOp, ModRemOp,
    PositivePercentageBasis, ProgressClampingMode, RoundingStrategy, SimplificationResult, SortKey,
};
use crate::values::generics::length::GenericAnchorSizeFunction;
use crate::values::generics::position::{
    AnchorSideKeyword, GenericAnchorFunction, GenericAnchorSide, TreeScoped,
};
use crate::values::specified::length::NoCalcLength;
use crate::values::specified::{
    NoCalcAngle, NoCalcNumber, NoCalcPercentage, NoCalcResolution, NoCalcTime, TreeCountingFunction,
};
use crate::values::{CSSFloat, DashedIdent};
use cssparser::{match_ignore_ascii_case, CowRcStr, Parser, Token};
use debug_unreachable::debug_unreachable;
use smallvec::SmallVec;
use std::cmp;
use std::convert::AsRef;
use strum::IntoEnumIterator;
use strum_macros::{AsRefStr, EnumIter};
use style_traits::values::specified::AllowedNumericType;
use style_traits::{ParseError, SpecifiedValueInfo, StyleParseErrorKind};
use thin_vec::ThinVec;

/// The name of the mathematical function that we're parsing.
#[derive(AsRefStr, Clone, Copy, Debug, EnumIter, Parse)]
#[strum(serialize_all = "lowercase")]
pub enum MathFunction {
    /// `calc()`: https://drafts.csswg.org/css-values-4/#funcdef-calc
    Calc,
    /// `min()`: https://drafts.csswg.org/css-values-4/#funcdef-min
    Min,
    /// `max()`: https://drafts.csswg.org/css-values-4/#funcdef-max
    Max,
    /// `clamp()`: https://drafts.csswg.org/css-values-4/#funcdef-clamp
    Clamp,
    /// `round()`: https://drafts.csswg.org/css-values-4/#funcdef-round
    Round,
    /// `mod()`: https://drafts.csswg.org/css-values-4/#funcdef-mod
    Mod,
    /// `rem()`: https://drafts.csswg.org/css-values-4/#funcdef-rem
    Rem,
    /// `sin()`: https://drafts.csswg.org/css-values-4/#funcdef-sin
    Sin,
    /// `cos()`: https://drafts.csswg.org/css-values-4/#funcdef-cos
    Cos,
    /// `tan()`: https://drafts.csswg.org/css-values-4/#funcdef-tan
    Tan,
    /// `asin()`: https://drafts.csswg.org/css-values-4/#funcdef-asin
    Asin,
    /// `acos()`: https://drafts.csswg.org/css-values-4/#funcdef-acos
    Acos,
    /// `atan()`: https://drafts.csswg.org/css-values-4/#funcdef-atan
    Atan,
    /// `atan2()`: https://drafts.csswg.org/css-values-4/#funcdef-atan2
    Atan2,
    /// `pow()`: https://drafts.csswg.org/css-values-4/#funcdef-pow
    Pow,
    /// `sqrt()`: https://drafts.csswg.org/css-values-4/#funcdef-sqrt
    Sqrt,
    /// `hypot()`: https://drafts.csswg.org/css-values-4/#funcdef-hypot
    Hypot,
    /// `log()`: https://drafts.csswg.org/css-values-4/#funcdef-log
    Log,
    /// `exp()`: https://drafts.csswg.org/css-values-4/#funcdef-exp
    Exp,
    /// `abs()`: https://drafts.csswg.org/css-values-4/#funcdef-abs
    Abs,
    /// `sign()`: https://drafts.csswg.org/css-values-4/#funcdef-sign
    Sign,
    /// `progress()`: https://drafts.csswg.org/css-values-5/#funcdef-progress
    Progress,
    /// `sibling-count()`: https://drafts.csswg.org/css-values-5/#funcdef-sibling-count
    #[strum(serialize = "sibling-count")]
    SiblingCount,
    /// `sibling-index()`: https://drafts.csswg.org/css-values-5/#funcdef-sibling-index
    #[strum(serialize = "sibling-index")]
    SiblingIndex,
}

impl MathFunction {
    /// Returns an iterator for the enum variants
    pub fn variants() -> MathFunctionIter {
        return MathFunction::iter();
    }
}

/// A leaf node inside a `Calc` expression's AST.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToCss, ToShmem)]
#[repr(u8)]
pub enum Leaf {
    /// `<length>`
    Length(NoCalcLength),
    /// `<angle>`
    Angle(NoCalcAngle),
    /// `<time>`
    Time(NoCalcTime),
    /// `<resolution>`
    Resolution(NoCalcResolution),
    /// A component of a color.
    ColorComponent(ChannelKeyword),
    /// `<percentage>`
    Percentage(NoCalcPercentage),
    /// `<number>`
    Number(NoCalcNumber),
    /// A tree-counting function.
    TreeCountingFunction(TreeCountingFunction),
}

impl ToTyped for Leaf {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        // XXX Only supporting Length, Number, Percentage, Angle and Time for now
        match *self {
            Self::Length(ref l) => l.to_typed(dest),
            Self::Number(n) => n.to_typed(dest),
            Self::Percentage(p) => p.to_typed(dest),
            Self::Angle(ref a) => a.to_typed(dest),
            Self::Time(t) => t.to_typed(dest),
            _ => Err(()),
        }
    }
}

impl Leaf {
    /// Computes this leaf against the given context (if any), substituting color
    /// channel references with the matching channel of `origin_color` when it is
    /// provided. If no origin color is available, channel references are kept
    /// symbolic so they can be resolved later.
    pub fn to_computed_value(
        &self,
        context: Option<&computed::Context>,
        origin_color: Option<&AbsoluteColor>,
    ) -> Self {
        match self {
            Self::Length(l) => {
                let px = match context {
                    Some(context) => Ok(l.to_computed_value(context).px()),
                    None => l.to_computed_pixel_length_without_context(),
                };
                match px {
                    Ok(px) => Self::Length(NoCalcLength::from_px(px)),
                    Err(()) => self.clone(),
                }
            },
            Self::TreeCountingFunction(f) => match context {
                Some(context) => {
                    Self::Number(NoCalcNumber::new(f.to_computed_value(context) as f32))
                },
                None => self.clone(),
            },
            Self::ColorComponent(channel_keyword) => match origin_color {
                Some(origin_color) => {
                    match origin_color.get_component_by_channel_keyword(*channel_keyword) {
                        Ok(value) => Self::Number(NoCalcNumber::new(value.unwrap_or(0.0))),
                        // The channel is not valid for this color; keep it
                        // symbolic, which makes resolution fail later.
                        Err(()) => self.clone(),
                    }
                },
                None => self.clone(),
            },
            // The remaining leaves are already absolute (and thus
            // context-independent).
            Self::Angle(..)
            | Self::Time(..)
            | Self::Resolution(..)
            | Self::Percentage(..)
            | Self::Number(..) => self.clone(),
        }
    }
}

/// A struct to hold a simplified calc expression and associated clamping mode.
///
/// In some cases, e.g. DOMMatrix, we support calc(), but reject all the
/// relative lengths, and to_computed_pixel_length_without_context() handles
/// this case. Therefore, if you want to add a new field, please make sure this
/// function work properly.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToCss, ToShmem, ToTyped)]
#[allow(missing_docs)]
pub struct CalcNumeric {
    #[css(skip)]
    pub clamping_mode: AllowedNumericType,
    pub node: CalcNode,
}

impl CalcNumeric {
    /// Returns a new CalcNumeric with the same expression but the specified clamping mode
    pub fn with_clamping_mode(&self, clamping_mode: AllowedNumericType) -> Self {
        Self {
            clamping_mode,
            node: self.node.clone(),
        }
    }

    /// Returns a new CalcNumeric with the same clamping mode but a different leaf node
    pub fn with_leaf_node(&self, leaf: Leaf) -> Self {
        Self {
            clamping_mode: self.clamping_mode,
            node: CalcNode::Leaf(leaf),
        }
    }

    /// Resolves this calc expression given a computed context, applying clamping.
    pub fn resolve(
        &self,
        context: &computed::Context,
        leaf_to_f32: impl FnOnce(Result<Leaf, ()>) -> f32,
    ) -> f32 {
        let result = self.node.to_computed_value(Some(context), None);
        self.clamping_mode.clamp(leaf_to_f32(result.resolve()))
    }

    /// Gets this calc expression as a number
    pub fn as_number(&self) -> Option<NoCalcNumber> {
        match self.node.resolve() {
            Ok(Leaf::Number(n)) => Some(n),
            _ => None,
        }
    }

    /// Gets this calc expression as a percentage
    pub fn as_percentage(&self) -> Option<NoCalcPercentage> {
        match self.node.resolve() {
            Ok(Leaf::Percentage(p)) => Some(p),
            _ => None,
        }
    }

    /// Gets this calc expression as a time
    pub fn as_time(&self) -> Option<NoCalcTime> {
        match self.node.resolve() {
            Ok(Leaf::Time(t)) => Some(t),
            _ => None,
        }
    }

    /// Gets this calc expression as a resolution
    pub fn as_resolution(&self) -> Option<NoCalcResolution> {
        match self.node.resolve() {
            Ok(Leaf::Resolution(r)) => Some(r),
            _ => None,
        }
    }

    /// Gets this calc expression as an angle
    pub fn as_angle(&self) -> Option<NoCalcAngle> {
        match self.node.resolve() {
            Ok(Leaf::Angle(a)) => Some(a),
            _ => None,
        }
    }
}

impl SpecifiedValueInfo for CalcNumeric {}

/// A `calc()` expression that is known to resolve to a `<length-percentage>`.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToCss, ToShmem, ToTyped)]
pub struct CalcLengthPercentage(pub CalcNumeric);

impl SpecifiedValueInfo for CalcLengthPercentage {}

/// Should parsing anchor-positioning functions in `calc()` be allowed?
#[derive(Clone, Copy, PartialEq)]
pub enum AllowAnchorPositioningFunctions {
    /// Don't allow any anchor positioning function.
    No,
    /// Allow `anchor-size()` to be parsed.
    AllowAnchorSize,
    /// Allow `anchor()` and `anchor-size()` to be parsed.
    AllowAnchorAndAnchorSize,
}

bitflags! {
    /// Additional functions within math functions that are permitted to be parsed depending on
    /// the context of parsing (e.g. Parsing `inset` allows use of `anchor()` within `calc()`).
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct AdditionalFunctions: u8 {
        /// `anchor()` function.
        const ANCHOR = 1 << 0;
        /// `anchor-size()` function.
        const ANCHOR_SIZE = 1 << 1;
    }
}

/// What is allowed to be parsed for math functions within in this context?
#[derive(Clone, Copy)]
pub struct CalcParseFlags {
    /// Units allowed to be parsed.
    units: CalcUnits,
    /// Which relative color components, if any, are allowed.
    pub color_components: ChannelKeyword,
    /// Additional functions allowed to be parsed in this context.
    additional_functions: AdditionalFunctions,
    /// Whether or not in place operations should be performed. Normally, we aggressive
    /// simplify via in-place operations, but it is disabled for generating a trace of steps.
    in_place_operations: CalcNodeParseInPlaceOperations,
}

impl CalcParseFlags {
    /// Allow only specified units to be parsed, without any additional functions
    pub fn new(units: CalcUnits) -> Self {
        Self {
            units,
            color_components: ChannelKeyword::empty(),
            additional_functions: AdditionalFunctions::empty(),
            in_place_operations: CalcNodeParseInPlaceOperations::Yes,
        }
    }

    /// Add new units to the allowed units to be parsed.
    fn new_including(mut self, units: CalcUnits) -> Self {
        self.units |= units;
        self
    }

    /// Prevents in place operations to be performed
    pub fn new_without_in_place_operations(mut self) -> Self {
        self.in_place_operations = CalcNodeParseInPlaceOperations::No;
        self
    }

    /// Should given unit be allowed to parse?
    fn includes(&self, unit: CalcUnits) -> bool {
        self.units.intersects(unit)
    }
}

impl generic::CalcNodeLeaf for Leaf {
    fn unit(&self) -> CalcUnits {
        match self {
            Leaf::Length(_) => CalcUnits::LENGTH,
            Leaf::Angle(_) => CalcUnits::ANGLE,
            Leaf::Time(_) => CalcUnits::TIME,
            Leaf::Resolution(_) => CalcUnits::RESOLUTION,
            Leaf::Percentage(_) => CalcUnits::PERCENTAGE,
            Leaf::ColorComponent(_) | Leaf::Number(_) | Leaf::TreeCountingFunction(_) => {
                CalcUnits::empty()
            },
        }
    }

    fn unitless_value(&self) -> Option<f32> {
        Some(match *self {
            Self::Length(ref l) => l.unitless_value(),
            Self::Percentage(ref p) => p.get(),
            Self::Number(ref n) => n.value(),
            Self::Resolution(ref r) => r.dppx(),
            Self::Angle(ref a) => a.degrees(),
            Self::Time(ref t) => t.seconds(),
            Self::ColorComponent(_) | Self::TreeCountingFunction(_) => return None,
        })
    }

    fn is_same_unit_as(&self, other: &Self) -> bool {
        use self::Leaf::*;

        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return false;
        }

        match (self, other) {
            (Length(a), Length(b)) => a.length_unit() == b.length_unit(),
            (Angle(a), Angle(b)) => a.angle_unit() == b.angle_unit(),
            (Time(a), Time(b)) => a.time_unit() == b.time_unit(),
            (Resolution(a), Resolution(b)) => a.resolution_unit() == b.resolution_unit(),
            (ColorComponent(_), ColorComponent(_))
            | (Percentage(_), Percentage(_))
            | (Number(_), Number(_))
            | (TreeCountingFunction(_), TreeCountingFunction(_)) => true,
            _ => {
                match *other {
                    Number(..)
                    | Percentage(..)
                    | Angle(..)
                    | Time(..)
                    | Resolution(..)
                    | Length(..)
                    | ColorComponent(..)
                    | TreeCountingFunction(..) => {},
                }
                unsafe {
                    debug_unreachable!();
                }
            },
        }
    }

    fn as_angle_radians(&self) -> Option<f32> {
        if let Self::Angle(ref a) = *self {
            Some(a.radians())
        } else {
            None
        }
    }

    fn new_angle_from_radians(radians: f32) -> Self {
        Self::Angle(NoCalcAngle::from_degrees(radians.to_degrees()))
    }
    fn new_number(value: f32) -> Self {
        Self::Number(NoCalcNumber::new(value))
    }

    fn compare(&self, other: &Self, basis: PositivePercentageBasis) -> Option<cmp::Ordering> {
        use self::Leaf::*;

        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return None;
        }

        if matches!(self, Percentage(..)) && matches!(basis, PositivePercentageBasis::Unknown) {
            return None;
        }

        let self_negative = self.is_negative().unwrap_or(false);
        if self_negative != other.is_negative().unwrap_or(false) {
            return Some(if self_negative {
                cmp::Ordering::Less
            } else {
                cmp::Ordering::Greater
            });
        }

        match (self, other) {
            (&Percentage(ref one), &Percentage(ref other)) => one.get().partial_cmp(&other.get()),
            (&Length(ref one), &Length(ref other)) => one.partial_cmp(other),
            (&Angle(ref one), &Angle(ref other)) => one.degrees().partial_cmp(&other.degrees()),
            (&Time(ref one), &Time(ref other)) => one.seconds().partial_cmp(&other.seconds()),
            (&Resolution(ref one), &Resolution(ref other)) => one.dppx().partial_cmp(&other.dppx()),
            (&Number(ref one), &Number(ref other)) => one.partial_cmp(other),
            (&ColorComponent(ref one), &ColorComponent(ref other)) => one.partial_cmp(other),
            (&TreeCountingFunction(ref one), &TreeCountingFunction(ref other)) => {
                one.partial_cmp(other)
            },
            _ => {
                match *self {
                    Length(..)
                    | Percentage(..)
                    | Angle(..)
                    | Time(..)
                    | Number(..)
                    | Resolution(..)
                    | ColorComponent(..)
                    | TreeCountingFunction(..) => {},
                }
                unsafe {
                    debug_unreachable!("Forgot a branch?");
                }
            },
        }
    }

    fn as_number(&self) -> Option<f32> {
        match *self {
            Leaf::Length(_)
            | Leaf::Angle(_)
            | Leaf::Time(_)
            | Leaf::Resolution(_)
            | Leaf::Percentage(_)
            | Leaf::ColorComponent(_)
            | Leaf::TreeCountingFunction(_) => None,
            Leaf::Number(n) => Some(n.value()),
        }
    }

    fn sort_key(&self) -> SortKey {
        match *self {
            Self::Number(..) => SortKey::Number,
            Self::Percentage(..) => SortKey::Percentage,
            Self::Time(..) => SortKey::S,
            Self::Resolution(..) => SortKey::Dppx,
            Self::Angle(..) => SortKey::Deg,
            Self::Length(ref l) => l.sort_key(),
            Self::ColorComponent(..) => SortKey::ColorComponent,
            Self::TreeCountingFunction(..) => SortKey::Other,
        }
    }

    fn simplify(&mut self) -> SimplificationResult {
        match self {
            Leaf::Length(ref mut l) => {
                if let Some(px) = l.to_px_if_absolute() {
                    *l = NoCalcLength::from_px(px);
                    return SimplificationResult::Simplified;
                }
            },
            Leaf::Resolution(ref mut r) => {
                *r = NoCalcResolution::from_dppx(r.dppx());
                return SimplificationResult::Simplified;
            },
            Leaf::Time(ref mut t) => {
                *t = NoCalcTime::from_seconds(t.seconds());
                return SimplificationResult::Simplified;
            },
            Leaf::Angle(ref mut a) => {
                *a = NoCalcAngle::from_degrees(a.degrees());
                return SimplificationResult::Simplified;
            },
            _ => (),
        }
        return SimplificationResult::Unchanged;
    }

    /// Tries to merge one sum to another, that is, perform `x` + `y`.
    ///
    /// Only handles leaf nodes, it's the caller's responsibility to simplify
    /// them before calling this if needed.
    fn try_sum_in_place(&mut self, other: &Self) -> Result<(), ()> {
        use self::Leaf::*;

        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return Err(());
        }

        match (self, other) {
            (&mut Number(ref mut one), &Number(ref other)) => {
                *one = NoCalcNumber::new(one.value() + other.value());
            },
            (&mut Percentage(ref mut one), &Percentage(ref other)) => {
                *one = NoCalcPercentage::new(one.get() + other.get());
            },
            (&mut Angle(ref mut one), &Angle(ref other)) => {
                *one = NoCalcAngle::from_degrees(one.degrees() + other.degrees());
            },
            (&mut Time(ref mut one), &Time(ref other)) => {
                *one = NoCalcTime::from_seconds(one.seconds() + other.seconds());
            },
            (&mut Resolution(ref mut one), &Resolution(ref other)) => {
                *one = NoCalcResolution::from_dppx(one.dppx() + other.dppx());
            },
            (&mut Length(ref mut one), &Length(ref other)) => {
                *one = one.try_op(other, std::ops::Add::add)?;
            },
            (&mut ColorComponent(_), &ColorComponent(_)) => {
                // Can not get the sum of color components, because they haven't been resolved yet.
                return Err(());
            },
            (&mut TreeCountingFunction(_), &TreeCountingFunction(_)) => {
                // Can not get the sum of tree counting functions, because they haven't been resolved yet.
                return Err(());
            },
            _ => {
                match *other {
                    Number(..)
                    | Percentage(..)
                    | Angle(..)
                    | Time(..)
                    | Resolution(..)
                    | Length(..)
                    | ColorComponent(..)
                    | TreeCountingFunction(..) => {},
                }
                unsafe {
                    debug_unreachable!();
                }
            },
        }

        Ok(())
    }

    fn try_product_in_place(&mut self, other: &mut Self) -> bool {
        if let Self::Number(ref mut left) = *self {
            if let Self::Number(ref right) = *other {
                // Both sides are numbers, so we can just modify the left side.
                *left = NoCalcNumber::new(left.value() * right.value());
                true
            } else {
                // The right side is not a number, so the result should be in the units of the right
                // side.
                let left_val = left.value();
                if other.map(|v| v * left_val).is_ok() {
                    std::mem::swap(self, other);
                    true
                } else {
                    false
                }
            }
        } else if let Self::Number(ref right) = *other {
            // The left side is not a number, but the right side is, so the result is the left
            // side unit.
            let right_val = right.value();
            self.map(|v| v * right_val).is_ok()
        } else {
            // Neither side is a number, so a product is not possible.
            false
        }
    }

    fn try_op<O>(&self, other: &Self, op: O) -> Result<Self, ()>
    where
        O: Fn(f32, f32) -> f32,
    {
        use self::Leaf::*;

        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return Err(());
        }

        match (self, other) {
            (&Number(one), &Number(other)) => {
                return Ok(Leaf::Number(NoCalcNumber::new(op(
                    one.value(),
                    other.value(),
                ))));
            },
            (&Percentage(one), &Percentage(other)) => {
                return Ok(Leaf::Percentage(NoCalcPercentage::new(op(
                    one.get(),
                    other.get(),
                ))));
            },
            (&Angle(ref one), &Angle(ref other)) => {
                return Ok(Leaf::Angle(NoCalcAngle::from_degrees(op(
                    one.degrees(),
                    other.degrees(),
                ))));
            },
            (&Resolution(ref one), &Resolution(ref other)) => {
                return Ok(Leaf::Resolution(NoCalcResolution::from_dppx(op(
                    one.dppx(),
                    other.dppx(),
                ))));
            },
            (&Time(ref one), &Time(ref other)) => {
                return Ok(Leaf::Time(NoCalcTime::from_seconds(op(
                    one.seconds(),
                    other.seconds(),
                ))));
            },
            (&Length(ref one), &Length(ref other)) => {
                return Ok(Leaf::Length(one.try_op(other, op)?));
            },
            (&ColorComponent(..), &ColorComponent(..)) => {
                return Err(());
            },
            (&TreeCountingFunction(_), &TreeCountingFunction(_)) => {
                return Err(());
            },
            _ => {
                match *other {
                    Number(..)
                    | Percentage(..)
                    | Angle(..)
                    | Time(..)
                    | Length(..)
                    | Resolution(..)
                    | ColorComponent(..)
                    | TreeCountingFunction(..) => {},
                }
                unsafe {
                    debug_unreachable!();
                }
            },
        }
    }

    fn map(&mut self, mut op: impl FnMut(f32) -> f32) -> Result<(), ()> {
        Ok(match self {
            Leaf::Length(one) => *one = one.map(op),
            Leaf::Angle(one) => *one = NoCalcAngle::from_degrees(op(one.degrees())),
            Leaf::Time(one) => *one = NoCalcTime::from_seconds(op(one.seconds())),
            Leaf::Resolution(one) => *one = NoCalcResolution::from_dppx(op(one.dppx())),
            Leaf::Percentage(one) => *one = NoCalcPercentage::new(op(one.get())),
            Leaf::Number(one) => *one = NoCalcNumber::new(op(one.value())),
            Leaf::ColorComponent(..) | Leaf::TreeCountingFunction(..) => return Err(()),
        })
    }

    fn should_serialize_with_root_calc_wrapper(&self) -> bool {
        match self {
            Leaf::Length(_)
            | Leaf::Angle(_)
            | Leaf::Time(_)
            | Leaf::Resolution(_)
            | Leaf::ColorComponent(_)
            | Leaf::Percentage(_)
            | Leaf::Number(_) => true,
            Leaf::TreeCountingFunction(_) => false,
        }
    }
}

impl GenericAnchorSide<Box<CalcNode>> {
    fn parse_in_calc<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        if let Ok(k) = input.try_parse(|i| AnchorSideKeyword::parse(i)) {
            return Ok(Self::Keyword(k));
        }
        Ok(Self::Percentage(Box::new(CalcNode::parse_argument(
            context,
            input,
            CalcParseFlags::new(CalcUnits::PERCENTAGE),
        )?)))
    }
}

fn parse_anchor_function_fallback<'i, 't>(
    context: &ParserContext,
    additional_functions: AdditionalFunctions,
    input: &mut Parser<'i, 't>,
) -> Result<Box<GenericAnchorFunctionFallback<Leaf>>, ParseError<'i>> {
    if let Ok(l) = input.try_parse(|i| -> Result<CalcNode, ParseError<'i>> {
        Ok(CalcNode::Leaf(match i.next()? {
            &Token::Number { value, .. } => {
                if value != 0.0 {
                    return Err(i.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                }
                Leaf::Length(NoCalcLength::from_px(0.0))
            },
            &Token::Dimension {
                value, ref unit, ..
            } => Leaf::Length(
                NoCalcLength::parse_dimension_with_context(context, value, unit)
                    .map_err(|_| i.new_custom_error(StyleParseErrorKind::UnspecifiedError))?,
            ),
            &Token::Percentage { unit_value, .. } => {
                Leaf::Percentage(NoCalcPercentage::new(unit_value))
            },
            _ => return Err(i.new_custom_error(StyleParseErrorKind::UnspecifiedError)),
        }))
    }) {
        return Ok(Box::new(GenericAnchorFunctionFallback::new(false, l)));
    }
    let node = CalcNode::parse_argument(
        context,
        input,
        CalcParseFlags {
            units: CalcUnits::LENGTH_PERCENTAGE,
            color_components: ChannelKeyword::empty(),
            additional_functions,
            in_place_operations: CalcNodeParseInPlaceOperations::Yes,
        },
    )?
    .into_length_or_percentage(AllowedNumericType::All)
    .map_err(|_| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))?
    .0
    .node;
    Ok(Box::new(GenericAnchorFunctionFallback::new(true, node)))
}

impl GenericAnchorFunction<Box<CalcNode>, Box<GenericAnchorFunctionFallback<Leaf>>> {
    fn parse_in_calc<'i, 't>(
        context: &ParserContext,
        additional_functions: AdditionalFunctions,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        if !static_prefs::pref!("layout.css.anchor-positioning.enabled") {
            return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }
        input.parse_nested_block(|i| {
            let target_element = i.try_parse(|i| DashedIdent::parse(context, i)).ok();
            let side = GenericAnchorSide::parse_in_calc(context, i)?;
            let target_element = if target_element.is_none() {
                i.try_parse(|i| DashedIdent::parse(context, i)).ok()
            } else {
                target_element
            };
            let fallback = i
                .try_parse(|i| {
                    i.expect_comma()?;
                    parse_anchor_function_fallback(context, additional_functions, i)
                })
                .ok();
            Ok(Self {
                target_element: TreeScoped::with_default_level(
                    target_element.unwrap_or_else(DashedIdent::empty),
                ),
                side,
                fallback: fallback.into(),
            })
        })
    }
}

impl GenericAnchorSizeFunction<Box<GenericAnchorFunctionFallback<Leaf>>> {
    fn parse_in_calc<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        if !static_prefs::pref!("layout.css.anchor-positioning.enabled") {
            return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }
        GenericAnchorSizeFunction::parse_inner(context, input, |i| {
            parse_anchor_function_fallback(context, AdditionalFunctions::ANCHOR_SIZE, i)
        })
    }
}

/// Specified `anchor()` function in math functions.
pub type CalcAnchorFunction = generic::GenericCalcAnchorFunction<Leaf>;
/// Specified `anchor-size()` function in math functions.
pub type CalcAnchorSizeFunction = generic::GenericCalcAnchorSizeFunction<Leaf>;

/// Whether in place operations should be done when parsing expressions to create CalcNode
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CalcNodeParseInPlaceOperations {
    /// Avoid in place operations
    No,
    /// Alow in place operations
    Yes,
}

/// A calc node representation for specified values.
pub type CalcNode = generic::GenericCalcNode<Leaf>;
impl CalcNode {
    /// Returns whether this expression contains any font-relative length.
    pub fn has_font_relative_length(&self) -> bool {
        let mut found = false;
        let _ = self.map_leaves(|leaf| {
            found |=
                matches!(leaf, Leaf::Length(length) if length.length_unit().is_font_relative());
            leaf.clone()
        });
        found
    }

    /// Tries to parse a single element in the expression, that is, a
    /// `<length>`, `<angle>`, `<time>`, `<percentage>`, `<resolution>`, etc.
    ///
    /// May return a "complex" `CalcNode`, in the presence of a parenthesized
    /// expression, for example.
    fn parse_one<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        flags: CalcParseFlags,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();
        match input.next()? {
            &Token::Number { value, .. } => {
                Ok(CalcNode::Leaf(Leaf::Number(NoCalcNumber::new(value))))
            },
            &Token::Dimension {
                value, ref unit, ..
            } => {
                if flags.includes(CalcUnits::LENGTH) {
                    if let Ok(l) = NoCalcLength::parse_dimension_with_context(context, value, unit)
                    {
                        return Ok(CalcNode::Leaf(Leaf::Length(l)));
                    }
                }
                if flags.includes(CalcUnits::ANGLE) {
                    if let Ok(a) = NoCalcAngle::parse_dimension(value, unit) {
                        return Ok(CalcNode::Leaf(Leaf::Angle(a)));
                    }
                }
                if flags.includes(CalcUnits::TIME) {
                    if let Ok(t) = NoCalcTime::parse_dimension(value, unit) {
                        return Ok(CalcNode::Leaf(Leaf::Time(t)));
                    }
                }
                if flags.includes(CalcUnits::RESOLUTION) {
                    if let Ok(t) = NoCalcResolution::parse_dimension(value, unit) {
                        return Ok(CalcNode::Leaf(Leaf::Resolution(t)));
                    }
                }
                return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
            },
            &Token::Percentage { unit_value, .. } if flags.includes(CalcUnits::PERCENTAGE) => Ok(
                CalcNode::Leaf(Leaf::Percentage(NoCalcPercentage::new(unit_value))),
            ),
            &Token::ParenthesisBlock => {
                input.parse_nested_block(|input| CalcNode::parse_argument(context, input, flags))
            },
            &Token::Function(ref name)
                if flags
                    .additional_functions
                    .intersects(AdditionalFunctions::ANCHOR)
                    && name.eq_ignore_ascii_case("anchor") =>
            {
                let anchor_function = GenericAnchorFunction::parse_in_calc(
                    context,
                    flags.additional_functions,
                    input,
                )?;
                Ok(CalcNode::Anchor(Box::new(anchor_function)))
            },
            &Token::Function(ref name)
                if flags
                    .additional_functions
                    .intersects(AdditionalFunctions::ANCHOR_SIZE)
                    && name.eq_ignore_ascii_case("anchor-size") =>
            {
                let anchor_size_function =
                    GenericAnchorSizeFunction::parse_in_calc(context, input)?;
                Ok(CalcNode::AnchorSize(Box::new(anchor_size_function)))
            },
            &Token::Function(ref name) => {
                let function = CalcNode::math_function(context, &name, location)?;
                CalcNode::parse(context, input, function, flags)
            },
            &Token::Ident(ref ident) => {
                let leaf = match_ignore_ascii_case! { &**ident,
                    "e" => Leaf::Number(NoCalcNumber::new(std::f32::consts::E)),
                    "pi" => Leaf::Number(NoCalcNumber::new(std::f32::consts::PI)),
                    "infinity" => Leaf::Number(NoCalcNumber::new(f32::INFINITY)),
                    "-infinity" => Leaf::Number(NoCalcNumber::new(f32::NEG_INFINITY)),
                    "nan" => Leaf::Number(NoCalcNumber::new(f32::NAN)),
                    _ => {
                        match ChannelKeyword::from_ident(&ident) {
                            Ok(channel_keyword) if flags.color_components.contains(channel_keyword) => Leaf::ColorComponent(channel_keyword),
                            _ => return Err(location.new_unexpected_token_error(Token::Ident(ident.clone()))),
                        }
                    },
                };
                Ok(CalcNode::Leaf(leaf))
            },
            t => Err(location.new_unexpected_token_error(t.clone())),
        }
    }

    /// Parse a top-level `calc` expression, with all nested sub-expressions.
    ///
    /// This is in charge of parsing, for example, `2 + 3 * 100%`.
    pub fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        function: MathFunction,
        flags: CalcParseFlags,
    ) -> Result<Self, ParseError<'i>> {
        input.parse_nested_block(|input| {
            match function {
                MathFunction::Calc => Self::parse_argument(context, input, flags),
                MathFunction::Clamp => {
                    let min_val = if input
                        .try_parse(|min| min.expect_ident_matching("none"))
                        .ok()
                        .is_none()
                    {
                        Some(Self::parse_argument(context, input, flags)?)
                    } else {
                        None
                    };

                    input.expect_comma()?;
                    let center = Self::parse_argument(context, input, flags)?;
                    input.expect_comma()?;

                    let max_val = if input
                        .try_parse(|max| max.expect_ident_matching("none"))
                        .ok()
                        .is_none()
                    {
                        Some(Self::parse_argument(context, input, flags)?)
                    } else {
                        None
                    };

                    // Specification does not state how serialization should occur for clamp
                    // https://github.com/w3c/csswg-drafts/issues/13535
                    // tentatively partially serialize to min/max
                    // clamp(MIN, VAL, none) is equivalent to max(MIN, VAL)
                    // clamp(none, VAL, MAX) is equivalent to min(VAL, MAX)
                    // clamp(none, VAL, none) is equivalent to just calc(VAL)
                    Ok(match (min_val, max_val) {
                        (None, None) => center,
                        (None, Some(max)) => Self::MinMax(vec![center, max].into(), MinMaxOp::Min),
                        (Some(min), None) => Self::MinMax(vec![min, center].into(), MinMaxOp::Max),
                        (Some(min), Some(max)) => Self::Clamp {
                            min: Box::new(min),
                            center: Box::new(center),
                            max: Box::new(max),
                        },
                    })
                },
                MathFunction::Round => {
                    let strategy = input.try_parse(parse_rounding_strategy);

                    // <rounding-strategy> = nearest | up | down | to-zero
                    // https://drafts.csswg.org/css-values-4/#calc-syntax
                    fn parse_rounding_strategy<'i, 't>(
                        input: &mut Parser<'i, 't>,
                    ) -> Result<RoundingStrategy, ParseError<'i>> {
                        Ok(try_match_ident_ignore_ascii_case! { input,
                            "nearest" => RoundingStrategy::Nearest,
                            "up" => RoundingStrategy::Up,
                            "down" => RoundingStrategy::Down,
                            "to-zero" => RoundingStrategy::ToZero,
                        })
                    }

                    if strategy.is_ok() {
                        input.expect_comma()?;
                    }

                    let value = Self::parse_argument(context, input, flags)?;

                    // <step> defaults to the number 1 if not provided
                    // https://drafts.csswg.org/css-values-4/#funcdef-round
                    let step = input.try_parse(|input| {
                        input.expect_comma()?;
                        Self::parse_argument(context, input, flags)
                    });

                    let step = step.unwrap_or(Self::Leaf(Leaf::Number(NoCalcNumber::new(1.0))));

                    Ok(Self::Round {
                        strategy: strategy.unwrap_or(RoundingStrategy::Nearest),
                        value: Box::new(value),
                        step: Box::new(step),
                    })
                },
                MathFunction::Mod | MathFunction::Rem => {
                    let dividend = Self::parse_argument(context, input, flags)?;
                    input.expect_comma()?;
                    let divisor = Self::parse_argument(context, input, flags)?;

                    let op = match function {
                        MathFunction::Mod => ModRemOp::Mod,
                        MathFunction::Rem => ModRemOp::Rem,
                        _ => unreachable!(),
                    };
                    Ok(Self::ModRem {
                        dividend: Box::new(dividend),
                        divisor: Box::new(divisor),
                        op,
                    })
                },
                MathFunction::Min | MathFunction::Max => {
                    // TODO(emilio): The common case for parse_comma_separated
                    // is just one element, but for min / max is two, really...
                    //
                    // Consider adding an API to cssparser to specify the
                    // initial vector capacity?
                    let arguments = input.parse_comma_separated(|input| {
                        let result = Self::parse_argument(context, input, flags)?;
                        Ok(result)
                    })?;

                    let op = match function {
                        MathFunction::Min => MinMaxOp::Min,
                        MathFunction::Max => MinMaxOp::Max,
                        _ => unreachable!(),
                    };

                    Ok(Self::MinMax(arguments.into(), op))
                },
                MathFunction::Sin | MathFunction::Cos | MathFunction::Tan => {
                    let node = Self::parse_argument(
                        context,
                        input,
                        flags.new_including(CalcUnits::ANGLE),
                    )?;
                    Ok(match function {
                        MathFunction::Sin => Self::Sin(Box::new(node)),
                        MathFunction::Cos => Self::Cos(Box::new(node)),
                        MathFunction::Tan => Self::Tan(Box::new(node)),
                        _ => unsafe { debug_unreachable!("We just checked!") },
                    })
                },
                MathFunction::Asin | MathFunction::Acos | MathFunction::Atan => {
                    let node = Self::parse_argument(context, input, flags)?;
                    Ok(match function {
                        MathFunction::Asin => Self::Asin(Box::new(node)),
                        MathFunction::Acos => Self::Acos(Box::new(node)),
                        MathFunction::Atan => Self::Atan(Box::new(node)),
                        _ => unsafe { debug_unreachable!("We just checked!") },
                    })
                },
                MathFunction::Atan2 => {
                    let allow_all = flags.new_including(CalcUnits::ALL);
                    let a = Self::parse_argument(context, input, allow_all)?;
                    input.expect_comma()?;
                    let b = Self::parse_argument(context, input, allow_all)?;
                    // TODO(Bug 2042060) - Allow combining length and percentage arguments (if it can be resolved).
                    if a.unit() != b.unit() {
                        return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                    }
                    Ok(Self::Atan2(Box::new(a), Box::new(b)))
                },
                MathFunction::Pow => {
                    let a = Self::parse_argument(context, input, flags)?;
                    input.expect_comma()?;
                    let b = Self::parse_argument(context, input, flags)?;
                    Ok(Self::Pow(Box::new(a), Box::new(b)))
                },
                MathFunction::Sqrt => {
                    let a = Self::parse_argument(context, input, flags)?;
                    Ok(Self::Sqrt(Box::new(a)))
                },
                MathFunction::Hypot => {
                    let arguments = input.parse_comma_separated(|input| {
                        let result = Self::parse_argument(context, input, flags)?;
                        Ok(result)
                    })?;

                    Ok(Self::Hypot(arguments.into()))
                },
                MathFunction::Log => {
                    let a = Self::parse_argument(context, input, flags)?;
                    let b = input
                        .try_parse(|input| {
                            input.expect_comma()?;
                            Self::parse_argument(context, input, flags)
                        })
                        .ok();
                    Ok(Self::Log(Box::new(a), b.map(Box::new).into()))
                },
                MathFunction::Exp => {
                    let a = Self::parse_argument(context, input, flags)?;
                    Ok(Self::Exp(Box::new(a)))
                },
                MathFunction::Abs => {
                    let node = Self::parse_argument(context, input, flags)?;
                    Ok(Self::Abs(Box::new(node)))
                },
                MathFunction::Sign => {
                    // The sign of a percentage is dependent on the percentage basis, so if
                    // percentages aren't allowed (so there's no basis) we shouldn't allow them in
                    // sign(). The rest of the units are safe tho.
                    let node = Self::parse_argument(
                        context,
                        input,
                        flags.new_including(CalcUnits::ALL - CalcUnits::PERCENTAGE),
                    )?;
                    Ok(Self::Sign(Box::new(node)))
                },
                MathFunction::Progress => {
                    if !static_prefs::pref!("layout.css.progress-function.enabled") {
                        return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                    }

                    let clamping_mode = input
                        .try_parse(|i| ProgressClampingMode::parse(i))
                        .unwrap_or(ProgressClampingMode::Clamp);

                    let allow_all = flags.new_including(CalcUnits::ALL);
                    let value = Self::parse_argument(context, input, allow_all)?;
                    input.expect_comma()?;
                    let start = Self::parse_argument(context, input, allow_all)?;
                    input.expect_comma()?;
                    let end = Self::parse_argument(context, input, allow_all)?;

                    // TODO(Bug 2042060) - Allow combining length and percentage arguments (if it can be resolved).
                    if value.unit() != start.unit() || value.unit() != end.unit() {
                        return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                    }

                    Ok(Self::Progress {
                        clamping_mode,
                        value: Box::new(value),
                        start: Box::new(start),
                        end: Box::new(end),
                    })
                },
                MathFunction::SiblingCount | MathFunction::SiblingIndex => {
                    if !static_prefs::pref!("layout.css.tree-counting-functions.enabled") {
                        return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                    }

                    if !context.has_element_context() {
                        return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                    }

                    // Tree-counting functions have no arguments
                    input.expect_exhausted()?;

                    Ok(Self::Leaf(Leaf::TreeCountingFunction(match function {
                        MathFunction::SiblingCount => TreeCountingFunction::SiblingCount,
                        MathFunction::SiblingIndex => TreeCountingFunction::SiblingIndex,
                        _ => unsafe { debug_unreachable!("We just checked!") },
                    })))
                },
            }
        })
    }

    fn parse_argument<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        flags: CalcParseFlags,
    ) -> Result<Self, ParseError<'i>> {
        let mut sum = SmallVec::<[CalcNode; 1]>::new();
        let first = Self::parse_product(context, input, flags)?;
        sum.push(first);
        loop {
            let start = input.state();
            match input.next_including_whitespace() {
                Ok(&Token::WhiteSpace(_)) => {
                    if input.is_exhausted() {
                        break; // allow trailing whitespace
                    }
                    match *input.next()? {
                        Token::Delim('+') => {
                            let rhs = Self::parse_product(context, input, flags)?;
                            if flags.in_place_operations == CalcNodeParseInPlaceOperations::No
                                || sum.last_mut().unwrap().try_sum_in_place(&rhs).is_err()
                            {
                                sum.push(rhs);
                            }
                        },
                        Token::Delim('-') => {
                            let mut rhs = Self::parse_product(context, input, flags)?;
                            rhs.negate();
                            if flags.in_place_operations == CalcNodeParseInPlaceOperations::No
                                || sum.last_mut().unwrap().try_sum_in_place(&rhs).is_err()
                            {
                                sum.push(rhs);
                            }
                        },
                        _ => {
                            input.reset(&start);
                            break;
                        },
                    }
                },
                _ => {
                    input.reset(&start);
                    break;
                },
            }
        }

        Ok(if sum.len() == 1 {
            sum.drain(..).next().unwrap()
        } else {
            Self::Sum(sum.into_boxed_slice().into())
        })
    }

    /// Parse a top-level `calc` expression, and all the products that may
    /// follow, and stop as soon as a non-product expression is found.
    ///
    /// This should parse correctly:
    ///
    /// * `2`
    /// * `2 * 2`
    /// * `2 * 2 + 2` (but will leave the `+ 2` unparsed).
    ///
    fn parse_product<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        flags: CalcParseFlags,
    ) -> Result<Self, ParseError<'i>> {
        let mut product = SmallVec::<[CalcNode; 1]>::new();
        let first = Self::parse_one(context, input, flags)?;
        product.push(first);

        loop {
            let start = input.state();
            match input.next() {
                Ok(&Token::Delim('*')) => {
                    let mut rhs = Self::parse_one(context, input, flags)?;

                    // We can unwrap here, because we start the function by adding a node to
                    // the list.
                    if flags.in_place_operations == CalcNodeParseInPlaceOperations::No
                        || !product.last_mut().unwrap().try_product_in_place(&mut rhs)
                    {
                        product.push(rhs);
                    }
                },
                Ok(&Token::Delim('/')) => {
                    let rhs = Self::parse_one(context, input, flags)?;

                    enum InPlaceDivisionResult {
                        /// The right was merged into the left.
                        Merged,
                        /// The right is not a number or could not be resolved, so the left is
                        /// unchanged.
                        Unchanged,
                        /// The right was resolved, but was not a number, so the calculation is
                        /// invalid.
                        Invalid,
                    }

                    fn try_division_in_place(
                        left: &mut CalcNode,
                        right: &CalcNode,
                        in_place_operations: CalcNodeParseInPlaceOperations,
                    ) -> InPlaceDivisionResult {
                        match right.unit() {
                            Ok(unit) if unit.is_empty() => {},
                            _ => return InPlaceDivisionResult::Invalid,
                        }
                        if in_place_operations == CalcNodeParseInPlaceOperations::No {
                            return InPlaceDivisionResult::Unchanged;
                        }

                        if let Ok(resolved) = right.resolve() {
                            if let Some(number) = resolved.as_number() {
                                if number != 1.0 && left.is_product_distributive() {
                                    if left.map(|l| l / number).is_err() {
                                        return InPlaceDivisionResult::Invalid;
                                    }
                                    return InPlaceDivisionResult::Merged;
                                }
                            }
                        }
                        InPlaceDivisionResult::Unchanged
                    }

                    // If the right hand side is already a number, merge it with the last node on
                    // the product list. Otherwise keep unresolved number-like denominators as
                    // inverted nodes so tree-counting math can resolve them later.
                    // We can unwrap here, becuase we start the function by adding a node to
                    // the list.
                    match try_division_in_place(
                        &mut product.last_mut().unwrap(),
                        &rhs,
                        flags.in_place_operations,
                    ) {
                        InPlaceDivisionResult::Merged => {},
                        InPlaceDivisionResult::Unchanged => {
                            product.push(Self::Invert(Box::new(rhs)))
                        },
                        InPlaceDivisionResult::Invalid => {
                            return Err(
                                input.new_custom_error(StyleParseErrorKind::UnspecifiedError)
                            )
                        },
                    }
                },
                _ => {
                    input.reset(&start);
                    break;
                },
            }
        }

        Ok(if product.len() == 1 {
            product.drain(..).next().unwrap()
        } else {
            Self::Product(product.into_boxed_slice().into())
        })
    }

    /// Computes this calc tree against the given context (if any), resolving
    /// context-dependent leaves (e.g. lengths) and substituting color channel
    /// references against `origin_color` when provided.
    pub fn to_computed_value(
        &self,
        context: Option<&computed::Context>,
        origin_color: Option<&AbsoluteColor>,
    ) -> Self {
        self.map_leaves(|leaf| leaf.to_computed_value(context, origin_color))
    }

    /// Tries to simplify this expression into a `<length>` or `<percentage>`
    /// value.
    pub fn into_length_or_percentage(
        mut self,
        clamping_mode: AllowedNumericType,
    ) -> Result<CalcLengthPercentage, ()> {
        self.simplify_and_sort();

        // Although we allow numbers inside CalcNumeric, calculations that resolve to a
        // number result is still not allowed.
        let unit = self.unit()?;
        if !CalcUnits::LENGTH_PERCENTAGE.intersects(unit) {
            Err(())
        } else {
            Ok(CalcLengthPercentage(CalcNumeric {
                clamping_mode,
                node: self,
            }))
        }
    }

    /// Tries to simplify this expression into a `<time>` value.
    fn into_time(mut self, clamping_mode: AllowedNumericType) -> Result<CalcNumeric, ()> {
        self.simplify_and_sort();

        let unit: CalcUnits = self.unit()?;
        if !CalcUnits::TIME.intersects(unit) {
            Err(())
        } else {
            Ok(CalcNumeric {
                clamping_mode,
                node: self,
            })
        }
    }

    /// Tries to simplify this expression into a `<resolution>` value.
    fn into_resolution(mut self) -> Result<CalcNumeric, ()> {
        self.simplify_and_sort();

        let unit: CalcUnits = self.unit()?;
        if !CalcUnits::RESOLUTION.intersects(unit) {
            Err(())
        } else {
            Ok(CalcNumeric {
                clamping_mode: AllowedNumericType::NonNegative,
                node: self,
            })
        }
    }

    /// Tries to simplify this expression into a `CalcNumeric` value.
    fn into_angle(mut self, clamping_mode: AllowedNumericType) -> Result<CalcNumeric, ()> {
        self.simplify_and_sort();

        let unit: CalcUnits = self.unit()?;
        if !CalcUnits::ANGLE.intersects(unit) {
            Err(())
        } else {
            Ok(CalcNumeric {
                clamping_mode,
                node: self,
            })
        }
    }

    /// Tries to convert this expression into a `CalcNumeric`, keeping the
    /// AST for later evaluation at computed-value time.
    fn into_number(mut self, clamping_mode: AllowedNumericType) -> Result<CalcNumeric, ()> {
        self.simplify_and_sort();

        let unit: CalcUnits = self.unit()?;
        if !unit.is_empty() {
            Err(())
        } else {
            Ok(CalcNumeric {
                clamping_mode,
                node: self,
            })
        }
    }

    /// Tries to convert this expression into a `CalcNumeric`, keeping the
    /// AST for later evaluation at computed-value time.
    fn into_percentage(mut self, clamping_mode: AllowedNumericType) -> Result<CalcNumeric, ()> {
        self.simplify_and_sort();

        let unit: CalcUnits = self.unit()?;
        if !CalcUnits::PERCENTAGE.intersects(unit) {
            Err(())
        } else {
            Ok(CalcNumeric {
                clamping_mode,
                node: self,
            })
        }
    }

    /// Resolves a number expression against computed length context.
    pub fn to_number_with_context(&self, context: &computed::Context) -> Result<CSSFloat, ()> {
        use crate::values::computed::calc::ComputedLeaf;

        let mut unsupported = false;
        let node = self.map_leaves(|leaf| match leaf {
            Leaf::Number(number) => ComputedLeaf::Number(number.value()),
            Leaf::Percentage(percentage) => {
                ComputedLeaf::Percentage(crate::values::computed::Percentage(percentage.get()))
            },
            Leaf::Length(length) => ComputedLeaf::Length(length.to_computed_value(context)),
            Leaf::Angle(..)
            | Leaf::Time(..)
            | Leaf::Resolution(..)
            | Leaf::ColorComponent(..)
            | Leaf::TreeCountingFunction(..) => {
                unsupported = true;
                ComputedLeaf::Number(0.0)
            },
        });
        if unsupported {
            return Err(());
        }
        match node.resolve()? {
            ComputedLeaf::Number(number) => Ok(number),
            _ => Err(()),
        }
    }

    /// Given a function name, and the location from where the token came from,
    /// return a mathematical function corresponding to that name or an error.
    #[inline]
    pub fn math_function<'i>(
        _: &ParserContext,
        name: &CowRcStr<'i>,
        location: cssparser::SourceLocation,
    ) -> Result<MathFunction, ParseError<'i>> {
        let function = match MathFunction::from_ident(&*name) {
            Ok(f) => f,
            Err(()) => {
                return Err(location.new_unexpected_token_error(Token::Function(name.clone())))
            },
        };

        Ok(function)
    }

    /// Convenience parsing function for `<length> | <percentage>`, and, optionally, `anchor()`.
    pub fn parse_length_or_percentage<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        clamping_mode: AllowedNumericType,
        function: MathFunction,
        allow_anchor: AllowAnchorPositioningFunctions,
    ) -> Result<CalcLengthPercentage, ParseError<'i>> {
        let allowed = if allow_anchor == AllowAnchorPositioningFunctions::No {
            CalcParseFlags::new(CalcUnits::LENGTH_PERCENTAGE)
        } else {
            CalcParseFlags {
                units: CalcUnits::LENGTH_PERCENTAGE,
                color_components: ChannelKeyword::empty(),
                additional_functions: match allow_anchor {
                    AllowAnchorPositioningFunctions::No => unreachable!(),
                    AllowAnchorPositioningFunctions::AllowAnchorSize => {
                        AdditionalFunctions::ANCHOR_SIZE
                    },
                    AllowAnchorPositioningFunctions::AllowAnchorAndAnchorSize => {
                        AdditionalFunctions::ANCHOR | AdditionalFunctions::ANCHOR_SIZE
                    },
                },
                in_place_operations: CalcNodeParseInPlaceOperations::Yes,
            }
        };
        Self::parse(context, input, function, allowed)?
            .into_length_or_percentage(clamping_mode)
            .map_err(|()| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))
    }

    /// Convenience parsing function for percentages.
    pub fn parse_percentage<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        clamping_mode: AllowedNumericType,
        function: MathFunction,
    ) -> Result<CalcNumeric, ParseError<'i>> {
        Self::parse(
            context,
            input,
            function,
            CalcParseFlags::new(CalcUnits::PERCENTAGE),
        )?
        .into_percentage(clamping_mode)
        .map_err(|()| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))
    }

    /// Convenience parsing function for `<length>`.
    pub fn parse_length<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        clamping_mode: AllowedNumericType,
        function: MathFunction,
    ) -> Result<CalcLengthPercentage, ParseError<'i>> {
        Self::parse(
            context,
            input,
            function,
            CalcParseFlags::new(CalcUnits::LENGTH),
        )?
        .into_length_or_percentage(clamping_mode)
        .map_err(|()| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))
    }

    /// Convenience parsing function for `<number>`.
    pub fn parse_number_node<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        function: MathFunction,
    ) -> Result<Self, ParseError<'i>> {
        let node = Self::parse(
            context,
            input,
            function,
            CalcParseFlags::new(CalcUnits::ALL),
        )?;
        if node
            .unit()
            .map_err(|()| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))?
            != CalcUnits::empty()
        {
            return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }
        Ok(node)
    }

    /// Convenience parsing function for `<number>`.
    pub fn parse_number<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        clamping_mode: AllowedNumericType,
        function: MathFunction,
    ) -> Result<CalcNumeric, ParseError<'i>> {
        Self::parse(
            context,
            input,
            function,
            CalcParseFlags::new(CalcUnits::empty()),
        )?
        .into_number(clamping_mode)
        .map_err(|()| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))
    }

    /// Convenience parsing function for `<angle>`.
    pub fn parse_angle<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        function: MathFunction,
    ) -> Result<CalcNumeric, ParseError<'i>> {
        Self::parse(
            context,
            input,
            function,
            CalcParseFlags::new(CalcUnits::ANGLE),
        )?
        .into_angle(AllowedNumericType::All)
        .map_err(|()| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))
    }

    /// Convenience parsing function for `<time>`.
    pub fn parse_time<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        clamping_mode: AllowedNumericType,
        function: MathFunction,
    ) -> Result<CalcNumeric, ParseError<'i>> {
        Self::parse(
            context,
            input,
            function,
            CalcParseFlags::new(CalcUnits::TIME),
        )?
        .into_time(clamping_mode)
        .map_err(|()| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))
    }

    /// Convenience parsing function for `<resolution>`.
    pub fn parse_resolution<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        function: MathFunction,
    ) -> Result<CalcNumeric, ParseError<'i>> {
        Self::parse(
            context,
            input,
            function,
            CalcParseFlags::new(CalcUnits::RESOLUTION),
        )?
        .into_resolution()
        .map_err(|()| input.new_custom_error(StyleParseErrorKind::UnspecifiedError))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::QuirksMode;
    use crate::custom_properties::AttrTaint;
    use crate::stylesheets::{CssRuleType, Origin, UrlExtraData};
    use crate::values::specified::LengthPercentage;
    use cssparser::{Parser, ParserInput};
    use style_traits::ParsingMode;

    fn length_percentage_parses(value: &str) -> bool {
        let url_data = UrlExtraData::from(url::Url::parse("about:blank").unwrap());
        let context = ParserContext::new(
            Origin::Author,
            &url_data,
            Some(CssRuleType::Style),
            ParsingMode::DEFAULT,
            QuirksMode::NoQuirks,
            /* namespaces = */ Default::default(),
            None,
            None,
            AttrTaint::default(),
        );
        let mut input = ParserInput::new(value);
        let mut parser = Parser::new(&mut input);
        parser
            .parse_entirely(|input| LengthPercentage::parse(&context, input))
            .is_ok()
    }

    #[test]
    fn web_exposed_calc_division_requires_number_denominator() {
        assert!(length_percentage_parses("calc(5px / 2)"));
        assert!(length_percentage_parses("calc((10% + 1em) * 0.5)"));
        assert!(!length_percentage_parses("calc(5px / 1px)"));
        assert!(!length_percentage_parses("calc(5px * 10lh / 1px)"));
        assert!(!length_percentage_parses("calc(1em / 1rem * 1px)"));
    }
}
