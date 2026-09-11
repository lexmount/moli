use super::accessors::{
    attr_instance_base_uri_getter, attr_instance_local_name_getter, attr_instance_name_getter,
    attr_instance_namespace_uri_getter, attr_instance_node_name_getter,
    attr_instance_node_type_getter, attr_instance_owner_document_getter,
    attr_instance_owner_element_getter, attr_instance_prefix_getter,
    attr_instance_specified_getter, attr_instance_value_getter, attr_instance_value_setter,
};
use super::value::{attr_owner_document_object, attr_owner_element_object};
use super::*;
use crate::definitions::{
    define_native_data_property as define_attr_instance_native_data_property,
    define_native_data_property_with_setter as define_attr_instance_native_data_property_with_setter,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Attr)]
struct AttrInstanceMethodsDeclaration {
    #[webapi(method, length = 0, callback = attr_instance_is_same_node_callback)]
    is_same_node: (),
    #[webapi(method, length = 0, callback = attr_instance_clone_node_callback)]
    clone_node: (),
    #[webapi(
        method = "lookupNamespaceURI",
        length = 0,
        callback = attr_instance_lookup_namespace_uri_callback
    )]
    lookup_namespace_uri: (),
}

fn attr_instance_clone_node_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = attr_state_object(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(name) = object_string_property(scope, state, "name") else {
        rv.set_null();
        return;
    };
    let value = attr_current_value(scope, args.this());
    let namespace_uri = nullable_attr_state_string(scope, state, "namespaceURI");
    let prefix = nullable_attr_state_string(scope, state, "prefix");
    let owner_document = attr_owner_document_object(scope, args.this());
    let local_name = object_string_property(scope, state, "localName")
        .filter(|local_name| !local_name.is_empty())
        .or_else(|| name.rsplit_once(':').map(|(_, local)| local.to_owned()))
        .unwrap_or_else(|| name.clone());
    match new_attr_object(
        scope,
        &name,
        &value,
        None,
        owner_document,
        namespace_uri.as_deref(),
        prefix.as_deref(),
        &local_name,
    ) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}

fn attr_instance_is_same_node_callback<'a>(
    _scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(args.this().strict_equals(args.get(0)));
}

fn attr_instance_lookup_namespace_uri_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(owner) = attr_owner_element_object(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(value) = call_object_method(scope, owner, "lookupNamespaceURI", &[args.get(0)]) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(super) fn install_attr_instance_properties<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    define_attr_instance_native_data_property(
        scope,
        object,
        "nodeType",
        attr_instance_node_type_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "nodeName",
        attr_instance_node_name_getter,
    );
    define_attr_instance_native_data_property(scope, object, "name", attr_instance_name_getter);
    define_attr_instance_native_data_property(
        scope,
        object,
        "localName",
        attr_instance_local_name_getter,
    );
    define_attr_instance_native_data_property(scope, object, "prefix", attr_instance_prefix_getter);
    define_attr_instance_native_data_property(
        scope,
        object,
        "namespaceURI",
        attr_instance_namespace_uri_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "ownerElement",
        attr_instance_owner_element_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "ownerDocument",
        attr_instance_owner_document_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "specified",
        attr_instance_specified_getter,
    );
    define_attr_instance_native_data_property(
        scope,
        object,
        "baseURI",
        attr_instance_base_uri_getter,
    );
    define_attr_instance_native_data_property_with_setter(
        scope,
        object,
        "value",
        attr_instance_value_getter,
        attr_instance_value_setter,
    );
    define_attr_instance_native_data_property_with_setter(
        scope,
        object,
        "nodeValue",
        attr_instance_value_getter,
        attr_instance_value_setter,
    );
    define_attr_instance_native_data_property_with_setter(
        scope,
        object,
        "textContent",
        attr_instance_value_getter,
        attr_instance_value_setter,
    );
    AttrInstanceMethodsDeclaration::new()
        .initialize(scope, object)
        .expect("Attr instance method declaration should initialize");
}

fn nullable_attr_state_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<String> {
    let key = v8_string(scope, property)?;
    let value = state.get(scope, key.into())?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
}
