mod condition;
mod properties;
mod values;

pub(crate) use condition::css_supports_condition_text;

use crate::webidl;
use cssparser::{Parser, ParserInput};
use style::{
    context::QuirksMode,
    parser::ParserContext,
    stylesheets::{
        CssRuleType, Origin, UrlExtraData,
        supports_rule::{SupportsCondition, parse_condition_or_declaration},
    },
};
use style_traits::{ParsingMode, ToCss};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSS.supports")]
struct CssSupportsConditionArgs {
    #[webidl(
        required,
        missing_message = "CSS.supports requires a conditionText or property/value pair"
    )]
    condition_text: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSS.supports")]
struct CssSupportsPropertyValueArgs {
    #[webidl(required, missing_message = "CSS.supports requires a property")]
    property: String,
    #[webidl(required, missing_message = "CSS.supports requires a value")]
    value: String,
}

pub(super) fn css_supports_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let supported = if args.length() < 2 {
        let Some(parsed) = webidl::parse_args::<CssSupportsConditionArgs>(scope, &args) else {
            return;
        };
        condition::css_supports_condition_text(&parsed.condition_text)
    } else {
        let Some(parsed) = webidl::parse_args::<CssSupportsPropertyValueArgs>(scope, &args) else {
            return;
        };
        properties::css_supports_property_value(&parsed.property, &parsed.value)
    };
    rv.set(v8::Boolean::new(scope, supported).into());
}

fn stylo_supports_property_value(property: &str, value: &str) -> bool {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let mut block = moli_css_parse::CssDeclarationBlock::default();
    block
        .set_property_with_projection(property, value, false)
        .set_result
        != moli_css_parse::CssSetResult::ParseError
}

fn stylo_supports_condition_text(condition: &str) -> Option<bool> {
    let mut input = ParserInput::new(condition);
    let mut parser = Parser::new(&mut input);
    let condition = parser
        .parse_entirely(|input| parse_condition_or_declaration(input))
        .ok()?;
    with_stylo_supports_context(|context| eval_stylo_supports_condition(&condition, context))
}

fn eval_stylo_supports_condition(condition: &SupportsCondition, context: &ParserContext) -> bool {
    match condition {
        SupportsCondition::Not(condition) => !eval_stylo_supports_condition(condition, context),
        SupportsCondition::Parenthesized(condition) => {
            eval_stylo_supports_condition(condition, context)
        }
        SupportsCondition::And(conditions) => conditions
            .iter()
            .all(|condition| eval_stylo_supports_condition(condition, context)),
        SupportsCondition::Or(conditions) => conditions
            .iter()
            .any(|condition| eval_stylo_supports_condition(condition, context)),
        SupportsCondition::Declaration(declaration) => {
            shell_supports_declaration(&declaration.0).unwrap_or_else(|| declaration.eval(context))
        }
        SupportsCondition::Selector(selector) => selector.eval(context),
        SupportsCondition::FontFormat(_) | SupportsCondition::FontTech(_) => {
            let css = condition.to_css_string();
            condition::css_supports_font_feature_condition(&css).unwrap_or(false)
        }
        SupportsCondition::FutureSyntax(css) => condition::css_supports_font_feature_condition(css)
            .or_else(|| condition::css_supports_at_rule_condition(css))
            .unwrap_or(false),
    }
}

fn shell_supports_declaration(declaration: &str) -> Option<bool> {
    let mut input = ParserInput::new(declaration);
    let mut parser = Parser::new(&mut input);
    let property = parser.expect_ident_cloned().ok()?;
    let property = crate::css_style::canonical_style_property_name(&property);
    if !crate::detached_css_style::css_style_declaration_is_shell_property(&property) {
        return None;
    }
    if parser.expect_colon().is_err() {
        return Some(false);
    }
    let supported = parser
        .parse_until_before(cssparser::Delimiter::Bang, |input| {
            style::properties::CSSWideKeyword::parse(input)
                .map_err(|()| input.new_custom_error::<(), ()>(()))
        })
        .is_ok();
    let _ = parser.try_parse(cssparser::parse_important);
    Some(supported && parser.expect_exhausted().is_ok())
}

fn with_stylo_supports_context<R>(f: impl FnOnce(&ParserContext) -> R) -> Option<R> {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let base_url = url::Url::parse("about:blank").ok()?;
    let url_data = UrlExtraData::from(base_url);
    let context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Default::default(),
        None,
        None,
        Default::default(),
    );
    Some(f(&context))
}
