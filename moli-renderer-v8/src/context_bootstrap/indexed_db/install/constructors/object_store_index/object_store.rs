use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBObjectStore, enumerable)]
struct IdbObjectStorePrototypeDeclaration {
    #[webapi(method, length = 1, callback = idb_object_store_get_callback)]
    get: (),
    #[webapi(method, length = 2, callback = idb_object_store_get_all_callback)]
    get_all: (),
    #[webapi(method, length = 1, callback = idb_object_store_get_key_callback)]
    get_key: (),
    #[webapi(method, length = 2, callback = idb_object_store_get_all_keys_callback)]
    get_all_keys: (),
    #[webapi(method, length = 1, callback = idb_object_store_count_callback)]
    count: (),
    #[webapi(method, length = 2, callback = idb_object_store_put_callback)]
    put: (),
    #[webapi(method, length = 2, callback = idb_object_store_add_callback)]
    add: (),
    #[webapi(method = "delete", length = 1, callback = idb_object_store_delete_callback)]
    _delete: (),
    #[webapi(method, length = 0, callback = idb_object_store_clear_callback)]
    clear: (),
    #[webapi(method, length = 3, callback = idb_object_store_create_index_callback)]
    create_index: (),
    #[webapi(method, length = 1, callback = idb_object_store_index_callback)]
    index: (),
    #[webapi(method, length = 1, callback = idb_object_store_delete_index_callback)]
    delete_index: (),
    #[webapi(method, length = 2, callback = idb_object_store_open_cursor_callback)]
    open_cursor: (),
    #[webapi(method, length = 2, callback = idb_object_store_open_key_cursor_callback)]
    open_key_cursor: (),
}

pub(super) fn install_object_store_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    IdbObjectStorePrototypeDeclaration::initialize_prototype_template(scope, prototype);
}
