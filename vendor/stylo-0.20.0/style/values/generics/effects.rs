/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Generic types for CSS values related to effects.
use crate::derives::*;

use crate::values::generics::{NonNegative, ZeroToOne};

/// A generic value for a single `box-shadow`.
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToAnimatedValue,
    ToAnimatedZero,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C)]
#[typed(todo_derive_fields)]
pub struct GenericBoxShadow<Color, SizeLength, BlurShapeLength, ShapeLength> {
    /// The base shadow.
    pub base: GenericSimpleShadow<Color, SizeLength, BlurShapeLength>,
    /// The spread radius.
    pub spread: ShapeLength,
    /// Whether this is an inset box shadow.
    #[animation(constant)]
    #[css(represents_keyword)]
    pub inset: bool,
}

pub use self::GenericBoxShadow as BoxShadow;

/// A generic value for a single `filter`.
#[derive(
    Clone,
    ComputeSquaredDistance,
    Debug,
    Deserialize,
    MallocSizeOf,
    PartialEq,
    Serialize,
    SpecifiedValueInfo,
    ToAnimatedValue,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[animation(no_bound(U))]
#[repr(C, u8)]
#[typed(todo_derive_fields)]
pub enum GenericFilter<Angle, Factor, Length, Shadow, U> {
    /// `blur(<length>)`
    #[css(function)]
    Blur(#[animation(field_bound)] NonNegative<Length>),
    /// `brightness(<factor>)`
    #[css(function)]
    Brightness(#[animation(field_bound)] NonNegative<Factor>),
    /// `contrast(<factor>)`
    #[css(function)]
    Contrast(#[animation(field_bound)] NonNegative<Factor>),
    /// `grayscale(<factor>)`
    #[css(function)]
    Grayscale(#[animation(field_bound)] ZeroToOne<Factor>),
    /// `hue-rotate(<angle>)`
    #[css(function)]
    HueRotate(Angle),
    /// `invert(<factor>)`
    #[css(function)]
    Invert(#[animation(field_bound)] ZeroToOne<Factor>),
    /// `opacity(<factor>)`
    #[css(function)]
    Opacity(#[animation(field_bound)] ZeroToOne<Factor>),
    /// `saturate(<factor>)`
    #[css(function)]
    Saturate(#[animation(field_bound)] NonNegative<Factor>),
    /// `sepia(<factor>)`
    #[css(function)]
    Sepia(#[animation(field_bound)] ZeroToOne<Factor>),
    /// `drop-shadow(...)`
    #[css(function)]
    DropShadow(Shadow),
    /// `<url>`
    #[animation(error)]
    Url(U),
}

pub use self::GenericFilter as Filter;

/// A generic value for the `drop-shadow()` filter and the `text-shadow` property.
///
/// Contrary to the canonical order from the spec, the color is serialised
/// first, like in Gecko and Webkit.
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToAnimatedValue,
    ToAnimatedZero,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C)]
#[typed(todo_derive_fields)]
pub struct GenericSimpleShadow<Color, SizeLength, ShapeLength> {
    /// Color.
    pub color: Color,
    /// Horizontal radius.
    pub horizontal: SizeLength,
    /// Vertical radius.
    pub vertical: SizeLength,
    /// Blur radius.
    pub blur: ShapeLength,
}

pub use self::GenericSimpleShadow as SimpleShadow;
