use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBCursor, enumerable)]
struct IdbCursorPrototypeDeclaration {
    #[webapi(method, length = 1, callback = idb_cursor_advance_callback)]
    advance: (),
    #[webapi(method = "continue", length = 1, callback = idb_cursor_continue_callback)]
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
#[webapi(interface = web_api_interfaces::IDBKeyRange, enumerable)]
struct IdbKeyRangePrototypeDeclaration {
    #[webapi(method, length = 1, callback = idb_key_range_includes_callback)]
    includes: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBKeyRange, enumerable)]
struct IdbKeyRangeConstructorDeclaration {
    #[webapi(static_method, length = 1, callback = idb_key_range_only_callback)]
    only: (),
    #[webapi(static_method, length = 4, callback = idb_key_range_bound_callback)]
    bound: (),
    #[webapi(
        static_method,
        length = 2,
        callback = idb_key_range_lower_bound_callback
    )]
    lower_bound: (),
    #[webapi(
        static_method,
        length = 2,
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
