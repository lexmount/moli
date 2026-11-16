use crate::context_bootstrap::{
    simple_event_target_handler_getter_callback, simple_event_target_handler_setter_callback,
};
use crate::util::v8str;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBRequest, enumerable, receiver)]
struct IDBRequestEventHandlers {
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onsuccess"), setter_data = v8str(scope, "onsuccess"))]
    onsuccess: (),
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onerror"), setter_data = v8str(scope, "onerror"))]
    onerror: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBOpenDBRequest, enumerable, receiver)]
struct IDBOpenDBRequestEventHandlers {
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onupgradeneeded"), setter_data = v8str(scope, "onupgradeneeded"))]
    onupgradeneeded: (),
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onblocked"), setter_data = v8str(scope, "onblocked"))]
    onblocked: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBTransaction, enumerable, receiver)]
struct IDBTransactionEventHandlers {
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onabort"), setter_data = v8str(scope, "onabort"))]
    onabort: (),
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "oncomplete"), setter_data = v8str(scope, "oncomplete"))]
    oncomplete: (),
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onerror"), setter_data = v8str(scope, "onerror"))]
    onerror: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBDatabase, enumerable, receiver)]
struct IDBDatabaseEventHandlers {
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onabort"), setter_data = v8str(scope, "onabort"))]
    onabort: (),
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onclose"), setter_data = v8str(scope, "onclose"))]
    onclose: (),
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onerror"), setter_data = v8str(scope, "onerror"))]
    onerror: (),
    #[webapi(accessor_property, getter = simple_event_target_handler_getter_callback, setter = simple_event_target_handler_setter_callback, data = v8str(scope, "onversionchange"), setter_data = v8str(scope, "onversionchange"))]
    onversionchange: (),
}

pub(super) fn install_idb_event_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "IDBRequest" => IDBRequestEventHandlers::initialize_prototype_template(scope, prototype),
        "IDBOpenDBRequest" => {
            IDBOpenDBRequestEventHandlers::initialize_prototype_template(scope, prototype)
        }
        "IDBTransaction" => {
            IDBTransactionEventHandlers::initialize_prototype_template(scope, prototype)
        }
        "IDBDatabase" => IDBDatabaseEventHandlers::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}
