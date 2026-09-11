use crate::document_runtime::DomHandle;
use crate::native_bridge::collections;
use crate::util::v8str;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

use super::super::super::super::JsContextHost;
use super::super::super::define_collection_value_property;
use super::callbacks::{
    document_all_call_callback, document_all_item_callback, document_all_named_collection_getter,
    document_all_named_item_callback,
};
use super::items::{document_all_items_array, document_all_named_lookup};

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct DocumentAllCollectionDataDeclaration<'scope> {
    items: v8::Local<'scope, v8::Array>,
    named: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::HTMLAllCollection)]
struct DocumentAllCollectionSurfaceDeclaration<'scope> {
    /// Declaration-only input shared by `item` and `namedItem`.
    ///
    /// Live `document.all` is allocated through an `ObjectTemplate` so V8 can
    /// preserve the legacy `[[IsHTMLDDA]]` behavior and call-as-function
    /// handler. This declaration only installs fixed string members; the
    /// backing lookup data stays in callback data and must not become a
    /// web-visible `"data"` own property.
    data: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    length: f64,
    #[webapi(method, callback = document_all_item_callback, data = self.data)]
    item: (),
    #[webapi(
        method,
        length = 1,
        callback = document_all_named_item_callback,
        data = self.data
    )]
    named_item: (),
}

pub(super) fn build_document_all_collection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    let items = document_all_items_array(scope, runtime_ptr, document_handle)?;
    let named = document_all_named_lookup(scope, runtime_ptr, document_handle, items);
    let data = DocumentAllCollectionDataDeclaration::new(items, named)
        .bind(scope)
        .ok()?;
    let global = scope.get_current_context().global(scope);
    let html_all_ctor = global.get(scope, v8str(scope, "HTMLAllCollection").into())?;
    let html_all_ctor = v8::Local::<v8::Function>::try_from(html_all_ctor).ok()?;
    let prototype = html_all_ctor.get(scope, v8str(scope, "prototype").into())?;
    let prototype = v8::Local::<v8::Object>::try_from(prototype).ok()?;

    let object_template = v8::ObjectTemplate::new(scope);
    object_template.mark_as_undetectable();
    object_template
        .set_call_as_function_handler_with_data(document_all_call_callback, Some(data.into()));
    let collection = object_template.new_instance(scope)?;
    let _ = collection.set_prototype(scope, prototype.into());
    let _ = data.set(scope, v8str(scope, "self").into(), collection.into());

    for index in 0..items.length() {
        let Some(value) = items.get_index(scope, index) else {
            continue;
        };
        define_collection_value_property(
            scope,
            collection,
            v8::Integer::new_from_unsigned(scope, index).into(),
            value,
            v8::PropertyAttribute::DONT_ENUM,
        );
    }

    let names = named.get_property_names(scope, Default::default())?;
    let names_length = names.length();
    for index in 0..names_length {
        let Some(key) = names.get_index(scope, index) else {
            continue;
        };
        let Some(value) = named.get(scope, key) else {
            continue;
        };
        let Ok(key_name) = v8::Local::<v8::Name>::try_from(key) else {
            continue;
        };
        let Ok(key_string) = v8::Local::<v8::String>::try_from(key) else {
            continue;
        };
        if collections::array_index_property_name(&key_string.to_rust_string_lossy(scope)).is_some()
        {
            continue;
        }
        if collections::is_document_all_named_collection_value(scope, value) {
            let _ = collection.set_native_data_property_with_configuration(
                scope,
                key_name,
                v8::NativeDataPropertyConfiguration::new(document_all_named_collection_getter)
                    .data(value)
                    .property_attribute(v8::PropertyAttribute::DONT_ENUM),
            );
            continue;
        }
        define_collection_value_property(
            scope,
            collection,
            key,
            value,
            v8::PropertyAttribute::DONT_ENUM,
        );
    }

    DocumentAllCollectionSurfaceDeclaration::new(data, items.length() as f64)
        .initialize(scope, collection)
        .ok()?;

    Some(collection)
}
