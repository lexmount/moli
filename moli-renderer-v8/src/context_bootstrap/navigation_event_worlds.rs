//! Event views for one Navigation dispatch. User callbacks never receive the
//! internal control object, so page expandos cannot cross into another world.

use super::{events, history_runtime::native, shared_event_targets, world_wrappers};

pub(super) fn event_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !shared_event_targets::is_shared_target(scope, target) {
        return Some(event);
    }
    let backing = events::event_backing(scope, event);
    let target = shared_event_targets::target_in_realm(scope, target, context);
    // A callback in another same-world Document still receives the owning
    // Window's wrapper. Its callback realm is entered separately by the invoker.
    let context = target.get_creation_context(scope).unwrap_or(context);
    if let Some(wrapper) = world_wrappers::get(scope, backing, context) {
        return Some(wrapper);
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    events::new_event_wrapper(scope, backing)
}

pub(super) fn attribute_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    property: &str,
    value: v8::Local<'s, v8::Value>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Value>> {
    match property {
        "from" => Some(native::entry_value_in_realm(scope, value, context)),
        "destination" => {
            let destination = v8::Local::<v8::Object>::try_from(value).ok()?;
            let scope = &mut v8::ContextScope::new(scope, context);
            super::navigation_events::navigation_destination_for_realm(scope, destination)
                .map(Into::into)
        }
        "signal" | "formData" | "sourceElement" | "relatedTarget" | "submitter" | "source" => {
            super::platform_object_worlds::in_realm(scope, value, context)
        }
        // info, error, detail, data, reason, and state can contain arbitrary JS
        // values. They are not platform wrappers or storage-serialized payloads.
        _ => Some(value),
    }
}
