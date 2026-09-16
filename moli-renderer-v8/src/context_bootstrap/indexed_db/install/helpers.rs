use crate::context_bootstrap::indexed_db::{
    idb_onabort_getter, idb_onabort_setter, idb_onblocked_getter, idb_onblocked_setter,
    idb_onclose_getter, idb_onclose_setter, idb_oncomplete_getter, idb_oncomplete_setter,
    idb_onerror_getter, idb_onerror_setter, idb_onsuccess_getter, idb_onsuccess_setter,
    idb_onupgradeneeded_getter, idb_onupgradeneeded_setter, idb_onversionchange_getter,
    idb_onversionchange_setter,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBRequest, enumerable, receiver)]
struct IDBRequestEventHandlers {
    #[webapi(accessor_property, getter = idb_onsuccess_getter, setter = idb_onsuccess_setter)]
    onsuccess: (),
    #[webapi(accessor_property, getter = idb_onerror_getter, setter = idb_onerror_setter)]
    onerror: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBOpenDBRequest, enumerable, receiver)]
struct IDBOpenDBRequestEventHandlers {
    #[webapi(accessor_property, getter = idb_onupgradeneeded_getter, setter = idb_onupgradeneeded_setter)]
    onupgradeneeded: (),
    #[webapi(accessor_property, getter = idb_onblocked_getter, setter = idb_onblocked_setter)]
    onblocked: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBTransaction, enumerable, receiver)]
struct IDBTransactionEventHandlers {
    #[webapi(accessor_property, getter = idb_onabort_getter, setter = idb_onabort_setter)]
    onabort: (),
    #[webapi(accessor_property, getter = idb_oncomplete_getter, setter = idb_oncomplete_setter)]
    oncomplete: (),
    #[webapi(accessor_property, getter = idb_onerror_getter, setter = idb_onerror_setter)]
    onerror: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IDBDatabase, enumerable, receiver)]
struct IDBDatabaseEventHandlers {
    #[webapi(accessor_property, getter = idb_onabort_getter, setter = idb_onabort_setter)]
    onabort: (),
    #[webapi(accessor_property, getter = idb_onclose_getter, setter = idb_onclose_setter)]
    onclose: (),
    #[webapi(accessor_property, getter = idb_onerror_getter, setter = idb_onerror_setter)]
    onerror: (),
    #[webapi(accessor_property, getter = idb_onversionchange_getter, setter = idb_onversionchange_setter)]
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
