use moli_v8_util::throw_type_error;
use moli_v8_util::{v8_string_from_utf16_units, v8_string_to_u16_string};

pub fn css_escape_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() < 1 {
        throw_type_error(
            scope,
            "Failed to execute 'escape' on 'CSS': 1 argument required, but only 0 present.",
        );
        return;
    }

    let Some(ident) = css_escape_argument_utf16(scope, &args) else {
        return;
    };
    let escaped = css_escape_utf16(&ident);
    if let Some(value) = v8_string_from_utf16_units(scope, &escaped) {
        rv.set(value.into());
    }
}

fn css_escape_argument_utf16<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<Vec<u16>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match args.get(0).to_string(&scope) {
        Some(value) => Some(v8_string_to_u16_string(&mut scope, value).into_vec()),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            None
        }
        None => {
            throw_type_error(
                &mut scope,
                "Failed to convert CSS.escape argument to string.",
            );
            let _ = scope.rethrow();
            None
        }
    }
}

fn css_escape_utf16(input: &[u16]) -> Vec<u16> {
    let mut escaped = Vec::with_capacity(input.len());
    for (index, &unit) in input.iter().enumerate() {
        if unit == 0 {
            escaped.push(0xFFFD);
        } else if should_hex_escape(unit, index, input) {
            push_hex_escape(&mut escaped, unit);
        } else if index == 0 && unit == b'-' as u16 && input.len() == 1 {
            push_simple_escape(&mut escaped, unit);
        } else if unit >= 0x80 || is_ascii_ident(unit) {
            escaped.push(unit);
        } else {
            push_simple_escape(&mut escaped, unit);
        }
    }
    escaped
}

fn should_hex_escape(unit: u16, index: usize, input: &[u16]) -> bool {
    is_control(unit)
        || (index == 0 && is_ascii_digit(unit))
        || (index == 1 && input.first() == Some(&(b'-' as u16)) && is_ascii_digit(unit))
}

fn is_control(unit: u16) -> bool {
    (0x0001..=0x001F).contains(&unit) || unit == 0x007F
}

fn is_ascii_digit(unit: u16) -> bool {
    (b'0' as u16..=b'9' as u16).contains(&unit)
}

fn is_ascii_ident(unit: u16) -> bool {
    is_ascii_digit(unit)
        || (b'a' as u16..=b'z' as u16).contains(&unit)
        || (b'A' as u16..=b'Z' as u16).contains(&unit)
        || unit == b'-' as u16
        || unit == b'_' as u16
}

fn push_hex_escape(output: &mut Vec<u16>, unit: u16) {
    output.push(b'\\' as u16);
    for byte in format!("{unit:x}").bytes() {
        output.push(u16::from(byte));
    }
    output.push(b' ' as u16);
}

fn push_simple_escape(output: &mut Vec<u16>, unit: u16) {
    output.push(b'\\' as u16);
    output.push(unit);
}

#[cfg(test)]
mod tests {
    use super::css_escape_utf16;

    fn escape_to_string(input: &str) -> String {
        String::from_utf16(&css_escape_utf16(&input.encode_utf16().collect::<Vec<_>>()))
            .expect("escaped test output should be valid UTF-16")
    }

    #[test]
    fn css_escape_uses_css_identifier_serialization() {
        assert_eq!(escape_to_string("\0"), "\u{fffd}");
        assert_eq!(escape_to_string("1a"), "\\31 a");
        assert_eq!(escape_to_string("-1"), "-\\31 ");
        assert_eq!(escape_to_string("-"), "\\-");
        assert_eq!(escape_to_string("a b"), "a\\ b");
        assert_eq!(escape_to_string("hello\\world"), "hello\\\\world");
        assert_eq!(
            escape_to_string("\u{1}\u{2}\u{1e}\u{1f}"),
            "\\1 \\2 \\1e \\1f "
        );
        assert_eq!(escape_to_string("é"), "é");
    }

    #[test]
    fn css_escape_preserves_surrogate_code_units() {
        assert_eq!(css_escape_utf16(&[0xD834, 0xDF06]), vec![0xD834, 0xDF06]);
        assert_eq!(css_escape_utf16(&[0xDF06]), vec![0xDF06]);
        assert_eq!(css_escape_utf16(&[0xD834]), vec![0xD834]);
    }
}
