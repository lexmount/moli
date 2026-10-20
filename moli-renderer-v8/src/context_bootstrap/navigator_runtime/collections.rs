//! Per-Window PDF plugin objects and their Web IDL legacy collections.
//! Native lists, not public JavaScript properties, own the object graph.

use super::super::*;
use crate::{
    util::{get_private_object, get_private_value},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};

mod legacy;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NavigatorPluginCollection.item")]
struct NavigatorPluginCollectionItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NavigatorPluginCollection.namedItem")]
struct NavigatorPluginCollectionNamedItemArgs {
    #[webidl(required)]
    name: String,
}

const PDF_PLUGIN_NAMES: &[&str] = &[
    "PDF Viewer",
    "Chrome PDF Viewer",
    "Chromium PDF Viewer",
    "Microsoft Edge PDF Viewer",
    "WebKit built-in PDF",
];
const PDF_MIME_TYPES: &[&str] = &["application/pdf", "text/pdf"];
const PDF_DESCRIPTION: &str = "Portable Document Format";
const COLLECTION_VALUES_SLOT: &str = "__moliNavigatorCollectionValues";
const COLLECTION_NAMES_SLOT: &str = "__moliNavigatorCollectionNames";
const PLUGIN_NAME_SLOT: &str = "__moliPluginName";
const MIME_TYPE_SLOT: &str = "__moliMimeType";
const MIME_ENABLED_PLUGIN_SLOT: &str = "__moliMimeEnabledPlugin";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MimeType, require_prototype)]
struct MimeTypeObjectDeclaration<'s> {
    #[webapi(slot = MIME_TYPE_SLOT)]
    r#type: v8::Local<'s, v8::String>,
    #[webapi(slot = MIME_ENABLED_PLUGIN_SLOT)]
    enabled_plugin: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Plugin, require_prototype)]
struct PluginObjectDeclaration<'s> {
    #[webapi(slot = PLUGIN_NAME_SLOT)]
    name: v8::Local<'s, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PluginArray, require_prototype)]
struct PluginArrayObjectDeclaration {}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MimeTypeArray, require_prototype)]
struct MimeTypeArrayObjectDeclaration {}

#[derive(WebApiObject)]
#[webapi(fragment)]
struct CollectionState<'s> {
    #[webapi(slot = COLLECTION_VALUES_SLOT)]
    values: v8::Local<'s, v8::Array>,
    #[webapi(slot = COLLECTION_NAMES_SLOT)]
    names: v8::Local<'s, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MimeType, enumerable, receiver)]
struct MimeTypePrototypeDeclaration {
    #[webapi(accessor_property, getter = stored_attribute_getter, data = v8str(scope, MIME_TYPE_SLOT))]
    r#type: (),
    #[webapi(accessor_property, getter = constant_attribute_getter, data = v8str(scope, "pdf"))]
    suffixes: (),
    #[webapi(accessor_property, getter = constant_attribute_getter, data = v8str(scope, PDF_DESCRIPTION))]
    description: (),
    #[webapi(accessor_property, getter = stored_attribute_getter, data = v8str(scope, MIME_ENABLED_PLUGIN_SLOT))]
    enabled_plugin: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MimeTypeArray, enumerable, receiver)]
struct MimeTypeArrayPrototypeDeclaration {
    #[webapi(accessor_property, getter = collection_length_getter)]
    length: (),

    #[webapi(method, callback = collection_item_callback, length = 1)]
    item: (),

    #[webapi(method, callback = collection_named_item_callback, length = 1)]
    named_item: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PluginArray, enumerable, receiver)]
struct PluginArrayPrototypeDeclaration {
    #[webapi(accessor_property, getter = collection_length_getter)]
    length: (),

    #[webapi(method, callback = collection_item_callback, length = 1)]
    item: (),

    #[webapi(method, callback = collection_named_item_callback, length = 1)]
    named_item: (),

    #[webapi(method, callback = plugin_array_refresh_callback, length = 0)]
    refresh: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Plugin, enumerable, receiver)]
struct PluginPrototypeDeclaration {
    #[webapi(accessor_property, getter = stored_attribute_getter, data = v8str(scope, PLUGIN_NAME_SLOT))]
    name: (),
    #[webapi(accessor_property, getter = constant_attribute_getter, data = v8str(scope, "internal-pdf-viewer"))]
    filename: (),
    #[webapi(accessor_property, getter = constant_attribute_getter, data = v8str(scope, PDF_DESCRIPTION))]
    description: (),
    #[webapi(accessor_property, getter = collection_length_getter)]
    length: (),

    #[webapi(method, callback = collection_item_callback, length = 1)]
    item: (),

    #[webapi(method, callback = collection_named_item_callback, length = 1)]
    named_item: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

pub(super) fn install_navigator_collection_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "MimeTypeArray" => {
            MimeTypeArrayPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "PluginArray" => {
            PluginArrayPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "Plugin" => PluginPrototypeDeclaration::initialize_prototype_template(scope, prototype),
        "MimeType" => MimeTypePrototypeDeclaration::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    web_api_object_target(scope, receiver).expect("validated native plugin receiver")
}

fn stored_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, object, &slot).expect("plugin attribute has native state"));
}

fn constant_attribute_getter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.data());
}

fn values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let object = target(scope, object);
    get_private_object(scope, object, COLLECTION_VALUES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("plugin collection has a native item list")
}

fn names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let object = target(scope, object);
    get_private_object(scope, object, COLLECTION_NAMES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("plugin collection has a native name list")
}

fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Value>> {
    let values = values(scope, object);
    (index < values.length())
        .then(|| values.get_index(scope, index))
        .flatten()
}

fn named_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: v8::Local<'s, v8::Name>,
) -> Option<u32> {
    let names = names(scope, object);
    (0..names.length()).find(|index| {
        names
            .get_index(scope, *index)
            .is_some_and(|value| value.strict_equals(name.into()))
    })
}

fn collection_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(values(scope, args.this()).length());
}

fn collection_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NavigatorPluginCollectionItemArgs>(scope, &args) else {
        return;
    };
    rv.set(item(scope, args.this(), parsed.index).unwrap_or_else(|| v8::null(scope).into()));
}

fn collection_named_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NavigatorPluginCollectionNamedItemArgs>(scope, &args)
    else {
        return;
    };
    let Some(name) = v8_string(scope, &parsed.name) else {
        return;
    };
    let value = named_index(scope, args.this(), name.into())
        .and_then(|index| item(scope, args.this(), index));
    rv.set(value.unwrap_or_else(|| v8::null(scope).into()));
}

fn plugin_array_refresh_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

#[derive(WebApiObject)]
#[webapi(fragment, data_properties, enumerable)]
pub(super) struct NavigatorPluginCollections<'s> {
    pub(super) mime_types: v8::Local<'s, v8::Object>,
    pub(super) plugins: v8::Local<'s, v8::Object>,
}

fn name_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    names: &[&'static str],
) -> v8::Local<'s, v8::Array> {
    let names = names
        .iter()
        .map(|name| v8str(scope, name).into())
        .collect::<Vec<_>>();
    v8::Array::new_with_elements(scope, &names)
}

pub(super) fn build_navigator_plugin_collections<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<NavigatorPluginCollections<'s>> {
    // HTML defines five Plugin objects and two MimeType objects per Window.
    // Construct the cycle before publishing either SameObject collection.
    let mut plugins = Vec::with_capacity(PDF_PLUGIN_NAMES.len());
    for name in PDF_PLUGIN_NAMES {
        let object = legacy::new_target(scope)?;
        PluginObjectDeclaration::new(v8str(scope, name))
            .bind_into(scope, object)
            .ok()?;
        plugins.push(legacy::wrap(scope, object)?);
    }
    let mut mime_types = Vec::with_capacity(PDF_MIME_TYPES.len());
    for type_name in PDF_MIME_TYPES {
        let object = MimeTypeObjectDeclaration::new(v8str(scope, type_name), plugins[0])
            .bind(scope)
            .ok()?;
        mime_types.push(object.into());
    }
    let mime_values = v8::Array::new_with_elements(scope, &mime_types);
    let mime_names = name_array(scope, PDF_MIME_TYPES);
    let mime_state = CollectionState::new(mime_values, mime_names);
    for plugin in &plugins {
        let object = target(scope, *plugin);
        mime_state.initialize(scope, object).ok()?;
    }
    let mime_array = legacy::new_target(scope)?;
    MimeTypeArrayObjectDeclaration::new()
        .bind_into(scope, mime_array)
        .ok()?;
    mime_state.initialize(scope, mime_array).ok()?;
    let mime_array = legacy::wrap(scope, mime_array)?;

    let plugin_array = legacy::new_target(scope)?;
    PluginArrayObjectDeclaration::new()
        .bind_into(scope, plugin_array)
        .ok()?;
    let plugin_values = plugins.into_iter().map(Into::into).collect::<Vec<_>>();
    let plugin_values = v8::Array::new_with_elements(scope, &plugin_values);
    let plugin_names = name_array(scope, PDF_PLUGIN_NAMES);
    CollectionState::new(plugin_values, plugin_names)
        .initialize(scope, plugin_array)
        .ok()?;
    let plugin_array = legacy::wrap(scope, plugin_array)?;
    Some(NavigatorPluginCollections {
        mime_types: mime_array,
        plugins: plugin_array,
    })
}
