use super::*;

pub(super) fn can_inject_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &str,
) -> bool {
    if key_path.is_empty() {
        return false;
    }
    let mut current = value;
    let mut segments = key_path.split('.').peekable();
    while let Some(segment) = segments.next() {
        let Ok(object) = v8::Local::<v8::Object>::try_from(current) else {
            return false;
        };
        if segments.peek().is_none() {
            return true;
        }
        let Some(property) = v8_string(scope, segment) else {
            return false;
        };
        if object.has_own_property(scope, property.into()) != Some(true) {
            return true;
        }
        let Some(next) = object.get(scope, property.into()) else {
            return false;
        };
        current = next;
    }
    false
}

pub(in crate::context_bootstrap::indexed_db) fn inject_key_path_into_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &str,
    key: &Key,
) -> Option<()> {
    // Admission checked injectability on the clone. CreateDataProperty avoids
    // inherited setters, including Object.prototype.__proto__.
    let mut current = v8::Local::<v8::Object>::try_from(value).ok()?;
    let mut segments = key_path.split('.').peekable();
    while let Some(segment) = segments.next() {
        let property = v8_string(scope, segment)?;
        if segments.peek().is_none() {
            let key = key_to_js_value(scope, key);
            return (current.create_data_property(scope, property.into(), key) == Some(true))
                .then_some(());
        }
        current = if current.has_own_property(scope, property.into())? {
            v8::Local::<v8::Object>::try_from(current.get(scope, property.into())?).ok()?
        } else {
            let nested = v8::Object::new(scope);
            if current.create_data_property(scope, property.into(), nested.into()) != Some(true) {
                return None;
            }
            nested
        };
    }
    None
}
