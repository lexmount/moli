pub(crate) use moli_webidl::*;

pub(crate) fn dom_string_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) -> Option<String> {
    match convert::<DomString>(scope, value, Context::member(owner, property)) {
        Ok(value) => Some(value.0),
        Err(error) => {
            throw_error(scope, &error);
            None
        }
    }
}

pub(crate) fn finite_double_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) -> Option<f64> {
    match convert::<Double>(scope, value, Context::member(owner, property)) {
        Ok(value) => Some(value.0),
        Err(error) => {
            throw_error(scope, &error);
            None
        }
    }
}
