/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Animated types for CSS colors.

use style_traits::owned_slice::OwnedSlice;

use crate::color::mix::ColorInterpolationMethod;
use crate::color::AbsoluteColor;
use crate::values::animated::{Animate, Procedure, ToAnimatedZero};
use crate::values::computed::Percentage;
use crate::values::distance::{ComputeSquaredDistance, SquaredDistance};
use crate::values::generics::color::{
    ColorMixFlags, GenericColor, GenericColorMix, GenericColorMixItem,
};

impl Animate for AbsoluteColor {
    #[inline]
    fn animate(&self, other: &Self, procedure: Procedure) -> Result<Self, ()> {
        use crate::color::mix;

        let (left_weight, right_weight) = procedure.weights();

        Ok(mix::mix_many(
            ColorInterpolationMethod::best_interpolation_between(self, other),
            [
                mix::ColorMixItem::new(*self, left_weight as f32),
                mix::ColorMixItem::new(*other, right_weight as f32),
            ],
            ColorMixFlags::empty(),
        ))
    }
}

impl ComputeSquaredDistance for AbsoluteColor {
    #[inline]
    fn compute_squared_distance(&self, other: &Self) -> Result<SquaredDistance, ()> {
        let start = [
            self.alpha,
            self.components.0 * self.alpha,
            self.components.1 * self.alpha,
            self.components.2 * self.alpha,
        ];
        let end = [
            other.alpha,
            other.components.0 * other.alpha,
            other.components.1 * other.alpha,
            other.components.2 * other.alpha,
        ];
        start
            .iter()
            .zip(&end)
            .map(|(this, other)| this.compute_squared_distance(other))
            .sum()
    }
}

/// An animated value for `<color>`.
pub type Color = GenericColor<Percentage>;

/// An animated value for `<color-mix>`.
pub type ColorMix = GenericColorMix<Color, Percentage>;

impl Animate for Color {
    #[inline]
    fn animate(&self, other: &Self, procedure: Procedure) -> Result<Self, ()> {
        let (left_weight, right_weight) = procedure.weights();

        Ok(Self::from_color_mix(ColorMix {
            interpolation: ColorInterpolationMethod::srgb(),
            items: OwnedSlice::from_slice(&[
                GenericColorMixItem {
                    color: self.clone(),
                    percentage: Percentage(left_weight as f32),
                },
                GenericColorMixItem {
                    color: other.clone(),
                    percentage: Percentage(right_weight as f32),
                },
            ]),
            // See https://github.com/w3c/csswg-drafts/issues/7324
            flags: ColorMixFlags::empty(),
        }))
    }
}

impl ComputeSquaredDistance for Color {
    #[inline]
    fn compute_squared_distance(&self, other: &Self) -> Result<SquaredDistance, ()> {
        let current_color = AbsoluteColor::TRANSPARENT_BLACK;
        self.resolve_to_absolute(&current_color)
            .compute_squared_distance(&other.resolve_to_absolute(&current_color))
    }
}

impl ToAnimatedZero for Color {
    #[inline]
    fn to_animated_zero(&self) -> Result<Self, ()> {
        Ok(Color::Absolute(AbsoluteColor::TRANSPARENT_BLACK))
    }
}
