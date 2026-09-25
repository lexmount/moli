//! Event views for one Navigation dispatch. User callbacks never receive the
//! internal control object, so page expandos cannot cross into another world.

use super::{events, history_runtime::native, shared_event_targets};
use crate::util::{get_private_value, set_private_value, v8str};

const WRAPPERS: &str = "__moliNavigationEventWrappers";

pub(super) fn event_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    event_type: &str,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    // Script-created events retain their dispatchEvent identity. This adapter
    // handles the UA events belonging to the shared Navigation transaction.
    if !shared_event_targets::is_shared_target(scope, target)
        || !events::event_trusted(scope, event)
    {
        return Some(event);
    }
    let target = shared_event_targets::target_in_realm(scope, target, context);
    // A callback in another same-world Document still receives the owning
    // Window's wrapper. Its callback realm is entered separately by the invoker.
    let context = target.get_creation_context(scope).unwrap_or(context);
    let wrappers = get_private_value(scope, event, WRAPPERS)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .unwrap_or_else(|| {
            let map = v8::Map::new(scope);
            set_private_value(scope, event, WRAPPERS, map.into());
            map
        });
    let global = context.global(scope);
    if let Some(wrapper) = wrappers
        .get(scope, global.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(wrapper);
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let init = crate::util::new_null_prototype_object(scope);
    let (interface, properties): (&str, &[&'static str]) = match event_type {
        "navigate" => (
            "NavigateEvent",
            &[
                "navigationType",
                "destination",
                "canIntercept",
                "userInitiated",
                "hashChange",
                "signal",
                "formData",
                "downloadRequest",
                "info",
                "sourceElement",
                "hasUAVisualTransition",
            ],
        ),
        "currententrychange" => (
            "NavigationCurrentEntryChangeEvent",
            &["from", "navigationType"],
        ),
        "navigateerror" => (
            "ErrorEvent",
            &["message", "filename", "lineno", "colno", "error"],
        ),
        _ => ("Event", &[]),
    };
    for property in properties
        .iter()
        .copied()
        .chain(["bubbles", "cancelable", "composed"])
    {
        let mut value = event.get(scope, v8str(scope, property).into())?;
        if property == "from" {
            if let Ok(entry) = v8::Local::<v8::Object>::try_from(value) {
                value = native::entry_in_realm(scope, entry, context).into();
            }
        } else if property == "destination" {
            let destination = v8::Local::<v8::Object>::try_from(value).ok()?;
            value = super::navigation_events::navigation_destination_for_realm(scope, destination)?
                .into();
        }
        let _ = init.set(scope, v8str(scope, property).into(), value);
    }
    let constructor =
        super::exposed_interfaces::ensure_intrinsic_interface_constructor(scope, interface).ok()?;
    let event_type = crate::util::v8_string(scope, event_type)?;
    let wrapper = constructor.new_instance(scope, &[event_type.into(), init.into()])?;
    for property in ["target", "srcElement"] {
        let _ = wrapper.set(scope, v8str(scope, property).into(), target.into());
    }
    events::bind_event_backing(scope, wrapper, event);
    let _ = wrappers.set(scope, global.into(), wrapper.into());
    Some(wrapper)
}
