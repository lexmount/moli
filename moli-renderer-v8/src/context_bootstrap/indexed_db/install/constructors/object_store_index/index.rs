use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBIndex, enumerable, receiver)]
struct IdbIndexPrototypeDeclaration {
    #[webapi(accessor_property, getter = idb_index_key_path_getter)]
    key_path: (),
    #[webapi(accessor_property, getter = idb_index_multi_entry_getter)]
    multi_entry: (),
    #[webapi(accessor_property, getter = idb_index_unique_getter)]
    unique: (),
    #[webapi(accessor_property, getter = idb_index_object_store_getter)]
    object_store: (),
    #[webapi(accessor_property, getter = idb_index_name_getter, setter = idb_index_name_setter)]
    name: (),
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
    #[webapi(method, length = 1, callback = idb_index_count_callback)]
    count: (),
    #[webapi(method, length = 2, callback = idb_index_open_cursor_callback)]
    open_cursor: (),
    #[webapi(method, length = 2, callback = idb_index_open_key_cursor_callback)]
    open_key_cursor: (),
}

pub(super) fn install_index_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    IdbIndexPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}
