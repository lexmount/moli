use crate::native_bridge::collections::{
    STATIC_COLLECTION_LENGTH_SLOT, initialize_collection_identity,
};
use crate::native_bridge::document::DETACHED_NATIVE_NODE_LIST_HANDLES_SLOT;
use crate::native_bridge::identity::CollectionKind;
use crate::native_bridge::named_access::{
    LegacyNamedAccessKind, html_element_name_attribute_is_exposed,
};
use crate::util::{
    context_host_ptr_from_global_bridge, get_private_value, serialize_v8_iter_array,
    set_private_value,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject};

use super::*;

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct DetachedCollectionItemsAndNamedDataDeclaration<'scope> {
    items: v8::Local<'scope, v8::Value>,
    named: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::HTMLAllCollection)]
struct DetachedDocumentAllCollectionDeclaration<'scope> {
    /// Declaration-only input shared by `item` and `namedItem`.
    ///
    /// Detached `document.all` still needs an `ObjectTemplate` shell so V8 can
    /// carry the legacy `[[IsHTMLDDA]]` behavior and call-as-function handler.
    /// This declaration only installs the fixed string members. The backing
    /// collection object is passed through callback data and must not become an
    /// own `"data"` property on the web-facing object.
    data: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    length: f64,
    #[webapi(method, callback = detached_document_all_item_callback, data = self.data)]
    item: (),
    #[webapi(method, callback = detached_html_collection_named_item_callback, data = self.data)]
    named_item: (),
}

/// Declares the detached `NodeList` wrapper surface.
///
/// All fixed members come from the shared `NodeList` prototype. The wrapper
/// only carries its indexed values and private collection brand/length state.
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NodeList, require_prototype)]
struct DetachedNodeListDeclaration {}

/// Declares the detached `HTMLCollection` wrapper surface.
///
/// Fixed `length`, `item`, `namedItem`, and iterator members come from the
/// reusable interface template. Only indexed/named snapshot entries remain
/// own properties.
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::HTMLCollection, require_prototype)]
struct DetachedHtmlCollectionDeclaration {}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLCollection.namedItem")]
struct DetachedHtmlCollectionNamedItemArgs {
    #[webidl(required)]
    name: String,
}

enum DetachedHtmlAllNameOrIndex {
    Index(u32),
    Name(String),
}

fn detached_document_all_name_or_index(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<DetachedHtmlAllNameOrIndex> {
    if args.length() == 0 || args.get(0).is_undefined() {
        return None;
    }
    let name_or_index = callback_arg_string(scope, args, 0)?;
    Some(match array_index_property_name(&name_or_index) {
        Some(index) => DetachedHtmlAllNameOrIndex::Index(index),
        None => DetachedHtmlAllNameOrIndex::Name(name_or_index),
    })
}

fn resolve_detached_document_all_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    items: v8::Local<'s, v8::Object>,
    named: v8::Local<'s, v8::Object>,
    name_or_index: DetachedHtmlAllNameOrIndex,
) -> Option<v8::Local<'s, v8::Value>> {
    match name_or_index {
        DetachedHtmlAllNameOrIndex::Index(index) => items
            .get_index(scope, index)
            .filter(|value| !value.is_null_or_undefined()),
        DetachedHtmlAllNameOrIndex::Name(key) => {
            let key = v8_string(scope, &key)?;
            named
                .get(scope, key.into())
                .filter(|value| !value.is_null_or_undefined())
        }
    }
}

fn detached_native_node_list_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, object, DETACHED_NATIVE_NODE_LIST_HANDLES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn detached_native_node_list_handle_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handles: v8::Local<'s, v8::Array>,
    index: u32,
) -> Option<DomHandle> {
    let value = handles.get_index(scope, index)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (raw, lossless) = big.u64_value();
    lossless.then(|| DomHandle::new(raw as usize))
}

fn detached_native_node_list_item_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<DomHandle> {
    let handles = detached_native_node_list_handles(scope, object)?;
    detached_native_node_list_handle_at(scope, handles, index)
}

fn detached_native_node_list_len<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    detached_native_node_list_handles(scope, object).map(|handles| handles.length())
}

pub(in crate::native_bridge::document) fn define_collection_value_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: v8::Local<'_, v8::Value>,
    value: v8::Local<'_, v8::Value>,
    attributes: v8::PropertyAttribute,
) {
    if let Ok(name) = v8::Local::<v8::Name>::try_from(key) {
        let _ = object.define_own_property(scope, name, value, attributes);
        return;
    }
    let Some(index) = key.integer_value(scope).filter(|index| *index >= 0) else {
        return;
    };
    let Some(index_key) = v8_string(scope, &index.to_string()) else {
        return;
    };
    let _ = object.define_own_property(scope, index_key.into(), value, attributes);
}

fn detached_native_node_list_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return v8::Intercepted::kNo;
    };
    let Some(node) = detached_native_node_list_item_handle(scope, args.holder(), index)
        .and_then(|handle| detached_native_object_for_handle(scope, runtime_ptr, handle))
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(node.into());
    v8::Intercepted::kYes
}

fn detached_native_node_list_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Some(length) = detached_native_node_list_len(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if index >= length {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
    v8::Intercepted::kYes
}

fn detached_native_node_list_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Some(length) = detached_native_node_list_len(scope, args.holder()) else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

pub(in crate::native_bridge::document) fn detached_document_all_item_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_null();
        return;
    };
    let Some(items) = object_property_as_object(scope, data, "items") else {
        rv.set_null();
        return;
    };
    let Some(named) = object_property_as_object(scope, data, "named") else {
        rv.set_null();
        return;
    };
    let Some(name_or_index) = detached_document_all_name_or_index(scope, &args) else {
        rv.set_null();
        return;
    };
    match resolve_detached_document_all_value(scope, items, named, name_or_index) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn detached_html_collection_named_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_null();
        return;
    };
    let Some(named) = object_property_as_object(scope, data, "named") else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedHtmlCollectionNamedItemArgs>(scope, &args)
    else {
        return;
    };
    let Some(key) = v8_string(scope, &parsed.name) else {
        rv.set_null();
        return;
    };
    match named.get(scope, key.into()) {
        Some(value) if !value.is_null_or_undefined() => rv.set(value),
        _ => rv.set_null(),
    }
}

fn detached_collection_named_lookup<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[v8::Local<'s, v8::Object>],
    name_access_kind: Option<LegacyNamedAccessKind>,
) -> v8::Local<'s, v8::Object> {
    let lookup = ObjectLiteralDeclaration::bind(scope);
    for value in values {
        for attribute_name in ["id", "name"] {
            if attribute_name == "name"
                && name_access_kind.is_some_and(|kind| {
                    detached_element_namespace_uri(scope, *value).as_deref() != Some(XHTML_NS)
                        || !detached_element_local_name(scope, *value).is_some_and(|local_name| {
                            html_element_name_attribute_is_exposed(&local_name, kind)
                        })
                })
            {
                continue;
            }
            let Some(key_text) = detached_collection_attribute_value(scope, *value, attribute_name)
            else {
                continue;
            };
            if key_text.is_empty() {
                continue;
            }
            let Some(key) = v8_string(scope, &key_text) else {
                continue;
            };
            if lookup
                .as_object()
                .get(scope, key.into())
                .is_some_and(|existing| !existing.is_null_or_undefined())
            {
                continue;
            }
            lookup.set_value_property(scope, key.into(), (*value).into());
        }
    }
    lookup.into_object()
}

fn detached_collection_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    if let Some(has_attribute) = read_detached_native_has_attribute(scope, element, name) {
        if has_attribute {
            return read_detached_native_attribute(scope, element, name);
        }
        return None;
    }
    object_string_property(scope, element, name)
}

fn install_detached_collection_members<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    values: &[v8::Local<'s, v8::Object>],
    named: Option<v8::Local<'s, v8::Object>>,
) -> Option<()> {
    for (index, value) in values.iter().enumerate() {
        define_collection_value_property(
            scope,
            target,
            v8::Integer::new_from_unsigned(scope, index as u32).into(),
            (*value).into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
    }
    let Some(named) = named else {
        return Some(());
    };
    let names = named.get_property_names(scope, Default::default())?;
    for index in 0..names.length() {
        let Some(key) = names.get_index(scope, index) else {
            continue;
        };
        let Some(value) = named.get(scope, key) else {
            continue;
        };
        define_collection_value_property(
            scope,
            target,
            key,
            value,
            v8::PropertyAttribute::DONT_ENUM,
        );
    }
    Some(())
}

pub(in crate::native_bridge::document) fn build_detached_node_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Object>> {
    let wrapper = DetachedNodeListDeclaration::new().bind(scope).ok()?;
    install_detached_collection_state(scope, wrapper, CollectionKind::NodeList, values.len());
    install_detached_collection_members(scope, wrapper, values, None)?;
    Some(wrapper)
}

pub(in crate::native_bridge::document) fn build_detached_native_node_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handles: &[DomHandle],
) -> Option<v8::Local<'s, v8::Object>> {
    let object_template = v8::ObjectTemplate::new(scope);
    object_template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(detached_native_node_list_indexed_getter)
            .query(detached_native_node_list_indexed_query)
            .enumerator(detached_native_node_list_indexed_enumerator),
    );
    let wrapper = object_template.new_instance(scope)?;
    let length = handles.len();
    let handle_values = handles
        .iter()
        .copied()
        .map(|handle| v8::BigInt::new_from_u64(scope, handle.index() as u64))
        .collect::<Vec<_>>();
    let handles_array =
        serialize_v8_iter_array(scope, handle_values).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_private_value(
        scope,
        wrapper,
        DETACHED_NATIVE_NODE_LIST_HANDLES_SLOT,
        handles_array.into(),
    );
    DetachedNodeListDeclaration::new()
        .bind_into(scope, wrapper)
        .ok()?;
    install_detached_collection_state(scope, wrapper, CollectionKind::NodeList, length);
    Some(wrapper)
}

fn install_detached_collection_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    kind: CollectionKind,
    length: usize,
) {
    initialize_collection_identity(scope, wrapper, kind);
    set_private_value(
        scope,
        wrapper,
        STATIC_COLLECTION_LENGTH_SLOT,
        v8::Number::new(scope, length as f64).into(),
    );
}

pub(in crate::native_bridge::document) fn build_detached_html_collection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[v8::Local<'s, v8::Object>],
) -> Option<v8::Local<'s, v8::Object>> {
    let named = detached_collection_named_lookup(scope, values, None);
    let wrapper = DetachedHtmlCollectionDeclaration::new().bind(scope).ok()?;
    install_detached_collection_state(scope, wrapper, CollectionKind::HtmlCollection, values.len());
    install_detached_collection_members(scope, wrapper, values, Some(named))?;
    Some(wrapper)
}

fn collect_detached_document_all_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    out: &mut Vec<v8::Local<'s, v8::Object>>,
) {
    if detached_node_type(scope, node) == Some(1) {
        out.push(node);
    }
    let children = detached_child_node_objects(scope, node);
    for child in children {
        collect_detached_document_all_values(scope, child, out);
    }
}

pub(in crate::native_bridge::document) fn detached_document_all_call_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_null();
        return;
    };
    let Some(items) = object_property_as_object(scope, data, "items") else {
        rv.set_null();
        return;
    };
    let Some(named) = object_property_as_object(scope, data, "named") else {
        rv.set_null();
        return;
    };
    let Some(name_or_index) = detached_document_all_name_or_index(scope, &args) else {
        rv.set_null();
        return;
    };
    match resolve_detached_document_all_value(scope, items, named, name_or_index) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn build_detached_document_all<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut values = Vec::new();
    if let Some(root) = detached_document_element_object(scope, document) {
        collect_detached_document_all_values(scope, root, &mut values);
    }

    let global = scope.get_current_context().global(scope);
    let html_all_ctor = global.get(scope, v8str(scope, "HTMLAllCollection").into())?;
    let html_all_ctor = v8::Local::<v8::Function>::try_from(html_all_ctor).ok()?;
    let prototype = html_all_ctor.get(scope, v8str(scope, "prototype").into())?;
    let prototype = v8::Local::<v8::Object>::try_from(prototype).ok()?;

    let items = build_object_array(scope, &values);
    let named =
        detached_collection_named_lookup(scope, &values, Some(LegacyNamedAccessKind::DocumentAll));
    let data = DetachedCollectionItemsAndNamedDataDeclaration::new(items.into(), named)
        .bind(scope)
        .ok()?;

    let object_template = v8::ObjectTemplate::new(scope);
    object_template.mark_as_undetectable();
    object_template.set_call_as_function_handler_with_data(
        detached_document_all_call_callback,
        Some(data.into()),
    );
    let collection = object_template.new_instance(scope)?;
    let _ = collection.set_prototype(scope, prototype.into());
    DetachedDocumentAllCollectionDeclaration::new(data, values.len() as f64)
        .initialize(scope, collection)
        .ok()?;
    install_detached_collection_members(scope, collection, &values, Some(named))?;
    Some(collection)
}
