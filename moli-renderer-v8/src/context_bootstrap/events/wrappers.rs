//! World-local Event wrappers over a private, V8-managed state object.
//!
//! The state is never passed to script. Keeping its JS values in the V8 heap
//! lets the collector trace cycles such as wrapper -> state -> detail -> wrapper.
//! Rust runs the dispatch algorithms; V8 owns the state and its lifetime.
//! The world cache contains weak wrappers, not persistent roots for that graph.

use super::*;
use crate::context_bootstrap::{exposed_interfaces, world_wrappers};

const EVENT_ATTRIBUTE_GETTERS_SLOT: &str = "__moliEventAttributeGetters";

// The callback carries the original V8 key. Its projection is chosen once when
// the realm's getter is created, so reads never convert field names to Rust.
#[derive(Clone, Copy)]
enum AttributeProjection {
    Value,
    Target,
    PlatformObject,
    NavigationEntry,
    NavigationDestination,
}

impl AttributeProjection {
    fn for_property(property: &str) -> Self {
        match property {
            "target" | "srcElement" | "currentTarget" => Self::Target,
            "signal" | "formData" | "sourceElement" | "relatedTarget" | "submitter" | "source" => {
                Self::PlatformObject
            }
            "from" => Self::NavigationEntry,
            "destination" => Self::NavigationDestination,
            // Arbitrary JS payloads (detail, data, reason, info, error, state)
            // retain their identity; they are not platform-object projections.
            _ => Self::Value,
        }
    }

    fn project<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        wrapper: v8::Local<'s, v8::Object>,
        value: v8::Local<'s, v8::Value>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        if matches!(self, Self::Value) {
            return Some(value);
        }
        let context = wrapper.get_creation_context(scope)?;
        match self {
            Self::Value => Some(value),
            Self::Target => Some(match v8::Local::<v8::Object>::try_from(value) {
                Ok(target) => crate::context_bootstrap::shared_event_targets::target_in_realm(
                    scope, target, context,
                )
                .into(),
                Err(_) => value,
            }),
            Self::PlatformObject => {
                crate::context_bootstrap::platform_object_worlds::in_realm(scope, value, context)
            }
            Self::NavigationEntry => Some(
                crate::context_bootstrap::history_runtime::native::entry_value_in_realm(
                    scope, value, context,
                ),
            ),
            Self::NavigationDestination => {
                let destination = v8::Local::<v8::Object>::try_from(value).ok()?;
                let scope = &mut v8::ContextScope::new(scope, context);
                crate::context_bootstrap::navigation_events::navigation_destination_for_realm(
                    scope,
                    destination,
                )
                .map(Into::into)
            }
        }
    }
}

pub(crate) fn new_event_state<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    crate::util::new_null_prototype_object(scope)
}

/// Bind a fresh wrapper without invoking a public constructor or inspecting an
/// author's properties. The original constructor's `this` remains its wrapper.
pub(crate) fn initialize_event_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let interface = moli_webapi_declare::web_api_object_type(scope, state)?.name();
    crate::web_api_interfaces::initialize(scope, wrapper, interface).ok()?;
    set_private_value(
        scope,
        wrapper,
        super::base::EVENT_BACKING_SLOT,
        state.into(),
    );
    // Keep the cache on the existing intrinsic, in V8's heap. It contains only
    // field getters, never an instance/backing or a native persistent root.
    let constructor =
        exposed_interfaces::ensure_intrinsic_interface_constructor(scope, "Event").ok()?;
    let getters =
        crate::util::get_private_object(scope, constructor.into(), EVENT_ATTRIBUTE_GETTERS_SLOT)
            .unwrap_or_else(|| {
                let getters = crate::util::new_null_prototype_object(scope);
                set_private_value(
                    scope,
                    constructor.into(),
                    EVENT_ATTRIBUTE_GETTERS_SLOT,
                    getters.into(),
                );
                getters
            });
    let trusted_key = v8str(scope, "isTrusted");
    // Some native events and optional dictionary members have different own
    // fields. Preserve the actual state shape and descriptor order.
    let names = state.get_own_property_names(
        scope,
        v8::GetPropertyNamesArgs {
            property_filter: v8::PropertyFilter::ALL_PROPERTIES | v8::PropertyFilter::SKIP_SYMBOLS,
            ..Default::default()
        },
    )?;
    for index in 0..names.length() {
        let property = names.get_index(scope, index)?;
        if property.strict_equals(trusted_key.into()) {
            super::base::define_event_is_trusted_accessor(scope, wrapper);
        } else {
            let attributes = state.get_property_attributes(scope, property)?;
            bind_attribute(scope, getters, wrapper, property, attributes)?;
        }
    }
    if interface == "NavigateEvent" {
        super::subclasses::initialize_navigate_event_methods(scope, wrapper);
    }
    world_wrappers::insert(scope, state, wrapper);
    Some(())
}

pub(crate) fn new_event_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = event_backing(scope, state);
    let interface = moli_webapi_declare::web_api_object_type(scope, state)?.name();
    let prototype =
        exposed_interfaces::ensure_intrinsic_interface_prototype(scope, interface).ok()?;
    let wrapper = v8::Object::new(scope);
    if wrapper.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    initialize_event_wrapper(scope, wrapper, state)?;
    Some(wrapper)
}

fn bind_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    getters: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
    property: v8::Local<'s, v8::Value>,
    attributes: v8::PropertyAttribute,
) -> Option<()> {
    let name = v8::Local::<v8::Name>::try_from(property).ok()?;
    let getter = if let Some(getter) = getters
        .get(scope, property)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        getter
    } else {
        let field = property.to_rust_string_lossy(scope);
        let getter = match AttributeProjection::for_property(&field) {
            AttributeProjection::Value => v8::Function::builder(event_attribute_getter::<0>)
                .data(property)
                .build(scope),
            AttributeProjection::Target => v8::Function::builder(event_attribute_getter::<1>)
                .data(property)
                .build(scope),
            AttributeProjection::PlatformObject => {
                v8::Function::builder(event_attribute_getter::<2>)
                    .data(property)
                    .build(scope)
            }
            AttributeProjection::NavigationEntry => {
                v8::Function::builder(event_attribute_getter::<3>)
                    .data(property)
                    .build(scope)
            }
            AttributeProjection::NavigationDestination => {
                v8::Function::builder(event_attribute_getter::<4>)
                    .data(property)
                    .build(scope)
            }
        }?;
        getter.set_name(v8_string(scope, &format!("get {field}"))?);
        getters.create_data_property(scope, name, getter.into())?;
        getter
    };
    let mut accessor_attributes = v8::PropertyAttribute::NONE;
    if attributes.is_dont_enum() {
        accessor_attributes = accessor_attributes | v8::PropertyAttribute::DONT_ENUM;
    }
    if attributes.is_dont_delete() {
        accessor_attributes = accessor_attributes | v8::PropertyAttribute::DONT_DELETE;
    }
    crate::definitions::define_get_set_property(
        scope,
        wrapper,
        name,
        getter.into(),
        v8::undefined(scope).into(),
        accessor_attributes,
        "Event attribute",
    )
    .ok()
}

fn event_attribute_getter<'s, const PROJECTION: u8>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_initialized(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let state = event_backing(scope, args.this());
    let Some(value) = state.get(scope, args.data()) else {
        return;
    };
    let projection = match PROJECTION {
        0 => AttributeProjection::Value,
        1 => AttributeProjection::Target,
        2 => AttributeProjection::PlatformObject,
        3 => AttributeProjection::NavigationEntry,
        4 => AttributeProjection::NavigationDestination,
        _ => unreachable!("Event getter projection"),
    };
    if let Some(value) = projection.project(scope, args.this(), value) {
        rv.set(value);
    }
}

pub(super) fn event_attribute_in_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = event_attribute(scope, wrapper, property)?;
    AttributeProjection::for_property(property).project(scope, wrapper, value)
}

/// Engine reads must bypass the public wrapper, including own shadows and
/// getters. Constructors initialize these fields on the private state directly.
pub(crate) fn event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
    property: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let state = event_backing(scope, event);
    // CommandEvent and ToggleEvent expose source through prototype accessors;
    // their native state holds the value in a private slot, not an own field.
    if property == "source" {
        match event_subclass_kind(scope, state) {
            Some(EventSubclassKind::CommandEvent) => {
                return get_private_value(scope, state, COMMAND_EVENT_SOURCE_SLOT);
            }
            Some(EventSubclassKind::ToggleEvent) => {
                return get_private_value(scope, state, TOGGLE_EVENT_SOURCE_SLOT);
            }
            _ => {}
        }
    }
    state.get(scope, v8_string(scope, property)?.into())
}

pub(crate) fn event_bool_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
    property: &str,
) -> bool {
    event_attribute(scope, event, property).is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn event_private_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let state = event_backing(scope, event);
    get_private_value(scope, state, slot)
}

pub(crate) fn set_event_private_value(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    slot: &str,
    value: v8::Local<'_, v8::Value>,
) {
    let state = event_backing(scope, event);
    set_private_value(scope, state, slot, value);
}
