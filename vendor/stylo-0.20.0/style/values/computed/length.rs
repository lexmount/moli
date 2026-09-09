/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! `<length>` computed values, and related ones.

use super::{Number, ToComputedValue};
use crate::derives::*;
use crate::logical_geometry::PhysicalSide;
use crate::typed_om::{NumericType, NumericValue, ToTyped, TypedValue, UnitValue};
use crate::values::animated::{Context as AnimatedContext, ToAnimatedValue};
use crate::values::computed::position::TryTacticAdjustment;
use crate::values::computed::{NonNegativeNumber, Percentage, Zoom};
use crate::values::generics::length::{
    GenericLengthOrNumber, GenericLengthPercentageOrNormal, GenericMaxSize, GenericSize,
};
#[cfg(feature = "gecko")]
use crate::values::generics::position::TreeScoped;
use crate::values::generics::NonNegative;
use crate::values::generics::{length as generics, ClampToNonNegative};
use crate::values::resolved::{Context as ResolvedContext, ToResolvedValue};
use crate::values::CSSFloat;
#[cfg(feature = "gecko")]
use crate::values::DashedIdent;
use crate::Zero;
use app_units::Au;
use std::fmt::{self, Write};
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};
use style_traits::{CSSPixel, CssString, CssWriter, ToCss};
use thin_vec::ThinVec;

pub use super::image::Image;
pub use super::length_percentage::{LengthPercentage, NonNegativeLengthPercentage};
pub use crate::values::specified::url::UrlOrNone;
pub use crate::values::specified::{Angle, BorderStyle, Time};

/// Some boilerplate to share between negative and non-negative
/// length-percentage or auto.
macro_rules! computed_length_percentage_or_auto {
    ($inner:ty) => {
        /// Returns the used value.
        #[inline]
        pub fn to_used_value(&self, percentage_basis: Au) -> Option<Au> {
            match *self {
                Self::Auto => None,
                Self::LengthPercentage(ref lp) => Some(lp.to_used_value(percentage_basis)),
            }
        }
    };
}

/// A computed type for `<length-percentage> | auto`.
pub type LengthPercentageOrAuto = generics::GenericLengthPercentageOrAuto<LengthPercentage>;

impl LengthPercentageOrAuto {
    /// Clamps the value to a non-negative value.
    pub fn clamp_to_non_negative(self) -> Self {
        use crate::values::generics::length::LengthPercentageOrAuto::*;
        match self {
            LengthPercentage(l) => LengthPercentage(l.clamp_to_non_negative()),
            Auto => Auto,
        }
    }

    /// Convert to have a borrow inside the enum
    pub fn as_ref(&self) -> generics::GenericLengthPercentageOrAuto<&LengthPercentage> {
        use crate::values::generics::length::LengthPercentageOrAuto::*;
        match *self {
            LengthPercentage(ref lp) => LengthPercentage(lp),
            Auto => Auto,
        }
    }

    computed_length_percentage_or_auto!(LengthPercentage);
}

impl generics::GenericLengthPercentageOrAuto<&LengthPercentage> {
    /// Resolves the percentage.
    #[inline]
    pub fn percentage_relative_to(&self, basis: Length) -> LengthOrAuto {
        use crate::values::generics::length::LengthPercentageOrAuto::*;
        match self {
            LengthPercentage(length_percentage) => {
                LengthPercentage(length_percentage.percentage_relative_to(basis))
            },
            Auto => Auto,
        }
    }

    /// Maybe resolves the percentage.
    #[inline]
    pub fn maybe_percentage_relative_to(&self, basis: Option<Length>) -> LengthOrAuto {
        use crate::values::generics::length::LengthPercentageOrAuto::*;
        match self {
            LengthPercentage(length_percentage) => length_percentage
                .maybe_percentage_relative_to(basis)
                .map_or(Auto, LengthPercentage),
            Auto => Auto,
        }
    }
}

/// A wrapper of LengthPercentageOrAuto, whose value must be >= 0.
pub type NonNegativeLengthPercentageOrAuto =
    generics::GenericLengthPercentageOrAuto<NonNegativeLengthPercentage>;

impl NonNegativeLengthPercentageOrAuto {
    computed_length_percentage_or_auto!(NonNegativeLengthPercentage);
}

/// The computed `<length>` value.
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Deserialize,
    MallocSizeOf,
    PartialEq,
    PartialOrd,
    Serialize,
    ToAnimatedZero,
    ToComputedValue,
    ToShmem,
)]
#[repr(C)]
pub struct CSSPixelLength(CSSFloat);

impl ToResolvedValue for CSSPixelLength {
    type ResolvedValue = Self;

    fn to_resolved_value(self, context: &ResolvedContext) -> Self::ResolvedValue {
        Self(context.style.effective_zoom.unzoom(self.0))
    }

    #[inline]
    fn from_resolved_value(value: Self::ResolvedValue) -> Self {
        value
    }
}

impl ToAnimatedValue for CSSPixelLength {
    type AnimatedValue = Self;

    fn to_animated_value(self, context: &AnimatedContext) -> Self::AnimatedValue {
        Self(context.style.effective_zoom.unzoom(self.0))
    }

    #[inline]
    fn from_animated_value(value: Self::AnimatedValue) -> Self {
        value
    }
}

impl fmt::Debug for CSSPixelLength {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)?;
        f.write_str(" px")
    }
}

impl CSSPixelLength {
    /// Return a new CSSPixelLength.
    #[inline]
    pub fn new(px: CSSFloat) -> Self {
        CSSPixelLength(px)
    }

    /// Returns a normalized (NaN turned to zero) version of this length.
    #[inline]
    pub fn normalized(self) -> Self {
        Self::new(crate::values::normalize(self.0))
    }

    /// Returns a finite (normalized and clamped to float min and max) version of this length.
    #[inline]
    pub fn finite(self) -> Self {
        Self::new(crate::values::normalize(self.0).min(f32::MAX).max(f32::MIN))
    }

    /// Scale the length by a given amount.
    #[inline]
    pub fn scale_by(self, scale: CSSFloat) -> Self {
        CSSPixelLength(self.0 * scale)
    }

    /// Return the containing pixel value.
    #[inline]
    pub fn px(self) -> CSSFloat {
        self.0
    }

    /// Zooms a particular length.
    #[inline]
    pub fn zoom(self, zoom: Zoom) -> Self {
        Self::new(zoom.zoom(self.px()))
    }

    /// Return the length with app_unit i32 type.
    #[inline]
    pub fn to_i32_au(self) -> i32 {
        Au::from(self).0
    }

    /// Return the absolute value of this length.
    #[inline]
    pub fn abs(self) -> Self {
        CSSPixelLength::new(self.0.abs())
    }

    /// Return the clamped value of this length.
    #[inline]
    pub fn clamp_to_non_negative(self) -> Self {
        CSSPixelLength::new(self.0.max(0.))
    }

    /// Returns the minimum between `self` and `other`.
    #[inline]
    pub fn min(self, other: Self) -> Self {
        CSSPixelLength::new(self.0.min(other.0))
    }

    /// Returns the maximum between `self` and `other`.
    #[inline]
    pub fn max(self, other: Self) -> Self {
        CSSPixelLength::new(self.0.max(other.0))
    }

    /// Sets `self` to the maximum between `self` and `other`.
    #[inline]
    pub fn max_assign(&mut self, other: Self) {
        *self = self.max(other);
    }

    /// Clamp the value to a lower bound and an optional upper bound.
    ///
    /// Can be used for example with `min-width` and `max-width`.
    #[inline]
    pub fn clamp_between_extremums(self, min_size: Self, max_size: Option<Self>) -> Self {
        self.clamp_below_max(max_size).max(min_size)
    }

    /// Clamp the value to an optional upper bound.
    ///
    /// Can be used for example with `max-width`.
    #[inline]
    pub fn clamp_below_max(self, max_size: Option<Self>) -> Self {
        match max_size {
            None => self,
            Some(max_size) => self.min(max_size),
        }
    }
}

impl num_traits::Zero for CSSPixelLength {
    fn zero() -> Self {
        CSSPixelLength::new(0.)
    }

    fn is_zero(&self) -> bool {
        self.px() == 0.
    }
}

impl ToCss for CSSPixelLength {
    #[inline]
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        self.0.to_css(dest)?;
        dest.write_str("px")
    }
}

impl ToTyped for CSSPixelLength {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        dest.push(TypedValue::Numeric(NumericValue::Unit(UnitValue {
            numeric_type: NumericType::length(),
            value: self.0 as f32,
            unit: CssString::from("px"),
        })));
        Ok(())
    }
}

impl std::iter::Sum for CSSPixelLength {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Length::zero(), Add::add)
    }
}

impl Add for CSSPixelLength {
    type Output = Self;

    #[inline]
    fn add(self, other: Self) -> Self {
        Self::new(self.px() + other.px())
    }
}

impl AddAssign for CSSPixelLength {
    #[inline]
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl Div for CSSPixelLength {
    type Output = CSSFloat;

    #[inline]
    fn div(self, other: Self) -> CSSFloat {
        self.px() / other.px()
    }
}

impl Div<CSSFloat> for CSSPixelLength {
    type Output = Self;

    #[inline]
    fn div(self, other: CSSFloat) -> Self {
        Self::new(self.px() / other)
    }
}

impl MulAssign<CSSFloat> for CSSPixelLength {
    #[inline]
    fn mul_assign(&mut self, other: CSSFloat) {
        self.0 *= other;
    }
}

impl Mul<CSSFloat> for CSSPixelLength {
    type Output = Self;

    #[inline]
    fn mul(self, other: CSSFloat) -> Self {
        Self::new(self.px() * other)
    }
}

impl Neg for CSSPixelLength {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self {
        CSSPixelLength::new(-self.0)
    }
}

impl Sub for CSSPixelLength {
    type Output = Self;

    #[inline]
    fn sub(self, other: Self) -> Self {
        Self::new(self.px() - other.px())
    }
}

impl SubAssign for CSSPixelLength {
    #[inline]
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl From<CSSPixelLength> for Au {
    #[inline]
    fn from(len: CSSPixelLength) -> Self {
        Au::from_f32_px(len.0)
    }
}

impl From<Au> for CSSPixelLength {
    #[inline]
    fn from(len: Au) -> Self {
        CSSPixelLength::new(len.to_f32_px())
    }
}

impl From<CSSPixelLength> for euclid::Length<CSSFloat, CSSPixel> {
    #[inline]
    fn from(length: CSSPixelLength) -> Self {
        Self::new(length.0)
    }
}

/// An alias of computed `<length>` value.
pub type Length = CSSPixelLength;

/// Either a computed `<length>` or the `auto` keyword.
pub type LengthOrAuto = generics::GenericLengthPercentageOrAuto<Length>;

/// Either a non-negative `<length>` or the `auto` keyword.
pub type NonNegativeLengthOrAuto = generics::GenericLengthPercentageOrAuto<NonNegativeLength>;

/// Either a computed `<length>` or a `<number>` value.
pub type LengthOrNumber = GenericLengthOrNumber<Length, Number>;

/// A wrapper of Length, whose value must be >= 0.
pub type NonNegativeLength = NonNegative<Length>;

impl ClampToNonNegative for Length {
    fn clamp_to_non_negative(self) -> Self {
        Self::new(self.px().max(0.))
    }
}

impl NonNegativeLength {
    /// Create a NonNegativeLength.
    #[inline]
    pub fn new(px: CSSFloat) -> Self {
        NonNegative(Length::new(px.max(0.)))
    }

    /// Return the pixel value of |NonNegativeLength|.
    #[inline]
    pub fn px(&self) -> CSSFloat {
        self.0.px()
    }

    #[inline]
    /// Ensures it is non negative
    pub fn clamp(self) -> Self {
        if (self.0).0 < 0. {
            Self::zero()
        } else {
            self
        }
    }
}

impl From<Length> for NonNegativeLength {
    #[inline]
    fn from(len: Length) -> Self {
        NonNegative(len)
    }
}

impl From<Au> for NonNegativeLength {
    #[inline]
    fn from(au: Au) -> Self {
        NonNegative(au.into())
    }
}

impl From<NonNegativeLength> for Au {
    #[inline]
    fn from(non_negative_len: NonNegativeLength) -> Self {
        Au::from(non_negative_len.0)
    }
}

/// Either a computed NonNegativeLengthPercentage or the `normal` keyword.
pub type NonNegativeLengthPercentageOrNormal =
    GenericLengthPercentageOrNormal<NonNegativeLengthPercentage>;

/// Either a non-negative `<length>` or a `<number>`.
pub type NonNegativeLengthOrNumber = GenericLengthOrNumber<NonNegativeLength, NonNegativeNumber>;

/// A computed value for `min-width`, `min-height`, `width` or `height` property.
pub type Size = GenericSize<NonNegativeLengthPercentage>;

/// A computed value for `max-width` or `max-height` property.
pub type MaxSize = GenericMaxSize<NonNegativeLengthPercentage>;

#[cfg(feature = "gecko")]
use crate::{
    gecko_bindings::structs::AnchorPosResolutionParams, logical_geometry::PhysicalAxis,
    values::generics::length::AnchorSizeKeyword,
};

/// Resolve the anchor function with the given resolver. Returns `Err()` if no anchor is found.
/// `prop_axis`, axis of the property (e.g. `margin-left` -> Horizontal axis), is used if the
/// anchor size keyword is not specified.
#[cfg(feature = "gecko")]
pub fn resolve_anchor_size(
    anchor_name: &TreeScoped<DashedIdent>,
    prop_axis: PhysicalAxis,
    anchor_size_keyword: AnchorSizeKeyword,
    params: &AnchorPosResolutionParams,
) -> Result<Length, ()> {
    use crate::gecko_bindings::structs::Gecko_GetAnchorPosSize;

    let mut offset = Length::zero();
    let valid = unsafe {
        Gecko_GetAnchorPosSize(
            params,
            anchor_name.value.0.as_ptr(),
            &anchor_name.scope,
            prop_axis as u8,
            anchor_size_keyword as u8,
            &mut offset,
        )
    };

    if !valid {
        return Err(());
    }

    Ok(offset)
}

/// A computed type for `margin` properties.
pub type Margin = generics::GenericMargin<LengthPercentage>;

impl TryTacticAdjustment for MaxSize {
    fn try_tactic_adjustment(&mut self, old_side: PhysicalSide, new_side: PhysicalSide) {
        debug_assert!(
            old_side.orthogonal_to(new_side),
            "Sizes should only change axes"
        );
        match self {
            Self::FitContentFunction(lp)
            | Self::LengthPercentage(lp)
            | Self::AnchorContainingCalcFunction(lp) => {
                lp.try_tactic_adjustment(old_side, new_side);
            },
            Self::AnchorSizeFunction(s) => s.try_tactic_adjustment(old_side, new_side),
            Self::None
            | Self::MaxContent
            | Self::MinContent
            | Self::FitContent
            | Self::WebkitFillAvailable
            | Self::Stretch => {},
            #[cfg(feature = "gecko")]
            Self::MozAvailable => {},
        }
    }
}

impl TryTacticAdjustment for Size {
    fn try_tactic_adjustment(&mut self, old_side: PhysicalSide, new_side: PhysicalSide) {
        debug_assert!(
            old_side.orthogonal_to(new_side),
            "Sizes should only change axes"
        );
        match self {
            Self::FitContentFunction(lp)
            | Self::LengthPercentage(lp)
            | Self::AnchorContainingCalcFunction(lp) => {
                lp.try_tactic_adjustment(old_side, new_side);
            },
            Self::AnchorSizeFunction(s) => s.try_tactic_adjustment(old_side, new_side),
            Self::Auto
            | Self::MaxContent
            | Self::MinContent
            | Self::FitContent
            | Self::WebkitFillAvailable
            | Self::Stretch => {},
            #[cfg(feature = "gecko")]
            Self::MozAvailable => {},
        }
    }
}

impl TryTacticAdjustment for Percentage {
    fn try_tactic_adjustment(&mut self, old_side: PhysicalSide, new_side: PhysicalSide) {
        if old_side.parallel_to(new_side) {
            self.0 = 1.0 - self.0;
        }
    }
}

impl TryTacticAdjustment for Margin {
    fn try_tactic_adjustment(&mut self, old_side: PhysicalSide, new_side: PhysicalSide) {
        match self {
            Self::Auto => {},
            Self::LengthPercentage(lp) | Self::AnchorContainingCalcFunction(lp) => {
                lp.try_tactic_adjustment(old_side, new_side)
            },
            Self::AnchorSizeFunction(anchor) => anchor.try_tactic_adjustment(old_side, new_side),
        }
    }
}
