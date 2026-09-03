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

pub fn private_key<'s>(scope: &mut PinScope<'s, '_, ()>, name: &str) -> Option<Local<'s, Private>> {
    let name = v8_string(scope, name)?;
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
