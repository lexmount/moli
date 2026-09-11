use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    plain,
    scope_lifetime = 'scope,
    data_properties,
    enumerable
)]
struct IndexDescriptorDeclaration<'scope, 'value> {
    name: &'value str,
    key_path: v8::Local<'scope, v8::Value>,
    unique: bool,
    multi_entry: bool,
}

#[derive(WebApiObject)]
#[webapi(
    plain,
    scope_lifetime = 'scope,
    data_properties,
    enumerable
)]
struct ObjectStoreDescriptorDeclaration<'scope, 'value> {
    name: &'value str,
    key_path: v8::Local<'scope, v8::Value>,
    auto_increment: bool,
}

pub(in crate::context_bootstrap::indexed_db) fn create_index_descriptor_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    info: &IndexInfo,
) -> Option<v8::Local<'s, v8::Object>> {
    let key_path = key_path_to_js_value(scope, &info.key_path)?;
    let descriptor = new_null_prototype_object(scope);
    IndexDescriptorDeclaration::new(&info.name, key_path, info.unique, info.multi_entry)
        .initialize(scope, descriptor)
        .ok()?;
    Some(descriptor)
}

pub(in crate::context_bootstrap::indexed_db) fn create_object_store_descriptor_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    info: &ObjectStoreInfo,
    indexes: &[IndexInfo],
) -> Option<v8::Local<'s, v8::Object>> {
    let key_path = match &info.key_path {
        Some(key_path) => key_path_to_js_value(scope, key_path)?,
        None => v8::null(scope).into(),
    };
    let descriptor = new_null_prototype_object(scope);
    ObjectStoreDescriptorDeclaration::new(&info.name, key_path, info.auto_increment)
        .initialize(scope, descriptor)
        .ok()?;
    let index_names = if indexes.is_empty() {
        info.index_names.clone()
    } else {
        indexes.iter().map(|index| index.name.clone()).collect()
    };
    let index_names_list = new_idb_dom_string_list(scope, &index_names);
    set_indexed_db_internal_object_property(
        scope,
        descriptor,
        "indexNames",
        index_names_list.into(),
    );
    let index_map = new_null_prototype_object(scope);
    for index in indexes {
        let descriptor_value = create_index_descriptor_object(scope, index)?;
        set_indexed_db_internal_object_property(
            scope,
            index_map,
            &index.name,
            descriptor_value.into(),
        );
    }
    set_indexed_db_internal_object_property(scope, descriptor, "indexes", index_map.into());
    Some(descriptor)
}
