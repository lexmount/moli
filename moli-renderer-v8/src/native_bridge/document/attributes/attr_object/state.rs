use super::*;
use crate::util::get_private_value;

pub(in crate::native_bridge::document) fn attr_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    attr: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let attr = moli_webapi_declare::web_api_object_target(scope, attr)?;
    get_private_value(scope, attr, ATTR_STATE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}
