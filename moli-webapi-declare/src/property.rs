use moli_v8_util::{
    define_non_enumerable_static_bool_property, define_non_enumerable_static_number_property,
    define_non_enumerable_static_property, define_non_enumerable_static_string_property,
    set_private_value,
};

use crate::{__private, BindError, WebApiValue, v8};

pub fn define_value_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    define_non_enumerable_static_property(scope, object, name, value);
}

pub fn define_enumerable_value_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    let _ = object.create_data_property(scope, __private::v8str(scope, name).into(), value);
}

/// Defines a non-enumerable, string-named own property for internal data.
///
/// Despite the name, this is not a V8 private slot. JavaScript cannot see it
/// through normal enumeration, but reflection that asks for all own property
/// names can still find it. Prefer `define_private_slot` for state that must not
/// be part of the JavaScript property surface.
pub fn define_hidden_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    let _ = object.define_own_property(
        scope,
        __private::v8str(scope, name).into(),
        value,
        v8::PropertyAttribute::DONT_ENUM,
    );
}

pub fn define_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: &str,
) {
    define_non_enumerable_static_string_property(scope, object, name, value);
}

pub fn define_number_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: f64,
) {
    define_non_enumerable_static_number_property(scope, object, name, value);
}

pub fn define_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: bool,
) {
    define_non_enumerable_static_bool_property(scope, object, name, value);
}

/// Stores internal data in a V8 private slot.
///
/// Private slots are the declaration layer's non-reflectable bookkeeping path.
/// They are appropriate for callback state or wrapper ownership data that page
/// scripts must not observe as own properties.
pub fn define_private_slot(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, object, name, value);
}

pub fn define_declared_data_property<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: &V,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    define_declared_data_property_with_attributes(
        scope,
        object,
        name,
        value,
        v8::PropertyAttribute::DONT_ENUM,
    )
}

pub fn define_declared_enumerable_data_property<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: &V,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    define_declared_data_property_with_attributes(
        scope,
        object,
        name,
        value,
        v8::PropertyAttribute::NONE,
    )
}

pub fn define_declared_data_property_with_attributes<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: &V,
    attributes: v8::PropertyAttribute,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    let value = value
        .to_v8_value(scope)
        .ok_or_else(|| BindError::new(format!("failed to convert `{name}` value")))?;
    object
        .define_own_property(
            scope,
            __private::v8str(scope, name).into(),
            value,
            attributes,
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| BindError::new(format!("failed to define `{name}` value")))
}

/// Returns the descriptor flags required for WebIDL constants.
///
/// WebIDL constants are enumerable own data properties that are read-only and
/// non-configurable.
pub fn webidl_constant_property_attributes() -> v8::PropertyAttribute {
    v8::PropertyAttribute::NONE
        | v8::PropertyAttribute::READ_ONLY
        | v8::PropertyAttribute::DONT_DELETE
}

/// Defines a WebIDL constant own data property on an already-created object.
///
/// This is the table-driven counterpart to `#[webapi(constant)]` for dynamic
/// constant lists whose names and values are already represented as Rust data.
pub fn define_declared_constant_property<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: &V,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    define_declared_data_property_with_attributes(
        scope,
        object,
        name,
        value,
        webidl_constant_property_attributes(),
    )
}

/// Defines a declared accessor property with WebIDL-style descriptor flags.
///
/// The derive-generated path passes already-built V8 getter/setter functions
/// here after evaluating any callback data expressions. Function names are set
/// to `get <name>` and `set <name>` for descriptor inspection parity.
pub fn define_declared_accessor_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    getter: v8::Local<'s, v8::Function>,
    setter: Option<v8::Local<'s, v8::Function>>,
    attributes: v8::PropertyAttribute,
) -> Result<(), BindError> {
    let key = __private::v8str(scope, name).into();
    define_declared_accessor_property_by_key(scope, object, key, name, getter, setter, attributes)
}

/// Defines a declared accessor property on an already-resolved V8 property key.
///
/// This is the symbol-capable form used by derive-generated declarations. The
/// separate `display_name` is used only for accessor function names and error
/// messages; the actual property identity is carried by `key`.
pub fn define_declared_accessor_property_by_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
    display_name: &str,
    getter: v8::Local<'s, v8::Function>,
    setter: Option<v8::Local<'s, v8::Function>>,
    attributes: v8::PropertyAttribute,
) -> Result<(), BindError> {
    if let Some(getter_name) = __private::v8_string(scope, &format!("get {display_name}")) {
        getter.set_name(getter_name);
    }
    let setter = if let Some(setter) = setter {
        if let Some(setter_name) = __private::v8_string(scope, &format!("set {display_name}")) {
            setter.set_name(setter_name);
        }
        setter.into()
    } else {
        v8::undefined(scope).into()
    };
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter);
    descriptor.set_enumerable(!attributes.is_dont_enum());
    descriptor.set_configurable(!attributes.is_dont_delete());
    object
        .define_property(scope, key, &descriptor)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| BindError::new(format!("failed to define `{display_name}` accessor")))
}

/// Converts and stores a declared `#[webapi(slot)]` field in a V8 private slot.
///
/// This is the generated-code counterpart of `define_private_slot`. It performs
/// `WebApiValue` conversion first so declaration fields can be plain Rust values
/// or V8 locals.
pub fn define_declared_private_slot<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: &V,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    let value = value
        .to_v8_value(scope)
        .ok_or_else(|| BindError::new(format!("failed to convert `{name}` private slot")))?;
    define_private_slot(scope, object, name, value);
    Ok(())
}

/// Converts and defines a declared `#[webapi(hidden)]` field.
///
/// The resulting value is a non-enumerable, string-named own property. It is
/// suitable for legacy object-local data that already uses string keys, but it
/// is still part of the own-property reflection surface.
pub fn define_declared_hidden_property<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: &V,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    let value = value
        .to_v8_value(scope)
        .ok_or_else(|| BindError::new(format!("failed to convert `{name}` hidden property")))?;
    define_hidden_property(scope, object, name, value);
    Ok(())
}

/// Converts and defines a declared `#[webapi(hidden)]` field with descriptors.
///
/// The property stays non-enumerable, and the supplied descriptor flags preserve
/// declaration-level `readonly` and `dont_delete` semantics. Use
/// `define_declared_private_slot` instead when the state must not be visible via
/// `Object.getOwnPropertyNames`.
pub fn define_declared_hidden_property_with_descriptor<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: &V,
    writable: bool,
    configurable: bool,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    let value = value
        .to_v8_value(scope)
        .ok_or_else(|| BindError::new(format!("failed to convert `{name}` hidden property")))?;
    let mut descriptor = v8::PropertyDescriptor::new_from_value_writable(value, writable);
    descriptor.set_enumerable(false);
    descriptor.set_configurable(configurable);
    object
        .define_property(scope, __private::v8str(scope, name).into(), &descriptor)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| BindError::new(format!("failed to define `{name}` hidden property")))
}

pub fn illegal_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    moli_v8_util::throw_type_error(scope, "Illegal constructor");
}
