//! HTML source sizes need CSS component boundaries and media conditions, but
//! must validate math before CSS property clamping or app-unit conversion.

use cssparser::{Delimiter, Parser, SourcePosition, Token};
use style::{
    context::QuirksMode,
    device::Device,
    dom::AttributeTracker,
    parser::ParserContext,
    queries::{FeatureType, QueryCondition},
    stylesheets::CustomMediaEvaluator,
    values::{
        computed::{Context, ToComputedValue},
        generics::calc::CalcUnits,
        specified::{
            Length, NoCalcLength,
            calc::{CalcNode, CalcNodeParseInPlaceOperations, CalcParseFlags, Leaf},
            length::LengthUnit,
        },
    },
};

pub(super) fn evaluate(sizes: &str, parser_context: &ParserContext, device: &Device) -> f64 {
    Context::for_media_query_evaluation(device, QuirksMode::NoQuirks, |context| {
        let mut parser = Parser::new(sizes);
        loop {
            let result = parser.parse_until_before(Delimiter::Comma, |component| {
                source_size(component, parser_context, context)
                    .ok_or_else(|| cssparser::ParseError::<()>::custom(()))
            });
            if let Ok(size) = result {
                return size;
            }
            if parser.expect_comma().is_err() {
                return f64::from(
                    Length::new(NoCalcLength::new(LengthUnit::Vw, 100.0))
                        .to_computed_value(context)
                        .px(),
                );
            }
        }
    })
}

fn source_size<'i>(
    input: &mut Parser<'i>,
    parser_context: &ParserContext,
    context: &Context,
) -> Option<f64> {
    let start = input.position();
    let mut last = None;
    while !input.is_exhausted() {
        let position = input.position();
        let token = input.next().ok()?.clone();
        if matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            input
                .parse_nested_block::<_, _, ()>(|block| {
                    block.expect_no_error_token().map_err(Into::into)
                })
                .ok()?;
        } else if token.is_parse_error() {
            return None;
        }
        last = Some((position, input.position()));
    }
    let (value_start, value_end) = last?;
    let size = source_length(input.slice(value_start..value_end), parser_context, context)?;
    let condition = input.slice(start..value_start);
    let normalized = normalize_zero_math(condition, parser_context, context)?;
    let mut condition_parser = Parser::new(&normalized);
    if condition_parser.is_exhausted() {
        return Some(size);
    }
    let condition = condition_parser
        .parse_entirely(|input| QueryCondition::parse(parser_context, input, FeatureType::Media))
        .ok()?;
    condition
        .matches(
            context,
            &mut CustomMediaEvaluator::none(),
            &mut AttributeTracker::new_dummy(),
        )
        .to_bool(false)
        .then_some(size)
}

fn source_length(raw: &str, parser_context: &ParserContext, context: &Context) -> Option<f64> {
    let mut parser = Parser::new(raw);
    let state = parser.state();
    let function = matches!(parser.next().ok()?, Token::Function(_));
    parser.reset(&state);
    let value = if function {
        let Leaf::Length(length) = math_value(raw, parser_context, context)? else {
            return None;
        };
        length.unitless_value()
    } else {
        parser
            .parse_entirely(|input| Length::parse_non_negative(parser_context, input))
            .ok()?
            .to_computed_value(context)
            .px()
    };
    value.is_finite().then_some(f64::from(value.max(0.0)))
}

fn math_value(raw: &str, parser_context: &ParserContext, context: &Context) -> Option<Leaf> {
    let mut parser = Parser::new(raw);
    let Token::Function(name) = parser.next().ok()?.clone() else {
        return None;
    };
    let function = CalcNode::math_function(parser_context, &name).ok()?;
    let node = CalcNode::parse(
        parser_context,
        &mut parser,
        function,
        // Keep division and multiplication until after unit conversion, so
        // CSS property conversion cannot hide a non-finite math result.
        CalcParseFlags {
            in_place_operations: CalcNodeParseInPlaceOperations::No,
            ..CalcParseFlags::new(CalcUnits::LENGTH)
        },
    )
    .ok()?;
    parser.expect_exhausted().ok()?;
    let mut node = node.to_computed_value(Some(context), None);
    node.simplify_and_sort();
    node.resolve().ok()
}

// Media length features accept unitless zero, including a number-valued math
// result. Resolve only those functions to the equivalent zero token before
// handing the condition to Stylo; preserve unknown terms and nonzero numbers.
fn normalize_zero_math(
    raw: &str,
    parser_context: &ParserContext,
    context: &Context,
) -> Option<String> {
    let mut parser = Parser::new(raw);
    let mut copied = parser.position();
    let mut replacements = Vec::new();
    zero_math_ranges(&mut parser, parser_context, context, &mut replacements)?;
    let mut result = String::new();
    for (start, end) in replacements {
        result.push_str(parser.slice(copied..start));
        result.push_str(" 0 ");
        copied = end;
    }
    result.push_str(parser.slice_from(copied));
    Some(result)
}

fn zero_math_ranges<'i>(
    input: &mut Parser<'i>,
    parser_context: &ParserContext,
    context: &Context,
    replacements: &mut Vec<(SourcePosition, SourcePosition)>,
) -> Option<()> {
    while !input.is_exhausted() {
        let start = input.position();
        match input.next().ok()?.clone() {
            Token::Function(_) => {
                input
                    .parse_nested_block::<_, _, ()>(|block| {
                        block.expect_no_error_token().map_err(Into::into)
                    })
                    .ok()?;
                let end = input.position();
                if let Some(Leaf::Number(number)) =
                    math_value(input.slice(start..end), parser_context, context)
                    && number.value() == 0.0
                {
                    replacements.push((start, end));
                }
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                input
                    .parse_nested_block::<_, _, ()>(|block| {
                        zero_math_ranges(block, parser_context, context, replacements)
                            .ok_or_else(|| cssparser::ParseError::<()>::custom(()))
                    })
                    .ok()?;
            }
            token if token.is_parse_error() => return None,
            _ => {}
        }
    }
    Some(())
}
