use crate::context_bootstrap::{events::modifiers::EventModifierInit, touch_runtime};
use crate::web_api_interfaces;

pub(crate) fn construct_native_touch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    touches: Vec<v8::Local<'s, v8::Object>>,
    target_touches: Vec<v8::Local<'s, v8::Object>>,
    changed_touches: Vec<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    use super::super::{
        EVENT_SUBCLASS_KIND_SLOT, define_event_property, initialize_event_object,
        mark_event_trusted, new_event_state, new_event_wrapper,
    };
    let state = new_event_state(scope);
    initialize_event_object(scope, state, event_type, true, true);
    let composed = v8::Boolean::new(scope, true);
    define_event_property(scope, state, "composed", composed.into());
    EventModifierInit::default().initialize(scope, state);
    touch_runtime::initialize_touch_event_lists(
        scope, state, &touches, &target_touches, &changed_touches,
    );
    web_api_interfaces::initialize(scope, state, web_api_interfaces::TouchEvent::NAME).ok()?;
    let kind = v8::Integer::new(scope, super::EventSubclassKind::TouchEvent as i32);
    crate::util::set_private_value(scope, state, EVENT_SUBCLASS_KIND_SLOT, kind.into());
    let event = new_event_wrapper(scope, state)?;
    mark_event_trusted(scope, event);
    Some(event)
}
