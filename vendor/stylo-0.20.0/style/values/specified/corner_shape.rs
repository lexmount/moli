/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified types for the `corner-shape` family of properties.
//!
//! <https://drafts.csswg.org/css-borders-4/#corner-shaping>

use crate::derives::*;
use crate::parser::{Parse, ParserContext};
use crate::values::computed::corner_shape as computed;
use crate::values::computed::{Context, ToComputedValue};
use crate::values::specified::Number;
use cssparser::{Parser, Token};
use style_traits::{ParseError, StyleParseErrorKind};

/// The argument to the `superellipse()` function.
///
/// `superellipse(K)` defines the corner shape using the unit equation
/// `x^(2^K) + y^(2^K) = 1`. `infinity` and `-infinity` are accepted as
/// special-cased keyword arguments.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, SpecifiedValueInfo, ToCss, ToShmem, ToTyped)]
#[typed(todo_derive_fields)]
pub enum SuperellipseArg {
    /// `<number>` argument.
    Number(Number),
    /// The `infinity` keyword.
    Infinity,
    /// The `-infinity` keyword (lexed as the literal text `-infinity`).
    #[css(keyword = "-infinity")]
    NegativeInfinity,
}

impl SuperellipseArg {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        if let Ok(arg) = input.try_parse(|i| -> Result<SuperellipseArg, ParseError<'i>> {
            let location = i.current_source_location();
            match i.next()? {
                Token::Ident(ref ident) if ident.eq_ignore_ascii_case("infinity") => {
                    Ok(SuperellipseArg::Infinity)
                },
                Token::Ident(ref ident) if ident.eq_ignore_ascii_case("-infinity") => {
                    Ok(SuperellipseArg::NegativeInfinity)
                },
                _ => Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError)),
            }
        }) {
            return Ok(arg);
        }
        Number::parse(context, input).map(SuperellipseArg::Number)
    }

    /// Resolve to a numeric `K` value (possibly +/- infinity).
    pub fn to_k(&self, context: &Context) -> f32 {
        match self {
            SuperellipseArg::Infinity => f32::INFINITY,
            SuperellipseArg::NegativeInfinity => f32::NEG_INFINITY,
            SuperellipseArg::Number(n) => n.to_computed_value(context),
        }
    }
}

/// The specified value of a single corner-shape (e.g. `corner-top-left-shape`).
///
/// <https://drafts.csswg.org/css-borders-4/#typedef-corner-shape-value>
#[derive(Clone, Debug, MallocSizeOf, PartialEq, SpecifiedValueInfo, ToCss, ToShmem, ToTyped)]
#[typed(todo_derive_fields)]
pub enum CornerShape {
    /// `round`, equivalent to `superellipse(1)`. Initial value.
    Round,
    /// `scoop`, equivalent to `superellipse(-1)`.
    Scoop,
    /// `bevel`, equivalent to `superellipse(0)`.
    Bevel,
    /// `notch`, equivalent to `superellipse(-infinity)`.
    Notch,
    /// `square`, equivalent to `superellipse(infinity)`.
    Square,
    /// `squircle`, equivalent to `superellipse(2)`.
    Squircle,
    /// `superellipse(<arg>)`.
    #[css(function)]
    Superellipse(SuperellipseArg),
}

impl CornerShape {
    /// The initial value: `round`.
    #[inline]
    pub fn round() -> Self {
        CornerShape::Round
    }
}

impl Parse for CornerShape {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        // Try the `superellipse(...)` function first.
        if let Ok(arg) = input.try_parse(|i| {
            i.expect_function_matching("superellipse")?;
            i.parse_nested_block(|i| SuperellipseArg::parse(context, i))
        }) {
            return Ok(CornerShape::Superellipse(arg));
        }
        Ok(try_match_ident_ignore_ascii_case! { input,
            "round" => CornerShape::Round,
            "scoop" => CornerShape::Scoop,
            "bevel" => CornerShape::Bevel,
            "notch" => CornerShape::Notch,
            "square" => CornerShape::Square,
            "squircle" => CornerShape::Squircle,
        })
    }
}

impl ToComputedValue for CornerShape {
    type ComputedValue = computed::CornerShape;

    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        // Per spec, the computed value is always the corresponding
        // `superellipse(K)` value.
        computed::CornerShape {
            k: match self {
                CornerShape::Round => 1.0,
                CornerShape::Scoop => -1.0,
                CornerShape::Bevel => 0.0,
                CornerShape::Notch => f32::NEG_INFINITY,
                CornerShape::Square => f32::INFINITY,
                CornerShape::Squircle => 2.0,
                CornerShape::Superellipse(arg) => arg.to_k(context),
            },
        }
    }

    fn from_computed_value(c: &Self::ComputedValue) -> Self {
        let arg = if c.k == f32::INFINITY {
            SuperellipseArg::Infinity
        } else if c.k == f32::NEG_INFINITY {
            SuperellipseArg::NegativeInfinity
        } else {
            SuperellipseArg::Number(Number::new(c.k))
        };
        CornerShape::Superellipse(arg)
    }
}

/// The specified value of `corner-shape`. Stored per-corner.
pub type CornerShapeRect = crate::values::generics::border::GenericCornerShapeRect<CornerShape>;

impl CornerShapeRect {
    /// Initial value: `round` for all four corners.
    pub fn round() -> Self {
        Self::all(CornerShape::Round)
    }
}
