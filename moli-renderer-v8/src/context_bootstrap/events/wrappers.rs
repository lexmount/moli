//! World-local Event wrappers over a private, V8-managed state object.
//!
//! The state is never passed to script. Keeping its JS values in the V8 heap
//! lets the collector trace cycles such as wrapper -> state -> detail -> wrapper.
//! Rust runs the dispatch algorithms; V8 owns the state and its lifetime.
//! The world cache contains weak wrappers, not persistent roots for that graph.

use super::*;
use crate::context_bootstrap::{exposed_interfaces, world_wrappers};

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
    let names = state.get_own_property_names(
        scope,
        v8::GetPropertyNamesArgs {
            property_filter: v8::PropertyFilter::ALL_PROPERTIES | v8::PropertyFilter::SKIP_SYMBOLS,
            ..Default::default()
        },
    )?;
    for index in 0..names.length() {
        let property = names.get_index(scope, index)?;
        if property.to_rust_string_lossy(scope) == "isTrusted" {
            super::base::define_event_is_trusted_accessor(scope, wrapper);
        } else {
            let attributes = state.get_property_attributes(scope, property)?;
            bind_attribute(scope, wrapper, property, attributes)?;
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
    wrapper: v8::Local<'s, v8::Object>,
    property: v8::Local<'s, v8::Value>,
    attributes: v8::PropertyAttribute,
) -> Option<()> {
    let name = v8::Local::<v8::Name>::try_from(property).ok()?;
    let getter = v8::Function::builder(event_attribute_getter)
        .data(property)
        .build(scope)?;
    let getter_name = format!("get {}", property.to_rust_string_lossy(scope));
    getter.set_name(v8_string(scope, &getter_name)?);
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

fn event_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_initialized(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let property = args.data().to_rust_string_lossy(scope);
    if let Some(value) = event_attribute_in_wrapper(scope, args.this(), &property) {
        rv.set(value);
    }
}

pub(super) fn event_attribute_in_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = event_attribute(scope, wrapper, property)?;
    let context = wrapper.get_creation_context(scope)?;
    if matches!(property, "target" | "srcElement" | "currentTarget")
        && let Ok(target) = v8::Local::<v8::Object>::try_from(value)
    {
        return Some(
            crate::context_bootstrap::shared_event_targets::target_in_realm(scope, target, context)
                .into(),
        );
    }
    crate::context_bootstrap::navigation_event_worlds::attribute_in_realm(
        scope, property, value, context,
    )
}

/// Engine reads must bypass the public wrapper, including own shadows and
/// getters. Constructors initialize these fields on the private state directly.
pub(crate) fn event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'_, v8::Object>,
    property: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let state = event_backing(scope, event);
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
