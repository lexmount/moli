/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Common handling for the specified value CSS param() values.

use crate::custom_properties::VariableValue;
use crate::parser::{Parse, ParserContext};
use crate::values::fmt;
use crate::values::CssWriter;
use crate::{derives::*, values::DashedIdent};
use cssparser::Parser;
use std::fmt::Write;
use style_traits::arc_slice::ArcSlice;
use style_traits::owned_str::OwnedStr;
use style_traits::{ParseError, ToCss};

/// A struct to hold a specific '<declaration-value>?', since an empty value (param(--a, )) serializes differently than
/// a non-provided value (param(--a))
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C, u8)]
pub enum LinkParamValueOrNone {
    /// Handles the case when no value is provided at all: param(--a)
    None,
    /// Handles the cases for provided, or provided but empty, e.g. param(--a, blue) and param(--a, )
    Specified(OwnedStr),
}

/// A single param(--ident, value) entry: https://drafts.csswg.org/css-link-params-1/#funcdef-param
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C)]
pub struct LinkParam {
    /// link-parameters' param name, in the form of an <dashed-ident>: https://drafts.csswg.org/css-values-4/#typedef-dashed-ident
    pub name: DashedIdent,
    /// link-parameters' value, in the form of a <declaration-value>: https://drafts.csswg.org/css-syntax-3/#typedef-declaration-value
    pub value: LinkParamValueOrNone,
}

/// A struct to hold all specified link-parameters: https://drafts.csswg.org/css-link-params-1/
///
/// We treat `none` as an empty slice and vis-versa
#[derive(
    Clone,
    Debug,
    Default,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C)]
#[css(comma)]
#[typed(skip_derive_fields)]
pub struct LinkParameters(
    /// Slice of specified link-parameters properties: https://drafts.csswg.org/css-link-params-1/#link-param-prop
    #[css(iterable, if_empty = "none")]
    #[ignore_malloc_size_of = "Arc"]
    pub ArcSlice<LinkParam>,
);

impl LinkParameters {
    /// Returns the `none` value.
    pub fn none() -> Self {
        Self(ArcSlice::default())
    }
}

impl Parse for LinkParameters {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            return Ok(Self::none());
        }

        let params = input.parse_comma_separated(|input| {
            input.expect_function_matching("param")?;
            input.parse_nested_block(|input| {
                let name = DashedIdent::parse(context, input)?;
                // if a comma exists then parse it and set value as specified, even if no value provided
                // need to handle url references properly https://bugzilla.mozilla.org/show_bug.cgi?id=2028998
                let value = if input.try_parse(|i| i.expect_comma()).is_ok() {
                    let parsed = VariableValue::parse(input, None, &context.url_data)?;
                    LinkParamValueOrNone::Specified(OwnedStr::from(parsed.css))
                } else {
                    LinkParamValueOrNone::None
                };
                Ok(LinkParam { name, value })
            })
        })?;

        Ok(Self(crate::ArcSlice::from_iter(params.into_iter())))
    }
}

impl ToCss for LinkParam {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        dest.write_str("param(")?;
        self.name.to_css(dest)?;
        if let LinkParamValueOrNone::Specified(param) = &self.value {
            dest.write_str(", ")?;
            if !param.is_empty() {
                // Don't use to_css, instead write the raw CSS value without extra quoting, else serialization will include un-de-serializable quotes
                dest.write_str(&param)?;
            }
        }
        dest.write_char(')')
    }
}
