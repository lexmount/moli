/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! A query condition:
//!
//! https://drafts.csswg.org/mediaqueries-4/#typedef-media-condition
//! https://drafts.csswg.org/css-contain-3/#typedef-container-condition

use super::{FeatureFlags, FeatureType, QueryFeatureExpression, QueryStyleRange};
use crate::computed_value_flags::ComputedValueFlags;
use crate::context::QuirksMode;
use crate::custom_properties;
use crate::derives::*;
use crate::dom::AttributeTracker;
use crate::properties::CSSWideKeyword;
use crate::properties_and_values::rule::Descriptors as PropertyDescriptors;
use crate::properties_and_values::value::{
    AllowComputationallyDependent, ComputedValue as ComputedRegisteredValue,
    SpecifiedValue as SpecifiedRegisteredValue,
};
use crate::stylesheets::{CssRuleType, CustomMediaEvaluator, Origin, UrlExtraData};
use crate::stylist::Stylist;
use crate::values::{computed, AtomString, DashedIdent};
use crate::{error_reporting::ContextualParseError, parser::Parse, parser::ParserContext};
use cssparser::{
    match_ignore_ascii_case, parse_important, Parser, ParserInput, SourcePosition, Token,
};
use selectors::kleene_value::KleeneValue;
use servo_arc::Arc;
use std::fmt::{self, Write};
use style_traits::{CssWriter, ParseError, ParsingMode, StyleParseErrorKind, ToCss};

/// A binary `and` or `or` operator.
#[derive(Clone, Copy, Debug, Eq, MallocSizeOf, Parse, PartialEq, ToCss, ToShmem)]
#[allow(missing_docs)]
pub enum Operator {
    And,
    Or,
}

/// Whether to allow an `or` condition or not during parsing.
#[derive(Clone, Copy, Debug, Eq, MallocSizeOf, PartialEq, ToCss)]
enum AllowOr {
    Yes,
    No,
}

#[derive(Clone, Debug, PartialEq, ToShmem)]
enum StyleFeatureValue {
    Value(Option<Arc<custom_properties::SpecifiedValue>>),
    Keyword(CSSWideKeyword),
}

/// Trait for query elements that parse a series of conditions separated by
/// AND or OR operators, or prefixed with NOT.
///
/// This is used by both QueryCondition and StyleQuery as they support similar
/// syntax for combining multiple conditions with a boolean operator.
trait OperationParser: Sized {
    /// https://drafts.csswg.org/mediaqueries-5/#typedef-media-condition or
    /// https://drafts.csswg.org/mediaqueries-5/#typedef-media-condition-without-or
    /// (depending on `allow_or`).
    fn parse_internal<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        feature_type: FeatureType,
        allow_or: AllowOr,
    ) -> Result<Self, ParseError<'i>> {
        let location = input.current_source_location();

        if input.try_parse(|i| i.expect_ident_matching("not")).is_ok() {
            let inner_condition = Self::parse_in_parens(context, input, feature_type)?;
            return Ok(Self::new_not(Box::new(inner_condition)));
        }

        let first_condition = Self::parse_in_parens(context, input, feature_type)?;
        let operator = match input.try_parse(Operator::parse) {
            Ok(op) => op,
            Err(..) => return Ok(first_condition),
        };

        if allow_or == AllowOr::No && operator == Operator::Or {
            return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }

        let mut conditions = vec![];
        conditions.push(first_condition);
        conditions.push(Self::parse_in_parens(context, input, feature_type)?);

        let delim = match operator {
            Operator::And => "and",
            Operator::Or => "or",
        };

        loop {
            if input.try_parse(|i| i.expect_ident_matching(delim)).is_err() {
                return Ok(Self::new_operation(conditions.into_boxed_slice(), operator));
            }

            conditions.push(Self::parse_in_parens(context, input, feature_type)?);
        }
    }

    // Parse a condition in parentheses, or `<general-enclosed>`.
    fn parse_in_parens<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        feature_type: FeatureType,
    ) -> Result<Self, ParseError<'i>>;

    // Helpers to create the appropriate enum variant of the implementing type:
    // Create a Not result that encapsulates the `inner` condition.
    fn new_not(inner: Box<Self>) -> Self;

    // Create an Operation result with the given list of `conditions` using `operator`.
    fn new_operation(conditions: Box<[Self]>, operator: Operator) -> Self;
}

/// https://drafts.csswg.org/css-conditional-5/#typedef-style-query
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToShmem)]
pub enum StyleQuery {
    /// A negation of a condition.
    Not(Box<StyleQuery>),
    /// A set of joint operations.
    Operation(Box<[StyleQuery]>, Operator),
    /// A condition wrapped in parenthesis.
    InParens(Box<StyleQuery>),
    /// A feature query (`--foo: bar` or just `--foo`).
    Feature(StyleFeature),
    /// An unknown "general-enclosed" term.
    GeneralEnclosed(String),
}

impl ToCss for StyleQuery {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: fmt::Write,
    {
        match *self {
            StyleQuery::Not(ref c) => {
                dest.write_str("not ")?;
                c.maybe_parenthesized(dest)
            },
            StyleQuery::Operation(ref list, op) => {
                let mut iter = list.iter();
                let item = iter.next().unwrap();
                item.maybe_parenthesized(dest)?;
                for item in iter {
                    dest.write_char(' ')?;
                    op.to_css(dest)?;
                    dest.write_char(' ')?;
                    item.maybe_parenthesized(dest)?;
                }
                Ok(())
            },
            StyleQuery::InParens(ref c) => match &**c {
                StyleQuery::Feature(_) | StyleQuery::InParens(_) => {
                    dest.write_char('(')?;
                    c.to_css(dest)?;
                    dest.write_char(')')
                },
                _ => c.to_css(dest),
            },
            StyleQuery::Feature(ref f) => f.to_css(dest),
            StyleQuery::GeneralEnclosed(ref s) => dest.write_str(&s),
        }
    }
}

impl StyleQuery {
    // Helper for to_css when handling values within boolean operators:
    // GeneralEnclosed includes its parens in the string, so we don't need to
    // wrap the value with an additional set here.
    fn maybe_parenthesized<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: fmt::Write,
    {
        if let StyleQuery::GeneralEnclosed(ref s) = self {
            dest.write_str(&s)
        } else {
            dest.write_char('(')?;
            self.to_css(dest)?;
            dest.write_char(')')
        }
    }

    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        feature_type: FeatureType,
    ) -> Result<Self, ParseError<'i>> {
        if !static_prefs::pref!("layout.css.style-queries.enabled")
            || feature_type != FeatureType::Container
        {
            return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }

        if let Ok(feature) = input.try_parse(|input| StyleFeature::parse(context, input)) {
            return Ok(Self::Feature(feature));
        }

        let inner = Self::parse_internal(context, input, feature_type, AllowOr::Yes)?;
        Ok(Self::InParens(Box::new(inner)))
    }

    fn parse_in_parenthesis_block<'i>(
        context: &ParserContext,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i>> {
        // Base case. Make sure to preserve this error as it's more generally
        // relevant.
        let feature_error = match input.try_parse(|input| StyleFeature::parse(context, input)) {
            Ok(feature) => return Ok(Self::Feature(feature)),
            Err(e) => e,
        };

        if let Ok(inner) = Self::parse(context, input, FeatureType::Container) {
            return Ok(inner);
        }

        Err(feature_error)
    }

    fn try_parse_block<'i, T, F>(
        context: &ParserContext,
        input: &mut Parser<'i, '_>,
        start: SourcePosition,
        parse: F,
    ) -> Option<T>
    where
        F: for<'tt> FnOnce(&mut Parser<'i, 'tt>) -> Result<T, ParseError<'i>>,
    {
        let nested = input.try_parse(|input| input.parse_nested_block(parse));
        match nested {
            Ok(nested) => Some(nested),
            Err(e) => {
                // We're about to swallow the error in a `<general-enclosed>`
                // condition, so report it while we can.
                let loc = e.location;
                let error = ContextualParseError::InvalidMediaRule(input.slice_from(start), e);
                context.log_css_error(loc, error);
                None
            },
        }
    }

    fn matches(
        &self,
        ctx: &computed::Context,
        attribute_tracker: &mut AttributeTracker,
    ) -> KleeneValue {
        ctx.builder
            .add_flags(ComputedValueFlags::DEPENDS_ON_CONTAINER_STYLE_QUERY);
        match *self {
            StyleQuery::Feature(ref f) => f.matches(ctx, attribute_tracker),
            StyleQuery::Not(ref c) => !c.matches(ctx, attribute_tracker),
            StyleQuery::InParens(ref c) => c.matches(ctx, attribute_tracker),
            StyleQuery::Operation(ref conditions, op) => {
                debug_assert!(!conditions.is_empty(), "We never create an empty op");
                match op {
                    Operator::And => KleeneValue::any_false(conditions.iter(), |c| {
                        c.matches(ctx, attribute_tracker)
                    }),
                    Operator::Or => {
                        KleeneValue::any(conditions.iter(), |c| c.matches(ctx, attribute_tracker))
                    },
                }
            },
            StyleQuery::GeneralEnclosed(_) => KleeneValue::Unknown,
        }
    }
}

impl OperationParser for StyleQuery {
    fn parse_in_parens<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        feature_type: FeatureType,
    ) -> Result<Self, ParseError<'i>> {
        assert!(feature_type == FeatureType::Container);
        input.skip_whitespace();
        let start = input.position();
        let start_location = input.current_source_location();
        match *input.next()? {
            Token::ParenthesisBlock => {
                if let Some(nested) = Self::try_parse_block(context, input, start, |i| {
                    Self::parse_in_parenthesis_block(context, i)
                }) {
                    return Ok(nested);
                }
                // Accept <ident>: <any-value> as a GeneralEnclosed (which evaluates
                // to false, but does not invalidate the query as a whole).
                input.parse_nested_block(|i| {
                    i.expect_ident()?;
                    i.expect_colon()?;
                    consume_any_value(i)
                })?;
                Ok(Self::GeneralEnclosed(input.slice_from(start).to_owned()))
            },
            ref t => return Err(start_location.new_unexpected_token_error(t.clone())),
        }
    }

    fn new_not(inner: Box<Self>) -> Self {
        Self::Not(inner)
    }

    fn new_operation(conditions: Box<[Self]>, operator: Operator) -> Self {
        Self::Operation(conditions, operator)
    }
}

/// A style query feature:
/// https://drafts.csswg.org/css-conditional-5/#typedef-style-feature
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToCss, ToShmem)]
pub enum StyleFeature {
    /// A property name and optional value to match.
    Plain(StyleFeaturePlain),
    /// A style query range expression.
    Range(QueryStyleRange),
}

impl StyleFeature {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        if let Ok(range) = input.try_parse(|i| QueryStyleRange::parse(context, i)) {
            return Ok(Self::Range(range));
        }

        Ok(Self::Plain(StyleFeaturePlain::parse(context, input)?))
    }

    fn matches(
        &self,
        ctx: &computed::Context,
        attribute_tracker: &mut AttributeTracker,
    ) -> KleeneValue {
        match self {
            Self::Plain(plain) => plain.matches(ctx, attribute_tracker),
            Self::Range(range) => range.evaluate(ctx, attribute_tracker),
        }
    }
}

/// A style feature consisting of a custom property name and (optionally) value.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToShmem)]
pub struct StyleFeaturePlain {
    name: custom_properties::Name,
    #[ignore_malloc_size_of = "StyleFeatureValue has an Arc variant"]
    value: StyleFeatureValue,
}

impl ToCss for StyleFeaturePlain {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: fmt::Write,
    {
        dest.write_str("--")?;
        crate::values::serialize_atom_identifier(&self.name, dest)?;
        match self.value {
            StyleFeatureValue::Keyword(k) => {
                dest.write_str(": ")?;
                k.to_css(dest)?;
            },
            StyleFeatureValue::Value(Some(ref v)) => {
                dest.write_str(": ")?;
                v.to_css(dest)?;
            },
            StyleFeatureValue::Value(None) => (),
        }
        Ok(())
    }
}

impl StyleFeaturePlain {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let ident = input.expect_ident()?;
        // TODO(emilio): Maybe support non-custom properties?
        let name = match custom_properties::parse_name(ident.as_ref()) {
            Ok(name) => custom_properties::Name::from(name),
            Err(()) => return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError)),
        };
        let value = if input.try_parse(|i| i.expect_colon()).is_ok() {
            input.skip_whitespace();
            if let Ok(keyword) = input.try_parse(|i| CSSWideKeyword::parse(i)) {
                StyleFeatureValue::Keyword(keyword)
            } else {
                let value = custom_properties::SpecifiedValue::parse(
                    input,
                    Some(&context.namespaces.prefixes),
                    &context.url_data,
                )?;
                // `!important` is allowed (but ignored) after the value.
                let _ = input.try_parse(parse_important);
                StyleFeatureValue::Value(Some(Arc::new(value)))
            }
        } else {
            StyleFeatureValue::Value(None)
        };
        Ok(Self { name, value })
    }

    // Substitute custom-property references in `value`, then re-parse and compute it,
    // and compare against `current_value`.
    fn substitute_and_compare(
        value: &Arc<custom_properties::SpecifiedValue>,
        registration: &PropertyDescriptors,
        stylist: &Stylist,
        ctx: &computed::Context,
        attribute_tracker: &mut AttributeTracker,
        current_value: Option<&ComputedRegisteredValue>,
    ) -> bool {
        let substitution_functions = custom_properties::ComputedSubstitutionFunctions::new(
            Some(ctx.inherited_custom_properties().clone()),
            None,
        );
        let custom_properties::SubstitutionResult { css, attr_taint } =
            match custom_properties::substitute(
                &value,
                &substitution_functions,
                stylist,
                ctx,
                attribute_tracker,
            ) {
                Ok(sub) => sub,
                Err(_) => return current_value.is_none(),
            };
        if registration.is_universal() {
            return match current_value {
                Some(v) => v.as_universal().is_some_and(|v| v.css == css),
                None => css.is_empty(),
            };
        }
        let mut input = cssparser::ParserInput::new(&css);
        let mut parser = Parser::new(&mut input);
        let computed = SpecifiedRegisteredValue::compute(
            &mut parser,
            registration,
            None,
            &value.url_data,
            ctx,
            AllowComputationallyDependent::Yes,
            attr_taint,
        )
        .ok();
        computed.as_ref() == current_value
    }

    fn matches(
        &self,
        ctx: &computed::Context,
        attribute_tracker: &mut AttributeTracker,
    ) -> KleeneValue {
        // FIXME(emilio): Confirm this is the right style to query.
        let stylist = ctx
            .builder
            .stylist
            .expect("container queries should have a stylist around");
        let registration = stylist.get_custom_property_registration(&self.name);
        let current_value = ctx
            .inherited_custom_properties()
            .get(registration, &self.name);
        KleeneValue::from(match self.value {
            StyleFeatureValue::Value(Some(ref v)) => {
                if ctx.container_info.is_none() {
                    // If no container, custom props are guaranteed-unknown.
                    false
                } else if v.has_references() {
                    // If there are --var() references in the query value,
                    // try to substitute them before comparing to current.
                    Self::substitute_and_compare(
                        v,
                        registration,
                        stylist,
                        ctx,
                        attribute_tracker,
                        current_value,
                    )
                } else {
                    custom_properties::compute_variable_value(&v, registration, ctx).as_ref()
                        == current_value
                }
            },
            StyleFeatureValue::Value(None) => current_value.is_some(),
            StyleFeatureValue::Keyword(kw) => {
                match kw {
                    CSSWideKeyword::Unset => current_value.is_none(),
                    CSSWideKeyword::Initial => {
                        if let Some(initial) = &registration.initial_value {
                            let v = custom_properties::compute_variable_value(
                                &initial,
                                registration,
                                ctx,
                            );
                            v.as_ref() == current_value
                        } else {
                            current_value.is_none()
                        }
                    },
                    CSSWideKeyword::Inherit => {
                        if let Some(inherited) = ctx
                            .container_info
                            .as_ref()
                            .expect("queries should provide container info")
                            .inherited_style()
                        {
                            inherited.custom_properties().get(registration, &self.name)
                                == current_value
                        } else {
                            false
                        }
                    },
                    // Cascade-dependent keywords, such as revert and revert-layer,
                    // are invalid as values in a style feature, and cause the
                    // container style query to be false.
                    // https://drafts.csswg.org/css-conditional-5/#evaluate-a-style-range
                    CSSWideKeyword::Revert
                    | CSSWideKeyword::RevertLayer
                    | CSSWideKeyword::RevertRule => false,
                }
            },
        })
    }
}

/// A boolean value for a pref query.
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    Eq,
    Parse,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToShmem,
)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum BoolValue {
    False,
    True,
}

/// Simple values we support for -moz-pref(). We don't want to deal with calc() and other
/// shenanigans for now.
#[derive(
    Clone,
    Debug,
    Eq,
    MallocSizeOf,
    Parse,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToShmem,
)]
#[repr(u8)]
pub enum MozPrefFeatureValue<I> {
    /// No pref value, implicitly bool, but also used to represent missing prefs.
    #[css(skip)]
    None,
    /// A bool value.
    Boolean(BoolValue),
    /// An integer value, useful for int prefs.
    Integer(I),
    /// A string pref value.
    String(crate::values::AtomString),
}

type SpecifiedMozPrefFeatureValue = MozPrefFeatureValue<crate::values::specified::Integer>;
/// The computed -moz-pref() value.
pub type ComputedMozPrefFeatureValue = MozPrefFeatureValue<crate::values::computed::Integer>;

/// A custom -moz-pref(<name>, <value>) query feature.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToShmem)]
pub struct MozPrefFeature {
    name: crate::values::AtomString,
    value: SpecifiedMozPrefFeatureValue,
}

impl MozPrefFeature {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        feature_type: FeatureType,
    ) -> Result<Self, ParseError<'i>> {
        use crate::parser::Parse;
        if !context.chrome_rules_enabled() || feature_type != FeatureType::Media {
            return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }
        let name = AtomString::parse(context, input)?;
        let value = if input.try_parse(|i| i.expect_comma()).is_ok() {
            SpecifiedMozPrefFeatureValue::parse(context, input)?
        } else {
            SpecifiedMozPrefFeatureValue::None
        };
        Ok(Self { name, value })
    }

    #[cfg(feature = "gecko")]
    fn matches(&self, ctx: &computed::Context) -> KleeneValue {
        use crate::values::computed::ToComputedValue;
        let value = self.value.to_computed_value(ctx);
        KleeneValue::from(unsafe {
            crate::gecko_bindings::bindings::Gecko_EvalMozPrefFeature(self.name.as_ptr(), &value)
        })
    }

    #[cfg(feature = "servo")]
    fn matches(&self, _: &computed::Context) -> KleeneValue {
        KleeneValue::Unknown
    }
}

impl ToCss for MozPrefFeature {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: fmt::Write,
    {
        self.name.to_css(dest)?;
        if !matches!(self.value, MozPrefFeatureValue::None) {
            dest.write_str(", ")?;
            self.value.to_css(dest)?;
        }
        Ok(())
    }
}

/// Represents a condition.
#[derive(Clone, Debug, MallocSizeOf, PartialEq, ToShmem)]
pub enum QueryCondition {
    /// A simple feature expression, implicitly parenthesized.
    Feature(QueryFeatureExpression),
    /// A custom media query reference in a boolean context, implicitly parenthesized.
    Custom(DashedIdent),
    /// A negation of a condition.
    Not(Box<QueryCondition>),
    /// A set of joint operations.
    Operation(Box<[QueryCondition]>, Operator),
    /// A condition wrapped in parenthesis.
    InParens(Box<QueryCondition>),
    /// A <style> query.
    Style(StyleQuery),
    /// A -moz-pref() query.
    MozPref(MozPrefFeature),
    /// [ <function-token> <any-value>? ) ] | [ ( <any-value>? ) ]
    GeneralEnclosed(String, UrlExtraData),
}

impl ToCss for QueryCondition {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: fmt::Write,
    {
        match *self {
            // NOTE(emilio): QueryFeatureExpression already includes the
            // parenthesis.
            QueryCondition::Feature(ref f) => f.to_css(dest),
            QueryCondition::Custom(ref name) => {
                dest.write_char('(')?;
                name.to_css(dest)?;
                dest.write_char(')')
            },
            QueryCondition::Not(ref c) => {
                dest.write_str("not ")?;
                c.to_css(dest)
            },
            QueryCondition::InParens(ref c) => {
                dest.write_char('(')?;
                c.to_css(dest)?;
                dest.write_char(')')
            },
            QueryCondition::Style(ref c) => {
                dest.write_str("style(")?;
                c.to_css(dest)?;
                dest.write_char(')')
            },
            QueryCondition::MozPref(ref c) => {
                dest.write_str("-moz-pref(")?;
                c.to_css(dest)?;
                dest.write_char(')')
            },
            QueryCondition::Operation(ref list, op) => {
                let mut iter = list.iter();
                iter.next().unwrap().to_css(dest)?;
                for item in iter {
                    dest.write_char(' ')?;
                    op.to_css(dest)?;
                    dest.write_char(' ')?;
                    item.to_css(dest)?;
                }
                Ok(())
            },
            QueryCondition::GeneralEnclosed(ref s, _) => dest.write_str(&s),
        }
    }
}

/// <https://drafts.csswg.org/css-syntax-3/#typedef-any-value>
fn consume_any_value<'i, 't>(input: &mut Parser<'i, 't>) -> Result<(), ParseError<'i>> {
    input.expect_no_error_token().map_err(Into::into)
}

impl QueryCondition {
    /// Parse a single condition.
    pub fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        feature_type: FeatureType,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(context, input, feature_type, AllowOr::Yes)
    }

    fn visit<F>(&self, visitor: &mut F)
    where
        F: FnMut(&Self),
    {
        visitor(self);
        match *self {
            Self::Custom(..)
            | Self::Feature(..)
            | Self::GeneralEnclosed(..)
            | Self::Style(..)
            | Self::MozPref(..) => {},
            Self::Not(ref cond) => cond.visit(visitor),
            Self::Operation(ref conds, _op) => {
                for cond in conds.iter() {
                    cond.visit(visitor);
                }
            },
            Self::InParens(ref cond) => cond.visit(visitor),
        }
    }

    /// Returns the union of all flags in the expression. This is useful for
    /// container queries.
    pub fn cumulative_flags(&self) -> FeatureFlags {
        let mut result = FeatureFlags::empty();
        self.visit(&mut |condition| {
            if let Self::Style(..) = condition {
                result.insert(FeatureFlags::STYLE);
            }
            if let Self::Feature(ref f) = condition {
                result.insert(f.feature_flags())
            }
        });
        result
    }

    /// Parse a single condition, disallowing `or` expressions.
    ///
    /// To be used from the legacy query syntax.
    pub fn parse_disallow_or<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        feature_type: FeatureType,
    ) -> Result<Self, ParseError<'i>> {
        Self::parse_internal(context, input, feature_type, AllowOr::No)
    }

    fn parse_in_parenthesis_block<'i>(
        context: &ParserContext,
        input: &mut Parser<'i, '_>,
        feature_type: FeatureType,
    ) -> Result<Self, ParseError<'i>> {
        // Base case. Make sure to preserve this error as it's more generally
        // relevant.
        let feature_error = match input.try_parse(|input| {
            QueryFeatureExpression::parse_in_parenthesis_block(context, input, feature_type)
        }) {
            Ok(expr) => return Ok(Self::Feature(expr)),
            Err(e) => e,
        };
        if static_prefs::pref!("layout.css.custom-media.enabled") {
            if let Ok(custom) = input.try_parse(|input| DashedIdent::parse(context, input)) {
                return Ok(Self::Custom(custom));
            }
        }
        if let Ok(inner) = Self::parse(context, input, feature_type) {
            return Ok(Self::InParens(Box::new(inner)));
        }
        Err(feature_error)
    }

    fn try_parse_block<'i, T, F>(
        context: &ParserContext,
        input: &mut Parser<'i, '_>,
        start: SourcePosition,
        parse: F,
    ) -> Option<T>
    where
        F: for<'tt> FnOnce(&mut Parser<'i, 'tt>) -> Result<T, ParseError<'i>>,
    {
        let nested = input.try_parse(|input| input.parse_nested_block(parse));
        match nested {
            Ok(nested) => Some(nested),
            Err(e) => {
                // We're about to swallow the error in a `<general-enclosed>`
                // condition, so report it while we can.
                let loc = e.location;
                let error = ContextualParseError::InvalidMediaRule(input.slice_from(start), e);
                context.log_css_error(loc, error);
                None
            },
        }
    }

    /// Whether this condition matches the device and quirks mode.
    /// https://drafts.csswg.org/mediaqueries/#evaluating
    /// https://drafts.csswg.org/mediaqueries/#typedef-general-enclosed
    /// Kleene 3-valued logic is adopted here due to the introduction of
    /// <general-enclosed>.
    pub fn matches(
        &self,
        context: &computed::Context,
        custom: &mut CustomMediaEvaluator,
        attribute_tracker: &mut AttributeTracker,
    ) -> KleeneValue {
        match *self {
            Self::Custom(ref f) => custom.matches(f, context),
            Self::Feature(ref f) => f.matches(context),
            Self::GeneralEnclosed(ref str, ref url_data) => {
                self.matches_general(&str, url_data, context, custom, attribute_tracker)
            },
            Self::InParens(ref c) => c.matches(context, custom, attribute_tracker),
            Self::Not(ref c) => !c.matches(context, custom, attribute_tracker),
            Self::Style(ref c) => c.matches(context, attribute_tracker),
            Self::MozPref(ref c) => c.matches(context),
            Self::Operation(ref conditions, op) => {
                debug_assert!(!conditions.is_empty(), "We never create an empty op");
                match op {
                    Operator::And => KleeneValue::any_false(conditions.iter(), |c| {
                        c.matches(context, custom, attribute_tracker)
                    }),
                    Operator::Or => KleeneValue::any(conditions.iter(), |c| {
                        c.matches(context, custom, attribute_tracker)
                    }),
                }
            },
        }
    }

    /// For a condition that was parsed as GeneralEnclosed, try applying custom-property
    /// substitution and re-parse the result.
    fn matches_general(
        &self,
        css_text: &str,
        url_data: &UrlExtraData,
        context: &computed::Context,
        custom: &mut CustomMediaEvaluator,
        attribute_tracker: &mut AttributeTracker,
    ) -> KleeneValue {
        // This only applies (currently, at least) to container queries.
        if !context.in_container_query {
            return KleeneValue::Unknown;
        }

        let stylist = context
            .builder
            .stylist
            .expect("container query should provide a Stylist");

        // Parse the text as a custom-property value to identify references.
        let mut input = ParserInput::new(css_text);
        let value = match custom_properties::SpecifiedValue::parse(
            &mut Parser::new(&mut input),
            None, // TODO: what Namespaces should we pass here?
            url_data,
        ) {
            Ok(val) => val,
            Err(_) => return KleeneValue::Unknown,
        };

        // If no references, we're not going to end up with a new result, just bail out.
        if !value.has_references() {
            return KleeneValue::Unknown;
        }

        // Substitute var() functions if possible.
        let substitution_functions = custom_properties::ComputedSubstitutionFunctions::new(
            Some(context.inherited_custom_properties().clone()),
            None,
        );
        let custom_properties::SubstitutionResult { css, attr_taint } =
            match custom_properties::substitute(
                &value,
                &substitution_functions,
                stylist,
                context,
                attribute_tracker,
            ) {
                Ok(sub) => sub,
                Err(_) => return KleeneValue::Unknown,
            };

        // Re-parse the result as a query-condition, and evaluate it.
        let parser_context = ParserContext::new(
            Origin::Author,
            url_data,
            Some(CssRuleType::Container),
            ParsingMode::DEFAULT,
            QuirksMode::NoQuirks,
            /* namespaces = */ Default::default(),
            /* error_reporter = */ None,
            /* use_counters = */ None,
            attr_taint,
        );
        let mut input = ParserInput::new(&css);
        let result = match Self::parse(
            &parser_context,
            &mut Parser::new(&mut input),
            FeatureType::Container,
        ) {
            Ok(Self::GeneralEnclosed(..)) => {
                // If the result is still GeneralEnclosed, the query is unknown.
                KleeneValue::Unknown
            },
            Ok(query) => query.matches(context, custom, attribute_tracker),
            Err(_) => KleeneValue::Unknown,
        };

        result
    }
}

impl OperationParser for QueryCondition {
    /// Parse a condition in parentheses, or `<general-enclosed>`.
    ///
    /// https://drafts.csswg.org/mediaqueries/#typedef-media-in-parens
    fn parse_in_parens<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
        feature_type: FeatureType,
    ) -> Result<Self, ParseError<'i>> {
        input.skip_whitespace();
        let start = input.position();
        let start_location = input.current_source_location();
        match *input.next()? {
            Token::ParenthesisBlock => {
                let nested = Self::try_parse_block(context, input, start, |input| {
                    Self::parse_in_parenthesis_block(context, input, feature_type)
                });
                if let Some(nested) = nested {
                    return Ok(nested);
                }
            },
            Token::Function(ref name) => {
                match_ignore_ascii_case! { name,
                    "style" => {
                        let query = Self::try_parse_block(context, input, start, |input| {
                            StyleQuery::parse(context, input, feature_type)
                        });
                        if let Some(query) = query {
                            return Ok(Self::Style(query));
                        }
                    },
                    "-moz-pref" => {
                        let feature = Self::try_parse_block(context, input, start, |input| {
                            MozPrefFeature::parse(context, input, feature_type)
                        });
                        if let Some(feature) = feature {
                            return Ok(Self::MozPref(feature));
                        }
                    },
                    _ => {},
                }
            },
            ref t => return Err(start_location.new_unexpected_token_error(t.clone())),
        }
        input.parse_nested_block(consume_any_value)?;
        Ok(Self::GeneralEnclosed(
            input.slice_from(start).to_owned(),
            context.url_data.clone(),
        ))
    }

    fn new_not(inner: Box<Self>) -> Self {
        Self::Not(inner)
    }

    fn new_operation(conditions: Box<[Self]>, operator: Operator) -> Self {
        Self::Operation(conditions, operator)
    }
}
