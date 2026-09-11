use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBFactory, enumerable)]
struct IdbFactoryPrototypeDeclaration {
    #[webapi(method, length = 2, callback = idb_factory_open_callback)]
    open: (),
    #[webapi(method, length = 1, callback = idb_factory_delete_database_callback)]
    delete_database: (),
    #[webapi(method, length = 0, callback = idb_factory_databases_callback)]
    databases: (),
    #[webapi(method, length = 2, callback = idb_factory_cmp_callback)]
    cmp: (),
}

pub(super) fn install_factory_and_request_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "IDBFactory" => {
            IdbFactoryPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "IDBRequest" => install_idb_event_target_methods(scope, prototype),
        _ => {}
    }
}
