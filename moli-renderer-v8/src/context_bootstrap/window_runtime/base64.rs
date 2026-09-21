use super::*;
use crate::webidl;
use ::base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose::STANDARD},
};

const FORGIVING_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

pub(in crate::context_bootstrap) fn base64_encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "btoa")]
struct WindowBtoaArgs {
    #[webidl(required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "atob")]
struct WindowAtobArgs {
    #[webidl(required)]
    data: String,
}

fn base64_decode(s: &str) -> std::result::Result<Vec<u8>, ()> {
    let compact = s
        .bytes()
        .filter(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
        .collect::<Vec<_>>();
    if compact.len() % 4 == 1 {
        return Err(());
    }
    FORGIVING_BASE64.decode(compact).map_err(|_| ())
}

pub(in crate::context_bootstrap) fn global_btoa_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WindowBtoaArgs>(scope, &args) else {
        return;
    };
    let input = parsed.data;
    for c in input.chars() {
        if c as u32 > 255 {
            throw_invalid_character_error(
                scope,
                "The string to be encoded contains characters outside of the Latin1 range.",
            );
            return;
        }
    }
    let bytes: Vec<u8> = input.chars().map(|c| c as u8).collect();
    let encoded = base64_encode(&bytes);
    if let Some(s) = v8_string(scope, &encoded) {
        rv.set(s.into());
    }
}

pub(in crate::context_bootstrap) fn global_atob_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<WindowAtobArgs>(scope, &args) else {
        return;
    };
    match base64_decode(&parsed.data) {
        Ok(bytes) => {
            let s: String = bytes.into_iter().map(|b| b as char).collect();
            if let Some(v) = v8_string(scope, &s) {
                rv.set(v.into());
            }
        }
        Err(()) => {
            throw_invalid_character_error(
                scope,
                "The string to be decoded is not correctly encoded.",
            );
        }
    }
}

pub(in crate::context_bootstrap) fn window_btoa_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !crate::context_bootstrap::require_same_origin_window_receiver(scope, args.this(), false) {
        return;
    }
    global_btoa_callback(scope, args, rv);
}

pub(in crate::context_bootstrap) fn window_atob_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !crate::context_bootstrap::require_same_origin_window_receiver(scope, args.this(), false) {
        return;
    }
    global_atob_callback(scope, args, rv);
}

fn throw_invalid_character_error(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    crate::context_bootstrap::throw_dom_exception_value(scope, message, "InvalidCharacterError");
}

#[cfg(test)]
mod tests {
    use super::base64_decode;

    #[test]
    fn base64_decode_uses_forgiving_base64_engine() {
        assert_eq!(base64_decode("YQ").unwrap(), b"a");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        assert_eq!(base64_decode(" Y\tQ\n==\r").unwrap(), b"a");
        assert!(base64_decode("Y").is_err());
    }
}
