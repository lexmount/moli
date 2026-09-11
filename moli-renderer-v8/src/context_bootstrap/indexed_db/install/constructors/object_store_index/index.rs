use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBIndex, enumerable)]
struct IdbIndexPrototypeDeclaration {
    #[webapi(method, length = 1, callback = idb_index_get_callback)]
    get: (),
    #[webapi(method, length = 1, callback = idb_index_get_key_callback)]
    get_key: (),
    #[webapi(method, length = 2, callback = idb_index_get_all_callback)]
    get_all: (),
    #[webapi(method, length = 2, callback = idb_index_get_all_keys_callback)]
    get_all_keys: (),
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
