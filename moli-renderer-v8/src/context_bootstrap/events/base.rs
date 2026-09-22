use super::init::read_event_init;
use super::*;
use crate::context_bootstrap::{current_performance_time_origin, dom_time_since_origin_millis};
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

pub(crate) const EVENT_DISPATCHING_SLOT: &str = "__lmDispatching";
pub(crate) const EVENT_STOP_PROPAGATION_SLOT: &str = "__lmSp";
pub(crate) const EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT: &str = "__lmSip";
pub(crate) const EVENT_PASSIVE_SLOT: &str = "__lmPassive";
pub(crate) const EVENT_COMPOSED_PATH_SLOT: &str = "__lmCp";
const EVENT_INITIALIZED_SLOT: &str = "__moliEventInitialized";
const EVENT_TRUSTED_PRIVATE_SLOT: &str = "__moliEventTrusted";
const EVENT_TIMESTAMP_PRIVATE_SLOT: &str = "__moliEventTimeStamp";
const EVENT_IS_TRUSTED_GETTER_FUNCTION_SLOT: &str = "__moliEventIsTrustedGetterFunction";
pub(super) const EVENT_BACKING_SLOT: &str = "__moliEventBacking";

pub(crate) fn event_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let event = v8::Local::new(scope, event);
    crate::util::get_private_object(scope, event, EVENT_BACKING_SLOT).unwrap_or(event)
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

pub(in crate::context_bootstrap) fn event_type_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor_name: &'static str,
) -> Option<String> {
    if args.length() == 0 {
        throw_type_error(
            scope,
            &format!("Failed to construct '{constructor_name}': 1 argument required."),
        );
        return None;
    }
    webidl::argument::<webidl::DomString>(
        scope,
        args,
        0,
        webidl::Context::argument(constructor_name, 1),
    )
    .map(Into::into)
    .map_or_else(
        |error| {
            webidl::throw_error(scope, &error);
            None
        },
        Some,
    )
}

pub(crate) fn define_event_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    let event = event_backing(scope, event);
    let _ = event.set(scope, v8str(scope, key).into(), value);
}

pub(crate) fn event_is_dispatching<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    event_internal_bool_flag(scope, event, EVENT_DISPATCHING_SLOT)
}

pub(in crate::context_bootstrap) fn set_event_dispatching(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    dispatching: bool,
) {
    set_event_internal_flag(scope, event, EVENT_DISPATCHING_SLOT, dispatching);
}

pub(crate) fn event_initialized<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    let event = event_backing(scope, event);
    get_private_value(scope, event, EVENT_INITIALIZED_SLOT).map(|value| value.boolean_value(scope))
}

pub(in crate::context_bootstrap) fn set_event_initialized(
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

pub(in crate::context_bootstrap) fn set_event_dispatch_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    set_event_dispatch_fields_with_original_target(scope, target, target, event);
}

pub(in crate::context_bootstrap) fn set_event_dispatch_fields_with_original_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    original_target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    let event = event_backing(scope, event);
    let _ = event.set(scope, v8str(scope, "target").into(), original_target.into());
    let _ = event.set(
        scope,
        v8str(scope, "srcElement").into(),
        original_target.into(),
    );
    let _ = event.set(scope, v8str(scope, "currentTarget").into(), target.into());
    let _ = event.set(
        scope,
        v8str(scope, "eventPhase").into(),
        v8::Integer::new_from_unsigned(scope, 2).into(),
    );
    set_event_dispatching(scope, event, true);
}

pub(in crate::context_bootstrap) fn clear_event_dispatch_fields(
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

pub(crate) fn event_internal_bool_flag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> bool {
    let event = event_backing(scope, event);
    get_private_value(scope, event, key).is_some_and(|value| value.is_true())
}

pub(crate) fn set_event_internal_flag(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: bool,
) {
    let event = event_backing(scope, event);
    set_private_value(scope, event, key, v8::Boolean::new(scope, value).into());
}

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
                    crate::context_bootstrap::shared_event_targets::target_in_realm(
                        scope, target, context,
                    )
                    .into()
                } else {
                    value
                };
                values.push(value);
            }
        }
    }
    v8::Array::new_with_elements(scope, &values).into()
}

pub(crate) fn set_event_composed_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    path: v8::Local<'s, v8::Array>,
) {
    let event = event_backing(scope, event);
    set_private_value(scope, event, EVENT_COMPOSED_PATH_SLOT, path.into());
}

pub(crate) fn clear_event_composed_path<'s>(
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

pub(in crate::context_bootstrap) fn event_trusted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    let event = event_backing(scope, event);
    get_private_value(scope, event, EVENT_TRUSTED_PRIVATE_SLOT)
        .map(|value| value.boolean_value(scope))
        .unwrap_or(false)
}

pub(crate) fn mark_event_trusted(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) {
    set_event_trusted(scope, event, true);
}

pub(crate) fn set_event_trusted(
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

fn event_timestamp_millis(scope: &mut v8::PinScope<'_, '_>) -> f64 {
    dom_time_since_origin_millis(current_performance_time_origin(scope))
}

pub(super) fn event_time_stamp<'s>(
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

pub(crate) fn initialize_event_object_with_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: v8::Local<'s, v8::String>,
    bubbles: bool,
    cancelable: bool,
) {
    let event = event_backing(scope, event);
    InitializedEventHeaderDeclaration::new(event_type)
        .initialize(scope, event)
        .expect("initialized event header declaration should initialize");
    initialize_event_after_header(scope, event, bubbles, cancelable);
}

fn initialize_event_after_header<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    bubbles: bool,
    cancelable: bool,
) {
    InitializedEventStateDeclaration::new(bubbles, cancelable)
        .initialize(scope, event)
        .expect("initialized event state declaration should initialize");
    if !event_has_own_property(scope, event, "isTrusted") {
        define_event_is_trusted_accessor(scope, event);
    }
    initialized_event_tail_declaration(scope)
        .initialize(scope, event)
        .expect("initialized event tail declaration should initialize");
}

pub(super) fn define_event_is_trusted_accessor<'s>(
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

fn initialized_event_tail_declaration(
    scope: &mut v8::PinScope<'_, '_>,
) -> InitializedEventTailDeclaration {
    let timestamp_slot = event_timestamp_millis(scope);
    InitializedEventTailDeclaration::new(timestamp_slot)
}

pub(in crate::context_bootstrap) fn event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Event': Please use the 'new' operator.",
        );
        return;
    }

    let wrapper = args.this();
    let event = new_event_state(scope);
    let Some(event_type) = event_type_argument(scope, &args, "Event") else {
        return;
    };

    let (bubbles, cancelable, composed) = read_event_init(scope, &args);
    initialize_event_object(scope, event, &event_type, bubbles, cancelable);
    define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, composed).into(),
    );

    if initialize_event_wrapper(scope, wrapper, event).is_some() {
        rv.set(wrapper.into());
    }
}
