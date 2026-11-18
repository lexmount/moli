use super::*;
use crate::context_bootstrap::indexed_db::{
    idb_cursor_direction_getter, idb_cursor_key_getter, idb_cursor_primary_key_getter,
    idb_cursor_request_getter, idb_cursor_source_getter, idb_cursor_value_getter,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBCursor, enumerable)]
struct IdbCursorPrototypeDeclaration {
    #[webapi(accessor_property, getter = idb_cursor_source_getter)]
    source: (),
    #[webapi(accessor_property, getter = idb_cursor_request_getter)]
    request: (),
    #[webapi(accessor_property, getter = idb_cursor_direction_getter)]
    direction: (),
    #[webapi(accessor_property, getter = idb_cursor_key_getter)]
    key: (),
    #[webapi(accessor_property, getter = idb_cursor_primary_key_getter)]
    primary_key: (),
    #[webapi(method, length = 1, callback = idb_cursor_advance_callback)]
    advance: (),
    #[webapi(method = "continue", length = 0, callback = idb_cursor_continue_callback)]
    _continue: (),
    #[webapi(
        method,
        length = 2,
        callback = idb_cursor_continue_primary_key_callback
    )]
    continue_primary_key: (),
    #[webapi(method, length = 1, callback = idb_cursor_update_callback)]
    update: (),
    #[webapi(method = "delete", length = 0, callback = idb_cursor_delete_callback)]
    _delete: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBCursorWithValue, enumerable)]
struct IdbCursorWithValuePrototypeDeclaration {
    #[webapi(accessor_property, getter = idb_cursor_value_getter)]
    value: (),
}

pub(super) fn install_cursor_and_key_range_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "IDBCursor" => {
            IdbCursorPrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        "IDBCursorWithValue" => {
            IdbCursorWithValuePrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        "IDBKeyRange" => moli_webapi::indexed_db_values::range::install(scope, template),
        _ => {}
    }
}
