use super::*;
use crate::dom::native::DomStringValue;
use crate::util::v8_string_from_utf16_units;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Attr, enumerable, receiver)]
struct AttrPrototypeDeclaration {
    #[webapi(accessor_property = "namespaceURI", getter = attr_namespace_uri_getter)]
    namespace_uri: (),
    #[webapi(accessor_property, getter = attr_prefix_getter)]
    prefix: (),
    #[webapi(accessor_property, getter = attr_local_name_getter)]
    local_name: (),
    #[webapi(accessor_property, getter = attr_name_getter)]
    name: (),
    #[webapi(accessor_property, getter = attr_value_getter, setter = attr_value_setter)]
    value: (),
    #[webapi(accessor_property, getter = attr_owner_element_getter)]
    owner_element: (),
    #[webapi(accessor_property, getter = attr_specified_getter)]
    specified: (),
}

pub(crate) fn install_attr_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    AttrPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn attr_metadata_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    name: &'static str,
) {
    if let Some(attr) = AttrReference::from_object(scope, args.this())
        && let Some(value) = attr.metadata(scope, name)
    {
        rv.set(value);
    }
}

fn attr_namespace_uri_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    attr_metadata_getter(scope, args, rv, "namespaceURI");
}

fn attr_prefix_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    attr_metadata_getter(scope, args, rv, "prefix");
}

fn attr_local_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    attr_metadata_getter(scope, args, rv, "localName");
}

fn attr_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    attr_metadata_getter(scope, args, rv, "name");
}

fn attr_value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(attr) = AttrReference::from_object(scope, args.this()) {
        let value = attr.value(scope);
        if let Some(value) = v8_string_from_utf16_units(scope, &value.utf16_units()) {
            rv.set(value.into());
        }
    }
}

fn attr_value_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Generated receiver validation runs before conversion in the callee realm.
    let value = match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::member("Attr", "value"),
    ) {
        Ok(value) => DomStringValue::from_utf16(&value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    // Conversion can detach or adopt the owner. Resolve its native state now.
    if let Some(attr) = AttrReference::from_object(scope, args.this()) {
        attr.set_value(scope, value);
    }
}

fn attr_owner_element_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match AttrReference::from_object(scope, args.this()).and_then(|attr| attr.owner_element(scope))
    {
        Some(owner) => rv.set(owner.into()),
        None => rv.set_null(),
    }
}

fn attr_specified_getter(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(true);
}
