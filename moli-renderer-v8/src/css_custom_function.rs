use cssparser::{
    AtRuleParser, CowRcStr, ParseError, Parser, QualifiedRuleParser, StyleSheetParser,
};

#[derive(Clone, Debug)]
pub(crate) struct CustomCssProjectionAtRule {
    pub(crate) name: String,
    pub(crate) prelude: String,
    pub(crate) block: Option<String>,
    pub(crate) css_text: String,
}

pub(crate) fn custom_css_projection_at_rules(css_text: &str) -> Vec<CustomCssProjectionAtRule> {
    let input = css_text;
    let mut input = Parser::new(input);
    let mut parser = CustomCssProjectionRuleTextParser;
    let style_sheet = StyleSheetParser::new(&mut input, &mut parser);
    let mut rules = Vec::new();
    for parsed in style_sheet {
        if let Ok(CustomCssProjectionRule::AtRule(parsed)) = parsed
            && !parsed.name.is_empty()
        {
            rules.push(parsed.into_rule());
        }
    }
    rules
}

pub(crate) fn single_custom_css_function_projection(
    css_text: &str,
) -> Option<CustomCssProjectionAtRule> {
    let input = css_text;
    let mut input = Parser::new(input);
    let mut parser = CustomCssProjectionRuleTextParser;
    let mut style_sheet = StyleSheetParser::new(&mut input, &mut parser);
    let parsed = match style_sheet.next()? {
        Ok(CustomCssProjectionRule::AtRule(parsed)) => parsed,
        Ok(CustomCssProjectionRule::Qualified) | Err(_) => return None,
    };
    if style_sheet.next().is_some() || !parsed.name.eq_ignore_ascii_case("function") {
        return None;
    }
    Some(parsed.into_rule())
}

struct ParsedCustomCssProjectionAtRule {
    name: String,
    prelude: String,
    block: Option<String>,
}

impl ParsedCustomCssProjectionAtRule {
    fn into_rule(self) -> CustomCssProjectionAtRule {
        let css_text = custom_css_projection_at_rule_css_text(
            &self.name,
            &self.prelude,
            self.block.as_deref(),
        );
        CustomCssProjectionAtRule {
            name: self.name,
            prelude: self.prelude,
            block: self.block,
            css_text,
        }
    }
}

enum CustomCssProjectionRule {
    AtRule(ParsedCustomCssProjectionAtRule),
    Qualified,
}

struct CustomCssProjectionAtRulePrelude {
    name: String,
    prelude: String,
}

struct CustomCssProjectionRuleTextParser;

impl<'i> AtRuleParser<'i> for CustomCssProjectionRuleTextParser {
    type Prelude = CustomCssProjectionAtRulePrelude;
    type AtRule = CustomCssProjectionRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        let start = input.position();
        custom_css_projection_consume_rule_tokens(input);
        Ok(CustomCssProjectionAtRulePrelude {
            name: name.as_ref().to_owned(),
            prelude: input.slice_from(start).trim().to_owned(),
        })
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::AtRule, ParseError<Self::Error>> {
        let block_start = input.position();
        custom_css_projection_consume_rule_tokens(input);
        Ok(CustomCssProjectionRule::AtRule(
            ParsedCustomCssProjectionAtRule {
                name: prelude.name,
                prelude: prelude.prelude,
                block: Some(input.slice_from(block_start).trim().to_owned()),
            },
        ))
    }

    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
    ) -> Result<Self::AtRule, ()> {
        Ok(CustomCssProjectionRule::AtRule(
            ParsedCustomCssProjectionAtRule {
                name: prelude.name,
                prelude: prelude.prelude.trim_end_matches(';').trim().to_owned(),
                block: None,
            },
        ))
    }
}

impl<'i> QualifiedRuleParser<'i> for CustomCssProjectionRuleTextParser {
    type Prelude = ();
    type QualifiedRule = CustomCssProjectionRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        custom_css_projection_consume_rule_tokens(input);
        Ok(())
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::QualifiedRule, ParseError<Self::Error>> {
        custom_css_projection_consume_rule_tokens(input);
        Ok(CustomCssProjectionRule::Qualified)
    }
}

fn custom_css_projection_consume_rule_tokens(input: &mut Parser<'_>) {
    while input.next_including_whitespace_and_comments().is_ok() {}
}

fn custom_css_projection_at_rule_css_text(
    name: &str,
    prelude: &str,
    block: Option<&str>,
) -> String {
    let prelude = prelude.trim();
    let mut css_text = String::from("@");
    css_text.push_str(name);
    if !prelude.is_empty() {
        css_text.push(' ');
        css_text.push_str(prelude);
    }
    if let Some(block) = block {
        css_text.push_str(" {");
        css_text.push_str(block.trim());
        css_text.push('}');
    } else {
        css_text.push(';');
    }
    css_text
}
