/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Computed types for text properties.

use crate::derives::*;
#[cfg(feature = "gecko")]
use crate::gecko_bindings::bindings;
use crate::typed_om::{KeywordValue, ToTyped, TypedValue};
use crate::values::animated::text::TextDecorationInset as AnimatedTextDecorationInset;
use crate::values::animated::{Context as AnimatedContext, ToAnimatedValue};
use crate::values::computed::length::{CSSPixelLength, LengthPercentage};
use crate::values::generics::text::{
    GenericHyphenateLimitChars, GenericInitialLetter, GenericTextDecorationInset,
    GenericTextDecorationLength, GenericTextIndent,
};
use crate::values::generics::NumberOrAuto;
use crate::values::specified::text as specified;
use crate::values::specified::text::{TextEmphasisFillMode, TextEmphasisShapeKeyword};
use crate::values::{CSSFloat, CSSInteger, ComputeSquaredDistance};
use crate::Zero;
use std::fmt::{self, Write};
use style_traits::{CssString, CssWriter, ToCss};
use thin_vec::ThinVec;

pub use crate::values::specified::text::{
    HyphenateCharacter, LineBreak, MozControlCharacterVisibility, OverflowWrap, RubyPosition,
    TextAlignLast, TextAutospace, TextBoxEdge, TextBoxTrim, TextDecorationLine,
    TextDecorationSkipInk, TextDecorationSkipSpaces, TextEmphasisPosition, TextJustify,
    TextOverflow, TextSizeAdjust, TextTransform, TextUnderlinePosition, WordBreak,
};

/// A computed value for the `initial-letter` property.
pub type InitialLetter = GenericInitialLetter<CSSFloat, CSSInteger>;

/// Implements type for `text-decoration-thickness` property.
pub type TextDecorationLength = GenericTextDecorationLength<LengthPercentage>;

/// Implements type for `text-decoration-inset` property.
pub type TextDecorationInset = GenericTextDecorationInset<LengthPercentage>;

impl ToAnimatedValue for TextDecorationInset {
    type AnimatedValue = AnimatedTextDecorationInset;

    fn to_animated_value(self, context: &AnimatedContext) -> Self::AnimatedValue {
        match self {
            Self::Auto => {
                let font_size_px = context
                    .style
                    .get_font()
                    .clone_font_size()
                    .computed_size()
                    .px();
                #[cfg(feature = "gecko")]
                let auto_length = bindings::Gecko_CalcAutoDecorationInset(font_size_px);
                #[cfg(feature = "servo")]
                let auto_length = {
                    // Use an inset factor of 1/12.5, so we get 2px of inset (resulting in 4px
                    // gap between adjacent lines) at font-size 25px.
                    let auto_inset_factor = 1.0 / 12.5;

                    // Use the em size multiplied by auto_inset_factor, with a minimum of one
                    // CSS pixel to ensure that at least some separation occurs.
                    (font_size_px * auto_inset_factor).max(1.0)
                };
                let auto_length = CSSPixelLength::new(auto_length);
                Self::AnimatedValue {
                    start: LengthPercentage::new_length(auto_length),
                    end: LengthPercentage::new_length(auto_length),
                    is_auto: true,
                }
            },
            Self::LengthPercentage { start, end } => Self::AnimatedValue {
                start: start.to_animated_value(context),
                end: end.to_animated_value(context),
                is_auto: false,
            },
        }
    }

    #[inline]
    fn from_animated_value(value: Self::AnimatedValue) -> Self {
        if value.is_auto {
            Self::Auto
        } else {
            Self::LengthPercentage {
                start: value.start,
                end: value.end,
            }
        }
    }
}

/// The computed value of `text-align`.
pub type TextAlign = specified::TextAlignKeyword;

/// The computed value of `text-indent`.
pub type TextIndent = GenericTextIndent<LengthPercentage>;

/// A computed value for the `hyphenate-character` property.
pub type HyphenateLimitChars = GenericHyphenateLimitChars<CSSInteger>;

impl HyphenateLimitChars {
    /// Return the `auto` value, which has all three component values as `auto`.
    #[inline]
    pub fn auto() -> Self {
        Self {
            total_word_length: NumberOrAuto::Auto,
            pre_hyphen_length: NumberOrAuto::Auto,
            post_hyphen_length: NumberOrAuto::Auto,
        }
    }
}

/// A computed value for the `letter-spacing` property.
#[repr(transparent)]
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    ToAnimatedValue,
    ToAnimatedZero,
    ToResolvedValue,
)]
pub struct GenericLetterSpacing<L>(pub L);
/// This is generic just to make the #[derive()] code do the right thing for lengths.
pub type LetterSpacing = GenericLetterSpacing<LengthPercentage>;

impl LetterSpacing {
    /// Return the `normal` computed value, which is just zero.
    #[inline]
    pub fn normal() -> Self {
        Self(LengthPercentage::zero())
    }
}

impl ToCss for LetterSpacing {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        // https://drafts.csswg.org/css-text/#propdef-letter-spacing
        //
        // For legacy reasons, a computed letter-spacing of zero yields a
        // resolved value (getComputedStyle() return value) of normal.
        if self.0.is_zero() {
            return dest.write_str("normal");
        }
        self.0.to_css(dest)
    }
}

impl ToTyped for LetterSpacing {
    // Note: The specification does not currently define how letter spacing
    // should be reified into Typed OM. The current behavior follows existing
    // WPT coverage (letter-spacing.html). Syncing spec with UA/WPT behavior
    // tracked in https://github.com/w3c/csswg-drafts/issues/13907
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        if !self.0.has_percentage() && self.0.is_zero() {
            dest.push(TypedValue::Keyword(KeywordValue(CssString::from("normal"))));
            return Ok(());
        }
        self.0.to_typed(dest)
    }
}

/// A computed value for the `word-spacing` property.
pub type WordSpacing = LengthPercentage;

impl WordSpacing {
    /// Return the `normal` computed value, which is just zero.
    #[inline]
    pub fn normal() -> Self {
        LengthPercentage::zero()
    }
}

/// Computed value for the text-emphasis-style property
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToCss, ToResolvedValue, ToTyped)]
#[allow(missing_docs)]
#[repr(C, u8)]
#[typed(todo_derive_fields)]
pub enum TextEmphasisStyle {
    /// [ <fill> || <shape> ]
    Keyword {
        #[css(skip_if = "TextEmphasisFillMode::is_filled")]
        fill: TextEmphasisFillMode,
        shape: TextEmphasisShapeKeyword,
    },
    /// `none`
    None,
    /// `<string>` (of which only the first grapheme cluster will be used).
    String(crate::OwnedStr),
}
