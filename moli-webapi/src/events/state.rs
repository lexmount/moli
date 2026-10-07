//! V8-managed Event state shared by payload bindings and the host dispatcher.
//! This module does not retain native hosts or persistent V8 handles.

pub const EVENT_MOUSE_POSITIONLESS_SLOT: &str = "__moliMouseEventPositionless";

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, set_private_value, throw_type_error, v8_string, v8str};
use moli_webapi_declare::WebApiObject;

pub const EVENT_DISPATCHING_SLOT: &str = "__lmDispatching";
pub const EVENT_STOP_PROPAGATION_SLOT: &str = "__lmSp";
pub const EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT: &str = "__lmSip";
pub const EVENT_PASSIVE_SLOT: &str = "__lmPassive";
pub const EVENT_COMPOSED_PATH_SLOT: &str = "__lmCp";
const EVENT_INITIALIZED_SLOT: &str = "__moliEventInitialized";
const EVENT_TRUSTED_PRIVATE_SLOT: &str = "__moliEventTrusted";
const EVENT_TIMESTAMP_PRIVATE_SLOT: &str = "__moliEventTimeStamp";
const EVENT_IS_TRUSTED_GETTER_FUNCTION_SLOT: &str = "__moliEventIsTrustedGetterFunction";
pub const EVENT_BACKING_SLOT: &str = "__moliEventBacking";

pub fn event_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let event = v8::Local::new(scope, event);
    let event = moli_webapi_declare::web_api_object_target(scope, event).unwrap_or(event);
    moli_v8_util::get_private_object(scope, event, EVENT_BACKING_SLOT).unwrap_or(event)
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct InitializedEventHeaderDeclaration<'scope> {
    #[webapi(data_property = "type")]
    event_type: v8::Local<'scope, v8::String>,
    #[webapi(init = "null")]
    target: (),
    #[webapi(init = "null")]
    src_element: (),
    #[webapi(init = "null")]
    current_target: (),
    #[webapi(init = false)]
    default_prevented: (),
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::Event, data_properties, enumerable)]
struct InitializedEventStateDeclaration {
    bubbles: bool,
    cancelable: bool,
    #[webapi(slot = EVENT_STOP_PROPAGATION_SLOT, init = false)]
    stop_propagation: (),
    #[webapi(slot = EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, init = false)]
    stop_immediate_propagation: (),
    #[webapi(slot = EVENT_PASSIVE_SLOT, init = false)]
    passive: (),
    #[webapi(slot = EVENT_TRUSTED_PRIVATE_SLOT, init = false)]
    trusted_slot: (),
    #[webapi(slot = EVENT_INITIALIZED_SLOT, init = true)]
    initialized: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct InitializedEventIsTrustedAccessorDeclaration {
    #[webapi(
        accessor_property,
        getter_value = shared_event_is_trusted_getter_function(scope)?,
        enumerable,
        dont_delete
    )]
    is_trusted: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct InitializedEventTailDeclaration {
    #[webapi(init = false)]
    composed: (),
    #[webapi(init = 0)]
    event_phase: (),
    #[webapi(slot = EVENT_DISPATCHING_SLOT, init = false)]
    dispatching: (),
    #[webapi(slot = EVENT_TIMESTAMP_PRIVATE_SLOT)]
    timestamp_slot: f64,
    #[webapi(slot = EVENT_COMPOSED_PATH_SLOT, init = "array")]
    composed_path: (),
}

pub fn define_event_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    let event = event_backing(scope, event);
    let _ = event.set(scope, v8str(scope, key).into(), value);
}

pub fn event_is_dispatching<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    event_internal_bool_flag(scope, event, EVENT_DISPATCHING_SLOT)
}

pub fn set_event_dispatching(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    dispatching: bool,
) {
    set_event_internal_flag(scope, event, EVENT_DISPATCHING_SLOT, dispatching);
}

pub fn event_initialized<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    let event = event_backing(scope, event);
    get_private_value(scope, event, EVENT_INITIALIZED_SLOT).map(|value| value.boolean_value(scope))
}

pub fn set_event_initialized(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    initialized: bool,
) {
    let event = event_backing(scope, event);
    set_private_value(
        scope,
        event,
        EVENT_INITIALIZED_SLOT,
        v8::Boolean::new(scope, initialized).into(),
    );
}

pub fn set_event_dispatch_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    set_event_dispatch_fields_for_targets(scope, target, target, event);
}

/// Starts an at-target dispatch with a separate target override, such as the
/// Document exposed by a legacy Window event.
pub fn set_event_dispatch_fields_for_targets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    current_target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    let event = event_backing(scope, event);
    let _ = event.set(scope, v8str(scope, "target").into(), target.into());
    let _ = event.set(scope, v8str(scope, "srcElement").into(), target.into());
    let _ = event.set(
        scope,
        v8str(scope, "currentTarget").into(),
        current_target.into(),
    );
    let _ = event.set(
        scope,
        v8str(scope, "eventPhase").into(),
        v8::Integer::new_from_unsigned(scope, 2).into(),
    );
    set_event_dispatching(scope, event, true);
}

pub fn clear_event_dispatch_fields(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) {
    let event = event_backing(scope, event);
    let _ = event.set(
        scope,
        v8str(scope, "currentTarget").into(),
        v8::null(scope).into(),
    );
    let _ = event.set(
        scope,
        v8str(scope, "eventPhase").into(),
        v8::Integer::new_from_unsigned(scope, 0).into(),
    );
    set_event_dispatching(scope, event, false);
}

pub fn event_internal_bool_flag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> bool {
    let event = event_backing(scope, event);
    get_private_value(scope, event, key).is_some_and(|value| value.is_true())
}

pub fn set_event_internal_flag(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: bool,
) {
    let event = event_backing(scope, event);
    set_private_value(scope, event, key, v8::Boolean::new(scope, value).into());
}

pub fn set_event_composed_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    path: v8::Local<'s, v8::Array>,
) {
    let event = event_backing(scope, event);
    set_private_value(scope, event, EVENT_COMPOSED_PATH_SLOT, path.into());
}

pub fn clear_event_composed_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    let path = v8::Array::new(scope, 0);
    set_event_composed_path(scope, event, path);
}

fn event_is_trusted_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_initialized(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let trusted = event_trusted(scope, args.this());
    rv.set(v8::Boolean::new(scope, trusted).into());
}

pub fn event_trusted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    let event = event_backing(scope, event);
    get_private_value(scope, event, EVENT_TRUSTED_PRIVATE_SLOT)
        .map(|value| value.boolean_value(scope))
        .unwrap_or(false)
}

pub fn mark_event_trusted(scope: &mut v8::PinScope<'_, '_>, event: v8::Local<'_, v8::Object>) {
    set_event_trusted(scope, event, true);
}

pub fn set_event_trusted(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    trusted: bool,
) {
    let event = event_backing(scope, event);
    set_private_value(
        scope,
        event,
        EVENT_TRUSTED_PRIVATE_SLOT,
        v8::Boolean::new(scope, trusted).into(),
    );
}

pub fn event_time_stamp<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> f64 {
    let event = event_backing(scope, event);
    get_private_value(scope, event, EVENT_TIMESTAMP_PRIVATE_SLOT)
        .and_then(|value| value.number_value(scope))
        .or_else(|| {
            event
                .get_own_property_descriptor(scope, v8str(scope, "timeStamp").into())
                .and_then(|descriptor| v8::Local::<v8::Object>::try_from(descriptor).ok())
                .and_then(|descriptor| descriptor.get(scope, v8str(scope, "value").into()))
                .and_then(|value| value.number_value(scope))
        })
        .unwrap_or(0.0)
}

/// A derived trusted event is associated with the input that created its source.
pub fn inherit_event_time_stamp<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    derived: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
) {
    let timestamp = event_time_stamp(scope, source);
    let event = event_backing(scope, derived);
    let timestamp = v8::Number::new(scope, timestamp);
    set_private_value(scope, event, EVENT_TIMESTAMP_PRIVATE_SLOT, timestamp.into());
}

pub fn initialize_event_object_with_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: v8::Local<'s, v8::String>,
    bubbles: bool,
    cancelable: bool,
    timestamp: f64,
) {
    let event = event_backing(scope, event);
    initialize_event_attributes(scope, event, event_type, bubbles, cancelable);
    if !event_has_own_property(scope, event, "isTrusted") {
        define_event_is_trusted_accessor(scope, event);
    }
    InitializedEventTailDeclaration::new(timestamp)
        .initialize(scope, event)
        .expect("initialized event tail declaration should initialize");
}

pub fn reinitialize_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: &[u16],
    bubbles: bool,
    cancelable: bool,
) -> bool {
    let event = event_backing(scope, event);
    // WebIDL arguments have already been converted before this method step.
    if event_is_dispatching(scope, event) {
        return false;
    }
    let Some(event_type) =
        v8::String::new_from_two_byte(scope, event_type, v8::NewStringType::Normal)
    else {
        return false;
    };
    // Creation-only state (including composed and timeStamp) is retained.
    initialize_event_attributes(scope, event, event_type, bubbles, cancelable);
    true
}

fn initialize_event_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: v8::Local<'s, v8::String>,
    bubbles: bool,
    cancelable: bool,
) {
    InitializedEventHeaderDeclaration::new(event_type)
        .initialize(scope, event)
        .expect("initialized event header declaration should initialize");
    InitializedEventStateDeclaration::new(bubbles, cancelable)
        .initialize(scope, event)
        .expect("initialized event state declaration should initialize");
}

pub fn define_event_is_trusted_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    InitializedEventIsTrustedAccessorDeclaration::default()
        .initialize(scope, event)
        .expect("initialized event isTrusted accessor declaration should initialize");
}

fn shared_event_is_trusted_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Function>, moli_webapi_declare::BindError> {
    let global = scope.get_current_context().global(scope);
    if let Some(getter) = get_private_value(scope, global, EVENT_IS_TRUSTED_GETTER_FUNCTION_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        return Ok(getter);
    }

    let getter = v8::Function::builder(event_is_trusted_getter_function)
        .length(0)
        .build(scope)
        .ok_or_else(|| {
            moli_webapi_declare::BindError::new("failed to build Event.isTrusted getter")
        })?;
    getter.set_name(v8str(scope, "get isTrusted"));
    set_private_value(
        scope,
        global,
        EVENT_IS_TRUSTED_GETTER_FUNCTION_SLOT,
        getter.into(),
    );
    Ok(getter)
}

fn event_has_own_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    key: &'static str,
) -> bool {
    event
        .has_own_property(scope, v8str(scope, key).into())
        .unwrap_or(false)
}

pub fn new_event_state<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    moli_v8_util::new_null_prototype_object(scope)
}

/// Engine reads bypass public wrappers, including own shadows and getters.
/// Constructors initialize these fields on the private state directly.
pub fn event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
    property: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let state = event_backing(scope, event);
    state.get(scope, v8_string(scope, property)?.into())
}

pub fn event_bool_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
    property: &str,
) -> bool {
    event_attribute(scope, event, property).is_some_and(|value| value.boolean_value(scope))
}

pub fn event_private_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let state = event_backing(scope, event);
    get_private_value(scope, state, slot)
}

pub fn set_event_private_value(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    slot: &str,
    value: v8::Local<'_, v8::Value>,
) {
    let state = event_backing(scope, event);
    set_private_value(scope, state, slot, value);
}

/// Returns a value payload without projecting DOM targets or platform objects.
pub fn event_value_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_initialized(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let state = event_backing(scope, args.this());
    if let Some(value) = state.get(scope, args.data()) {
        rv.set(value);
    }
}
