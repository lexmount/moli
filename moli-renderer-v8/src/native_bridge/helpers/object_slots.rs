use super::super::v8_string;

pub(crate) fn set_object_slot(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(key) = v8_string(scope, name) else {
        return;
    };
    let _ = object.define_own_property(scope, key.into(), value, v8::PropertyAttribute::DONT_ENUM);
}
