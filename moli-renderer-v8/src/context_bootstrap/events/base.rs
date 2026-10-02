use super::*;
use crate::context_bootstrap::{current_performance_time_origin, dom_time_since_origin_millis};
use crate::util::get_private_value;
pub(in crate::context_bootstrap) use moli_webapi::events::init::event_type_argument;

pub(crate) fn event_composed_path_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let backing = event_backing(scope, event);
    let path = get_private_value(scope, backing, EVENT_COMPOSED_PATH_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok());
    let context = event.get_creation_context(scope);
    let mut values = Vec::new();
    if let Some(path) = path {
        for index in 0..path.length() {
            if let Some(value) = path.get_index(scope, index) {
                let value = if let (Ok(target), Some(context)) =
                    (v8::Local::<v8::Object>::try_from(value), context)
                {
                    crate::context_bootstrap::event_worlds::target_in_realm(scope, target, context)
                        .map(Into::into)
                        .unwrap_or(value)
                } else {
                    value
                };
                values.push(value);
            }
        }
    }
    v8::Array::new_with_elements(scope, &values).into()
}

fn event_timestamp_millis(scope: &mut v8::PinScope<'_, '_>) -> f64 {
    dom_time_since_origin_millis(current_performance_time_origin(scope))
}

pub(crate) fn initialize_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
) {
    let event_type = v8_string(scope, event_type).expect("event type value");
    initialize_event_object_with_type(scope, event, event_type, bubbles, cancelable);
}

fn event_core_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    key: &'static str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_initialized(scope, receiver).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }

    if let Some(value) = super::wrappers::event_attribute_in_wrapper(scope, receiver, key) {
        rv.set(value);
    }
}

macro_rules! define_event_core_attribute_getter {
    ($function:ident, $property:literal) => {
        pub(in crate::context_bootstrap) fn $function<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            event_core_attribute_getter(scope, args.this(), $property, rv);
        }
    };
}

define_event_core_attribute_getter!(event_type_getter_function, "type");
define_event_core_attribute_getter!(event_target_getter_function, "target");
define_event_core_attribute_getter!(event_current_target_getter_function, "currentTarget");
define_event_core_attribute_getter!(event_event_phase_getter_function, "eventPhase");
define_event_core_attribute_getter!(event_bubbles_getter_function, "bubbles");
define_event_core_attribute_getter!(event_cancelable_getter_function, "cancelable");
define_event_core_attribute_getter!(event_default_prevented_getter_function, "defaultPrevented");
define_event_core_attribute_getter!(event_composed_getter_function, "composed");
define_event_core_attribute_getter!(event_src_element_getter_function, "srcElement");

pub(in crate::context_bootstrap) use moli_webapi::events::event_constructor_callback;

pub(crate) use moli_webapi::events::state::{
    EVENT_BACKING_SLOT, EVENT_COMPOSED_PATH_SLOT, EVENT_DISPATCHING_SLOT,
    EVENT_MOUSE_POSITIONLESS_SLOT, EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT,
    EVENT_STOP_PROPAGATION_SLOT, clear_event_composed_path, define_event_is_trusted_accessor,
    define_event_property, event_backing, event_initialized, event_internal_bool_flag,
    event_is_dispatching, event_time_stamp, event_trusted, mark_event_trusted,
    set_event_composed_path, set_event_initialized, set_event_internal_flag, set_event_trusted,
};

pub(crate) fn initialize_event_object_with_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: v8::Local<'s, v8::String>,
    bubbles: bool,
    cancelable: bool,
) {
    let timestamp = event_timestamp_millis(scope);
    moli_webapi::events::state::initialize_event_object_with_type(
        scope, event, event_type, bubbles, cancelable, timestamp,
    );
}
