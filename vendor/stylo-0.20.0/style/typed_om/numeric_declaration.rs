/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Typed OM Numeric Declaration.

use crate::derives::*;
use crate::parser::{Parse, ParserContext};
use crate::typed_om::numeric::NoCalcNumeric;
use crate::values::generics::calc::CalcUnits;
use crate::values::specified::calc::{CalcNode, CalcParseFlags};
use crate::values::specified::{
    NoCalcAngle, NoCalcLength, NoCalcNumber, NoCalcPercentage, NoCalcTime,
};
use cssparser::{Parser, Token};
use style_traits::values::specified::AllowedNumericType;
use style_traits::{ParseError, StyleParseErrorKind};

/// A numeric declaration, with or without a `calc()` expression.
#[derive(Clone, ToTyped)]
pub enum NumericDeclaration {
    /// A numeric value without a `calc()` expression.
    NoCalc(NoCalcNumeric),

    /// A numeric value represented by a `calc()` expression.
    ///
    /// <https://drafts.csswg.org/css-values/#calc-notation>
    Calc(CalcNode),
}

impl Parse for NumericDeclaration {
    /// <https://drafts.css-houdini.org/css-typed-om-1/#dom-cssnumericvalue-parse>
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();

        // Step 1.
        let token = input.next()?;

        // Step 2.
        match *token {
            Token::Number { value, .. } => Ok(Self::NoCalc(NoCalcNumeric::Number(
                NoCalcNumber::new(value),
            ))),

            Token::Percentage { unit_value, .. } => Ok(Self::NoCalc(NoCalcNumeric::Percentage(
                NoCalcPercentage::new(unit_value),
            ))),

            Token::Dimension {
                value, ref unit, ..
            } => {
                if let Ok(length) = NoCalcLength::parse_dimension_with_context(context, value, unit)
                {
                    return Ok(Self::NoCalc(NoCalcNumeric::Length(length)));
                }

                if let Ok(angle) = NoCalcAngle::parse_dimension(value, unit) {
                    return Ok(Self::NoCalc(NoCalcNumeric::Angle(angle)));
                }

                if let Ok(time) = NoCalcTime::parse_dimension(value, unit) {
                    return Ok(Self::NoCalc(NoCalcNumeric::Time(time)));
                }

                Err(location.new_unexpected_token_error(token.clone()))

                // Step 3.

                // TODO: A type should be created from unit and if that fails, the failure
                // should be propagated here.
            },

            Token::Function(ref name) => {
                let function = CalcNode::math_function(context, name, location)?;
                let allow_all_units = CalcParseFlags::new(CalcUnits::ALL);
                let node = CalcNode::parse(context, input, function, allow_all_units)?;

                let allow_all_types = AllowedNumericType::All;
                let _ = node
                    .clone()
                    .into_length_or_percentage(allow_all_types)
                    .map_err(|()| {
                        location.new_custom_error(StyleParseErrorKind::UnspecifiedError)
                    })?;

                // TODO: Add support for other values represented by a `calc()` expression.

                Ok(Self::Calc(node))
            },

            ref token => return Err(location.new_unexpected_token_error(token.clone())),
        }
    }
}
