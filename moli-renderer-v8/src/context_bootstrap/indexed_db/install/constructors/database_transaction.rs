use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBDatabase, enumerable)]
struct IdbDatabasePrototypeDeclaration {
    #[webapi(method, length = 2, callback = idb_database_create_object_store_callback)]
    create_object_store: (),
    #[webapi(method, length = 1, callback = idb_database_delete_object_store_callback)]
    delete_object_store: (),
    #[webapi(method, length = 2, callback = idb_database_transaction_callback)]
    transaction: (),
    #[webapi(method, length = 0, callback = idb_database_close_callback)]
    close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBTransaction, enumerable)]
struct IdbTransactionPrototypeDeclaration {
    #[webapi(method, length = 1, callback = idb_transaction_object_store_callback)]
    object_store: (),
    #[webapi(method, length = 0, callback = idb_transaction_abort_callback)]
    abort: (),
    #[webapi(method, length = 0, callback = idb_transaction_commit_callback)]
    commit: (),
}

pub(super) fn install_database_and_transaction_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "IDBDatabase" => {
            install_idb_event_target_methods(scope, prototype);
            IdbDatabasePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "IDBTransaction" => {
            install_idb_event_target_methods(scope, prototype);
            IdbTransactionPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}
