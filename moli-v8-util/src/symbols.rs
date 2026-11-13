use v8::{Local, Object, PinScope, Private, PropertyAttribute, Symbol};

use crate::strings::{v8_string, v8str};

pub fn define_symbol_to_string_tag(
    scope: &mut PinScope<'_, '_>,
    object: Local<'_, Object>,
    tag: &str,
    attributes: PropertyAttribute,
) {
    let Some(tag) = v8_string(scope, tag) else {
        return;
    };
    let _ = object.define_own_property(
        scope,
        Symbol::get_to_string_tag(scope).into(),
        tag.into(),
        attributes,
    );
}

pub fn define_static_symbol_to_string_tag(
    scope: &mut PinScope<'_, '_>,
    object: Local<'_, Object>,
    tag: &'static str,
    attributes: PropertyAttribute,
) {
    let _ = object.define_own_property(
        scope,
        Symbol::get_to_string_tag(scope).into(),
        v8str(scope, tag).into(),
        attributes,
    );
}

pub fn set_symbol_to_string_tag(
    scope: &mut PinScope<'_, '_>,
    object: Local<'_, Object>,
    tag: &str,
) {
    define_symbol_to_string_tag(scope, object, tag, PropertyAttribute::DONT_ENUM);
}

pub fn private_key<'s>(scope: &mut PinScope<'s, '_>, name: &str) -> Option<Local<'s, Private>> {
    // for_api interns its name anyway. Start with that canonical string rather
    // than allocate and hash a fresh ordinary string on every slot lookup.
    let name = v8::String::new_from_utf8(scope, name.as_bytes(), v8::NewStringType::Internalized)?;
    // V8 named API private symbols are isolate-wide, not context-local. They
    // are still invisible to JavaScript reflection, but native code must treat
    // the slot name as an isolate-level contract when objects cross contexts.
    Some(Private::for_api(scope, Some(name)))
}

pub fn set_private_value(
    scope: &mut PinScope<'_, '_>,
    object: Local<'_, Object>,
    slot: &str,
    value: Local<'_, v8::Value>,
) {
    let Some(key) = private_key(scope, slot) else {
        return;
    };
    let _ = object.set_private(scope, key, value);
}

pub fn get_private_value<'s>(
    scope: &mut PinScope<'s, '_>,
    object: Local<'s, Object>,
    slot: &str,
) -> Option<Local<'s, v8::Value>> {
    let key = private_key(scope, slot)?;
    let value = object.get_private(scope, key)?;
    if value.is_undefined() {
        None
    } else {
        Some(value)
    }
}

pub fn get_private_object<'s>(
    scope: &mut PinScope<'s, '_>,
    object: Local<'s, Object>,
    slot: &str,
) -> Option<Local<'s, Object>> {
    get_private_value(scope, object, slot).and_then(|value| Local::<Object>::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_v8_test_util::ensure_v8;
    use std::pin::pin;

    #[test]
    fn internalized_slot_names_preserve_legacy_private_symbols_across_contexts() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let first_context = v8::Context::new(scope, Default::default());
        let second_context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, first_context);
        for name in ["", "__moliDetachedNativeHandle", "slot\0雪🙂"] {
            let object = Object::new(scope);
            let legacy_name = v8_string(scope, name).unwrap();
            let legacy_key = Private::for_api(scope, Some(legacy_name));
            let value = v8::Number::new(scope, 7.0);
            assert_eq!(
                object.set_private(scope, legacy_key, value.into()),
                Some(true)
            );
            let dynamic_name = name.to_owned();
            assert_eq!(private_key(scope, &dynamic_name), Some(legacy_key));
            assert!(
                object
                    .get(scope, legacy_name.into())
                    .unwrap()
                    .is_undefined()
            );
            let scope = &mut v8::ContextScope::new(scope, second_context);
            assert_eq!(
                get_private_value(scope, object, name).and_then(|value| value.number_value(scope)),
                Some(7.0)
            );
        }
    }
}
