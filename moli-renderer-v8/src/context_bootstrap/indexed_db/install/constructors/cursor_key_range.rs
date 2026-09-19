use super::*;
use crate::context_bootstrap::indexed_db::{
    idb_cursor_direction_getter, idb_cursor_key_getter, idb_cursor_primary_key_getter,
    idb_cursor_request_getter, idb_cursor_source_getter, idb_cursor_value_getter,
    idb_key_range_lower_getter, idb_key_range_lower_open_getter, idb_key_range_upper_getter,
    idb_key_range_upper_open_getter,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBCursor, enumerable, receiver)]
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
#[webapi(interface = web_api_interfaces::IDBCursorWithValue, enumerable, receiver)]
struct IdbCursorWithValuePrototypeDeclaration {
    #[webapi(accessor_property, getter = idb_cursor_value_getter)]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBKeyRange, enumerable, receiver)]
struct IdbKeyRangePrototypeDeclaration {
    #[webapi(accessor_property, getter = idb_key_range_lower_getter)]
    lower: (),
    #[webapi(accessor_property, getter = idb_key_range_upper_getter)]
    upper: (),
    #[webapi(accessor_property, getter = idb_key_range_lower_open_getter)]
    lower_open: (),
    #[webapi(accessor_property, getter = idb_key_range_upper_open_getter)]
    upper_open: (),
    #[webapi(method, length = 1, callback = idb_key_range_includes_callback)]
    includes: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBKeyRange, enumerable)]
struct IdbKeyRangeConstructorDeclaration {
    #[webapi(static_method, length = 1, callback = idb_key_range_only_callback)]
    only: (),
    #[webapi(static_method, length = 2, callback = idb_key_range_bound_callback)]
    bound: (),
    #[webapi(
        static_method,
        length = 1,
        callback = idb_key_range_lower_bound_callback
    )]
    lower_bound: (),
    #[webapi(
        static_method,
        length = 1,
        callback = idb_key_range_upper_bound_callback
    )]
    upper_bound: (),
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
        "IDBKeyRange" => {
            IdbKeyRangeConstructorDeclaration::initialize_template(scope, template);
            IdbKeyRangePrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        _ => {}
    }
}
