use cssparser::{Parser, ParserInput, ToCss, Token, TokenSerializationType};
use style::typed_om::{UnparsedSegment, UnparsedValue, VariableReferenceValue};

struct Frame {
    variable: Option<String>,
    parts: UnparsedValue,
    text: String,
    previous: TokenSerializationType,
    blocks: usize,
}

impl Frame {
    fn new(variable: Option<String>) -> Self {
        Self {
            variable,
            parts: UnparsedValue::new(),
            text: String::new(),
            previous: TokenSerializationType::Nothing,
            blocks: 0,
        }
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.parts
                .push(UnparsedSegment::String(std::mem::take(&mut self.text)));
        }
        self.previous = TokenSerializationType::Nothing;
    }
}

// Consume raw tokens without cssparser's component parser skipping nested
// blocks; the input has already passed Stylo's declaration-value validation.
fn next<'a>(source: &mut &'a str) -> Option<Token<'a>> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let token = parser
        .next_including_whitespace_and_comments()
        .ok()?
        .clone();
    *source = &source[parser.position().byte_index()..];
    Some(token)
}

fn next_non_whitespace<'a>(source: &mut &'a str) -> Option<Token<'a>> {
    loop {
        let token = next(source)?;
        if !matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
            return Some(token);
        }
    }
}

pub(super) fn reify(mut source: &str) -> Option<UnparsedValue> {
    let mut stack = vec![Frame::new(None)];
    while let Some(token) = next(&mut source) {
        let frame = stack.last_mut()?;
        if matches!(&token, Token::Function(name) if name.eq_ignore_ascii_case("var")) {
            frame.flush();
            let Token::Ident(name) = next_non_whitespace(&mut source)? else {
                return None;
            };
            let variable = name.to_string();
            match next_non_whitespace(&mut source)? {
                Token::CloseParenthesis => {
                    frame
                        .parts
                        .push(UnparsedSegment::VariableReference(VariableReferenceValue {
                            variable,
                            fallback: UnparsedValue::new(),
                            has_fallback: false,
                        }));
                }
                Token::Comma => stack.push(Frame::new(Some(variable))),
                _ => return None,
            }
            continue;
        }
        if matches!(token, Token::CloseParenthesis) && frame.blocks == 0 && frame.variable.is_some()
        {
            let mut completed = stack.pop()?;
            completed.flush();
            stack
                .last_mut()?
                .parts
                .push(UnparsedSegment::VariableReference(VariableReferenceValue {
                    variable: completed.variable?,
                    fallback: completed.parts,
                    has_fallback: true,
                }));
            continue;
        }
        match token {
            Token::Comment(_) => continue,
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => frame.blocks += 1,
            Token::CloseParenthesis | Token::CloseSquareBracket | Token::CloseCurlyBracket => {
                frame.blocks = frame.blocks.checked_sub(1)?
            }
            _ => {}
        }
        let kind = token.serialization_type();
        if frame.previous.needs_separator_when_before(kind) {
            frame.text.push_str("/**/");
        }
        token.to_css(&mut frame.text).ok()?;
        frame.previous = kind;
    }
    if stack.len() != 1 {
        return None;
    }
    let mut result = stack.pop()?;
    result.flush();
    Some(result.parts)
}
