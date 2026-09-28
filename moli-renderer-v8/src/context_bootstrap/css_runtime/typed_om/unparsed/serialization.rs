use super::*;
use cssparser::{ToCss, TokenSerializationType, serialize_identifier};

struct Frame<'s> {
    owner: v8::Local<'s, v8::Object>,
    values: v8::Local<'s, v8::Array>,
    index: u32,
    close_fallback: bool,
}

pub(in super::super) fn serialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let mut stack = vec![Frame {
        owner: object,
        values: segments(scope, object)?,
        index: 0,
        close_fallback: false,
    }];
    let mut text = String::new();
    // Use an explicit stack: author-created fallback graphs can be deep or
    // cyclic, while sharing a fallback in separate branches remains valid.
    while let Some(frame) = stack.last_mut() {
        if frame.index == frame.values.length() {
            if frame.close_fallback {
                text.push(')');
            }
            stack.pop();
            continue;
        }
        if frame.index != 0 {
            text.push_str("/**/");
        }
        let value = frame.values.get_index(scope, frame.index)?;
        frame.index += 1;
        if let Ok(string) = v8::Local::<v8::String>::try_from(value) {
            text.push_str(&string.to_rust_string_lossy(scope));
            continue;
        }
        let reference = v8::Local::<v8::Object>::try_from(value).ok()?;
        let variable = get_private_value(scope, reference, VARIABLE_SLOT)?
            .to_string(scope)?
            .to_rust_string_lossy(scope);
        text.push_str("var(");
        serialize_identifier(&variable, &mut text).ok()?;
        let fallback = get_private_value(scope, reference, FALLBACK_SLOT)?;
        if fallback.is_null() {
            text.push(')');
            continue;
        }
        let fallback = v8::Local::<v8::Object>::try_from(fallback).ok()?;
        if stack
            .iter()
            .any(|frame| frame.owner.strict_equals(fallback.into()))
        {
            return Some(String::new());
        }
        text.push(',');
        stack.push(Frame {
            owner: fallback,
            values: segments(scope, fallback)?,
            index: 0,
            close_fallback: true,
        });
    }
    Some(serialize_tokens(&text))
}

fn serialize_tokens(mut source: &str) -> String {
    let mut output = String::new();
    let mut previous = TokenSerializationType::Nothing;
    // Read one raw token at a time, without a component-value parser consuming
    // nested blocks or synthesizing closing tokens for arbitrary string input.
    while !source.is_empty() {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        let consumed = parser.position().byte_index();
        if !matches!(token, Token::Comment(_)) {
            let next = token.serialization_type();
            if previous.needs_separator_when_before(next) {
                output.push_str("/**/");
            }
            token
                .to_css(&mut output)
                .expect("String writes cannot fail");
            previous = next;
        }
        source = &source[consumed..];
    }
    output
}
