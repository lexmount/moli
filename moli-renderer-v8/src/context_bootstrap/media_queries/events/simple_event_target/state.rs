use super::*;
use crate::util::get_private_value;

pub(crate) fn simple_event_target_slot_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let value = simple_event_target_private_value(scope, target, SIMPLE_EVENT_TARGET_SLOT)?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(in crate::context_bootstrap::media_queries::events::simple_event_target) fn simple_event_target_private_value<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let target = moli_webapi_declare::web_api_object_target(scope, target).unwrap_or(target);
    let mut current = Some(target);
    for _ in 0..64 {
        let object = current?;
        if let Some(value) = get_private_value(scope, object, slot) {
            return Some(value);
        }
        let prototype = object.get_prototype(scope)?;
        if prototype.is_null_or_undefined() {
            return None;
        }
        current = v8::Local::<v8::Object>::try_from(prototype).ok();
    }
    None
}
