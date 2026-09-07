use super::super::*;
use crate::{
    util::{get_private_value, throw_type_error},
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

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

const PDF_MIME_TYPES: &[(&str, &str, &str)] = &[
    ("application/pdf", "pdf", "Portable Document Format"),
    ("text/pdf", "pdf", "Portable Document Format"),
];

const PLUGIN_ARRAY_BRAND_SLOT: &str = "__moliPluginArrayBrand";
const MIME_TYPE_ARRAY_BRAND_SLOT: &str = "__moliMimeTypeArrayBrand";
const PLUGIN_BRAND_SLOT: &str = "__moliPluginBrand";
const COLLECTION_LENGTH_SLOT: &str = "__moliNavigatorCollectionLength";

#[derive(WebApiObject)]
#[webapi(interface = "MimeType")]
struct MimeTypeObjectDeclaration<'scope> {
    #[webapi(data_property)]
    r#type: v8::Local<'scope, v8::String>,

    #[webapi(data_property)]
    suffixes: v8::Local<'scope, v8::String>,

    #[webapi(data_property)]
    description: v8::Local<'scope, v8::String>,

    #[webapi(data_property)]
    enabled_plugin: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "MimeTypeArray")]
struct MimeTypeArrayObjectDeclaration {
    #[webapi(slot = MIME_TYPE_ARRAY_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = COLLECTION_LENGTH_SLOT)]
    length: u32,
}

#[derive(WebApiObject)]
#[webapi(interface = "PluginArray")]
struct PluginArrayObjectDeclaration {
    #[webapi(slot = PLUGIN_ARRAY_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = COLLECTION_LENGTH_SLOT)]
    length: u32,
}

#[derive(WebApiObject)]
#[webapi(interface = "Plugin")]
struct PluginObjectDeclaration<'scope> {
    #[webapi(slot = PLUGIN_BRAND_SLOT, init = true)]
    brand: (),

    #[webapi(slot = COLLECTION_LENGTH_SLOT)]
    length: u32,

    #[webapi(data_property)]
    name: v8::Local<'scope, v8::String>,

    #[webapi(data_property)]
    filename: v8::Local<'scope, v8::String>,

    #[webapi(data_property)]
    description: v8::Local<'scope, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "MimeTypeArray", enumerable)]
struct MimeTypeArrayPrototypeDeclaration {
    #[webapi(accessor_property, getter = mime_type_array_length_getter)]
    length: (),

    #[webapi(method, callback = mime_type_array_item_callback, length = 1)]
    item: (),

    #[webapi(method, callback = mime_type_array_named_item_callback, length = 1)]
    named_item: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "PluginArray", enumerable)]
struct PluginArrayPrototypeDeclaration {
    #[webapi(accessor_property, getter = plugin_array_length_getter)]
    length: (),

    #[webapi(method, callback = plugin_array_item_callback, length = 1)]
    item: (),

    #[webapi(method, callback = plugin_array_named_item_callback, length = 1)]
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
#[webapi(name = "Plugin", enumerable)]
struct PluginPrototypeDeclaration {
    #[webapi(accessor_property, getter = plugin_length_getter)]
    length: (),

    #[webapi(method, callback = plugin_item_callback, length = 1)]
    item: (),

    #[webapi(method, callback = plugin_named_item_callback, length = 1)]
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
        _ => {}
    }
}

fn plugin_array_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_length_getter_for(scope, args, rv, PLUGIN_ARRAY_BRAND_SLOT);
}

fn mime_type_array_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_length_getter_for(scope, args, rv, MIME_TYPE_ARRAY_BRAND_SLOT);
}

fn plugin_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_length_getter_for(scope, args, rv, PLUGIN_BRAND_SLOT);
}

fn collection_length_getter_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    brand_slot: &'static str,
) {
    if !receiver_has_brand(scope, args.this(), brand_slot) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if let Some(length) = get_private_value(scope, args.this(), COLLECTION_LENGTH_SLOT) {
        rv.set(length);
    }
}

fn plugin_array_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_item_callback_for(scope, args, rv, PLUGIN_ARRAY_BRAND_SLOT);
}

fn plugin_array_named_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_named_item_callback_for(scope, args, rv, PLUGIN_ARRAY_BRAND_SLOT);
}

fn mime_type_array_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_item_callback_for(scope, args, rv, MIME_TYPE_ARRAY_BRAND_SLOT);
}

fn mime_type_array_named_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_named_item_callback_for(scope, args, rv, MIME_TYPE_ARRAY_BRAND_SLOT);
}

fn plugin_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_item_callback_for(scope, args, rv, PLUGIN_BRAND_SLOT);
}

fn plugin_named_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    collection_named_item_callback_for(scope, args, rv, PLUGIN_BRAND_SLOT);
}

fn collection_item_callback_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    brand_slot: &'static str,
) {
    let Some(this_obj) = args.this().to_object(scope) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if !receiver_has_brand(scope, this_obj, brand_slot) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<NavigatorPluginCollectionItemArgs>(scope, &args) else {
        return;
    };
    match this_obj.get_index(scope, parsed.index) {
        Some(value) if !value.is_undefined() => rv.set(value),
        _ => rv.set(v8::null(scope).into()),
    }
}

fn collection_named_item_callback_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    brand_slot: &'static str,
) {
    let Some(this_obj) = args.this().to_object(scope) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if !receiver_has_brand(scope, this_obj, brand_slot) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<NavigatorPluginCollectionNamedItemArgs>(scope, &args)
    else {
        return;
    };
    let Some(key) = v8_string(scope, &parsed.name) else {
        rv.set(v8::null(scope).into());
        return;
    };
    match this_obj.get(scope, key.into()) {
        Some(value) if !value.is_undefined() => rv.set(value),
        _ => rv.set(v8::null(scope).into()),
    }
}

fn plugin_array_refresh_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !receiver_has_brand(scope, args.this(), PLUGIN_ARRAY_BRAND_SLOT) {
        throw_type_error(scope, "Illegal invocation");
    }
}

fn receiver_has_brand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    get_private_value(scope, receiver, slot).is_some_and(|value| value.boolean_value(scope))
}

fn build_mime_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    type_name: &str,
    suffixes: &str,
    description: &str,
    enabled_plugin: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let enabled_plugin_value = enabled_plugin
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    MimeTypeObjectDeclaration::new(
        v8_string(scope, type_name)?,
        v8_string(scope, suffixes)?,
        v8_string(scope, description)?,
        enabled_plugin_value,
    )
    .bind(scope)
    .ok()
}

fn build_plugin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let plugin = PluginObjectDeclaration::new(
        PDF_MIME_TYPES.len() as u32,
        v8_string(scope, name)?,
        v8_string(scope, "internal-pdf-viewer")?,
        v8_string(scope, "Portable Document Format")?,
    )
    .bind(scope)
    .ok()?;

    let mut mime_types = Vec::with_capacity(PDF_MIME_TYPES.len());
    for (type_name, suffixes, description) in PDF_MIME_TYPES {
        let mime_type = build_mime_type(scope, type_name, suffixes, description, Some(plugin))?;
        mime_types.push((type_name, mime_type));
    }
    define_collection_indices(scope, plugin, mime_types.iter().map(|(_, item)| *item))?;
    for (type_name, mime_type) in mime_types {
        let _ = plugin.define_own_property(
            scope,
            v8str(scope, type_name).into(),
            mime_type.into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
    }

    Some(plugin)
}

fn build_mime_type_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    enabled_plugin: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let array = MimeTypeArrayObjectDeclaration::new(PDF_MIME_TYPES.len() as u32)
        .bind(scope)
        .ok()?;
    let mut mime_types = Vec::with_capacity(PDF_MIME_TYPES.len());
    for (type_name, suffixes, description) in PDF_MIME_TYPES {
        let mime_type = build_mime_type(
            scope,
            type_name,
            suffixes,
            description,
            Some(enabled_plugin),
        )?;
        mime_types.push((type_name, mime_type));
    }
    define_collection_indices(scope, array, mime_types.iter().map(|(_, item)| *item))?;
    for (type_name, mime_type) in mime_types {
        let _ = array.define_own_property(
            scope,
            v8str(scope, type_name).into(),
            mime_type.into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
    }
    Some(array)
}

fn build_plugin_array<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let array = PluginArrayObjectDeclaration::new(PDF_PLUGIN_NAMES.len() as u32)
        .bind(scope)
        .ok()?;
    let mut plugins = Vec::with_capacity(PDF_PLUGIN_NAMES.len());
    for name in PDF_PLUGIN_NAMES {
        let plugin = build_plugin(scope, name)?;
        plugins.push((name, plugin));
    }
    define_collection_indices(scope, array, plugins.iter().map(|(_, item)| *item))?;
    for (name, plugin) in plugins {
        let _ = array.define_own_property(
            scope,
            v8str(scope, name).into(),
            plugin.into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
    }
    Some(array)
}

fn define_collection_indices<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    collection: v8::Local<'s, v8::Object>,
    items: impl IntoIterator<Item = v8::Local<'s, v8::Object>>,
) -> Option<()> {
    for (index, item) in items.into_iter().enumerate() {
        let key = v8_string(scope, &index.to_string())?;
        if collection.create_data_property(scope, key.into(), item.into()) != Some(true) {
            return None;
        }
    }
    Some(())
}

pub(super) struct NavigatorPluginCollections<'scope> {
    pub(super) mime_types: v8::Local<'scope, v8::Object>,
    pub(super) plugins: v8::Local<'scope, v8::Object>,
}

pub(super) fn build_navigator_plugin_collections<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<NavigatorPluginCollections<'s>> {
    let plugins = build_plugin_array(scope)?;
    let enabled_plugin = plugins
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let mime_types = build_mime_type_array(scope, enabled_plugin)?;
    Some(NavigatorPluginCollections {
        mime_types,
        plugins,
    })
}
