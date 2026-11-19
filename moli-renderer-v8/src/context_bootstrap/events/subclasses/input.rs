use super::*;
use crate::context_bootstrap::range;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use moli_webapi::events::subclasses::InputEventInit;

const INPUT_EVENT_TARGET_RANGES_SLOT: &str = "__moliInputEventTargetRanges";

pub(crate) fn clear_input_event_target_ranges<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    let event = event_backing(scope, event);
    if web_api_interfaces::InputEvent::is_instance(scope, event) {
        set_private_value(
            scope,
            event,
            INPUT_EVENT_TARGET_RANGES_SLOT,
            v8::undefined(scope).into(),
        );
    }
}

pub(super) fn initialize_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: InputEventInit<'s>,
) -> bool {
    if !init.values.initialize(scope, event) {
        return false;
    }
    let ranges = init
        .target_ranges
        .into_iter()
        .map(|range| range::live_range_from_static_range(scope, range).map(Into::into))
        .collect::<Option<Vec<_>>>();
    let Some(ranges) = ranges else {
        return false;
    };
    let array = v8::Array::new_with_elements(scope, &ranges);
    set_private_value(scope, event, INPUT_EVENT_TARGET_RANGES_SLOT, array.into());
    true
}

pub(in crate::context_bootstrap) fn input_event_get_target_ranges_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = event_backing(scope, args.this());
    let mut snapshots = Vec::new();
    if let Some(ranges) = get_private_value(scope, event, INPUT_EVENT_TARGET_RANGES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        snapshots.reserve(ranges.length() as usize);
        for index in 0..ranges.length() {
            let Some(range) = ranges
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            else {
                return;
            };
            let Some(snapshot) = range::static_range_snapshot(scope, range) else {
                return;
            };
            snapshots.push(snapshot.into());
        }
    }
    rv.set(v8::Array::new_with_elements(scope, &snapshots).into());
}
