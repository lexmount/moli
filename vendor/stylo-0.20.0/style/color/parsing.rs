/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#![deny(missing_docs)]

//! Parsing for CSS colors.

use std::fmt::Write;

use super::{
    color_function::ColorFunction,
    component::{ColorComponent, ColorComponentType},
    AbsoluteColor,
};
use crate::derives::*;
use crate::{
    parser::{Parse, ParserContext},
    values::{
        computed::Color as ComputedColor,
        generics::{calc::CalcUnits, Optional},
        specified::{angle::NoCalcAngle, calc::Leaf, color::Color as SpecifiedColor},
    },
};
use cssparser::{
    color::{parse_hash_color, PredefinedColorSpace, OPAQUE},
    match_ignore_ascii_case, CowRcStr, Parser, Token,
};
use style_traits::{CssWriter, ParseError, StyleParseErrorKind, ToCss};

/// Returns true if the relative color syntax pref is enabled.
#[inline]
pub fn rcs_enabled() -> bool {
    static_prefs::pref!("layout.css.relative-color-syntax.enabled")
}

/// Represents a channel keyword inside a color.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, PartialOrd, ToShmem)]
#[repr(C)]
pub struct ChannelKeyword(u16);
bitflags! {
    impl ChannelKeyword: u16 {
        /// alpha
        const ALPHA = 1 << 0;
        /// a
        const A = 1 << 1;
        /// b, blackness, blue
        const B = 1 << 2;
        /// chroma
        const C = 1 << 3;
        /// green
        const G = 1 << 4;
        /// hue
        const H = 1 << 5;
        /// lightness
        const L = 1 << 6;
        /// red
        const R = 1 << 7;
        /// saturation
        const S = 1 << 8;
        /// whiteness
        const W = 1 << 9;
        /// x
        const X = 1 << 10;
        /// y
        const Y = 1 << 11;
        /// z
        const Z = 1 << 12;
    }
}

impl ChannelKeyword {
    /// Channel keywords allowed in sRGB colors.
    /// https://drafts.csswg.org/css-color-5/#relative-RGB
    pub fn rgb() -> Self {
        Self::R | Self::G | Self::B | Self::ALPHA
    }

    /// Channel keywords allowed in HSL colors.
    /// https://drafts.csswg.org/css-color-5/#relative-HSL
    pub fn hsl() -> Self {
        Self::H | Self::S | Self::L | Self::ALPHA
    }

    /// Channel keywords allowed in HWB colors.
    /// https://drafts.csswg.org/css-color-5/#relative-HWB
    pub fn hwb() -> Self {
        Self::H | Self::W | Self::B | Self::ALPHA
    }

    /// Channel keywords allowed in Lab and Oklab colors.
    /// https://drafts.csswg.org/css-color-5/#relative-Lab
    pub fn lab() -> Self {
        Self::L | Self::A | Self::B | Self::ALPHA
    }

    /// Channel keywords allowed in LCH and OkLCh colors.
    /// https://drafts.csswg.org/css-color-5/#relative-LCH
    pub fn lch() -> Self {
        Self::L | Self::C | Self::H | Self::ALPHA
    }

    /// Channel keywords allowed in XYZ colors.
    /// https://drafts.csswg.org/css-color-5/#relative-color-function
    pub fn xyz() -> Self {
        Self::X | Self::Y | Self::Z | Self::ALPHA
    }

    /// Parse a channel keyword from an ident.
    pub fn from_ident(ident: &str) -> Result<Self, ()> {
        Ok(match_ignore_ascii_case! { ident,
            "alpha" => Self::ALPHA,
            "a" => Self::A,
            "b" => Self::B,
            "c" => Self::C,
            "g" => Self::G,
            "h" => Self::H,
            "l" => Self::L,
            "r" => Self::R,
            "s" => Self::S,
            "w" => Self::W,
            "x" => Self::X,
            "y" => Self::Y,
            "z" => Self::Z,
            _ => return Err(())
        })
    }
}

impl Parse for ChannelKeyword {
    fn parse<'i, 't>(
        _: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();
        let ident = input.expect_ident()?;
        Self::from_ident(ident.as_ref())
            .map_err(|()| location.new_unexpected_token_error(Token::Ident(ident.clone())))
    }
}

impl ToCss for ChannelKeyword {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> std::fmt::Result
    where
        W: std::fmt::Write,
    {
        dest.write_str(match *self {
            Self::ALPHA => "alpha",
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
            Self::G => "g",
            Self::H => "h",
            Self::L => "l",
            Self::R => "r",
            Self::S => "s",
            Self::W => "w",
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
            _ => {
                debug_assert!(
                    false,
                    "tried to serialize unexpected multi-value ChannelKeyword"
                );
                ""
            },
        })
    }
}

/// Return the named color with the given name.
///
/// Matching is case-insensitive in the ASCII range.
/// CSS escaping (if relevant) should be resolved before calling this function.
/// (For example, the value of an `Ident` token is fine.)
#[inline]
pub fn parse_color_keyword(ident: &str) -> Result<SpecifiedColor, ()> {
    Ok(match_ignore_ascii_case! { ident,
        "transparent" => {
            SpecifiedColor::from_absolute_color(AbsoluteColor::srgb_legacy(0u8, 0u8, 0u8, 0.0))
        },
        "currentcolor" => SpecifiedColor::CurrentColor,
        _ => {
            let (r, g, b) = cssparser::color::parse_named_color(ident)?;
            SpecifiedColor::from_absolute_color(AbsoluteColor::srgb_legacy(r, g, b, OPAQUE))
        },
    })
}

/// Parse a CSS color using the specified [`ColorParser`] and return a new color
/// value on success.
pub fn parse_color_with<'i, 't>(
    context: &ParserContext,
    input: &mut Parser<'i, 't>,
) -> Result<SpecifiedColor, ParseError<'i>> {
    let location = input.current_source_location();
    let token = input.next()?;
    match *token {
        Token::Hash(ref value) | Token::IDHash(ref value) => parse_hash_color(value.as_bytes())
            .map(|(r, g, b, a)| {
                SpecifiedColor::from_absolute_color(AbsoluteColor::srgb_legacy(r, g, b, a))
            }),
        Token::Ident(ref value) => parse_color_keyword(value),
        Token::Function(ref name) => {
            let name = name.clone();
            return input.parse_nested_block(|arguments| {
                let color_function = parse_color_function(context, name, arguments)?;
                if !color_function.has_origin_color() {
                    if let Ok(ComputedColor::Absolute(resolved)) =
                        color_function.to_computed_color(None)
                    {
                        return Ok(SpecifiedColor::from_absolute_color(resolved));
                    }
                }
                // Preserve the color as it was parsed.
                Ok(SpecifiedColor::ColorFunction(Box::new(color_function)))
            });
        },
        _ => Err(()),
    }
    .map_err(|()| location.new_unexpected_token_error(token.clone()))
}

/// Parse one of the color functions: rgba(), lab(), color(), etc.
#[inline]
fn parse_color_function<'i, 't>(
    context: &ParserContext,
    name: CowRcStr<'i>,
    arguments: &mut Parser<'i, 't>,
) -> Result<ColorFunction<SpecifiedColor>, ParseError<'i>> {
    let origin_color = parse_origin_color(context, arguments)?;
    let color = match_ignore_ascii_case! { &name,
        "rgb" | "rgba" => parse_rgb(context, arguments, origin_color),
        "hsl" | "hsla" => parse_hsl(context, arguments, origin_color),
        "hwb" => parse_hwb(context, arguments, origin_color),
        "lab" => parse_lab_like(context, arguments, origin_color, ColorFunction::Lab),
        "lch" => parse_lch_like(context, arguments, origin_color, ColorFunction::Lch),
        "oklab" => parse_lab_like(context, arguments, origin_color, ColorFunction::Oklab),
        "oklch" => parse_lch_like(context, arguments, origin_color, ColorFunction::Oklch),
        "color" => parse_color_with_color_space(context, arguments, origin_color),
        "alpha" if static_prefs::pref!("layout.css.alpha-color-function.enabled") => {
            parse_relative_alpha(
                context,
                arguments,
                origin_color.ok_or_else(|| arguments.new_custom_error(StyleParseErrorKind::UnspecifiedError))?
            )
        },
        _ => return Err(arguments.new_unexpected_token_error(Token::Ident(name))),
    }?;
    arguments.expect_exhausted()?;
    Ok(color)
}

/// Parse the relative color syntax "from" syntax `from <color>`.
fn parse_origin_color<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
) -> Result<Option<SpecifiedColor>, ParseError<'i>> {
    if !rcs_enabled() {
        return Ok(None);
    }

    // Not finding the from keyword is not an error, it just means we don't
    // have an origin color.
    if arguments
        .try_parse(|p| p.expect_ident_matching("from"))
        .is_err()
    {
        return Ok(None);
    }

    SpecifiedColor::parse(context, arguments).map(Option::Some)
}

#[inline]
fn parse_rgb<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
    origin_color: Option<SpecifiedColor>,
) -> Result<ColorFunction<SpecifiedColor>, ParseError<'i>> {
    let allowed_channel_keywords = if origin_color.is_some() {
        ChannelKeyword::rgb()
    } else {
        ChannelKeyword::empty()
    };
    let maybe_red = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;

    // If the first component is not "none" and is followed by a comma, then we
    // are parsing the legacy syntax.  Legacy syntax also doesn't support an
    // origin color.
    let is_legacy_syntax = origin_color.is_none()
        && !maybe_red.is_none()
        && arguments.try_parse(|p| p.expect_comma()).is_ok();

    Ok(if is_legacy_syntax {
        let (green, blue) = if maybe_red.could_be_percentage() {
            let green = parse_percentage(context, arguments, false, allowed_channel_keywords)?;
            arguments.expect_comma()?;
            let blue = parse_percentage(context, arguments, false, allowed_channel_keywords)?;
            (green, blue)
        } else {
            let green = parse_number(context, arguments, false, allowed_channel_keywords)?;
            arguments.expect_comma()?;
            let blue = parse_number(context, arguments, false, allowed_channel_keywords)?;
            (green, blue)
        };

        let alpha = parse_legacy_alpha(context, arguments)?;

        ColorFunction::Rgb(origin_color.into(), maybe_red, green, blue, alpha)
    } else {
        let green = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
        let blue = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;

        let alpha = parse_modern_alpha(context, arguments, allowed_channel_keywords)?;

        ColorFunction::Rgb(origin_color.into(), maybe_red, green, blue, alpha)
    })
}

/// Parses hsl syntax.
///
/// <https://drafts.csswg.org/css-color/#the-hsl-notation>
#[inline]
fn parse_hsl<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
    origin_color: Option<SpecifiedColor>,
) -> Result<ColorFunction<SpecifiedColor>, ParseError<'i>> {
    let allowed_channel_keywords = if origin_color.is_some() {
        ChannelKeyword::hsl()
    } else {
        ChannelKeyword::empty()
    };
    let hue = parse_number_or_angle(context, arguments, true, allowed_channel_keywords)?;

    // If the hue is not "none" and is followed by a comma, then we are parsing
    // the legacy syntax. Legacy syntax also doesn't support an origin color.
    let is_legacy_syntax = origin_color.is_none()
        && !hue.is_none()
        && arguments.try_parse(|p| p.expect_comma()).is_ok();

    let (saturation, lightness, alpha) = if is_legacy_syntax {
        let saturation = parse_percentage(context, arguments, false, allowed_channel_keywords)?;
        arguments.expect_comma()?;
        let lightness = parse_percentage(context, arguments, false, allowed_channel_keywords)?;
        let alpha = parse_legacy_alpha(context, arguments)?;
        (saturation, lightness, alpha)
    } else {
        let saturation =
            parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
        let lightness =
            parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
        let alpha = parse_modern_alpha(context, arguments, allowed_channel_keywords)?;
        (saturation, lightness, alpha)
    };

    Ok(ColorFunction::Hsl(
        origin_color.into(),
        hue,
        saturation,
        lightness,
        alpha,
    ))
}

/// Parses hwb syntax.
///
/// <https://drafts.csswg.org/css-color/#the-hbw-notation>
#[inline]
fn parse_hwb<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
    origin_color: Option<SpecifiedColor>,
) -> Result<ColorFunction<SpecifiedColor>, ParseError<'i>> {
    let allowed_channel_keywords = if origin_color.is_some() {
        ChannelKeyword::hwb()
    } else {
        ChannelKeyword::empty()
    };
    let hue = parse_number_or_angle(context, arguments, true, allowed_channel_keywords)?;
    let whiteness = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
    let blackness = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;

    let alpha = parse_modern_alpha(context, arguments, allowed_channel_keywords)?;

    Ok(ColorFunction::Hwb(
        origin_color.into(),
        hue,
        whiteness,
        blackness,
        alpha,
    ))
}

type IntoLabFn<Output> = fn(
    origin: Optional<SpecifiedColor>,
    l: ColorComponent<NumberOrPercentageComponent>,
    a: ColorComponent<NumberOrPercentageComponent>,
    b: ColorComponent<NumberOrPercentageComponent>,
    alpha: ColorComponent<NumberOrPercentageComponent>,
) -> Output;

#[inline]
fn parse_lab_like<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
    origin_color: Option<SpecifiedColor>,
    into_color: IntoLabFn<ColorFunction<SpecifiedColor>>,
) -> Result<ColorFunction<SpecifiedColor>, ParseError<'i>> {
    let allowed_channel_keywords = if origin_color.is_some() {
        ChannelKeyword::lab()
    } else {
        ChannelKeyword::empty()
    };
    let lightness = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
    let a = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
    let b = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;

    let alpha = parse_modern_alpha(context, arguments, allowed_channel_keywords)?;

    Ok(into_color(origin_color.into(), lightness, a, b, alpha))
}

type IntoLchFn<Output> = fn(
    origin: Optional<SpecifiedColor>,
    l: ColorComponent<NumberOrPercentageComponent>,
    a: ColorComponent<NumberOrPercentageComponent>,
    b: ColorComponent<NumberOrAngleComponent>,
    alpha: ColorComponent<NumberOrPercentageComponent>,
) -> Output;

#[inline]
fn parse_lch_like<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
    origin_color: Option<SpecifiedColor>,
    into_color: IntoLchFn<ColorFunction<SpecifiedColor>>,
) -> Result<ColorFunction<SpecifiedColor>, ParseError<'i>> {
    let allowed_channel_keywords = if origin_color.is_some() {
        ChannelKeyword::lch()
    } else {
        ChannelKeyword::empty()
    };
    let lightness = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
    let chroma = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
    let hue = parse_number_or_angle(context, arguments, true, allowed_channel_keywords)?;

    let alpha = parse_modern_alpha(context, arguments, allowed_channel_keywords)?;

    Ok(into_color(
        origin_color.into(),
        lightness,
        chroma,
        hue,
        alpha,
    ))
}

/// Parse the color() function.
#[inline]
fn parse_color_with_color_space<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
    origin_color: Option<SpecifiedColor>,
) -> Result<ColorFunction<SpecifiedColor>, ParseError<'i>> {
    let color_space = PredefinedColorSpace::parse(arguments)?;
    let allowed_channel_keywords = if origin_color.is_some() {
        match color_space {
            PredefinedColorSpace::Srgb
            | PredefinedColorSpace::SrgbLinear
            | PredefinedColorSpace::DisplayP3
            | PredefinedColorSpace::DisplayP3Linear
            | PredefinedColorSpace::A98Rgb
            | PredefinedColorSpace::ProphotoRgb
            | PredefinedColorSpace::Rec2020 => ChannelKeyword::rgb(),
            PredefinedColorSpace::XyzD50 | PredefinedColorSpace::XyzD65 => ChannelKeyword::xyz(),
        }
    } else {
        ChannelKeyword::empty()
    };

    let c1 = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
    let c2 = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;
    let c3 = parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)?;

    let alpha = parse_modern_alpha(context, arguments, allowed_channel_keywords)?;

    Ok(ColorFunction::Color(
        origin_color.into(),
        c1,
        c2,
        c3,
        alpha,
        color_space.into(),
    ))
}

/// Parse the alpha() function.
#[inline]
fn parse_relative_alpha<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
    origin_color: SpecifiedColor,
) -> Result<ColorFunction<SpecifiedColor>, ParseError<'i>> {
    let alpha = parse_modern_alpha(context, arguments, ChannelKeyword::ALPHA)?;
    Ok(ColorFunction::Alpha(origin_color.into(), alpha))
}

/// Either a percentage or a number.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, ToAnimatedValue, ToShmem)]
#[repr(u8)]
pub enum NumberOrPercentageComponent {
    /// `<number>`.
    Number(f32),
    /// `<percentage>`
    /// The value as a float, divided by 100 so that the nominal range is 0.0 to 1.0.
    Percentage(f32),
}

impl NumberOrPercentageComponent {
    /// Return the value as a number. Percentages will be adjusted to the range
    /// [0..percent_basis].
    pub fn to_number(&self, percentage_basis: f32) -> f32 {
        match *self {
            Self::Number(value) => value,
            Self::Percentage(unit_value) => unit_value * percentage_basis,
        }
    }
}

impl ColorComponentType for NumberOrPercentageComponent {
    fn from_value(value: f32) -> Self {
        Self::Number(value)
    }

    fn units() -> CalcUnits {
        CalcUnits::PERCENTAGE
    }

    fn try_from_token(token: &Token) -> Result<Self, ()> {
        Ok(match *token {
            Token::Number { value, .. } => Self::Number(value),
            Token::Percentage { unit_value, .. } => Self::Percentage(unit_value),
            _ => {
                return Err(());
            },
        })
    }

    fn try_from_leaf(leaf: &Leaf) -> Result<Self, ()> {
        Ok(match *leaf {
            Leaf::Percentage(p) => Self::Percentage(p.get()),
            Leaf::Number(n) => Self::Number(n.value()),
            _ => return Err(()),
        })
    }
}

/// Either an angle or a number.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, ToAnimatedValue, ToShmem)]
#[repr(u8)]
pub enum NumberOrAngleComponent {
    /// `<number>`.
    Number(f32),
    /// `<angle>`
    /// The value as a number of degrees.
    Angle(f32),
}

impl NumberOrAngleComponent {
    /// Return the angle in degrees. `NumberOrAngle::Number` is returned as
    /// degrees, because it is the canonical unit.
    pub fn degrees(&self) -> f32 {
        match *self {
            Self::Number(value) => value,
            Self::Angle(degrees) => degrees,
        }
    }
}

impl ColorComponentType for NumberOrAngleComponent {
    fn from_value(value: f32) -> Self {
        Self::Number(value)
    }

    fn units() -> CalcUnits {
        CalcUnits::ANGLE
    }

    fn try_from_token(token: &Token) -> Result<Self, ()> {
        Ok(match *token {
            Token::Number { value, .. } => Self::Number(value),
            Token::Dimension {
                value, ref unit, ..
            } => {
                let degrees = NoCalcAngle::parse_dimension(value, unit)?.degrees();
                NumberOrAngleComponent::Angle(degrees)
            },
            _ => {
                return Err(());
            },
        })
    }

    fn try_from_leaf(leaf: &Leaf) -> Result<Self, ()> {
        Ok(match *leaf {
            Leaf::Angle(angle) => Self::Angle(angle.degrees()),
            Leaf::Number(n) => Self::Number(n.value()),
            _ => return Err(()),
        })
    }
}

/// The raw f32 here is for <number>.
impl ColorComponentType for f32 {
    fn from_value(value: f32) -> Self {
        value
    }

    fn units() -> CalcUnits {
        CalcUnits::empty()
    }

    fn try_from_token(token: &Token) -> Result<Self, ()> {
        if let Token::Number { value, .. } = *token {
            Ok(value)
        } else {
            Err(())
        }
    }

    fn try_from_leaf(leaf: &Leaf) -> Result<Self, ()> {
        if let Leaf::Number(n) = *leaf {
            Ok(n.value())
        } else {
            Err(())
        }
    }
}

/// Parse an `<number>` or `<angle>` value.
fn parse_number_or_angle<'i, 't>(
    context: &ParserContext,
    input: &mut Parser<'i, 't>,
    allow_none: bool,
    allowed_channel_keywords: ChannelKeyword,
) -> Result<ColorComponent<NumberOrAngleComponent>, ParseError<'i>> {
    ColorComponent::parse(context, input, allow_none, allowed_channel_keywords)
}

/// Parse a `<percentage>` value.
fn parse_percentage<'i, 't>(
    context: &ParserContext,
    input: &mut Parser<'i, 't>,
    allow_none: bool,
    allowed_channel_keywords: ChannelKeyword,
) -> Result<ColorComponent<NumberOrPercentageComponent>, ParseError<'i>> {
    let location = input.current_source_location();

    let value = ColorComponent::<NumberOrPercentageComponent>::parse(
        context,
        input,
        allow_none,
        allowed_channel_keywords,
    )?;
    if !value.could_be_percentage() {
        return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
    }

    Ok(value)
}

/// Parse a `<number>` value.
fn parse_number<'i, 't>(
    context: &ParserContext,
    input: &mut Parser<'i, 't>,
    allow_none: bool,
    allowed_channel_keywords: ChannelKeyword,
) -> Result<ColorComponent<NumberOrPercentageComponent>, ParseError<'i>> {
    let location = input.current_source_location();

    let value = ColorComponent::<NumberOrPercentageComponent>::parse(
        context,
        input,
        allow_none,
        allowed_channel_keywords,
    )?;

    if !value.could_be_number() {
        return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
    }

    Ok(value)
}

/// Parse a `<number>` or `<percentage>` value.
fn parse_number_or_percentage<'i, 't>(
    context: &ParserContext,
    input: &mut Parser<'i, 't>,
    allow_none: bool,
    allowed_channel_keywords: ChannelKeyword,
) -> Result<ColorComponent<NumberOrPercentageComponent>, ParseError<'i>> {
    ColorComponent::parse(context, input, allow_none, allowed_channel_keywords)
}

fn parse_legacy_alpha<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
) -> Result<ColorComponent<NumberOrPercentageComponent>, ParseError<'i>> {
    if !arguments.is_exhausted() {
        arguments.expect_comma()?;
        parse_number_or_percentage(context, arguments, false, ChannelKeyword::empty())
    } else {
        Ok(ColorComponent::AlphaOmitted)
    }
}

fn parse_modern_alpha<'i, 't>(
    context: &ParserContext,
    arguments: &mut Parser<'i, 't>,
    allowed_channel_keywords: ChannelKeyword,
) -> Result<ColorComponent<NumberOrPercentageComponent>, ParseError<'i>> {
    if !arguments.is_exhausted() {
        arguments.expect_delim('/')?;
        parse_number_or_percentage(context, arguments, true, allowed_channel_keywords)
    } else {
        Ok(ColorComponent::AlphaOmitted)
    }
}

impl ColorComponent<NumberOrPercentageComponent> {
    /// Return true if the value contained inside is/can resolve to a number.
    /// Also returns false if the node is invalid somehow.
    fn could_be_number(&self) -> bool {
        match self {
            Self::None | Self::AlphaOmitted => true,
            Self::Value(value) => matches!(value, NumberOrPercentageComponent::Number { .. }),
            Self::ChannelKeyword(_) => {
                // Channel keywords always resolve to numbers.
                true
            },
            Self::Calc(node) => {
                if let Ok(unit) = node.unit() {
                    unit.is_empty()
                } else {
                    false
                }
            },
        }
    }

    /// Return true if the value contained inside is/can resolve to a percentage.
    /// Also returns false if the node is invalid somehow.
    fn could_be_percentage(&self) -> bool {
        match self {
            Self::None | Self::AlphaOmitted => true,
            Self::Value(value) => matches!(value, NumberOrPercentageComponent::Percentage { .. }),
            Self::ChannelKeyword(_) => {
                // Channel keywords always resolve to numbers.
                false
            },
            Self::Calc(node) => {
                if let Ok(unit) = node.unit() {
                    unit == CalcUnits::PERCENTAGE
                } else {
                    false
                }
            },
        }
    }
}
