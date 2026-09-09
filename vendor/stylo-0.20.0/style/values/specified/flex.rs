/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified types for CSS values related to flexbox.

use crate::parser::{Parse, ParserContext};
use crate::values::generics::flex::FlexBasis as GenericFlexBasis;
use crate::values::specified::Size;
use cssparser::Parser;
use style_traits::ParseError;

/// A specified value for the `flex-basis` property.
pub type FlexBasis = GenericFlexBasis<Size>;

impl FlexBasis {
    /// `auto`
    #[inline]
    pub fn auto() -> Self {
        GenericFlexBasis::Size(Size::auto())
    }

    /// `0%`
    #[inline]
    pub fn zero_percent() -> Self {
        GenericFlexBasis::Size(Size::zero_percent())
    }
}

impl Parse for FlexBasis {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let v = input.try_parse(|i| {
            Ok(try_match_ident_ignore_ascii_case! {i, "content" => Self::Content, })
        });
        if v.is_ok() {
            return v;
        }
        Ok(Self::Size(Size::parse_size_for_flex_basis_width(
            context, input,
        )?))
    }
}
