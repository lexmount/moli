/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Common handling for the specified value CSS url() values.

use crate::parser::{Parse, ParserContext};
use crate::stylesheets::CorsMode;
use cssparser::{Parser, SourceLocation};
use style_traits::ParseError;

#[cfg(feature = "gecko")]
pub mod gecko;
#[cfg(feature = "gecko")]
pub use gecko::{ComputedUrl, CssUrl, SpecifiedUrl};
#[cfg(feature = "servo")]
pub mod servo;
#[cfg(feature = "servo")]
pub use servo::{ComputedUrl, CssUrl, SpecifiedUrl};

impl CssUrl {
    /// Parse a URL with a particular CORS mode.
    pub fn parse_with_cors_mode<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        cors_mode: CorsMode,
    ) -> Result<Self, ParseError<'i>> {
        let start = input.position().byte_index();
        let location = input.current_source_location();
        let url = input.expect_url()?;
        let end = input.position().byte_index();
        Self::parse_from_string(
            url.as_ref().to_owned(),
            start,
            end,
            context,
            cors_mode,
            location,
        )
    }

    /// Parse a URL from a string value that is a valid CSS token for a URL,
    /// enforcing attr()-tainting constraints if applicable.
    /// https://drafts.csswg.org/css-values-5/#attr-security
    pub fn parse_from_string<'i>(
        url: String,
        url_start: usize,
        url_end: usize,
        context: &ParserContext,
        cors_mode: CorsMode,
        location: SourceLocation,
    ) -> Result<Self, ParseError<'i>> {
        use crate::custom_properties::AttrTaintedRange;
        use style_traits::StyleParseErrorKind;
        let range = AttrTaintedRange::new(url_start, url_end);
        if context.disallow_urls_in_range(&range) {
            return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }
        Ok(Self::new_from_string(url, context, cors_mode))
    }

    /// Create a new CSS URL that is attr()-untainted given a valid CSS token for a URL.
    /// Be cautious when calling `expect_url()` to not bypass attr()-tainting checks. If
    /// it's possible attr()'s were substituted into the `url`, DO NOT use this method.
    /// https://drafts.csswg.org/css-values-5/#attr-security
    pub fn new_from_untainted_string(
        url: String,
        context: &ParserContext,
        cors_mode: CorsMode,
    ) -> Self {
        debug_assert!(context.attr_tainted_regions.is_empty());
        Self::new_from_string(url, context, cors_mode)
    }
}

impl Parse for CssUrl {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_with_cors_mode(context, input, CorsMode::None)
    }
}
