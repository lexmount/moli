/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified types for CSS values related to borders.

use crate::derives::*;
use crate::parser::{Parse, ParserContext};
use crate::typed_om::{ToTyped, TypedValue};
use crate::values::computed::border::BorderSideWidth as ComputedBorderSideWidth;
use crate::values::computed::{Context, ToComputedValue};
use crate::values::generics::border::{
    GenericBorderCornerRadius, GenericBorderImageSideWidth, GenericBorderImageSlice,
    GenericBorderRadius, GenericBorderSpacing,
};
use crate::values::generics::rect::Rect;
use crate::values::generics::size::Size2D;
use crate::values::specified::length::{Length, NonNegativeLength, NonNegativeLengthPercentage};
use crate::values::specified::{AllowQuirks, NonNegativeNumber, NonNegativeNumberOrPercentage};
use crate::Zero;
use app_units::Au;
use cssparser::Parser;
use std::fmt::{self, Write};
use style_traits::{CssWriter, ParseError, ToCss};
use thin_vec::ThinVec;

/// A specified value for a single side of a `border-style` property.
///
/// The order here corresponds to the integer values from the border conflict
/// resolution rules in CSS 2.1 § 17.6.2.1. Higher values override lower values.
#[allow(missing_docs)]
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    FromPrimitive,
    MallocSizeOf,
    Ord,
    Parse,
    PartialEq,
    PartialOrd,
    Serialize,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(u8)]
pub enum BorderStyle {
    Hidden,
    None,
    Inset,
    Groove,
    Outset,
    Ridge,
    Dotted,
    Dashed,
    Solid,
    Double,
}

impl BorderStyle {
    /// Whether this border style is either none or hidden.
    #[inline]
    pub fn none_or_hidden(&self) -> bool {
        matches!(*self, BorderStyle::None | BorderStyle::Hidden)
    }
}

/// A specified value for the `border-image-width` property.
pub type BorderImageWidth = Rect<BorderImageSideWidth>;

impl ToTyped for BorderImageWidth {
    // Note: The specification does not currently define how border image width
    // should be reified into Typed OM. The current behavior follows existing
    // WPT coverage (border-image-width.html). Syncing spec with UA/WPT
    // behavior tracked in https://github.com/w3c/csswg-drafts/issues/13907
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        if !self.all_sides_equal() {
            return Err(());
        }

        self.0.to_typed(dest)
    }
}

/// A specified value for a single side of a `border-image-width` property.
pub type BorderImageSideWidth =
    GenericBorderImageSideWidth<NonNegativeLengthPercentage, NonNegativeNumber>;

/// A specified value for the `border-image-slice` property.
pub type BorderImageSlice = GenericBorderImageSlice<NonNegativeNumberOrPercentage>;

impl ToTyped for BorderImageSlice {
    // Note: The specification does not currently define how border image slice
    // should be reified into Typed OM. The current behavior follows existing
    // WPT coverage (border-image-slice.html). Syncing spec with UA/WPT
    // behavior tracked in https://github.com/w3c/csswg-drafts/issues/13907
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        if self.fill {
            return Err(());
        }

        let offsets = &self.offsets;

        if !offsets.all_sides_equal() {
            return Err(());
        }

        offsets.0.to_typed(dest)
    }
}

/// A specified value for the `border-radius` property.
pub type BorderRadius = GenericBorderRadius<NonNegativeLengthPercentage>;

/// A specified value for the `border-*-radius` longhand properties.
pub type BorderCornerRadius = GenericBorderCornerRadius<NonNegativeLengthPercentage>;

/// A specified value for the `border-spacing` longhand properties.
pub type BorderSpacing = GenericBorderSpacing<NonNegativeLength>;

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

/// https://drafts.csswg.org/css-backgrounds-3/#typedef-line-width
#[derive(Clone, Debug, MallocSizeOf, PartialEq, SpecifiedValueInfo, ToCss, ToShmem, ToTyped)]
pub enum LineWidth {
    /// `thin`
    Thin,
    /// `medium`
    Medium,
    /// `thick`
    Thick,
    /// `<length>`
    Length(NonNegativeLength),
}

impl LineWidth {
    /// Returns the `0px` value.
    #[inline]
    pub fn zero() -> Self {
        Self::Length(NonNegativeLength::zero())
    }

    fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        if let Ok(length) =
            input.try_parse(|i| NonNegativeLength::parse_quirky(context, i, allow_quirks))
        {
            return Ok(Self::Length(length));
        }
        Ok(try_match_ident_ignore_ascii_case! { input,
            "thin" => Self::Thin,
            "medium" => Self::Medium,
            "thick" => Self::Thick,
        })
    }
}

impl Parse for LineWidth {
    fn parse<'i>(
        context: &ParserContext,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_quirky(context, input, AllowQuirks::No)
    }
}

impl ToComputedValue for LineWidth {
    type ComputedValue = Au;

    #[inline]
    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        match *self {
            // https://drafts.csswg.org/css-backgrounds-3/#line-width
            Self::Thin => Au::from_px(1),
            Self::Medium => Au::from_px(3),
            Self::Thick => Au::from_px(5),
            Self::Length(ref length) => Au::from_f32_px(length.to_computed_value(context).px()),
        }
    }

    #[inline]
    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self::Length(NonNegativeLength::from_px(computed.to_f32_px()))
    }
}

/// A specified value for a single side of the `border-width` property. The difference between this
/// and LineWidth is whether we snap to device pixels or not.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, SpecifiedValueInfo, ToCss, ToShmem, ToTyped)]
pub struct BorderSideWidth(LineWidth);

impl BorderSideWidth {
    /// Returns the `medium` value.
    pub fn medium() -> Self {
        Self(LineWidth::Medium)
    }

    /// Returns a bare px value from the argument.
    pub fn from_px(px: f32) -> Self {
        Self(LineWidth::Length(Length::from_px(px).into()))
    }

    /// Parses, with quirks.
    pub fn parse_quirky<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        allow_quirks: AllowQuirks,
    ) -> Result<Self, ParseError<'i>> {
        Ok(Self(LineWidth::parse_quirky(context, input, allow_quirks)?))
    }
}

impl Parse for BorderSideWidth {
    fn parse<'i>(
        context: &ParserContext,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_quirky(context, input, AllowQuirks::No)
    }
}

// https://drafts.csswg.org/css-values-4/#snap-a-length-as-a-border-width
fn snap_as_border_width(len: Au, context: &Context) -> Au {
    debug_assert!(len >= Au(0));

    // Round `width` down to the nearest device pixel, but any non-zero value that would round
    // down to zero is clamped to 1 device pixel.
    if len == Au(0) {
        return len;
    }

    let au_per_dev_px = context.device().app_units_per_device_pixel();
    std::cmp::max(Au(au_per_dev_px), Au(len.0 / au_per_dev_px * au_per_dev_px))
}

impl ToComputedValue for BorderSideWidth {
    type ComputedValue = ComputedBorderSideWidth;

    #[inline]
    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        ComputedBorderSideWidth(snap_as_border_width(
            self.0.to_computed_value(context),
            context,
        ))
    }

    #[inline]
    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self(LineWidth::from_computed_value(&computed.0))
    }
}

/// A specified value for outline-offset.
#[derive(
    Clone, Debug, MallocSizeOf, PartialEq, Parse, SpecifiedValueInfo, ToCss, ToShmem, ToTyped,
)]
pub struct BorderSideOffset(Length);

impl ToComputedValue for BorderSideOffset {
    type ComputedValue = Au;

    #[inline]
    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        let offset = Au::from_f32_px(self.0.to_computed_value(context).px());
        let should_snap = match static_prefs::pref!("layout.css.outline-offset.snapping") {
            1 => true,
            2 => context.device().chrome_rules_enabled_for_document(),
            _ => false,
        };
        if !should_snap {
            return offset;
        }
        if offset < Au(0) {
            -snap_as_border_width(-offset, context)
        } else {
            snap_as_border_width(offset, context)
        }
    }

    #[inline]
    fn from_computed_value(computed: &Au) -> Self {
        Self(Length::from_px(computed.to_f32_px()))
    }
}

impl BorderImageSideWidth {
    /// Returns `1`.
    #[inline]
    pub fn one() -> Self {
        GenericBorderImageSideWidth::Number(NonNegativeNumber::new(1.))
    }
}

impl Parse for BorderImageSlice {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let mut fill = input.try_parse(|i| i.expect_ident_matching("fill")).is_ok();
        let offsets = Rect::parse_with(context, input, NonNegativeNumberOrPercentage::parse)?;
        if !fill {
            fill = input.try_parse(|i| i.expect_ident_matching("fill")).is_ok();
        }
        Ok(GenericBorderImageSlice { offsets, fill })
    }
}

impl Parse for BorderRadius {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let widths = Rect::parse_with(context, input, NonNegativeLengthPercentage::parse)?;
        let heights = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
            Rect::parse_with(context, input, NonNegativeLengthPercentage::parse)?
        } else {
            widths.clone()
        };

        Ok(GenericBorderRadius {
            top_left: BorderCornerRadius::new(widths.0, heights.0),
            top_right: BorderCornerRadius::new(widths.1, heights.1),
            bottom_right: BorderCornerRadius::new(widths.2, heights.2),
            bottom_left: BorderCornerRadius::new(widths.3, heights.3),
        })
    }
}

impl Parse for BorderCornerRadius {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Size2D::parse_with(context, input, NonNegativeLengthPercentage::parse)
            .map(GenericBorderCornerRadius)
    }
}

impl Parse for BorderSpacing {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Size2D::parse_with(context, input, |context, input| {
            NonNegativeLength::parse_quirky(context, input, AllowQuirks::Yes)
        })
        .map(GenericBorderSpacing)
    }
}

/// A single border-image-repeat keyword.
#[allow(missing_docs)]
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    MallocSizeOf,
    Parse,
    PartialEq,
    Serialize,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
)]
#[repr(u8)]
pub enum BorderImageRepeatKeyword {
    Stretch,
    Repeat,
    Round,
    Space,
}

/// The specified value for the `border-image-repeat` property.
///
/// https://drafts.csswg.org/css-backgrounds/#the-border-image-repeat
#[derive(
    Clone,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C)]
#[typed(todo_derive_fields)]
pub struct BorderImageRepeat(pub BorderImageRepeatKeyword, pub BorderImageRepeatKeyword);

impl ToCss for BorderImageRepeat {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        self.0.to_css(dest)?;
        if self.0 != self.1 {
            dest.write_char(' ')?;
            self.1.to_css(dest)?;
        }
        Ok(())
    }
}

impl BorderImageRepeat {
    /// Returns the `stretch` value.
    #[inline]
    pub fn stretch() -> Self {
        BorderImageRepeat(
            BorderImageRepeatKeyword::Stretch,
            BorderImageRepeatKeyword::Stretch,
        )
    }
}

impl Parse for BorderImageRepeat {
    fn parse<'i, 't>(
        _context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let horizontal = BorderImageRepeatKeyword::parse(input)?;
        let vertical = input.try_parse(BorderImageRepeatKeyword::parse).ok();
        Ok(BorderImageRepeat(
            horizontal,
            vertical.unwrap_or(horizontal),
        ))
    }
}
