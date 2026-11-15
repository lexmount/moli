pub(crate) use crate::util::{
    define_non_enumerable_static_bool_property as define_non_enumerable_bool_property,
    define_non_enumerable_static_number_property as define_non_enumerable_number_property,
    define_non_enumerable_static_property as define_non_enumerable_value_property,
    define_non_enumerable_static_string_property as define_non_enumerable_string_property,
};
use crate::util::{v8_string, v8str};
use anyhow::{Result, anyhow};

// These helpers intentionally take `&'static str` for fixed runtime-visible
// names so we can route through `v8str`, keep keys internalized, and avoid the
// dynamic-string allocation path during bootstrap/property installation.
pub(crate) fn define_get_set_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: v8::Local<'_, v8::Name>,
    getter: v8::Local<'_, v8::Value>,
    setter: v8::Local<'_, v8::Value>,
    attributes: v8::PropertyAttribute,
    label: &'static str,
) -> Result<()> {
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter, setter);
    descriptor.set_configurable(!attributes.is_dont_delete());
    descriptor.set_enumerable(!attributes.is_dont_enum());
    object
        .define_property(scope, key, &descriptor)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to define accessor `{label}`"))?;
    Ok(())
}

pub(crate) fn define_global_value(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) -> Result<()> {
    let key = v8str(scope, name);
    global
        .define_own_property(scope, key.into(), value, v8::PropertyAttribute::DONT_ENUM)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to define global `{name}`"))?;
    Ok(())
}

pub(crate) fn define_global_template_value(
    scope: &mut v8::PinScope<'_, '_, ()>,
    global: v8::Local<'_, v8::ObjectTemplate>,
    name: &'static str,
    value: v8::Local<'_, v8::Data>,
) -> Result<()> {
    let key = v8str(scope, name);
    global.set_with_attr(key.into(), value, v8::PropertyAttribute::DONT_ENUM);
    Ok(())
}

pub(crate) fn define_function_accessor_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    getter_callback: impl v8::MapFnTo<v8::FunctionCallback>,
    getter_data: Option<v8::Local<'_, v8::Value>>,
    setter_callback: impl v8::MapFnTo<v8::FunctionCallback>,
    setter_data: Option<v8::Local<'_, v8::Value>>,
    attributes: v8::PropertyAttribute,
) -> Result<()> {
    let key = v8str(scope, name);
    let getter = v8::Function::builder(getter_callback)
        .data(getter_data.unwrap_or_else(|| v8::undefined(scope).into()))
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build `{name}` getter"))?;
    let getter_name = v8_string(scope, &format!("get {name}"))
        .ok_or_else(|| anyhow!("failed to allocate getter name for `{name}`"))?;
    getter.set_name(getter_name);
    let setter = v8::Function::builder(setter_callback)
        .data(setter_data.unwrap_or_else(|| v8::undefined(scope).into()))
        .length(1)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build `{name}` setter"))?;
    let setter_name = v8_string(scope, &format!("set {name}"))
        .ok_or_else(|| anyhow!("failed to allocate setter name for `{name}`"))?;
    setter.set_name(setter_name);
    define_get_set_property(
        scope,
        object,
        key.into(),
        getter.into(),
        setter.into(),
        attributes,
        name,
    )
}
