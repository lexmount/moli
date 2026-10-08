use v8::{Local, NewStringType, PinScope, String, Value};
use widestring::U16String;

// Prefer this for fixed property/slot names. With `&'static str` we can create
// an internalized V8 string directly, avoid the fallible dynamic-string path,
// and reuse V8's canonical key for repeated property definitions/lookups.
pub fn v8str<'a>(scope: &PinScope<'a, '_, ()>, value: &'static str) -> Local<'a, String> {
    String::new_from_utf8(scope, value.as_bytes(), NewStringType::Internalized)
        .expect("v8 internalized string creation failed")
}

pub fn v8_string<'a>(scope: &PinScope<'a, '_, ()>, value: &str) -> Option<Local<'a, String>> {
    String::new(scope, value)
}

pub fn is_nullish(value: Local<'_, Value>) -> bool {
    value.is_null_or_undefined()
}

pub fn v8_string_from_utf16_units<'s>(
    scope: &v8::PinScope<'s, '_>,
    units: &[u16],
) -> Option<v8::Local<'s, v8::String>> {
    v8::String::new_from_two_byte(scope, units, v8::NewStringType::Normal)
}

pub fn v8_string_to_u16_string(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::String>,
) -> U16String {
    let mut units = vec![0; value.length()];
    // The vendored V8 binding for String::write_v2 returns `()`. The buffer is
    // sized from the same V8 string length and the binding writes at most that
    // many UTF-16 code units.
    value.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    U16String::from_vec(units)
}
