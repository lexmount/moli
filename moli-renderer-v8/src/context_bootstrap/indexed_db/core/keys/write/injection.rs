use super::*;

pub(super) fn can_inject_key_path_into_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut value: v8::Local<'s, v8::Value>,
    key_path: &str,
) -> bool {
    let mut segments = key_path.split('.').peekable();
    while let Some(segment) = segments.next() {
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            return false;
        };
        if segments.peek().is_none() {
            return true;
        }
        let property = v8_string(scope, segment).unwrap_or_else(|| v8::String::empty(scope));
        match object.has_own_property(scope, property.into()) {
            Some(false) => return true,
            Some(true) => {
                let Some(next) = object.get(scope, property.into()) else {
                    return false;
                };
                value = next;
            }
            None => return false,
        }
    }
    false
}

/// The caller already cloned the value and checked injection before admission.
/// Use data properties so prototype setters never run during storage execution.
pub(in crate::context_bootstrap::indexed_db) fn inject_key_path_into_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    key_path: &str,
    key: &Key,
) -> Option<()> {
    let mut current = v8::Local::<v8::Object>::try_from(value).ok()?;
    let mut segments = key_path.split('.').peekable();
    while let Some(segment) = segments.next() {
        let property = v8_string(scope, segment).unwrap_or_else(|| v8::String::empty(scope));
        if segments.peek().is_none() {
            let key = key_to_js_value(scope, key);
            return (current.create_data_property(scope, property.into(), key) == Some(true))
                .then_some(());
        }
        if current.has_own_property(scope, property.into())? {
            current =
                v8::Local::<v8::Object>::try_from(current.get(scope, property.into())?).ok()?;
        } else {
            let nested = v8::Object::new(scope);
            if current.create_data_property(scope, property.into(), nested.into()) != Some(true) {
                return None;
            }
            current = nested;
        }
    }
    None
}
