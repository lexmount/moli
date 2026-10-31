use super::*;
use crate::util::v8str;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBIndex, enumerable, receiver)]
struct IdbIndexPrototypeDeclaration {
    #[webapi(accessor_property, getter = idb_schema_attribute_getter, setter = idb_index_name_setter, data = v8str(scope, SCHEMA_NAME_SLOT))]
    name: (),
    #[webapi(accessor_property, getter = idb_schema_attribute_getter, data = v8str(scope, SCHEMA_KEY_PATH_SLOT))]
    key_path: (),
    #[webapi(accessor_property, getter = idb_schema_attribute_getter, data = v8str(scope, SCHEMA_MULTI_ENTRY_SLOT))]
    multi_entry: (),
    #[webapi(accessor_property, getter = idb_schema_attribute_getter, data = v8str(scope, SCHEMA_UNIQUE_SLOT))]
    unique: (),
    #[webapi(accessor_property, getter = idb_schema_attribute_getter, data = v8str(scope, INDEX_OBJECT_STORE))]
    object_store: (),
    #[webapi(method, length = 1, callback = idb_index_get_callback)]
    get: (),
    #[webapi(method, length = 1, callback = idb_index_get_key_callback)]
    get_key: (),
    #[webapi(method, length = 0, callback = idb_index_get_all_callback)]
    get_all: (),
    #[webapi(method, length = 0, callback = idb_index_get_all_keys_callback)]
    get_all_keys: (),
    #[webapi(method, length = 0, callback = idb_index_get_all_records_callback)]
    get_all_records: (),
    #[webapi(method, length = 0, callback = idb_index_count_callback)]
    count: (),
    #[webapi(method, length = 0, callback = idb_index_open_cursor_callback)]
    open_cursor: (),
    #[webapi(method, length = 0, callback = idb_index_open_key_cursor_callback)]
    open_key_cursor: (),
}

pub(super) fn install_index_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    IdbIndexPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}
