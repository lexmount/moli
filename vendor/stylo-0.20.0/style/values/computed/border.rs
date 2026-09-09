/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Computed types for CSS values related to borders.

use crate::derives::*;
use crate::properties::{LogicalGroupId, LonghandId};
use crate::typed_om::{ToTyped, TypedValue};
use crate::values::animated::{Context as AnimatedContext, ToAnimatedValue};
use crate::values::computed::length::{
    CSSPixelLength, NonNegativeLength, NonNegativeLengthPercentage,
};
use crate::values::computed::{NonNegativeNumber, NonNegativeNumberOrPercentage};
use crate::values::generics::border::{
    GenericBorderCornerRadius, GenericBorderImageSideWidth, GenericBorderImageSlice,
    GenericBorderRadius, GenericBorderSpacing,
};
use crate::values::generics::rect::Rect;
use crate::values::generics::size::Size2D;
use crate::values::generics::NonNegative;
use crate::values::resolved::{Context as ResolvedContext, ToResolvedValue};
use crate::Zero;
use app_units::Au;
use thin_vec::ThinVec;

pub use crate::values::specified::border::BorderImageRepeat;

/// A computed value for -webkit-text-stroke-width.
pub type LineWidth = Au;

/// A computed value for border-width (and the like).
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToCss, ToTyped, From)]
#[repr(transparent)]
pub struct BorderSideWidth(pub Au);

impl BorderSideWidth {
    /// The `medium` value.
    pub fn medium() -> Self {
        Self(Au::from_px(3))
    }
}

impl ToAnimatedValue for BorderSideWidth {
    type AnimatedValue = CSSPixelLength;

    #[inline]
    fn to_animated_value(self, context: &AnimatedContext) -> Self::AnimatedValue {
        self.0.to_animated_value(context)
    }

    #[inline]
    fn from_animated_value(animated: Self::AnimatedValue) -> Self {
        Self(Au::from_animated_value(animated))
    }
}

impl ToResolvedValue for BorderSideWidth {
    type ResolvedValue = CSSPixelLength;

    fn to_resolved_value(self, context: &ResolvedContext) -> Self::ResolvedValue {
        let resolved_length = CSSPixelLength::from(self.0).to_resolved_value(context);
        if !context
            .current_longhand
            .is_some_and(|l| l.logical_group() == Some(LogicalGroupId::BorderWidth))
        {
            return resolved_length;
        }
        // Only for border widths, a style of none/hidden causes the resolved value to be zero.
        let style = match context.current_longhand.unwrap() {
            LonghandId::BorderTopWidth => context.style.clone_border_top_style(),
            LonghandId::BorderRightWidth => context.style.clone_border_right_style(),
            LonghandId::BorderBottomWidth => context.style.clone_border_bottom_style(),
            LonghandId::BorderLeftWidth => context.style.clone_border_left_style(),
            _ => {
                debug_assert!(false, "Expected a physical longhand");
                return resolved_length;
            },
        };
        if style.none_or_hidden() {
            return CSSPixelLength::new(0.0);
        }
        resolved_length
    }

    #[inline]
    fn from_resolved_value(value: Self::ResolvedValue) -> Self {
        Self(Au::from_f32_px(value.px()))
    }
}

/// A computed value for outline-offset
pub type BorderSideOffset = Au;

/// A computed value for the `border-image-width` property.
pub type BorderImageWidth = Rect<BorderImageSideWidth>;

impl ToTyped for BorderImageWidth {
    fn to_typed(&self, _dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        return Err(());
    }
}

/// A computed value for a single side of a `border-image-width` property.
pub type BorderImageSideWidth =
    GenericBorderImageSideWidth<NonNegativeLengthPercentage, NonNegativeNumber>;

/// A computed value for the `border-image-slice` property.
pub type BorderImageSlice = GenericBorderImageSlice<NonNegativeNumberOrPercentage>;

impl ToTyped for BorderImageSlice {
    fn to_typed(&self, _dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        return Err(());
    }
}

/// A computed value for the `border-radius` property.
pub type BorderRadius = GenericBorderRadius<NonNegativeLengthPercentage>;

/// A computed value for the `border-*-radius` longhand properties.
pub type BorderCornerRadius = GenericBorderCornerRadius<NonNegativeLengthPercentage>;

/// A computed value for the `border-spacing` longhand property.
pub type BorderSpacing = GenericBorderSpacing<NonNegativeLength>;

impl BorderImageSideWidth {
    /// Returns `1`.
    #[inline]
    pub fn one() -> Self {
        GenericBorderImageSideWidth::Number(NonNegative(1.))
    }
}

impl BorderImageSlice {
    /// Returns the `100%` value.
    #[inline]
    pub fn hundred_percent() -> Self {
        GenericBorderImageSlice {
            offsets: Rect::all(NonNegativeNumberOrPercentage::hundred_percent()),
            fill: false,
        }
    }
}

impl BorderSpacing {
    /// Returns `0 0`.
    pub fn zero() -> Self {
        GenericBorderSpacing(Size2D::new(
            NonNegativeLength::zero(),
            NonNegativeLength::zero(),
        ))
    }

    /// Returns the horizontal spacing.
    pub fn horizontal(&self) -> Au {
        Au::from(*self.0.width())
    }

    /// Returns the vertical spacing.
    pub fn vertical(&self) -> Au {
        Au::from(*self.0.height())
    }
}
