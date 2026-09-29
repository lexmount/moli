use super::super::reference::native_owner_document;
use super::install::install_attr_instance_properties;
use super::*;
use crate::util::set_null_prototype;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct AttrStateDeclaration<'scope> {
    name: v8::Local<'scope, v8::String>,
    value: v8::Local<'scope, v8::String>,
    owner_element: v8::Local<'scope, v8::Value>,
    owner_document: v8::Local<'scope, v8::Value>,
    #[webapi(data_property = "namespaceURI")]
    namespace_uri: v8::Local<'scope, v8::Value>,
    prefix: v8::Local<'scope, v8::Value>,
    local_name: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Attr)]
struct AttrObjectDeclaration<'scope> {
    #[webapi(slot = ATTR_STATE_SLOT)]
    state: v8::Local<'scope, v8::Object>,
}

pub(crate) fn new_attr_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    value: impl Into<crate::dom::native::DomStringValue>,
    owner_element: Option<v8::Local<'s, v8::Object>>,
    owner_document: Option<v8::Local<'s, v8::Object>>,
    namespace_uri: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let value = value.into();
    let owner_document = owner_document
        .or_else(|| owner_element.and_then(|owner| native_owner_document(scope, owner)))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    let state = AttrStateDeclaration {
        name: v8_string(scope, name)?,
        value: crate::util::v8_string_from_utf16_units(scope, &value.utf16_units())?,
        owner_element: owner_element
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        owner_document,
        namespace_uri: namespace_uri
            .and_then(|value| v8_string(scope, value))
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        prefix: prefix
            .and_then(|value| v8_string(scope, value))
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into()),
        local_name: v8_string(scope, local_name)?,
    }
    .bind(scope)
    .ok()?;
    set_null_prototype(scope, state);
    let object = AttrObjectDeclaration { state }.bind(scope).ok()?;
    install_attr_instance_properties(scope, object);
    Some(object)
}
