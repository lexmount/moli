use moli_v8_util::{define_static_symbol_to_string_tag, global_constructor_prototype};

use crate::{__private, BindError, WebApiValue, v8};

pub fn set_interface_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
) -> bool {
    if interface == "Object" {
        return true;
    }
    if let Some(prototype) = global_constructor_prototype(scope, interface) {
        object
            .set_prototype(scope, prototype.into())
            .unwrap_or(false)
    } else {
        false
    }
}

pub fn set_required_interface_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
) -> Result<(), BindError> {
    if interface == "Object" {
        return Ok(());
    }
    let prototype = global_constructor_prototype(scope, interface)
        .ok_or_else(|| BindError::new(format!("missing `{interface}` prototype")))?;
    let installed = object
        .set_prototype(scope, prototype.into())
        .unwrap_or(false);
    if !installed {
        return Err(BindError::new(format!(
            "failed to set `{interface}` prototype"
        )));
    }
    Ok(())
}

pub fn define_to_string_tag(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    tag: &'static str,
) {
    define_to_string_tag_with_attributes(scope, object, tag, v8::PropertyAttribute::DONT_ENUM);
}

pub fn define_to_string_tag_with_attributes(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    tag: &'static str,
    attributes: v8::PropertyAttribute,
) {
    define_static_symbol_to_string_tag(scope, object, tag, attributes);
}

pub fn set_declared_prototype<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    prototype: &V,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    let prototype = prototype
        .to_v8_value(scope)
        .ok_or_else(|| BindError::new("failed to convert declared prototype"))?;
    let prototype = v8::Local::<v8::Object>::try_from(prototype)
        .map_err(|_| BindError::new("declared prototype must be an object"))?;
    let installed = object
        .set_prototype(scope, prototype.into())
        .unwrap_or(false);
    if !installed {
        return Err(BindError::new("failed to set declared prototype"));
    }
    Ok(())
}

pub fn define_declared_to_string_tag<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    tag: &V,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    define_declared_to_string_tag_with_attributes(
        scope,
        object,
        tag,
        v8::PropertyAttribute::DONT_ENUM,
    )
}

pub fn define_declared_to_string_tag_with_attributes<'s, V>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    tag: &V,
    attributes: v8::PropertyAttribute,
) -> Result<(), BindError>
where
    V: WebApiValue<'s> + ?Sized,
{
    let tag = tag
        .to_v8_value(scope)
        .ok_or_else(|| BindError::new("failed to convert declared toStringTag"))?;
    let tag = tag
        .to_string(scope)
        .ok_or_else(|| BindError::new("failed to stringify declared toStringTag"))?;
    object
        .define_own_property(
            scope,
            v8::Symbol::get_to_string_tag(scope).into(),
            tag.into(),
            attributes,
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| BindError::new("failed to define declared toStringTag"))
}

pub fn define_interface_prototype_property(
    scope: &mut v8::PinScope<'_, '_>,
    constructor: v8::Local<'_, v8::Function>,
    prototype: v8::Local<'_, v8::Object>,
) -> Result<(), BindError> {
    let mut descriptor = v8::PropertyDescriptor::new_from_value_writable(prototype.into(), false);
    descriptor.set_configurable(false);
    descriptor.set_enumerable(false);
    constructor
        .define_property(
            scope,
            __private::v8str(scope, "prototype").into(),
            &descriptor,
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| BindError::new("failed to define interface prototype property"))
}

pub fn define_interface_constructor_property(
    scope: &mut v8::PinScope<'_, '_>,
    prototype: v8::Local<'_, v8::Object>,
    constructor: v8::Local<'_, v8::Function>,
) -> Result<(), BindError> {
    let mut descriptor = v8::PropertyDescriptor::new_from_value_writable(constructor.into(), true);
    descriptor.set_configurable(true);
    descriptor.set_enumerable(false);
    prototype
        .define_property(
            scope,
            __private::v8str(scope, "constructor").into(),
            &descriptor,
        )
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| BindError::new("failed to define interface constructor property"))
}
