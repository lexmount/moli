use super::*;

pub(in crate::context_bootstrap) fn dispatch_media_query_list_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(event_type) = object_string_property_defined(scope, event, "type") else {
        return false;
    };
    dispatch_simple_event_target_event(
        scope,
        target,
        MEDIA_QUERY_LIST_LISTENERS_SLOT,
        &event_type,
        event,
    )
}
