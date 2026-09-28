use super::graph::Unit;
use super::*;
use crate::context_bootstrap::exposed_interfaces::TemplateBuildProfile;
use cssparser::{ParseError, ParseErrorKind};

mod expression;
use expression::Expression;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSNumericValue, enumerable)]
struct NumericConstructor {
    #[webapi(static_method, callback = parse_callback, length = 1)]
    parse: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSNumericValue.parse")]
struct ParseArgs {
    #[webidl(required, converter = "usv_string")]
    css_text: String,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
    profile: TemplateBuildProfile,
) {
    if name == "CSSNumericValue" && profile == TemplateBuildProfile::Window {
        NumericConstructor::initialize_template(scope, template);
    }
}

#[derive(Clone, Copy, Debug)]
enum Error {
    Syntax,
    TooLarge,
}

// Bound parser recursion and native object creation independently of how much
// eager algebraic simplification reduces the resulting expression.
const MAX_DEPTH: usize = 128;
const MAX_NODES: usize = 32_768;

struct Budget(usize);

impl Budget {
    fn spend(&mut self, amount: usize) -> Result<(), Error> {
        self.0 = self.0.checked_sub(amount).ok_or(Error::TooLarge)?;
        Ok(())
    }
}

fn parse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // This is a static operation: its this value does not participate in
    // receiver validation. Convert USVString once before inspecting syntax.
    let Some(parsed) = webidl::parse_args::<ParseArgs>(scope, &args) else {
        return;
    };
    let mut input = ParserInput::new(&parsed.css_text);
    let mut parser = Parser::new(&mut input);
    let mut budget = Budget(MAX_NODES);
    match parser.parse_entirely(|p| parse_value(p, &mut budget, 0, false)) {
        Ok(expression) => {
            if let Some(value) = expression.bind(scope) {
                rv.set(value.into());
            }
        }
        Err(error) => match error.kind {
            ParseErrorKind::Custom(Error::TooLarge) => {
                crate::util::throw_range_error(scope, "CSS numeric expression is too large")
            }
            _ => {
                webidl::throw_dom_exception(scope, "SyntaxError", "Invalid CSS numeric expression")
            }
        },
    }
}

fn parse_value<'i>(
    input: &mut Parser<'i, '_>,
    budget: &mut Budget,
    depth: usize,
    in_math: bool,
) -> Result<Expression, ParseError<'i, Error>> {
    if depth > MAX_DEPTH {
        return Err(input.new_custom_error(Error::TooLarge));
    }
    budget.spend(1).map_err(|e| input.new_custom_error(e))?;
    input.skip_whitespace();
    let start = input.position();
    let token = input.next()?.clone();
    let unit = match &token {
        Token::Number { .. } => Some("number".to_owned()),
        Token::Percentage { .. } => Some("percent".to_owned()),
        Token::Dimension { unit, .. } => {
            let unit = values::normalize_unit_name(unit)
                .filter(|name| name != "number" && name != "percent")
                .ok_or_else(|| input.new_custom_error(Error::Syntax))?;
            Some(unit)
        }
        _ => None,
    };
    if let Some(unit) = unit {
        // cssparser identifies and unescapes tokens; recover the source number
        // as f64 rather than exposing the tokenizer's f32 rounding to WebIDL.
        let value = moli_css_parse::css_numeric_token_value(&token, input.slice_from(start))
            .ok_or_else(|| input.new_custom_error(Error::Syntax))?;
        return Ok(Expression::unit(Unit { value, unit }, in_math));
    }
    match token {
        Token::ParenthesisBlock if in_math => {
            input.parse_nested_block(|p| parse_sum(p, budget, depth + 1))
        }
        Token::Ident(name) if in_math => {
            let value = match name.to_ascii_lowercase().as_str() {
                "e" => std::f64::consts::E,
                "pi" => std::f64::consts::PI,
                "infinity" => f64::INFINITY,
                "-infinity" => f64::NEG_INFINITY,
                "nan" => f64::NAN,
                _ => return Err(input.new_custom_error(Error::Syntax)),
            };
            Ok(Expression::unit(
                Unit {
                    value,
                    unit: "number".into(),
                },
                false,
            ))
        }
        Token::Function(name) => {
            let name = name.to_ascii_lowercase();
            let expression = input.parse_nested_block(|p| match name.as_str() {
                "calc" => parse_sum(p, budget, depth + 1),
                "min" | "max" => {
                    let children = p.parse_comma_separated(|p| parse_sum(p, budget, depth + 1))?;
                    let kind = if name == "min" { Kind::Min } else { Kind::Max };
                    Expression::operation(kind, children, budget).map_err(|e| p.new_custom_error(e))
                }
                "clamp" => {
                    let lower = optional_bound(p, budget, depth + 1)?;
                    p.expect_comma()?;
                    let value = parse_sum(p, budget, depth + 1)?;
                    p.expect_comma()?;
                    let upper = optional_bound(p, budget, depth + 1)?;
                    let (kind, children) = match (lower, upper) {
                        (Some(a), Some(b)) => (Kind::Clamp, vec![a, value, b]),
                        (Some(a), None) => (Kind::Max, vec![a, value]),
                        (None, Some(b)) => (Kind::Min, vec![value, b]),
                        (None, None) => return Ok(value),
                    };
                    Expression::operation(kind, children, budget).map_err(|e| p.new_custom_error(e))
                }
                _ => Err(p.new_custom_error(Error::Syntax)),
            })?;
            // A calc() that reduces to a scalar still reifies as CSSMathSum;
            // internal calc/parentheses do not create redundant wrapper nodes.
            if name == "calc" && !in_math && expression.is_unit() {
                Expression::unsimplified(Kind::Sum, vec![expression])
                    .map_err(|e| input.new_custom_error(e))
            } else {
                Ok(expression)
            }
        }
        _ => Err(input.new_custom_error(Error::Syntax)),
    }
}

fn optional_bound<'i>(
    input: &mut Parser<'i, '_>,
    budget: &mut Budget,
    depth: usize,
) -> Result<Option<Expression>, ParseError<'i, Error>> {
    if input.try_parse(|p| p.expect_ident_matching("none")).is_ok() {
        Ok(None)
    } else {
        parse_sum(input, budget, depth).map(Some)
    }
}

fn parse_sum<'i>(
    input: &mut Parser<'i, '_>,
    budget: &mut Budget,
    depth: usize,
) -> Result<Expression, ParseError<'i, Error>> {
    let mut children = vec![parse_product(input, budget, depth)?];
    loop {
        let start = input.state();
        // Comments alone are not whitespace around a binary + or -.
        if !matches!(input.next_including_whitespace(), Ok(Token::WhiteSpace(_))) {
            input.reset(&start);
            break;
        }
        let subtract = match input.next() {
            Ok(Token::Delim('+')) => false,
            Ok(Token::Delim('-')) => true,
            _ => {
                input.reset(&start);
                break;
            }
        };
        if !matches!(input.next_including_whitespace(), Ok(Token::WhiteSpace(_))) {
            return Err(input.new_custom_error(Error::Syntax));
        }
        let mut child = parse_product(input, budget, depth)?;
        if subtract {
            child = Expression::operation(Kind::Negate, vec![child], budget)
                .map_err(|e| input.new_custom_error(e))?;
        }
        children.push(child);
    }
    Expression::operation(Kind::Sum, children, budget).map_err(|e| input.new_custom_error(e))
}

fn parse_product<'i>(
    input: &mut Parser<'i, '_>,
    budget: &mut Budget,
    depth: usize,
) -> Result<Expression, ParseError<'i, Error>> {
    let mut children = vec![parse_value(input, budget, depth, true)?];
    loop {
        let start = input.state();
        let divide = match input.next() {
            Ok(Token::Delim('*')) => false,
            Ok(Token::Delim('/')) => true,
            _ => {
                input.reset(&start);
                break;
            }
        };
        let mut child = parse_value(input, budget, depth, true)?;
        if divide {
            child = Expression::operation(Kind::Invert, vec![child], budget)
                .map_err(|e| input.new_custom_error(e))?;
        }
        children.push(child);
    }
    Expression::operation(Kind::Product, children, budget).map_err(|e| input.new_custom_error(e))
}
