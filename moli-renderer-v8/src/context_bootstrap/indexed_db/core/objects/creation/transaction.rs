use super::*;
use crate::context_bootstrap::{
    SIMPLE_EVENT_TARGET_LISTENERS_SLOT, simple_event_target_handler_getter_callback,
    simple_event_target_handler_setter_callback,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::IDBTransaction,
    require_prototype,
    data_properties,
    enumerable
)]
struct IdbTransactionObjectDeclaration<'scope> {
    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    #[webapi(slot = crate::context_bootstrap::indexed_db::TRANSACTION_DB_SLOT)]
    db: v8::Local<'scope, v8::Object>,
    #[webapi(slot = crate::context_bootstrap::indexed_db::TRANSACTION_MODE_SLOT)]
    mode: &'static str,
    #[webapi(slot = crate::context_bootstrap::indexed_db::TRANSACTION_ERROR_SLOT, init = "null")]
    error: (),
    object_store_names: v8::Local<'scope, v8::Object>,
    #[webapi(
        accessor_property = "onabort",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onabort"),
        setter_data = v8str(scope, "onabort"),
        enumerable,
        receiver = web_api_interfaces::IDBTransaction::is_instance
    )]
    onabort: (),
    #[webapi(
        accessor_property = "oncomplete",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "oncomplete"),
        setter_data = v8str(scope, "oncomplete"),
        enumerable,
        receiver = web_api_interfaces::IDBTransaction::is_instance
    )]
    oncomplete: (),
    #[webapi(
        accessor_property = "onerror",
        getter = simple_event_target_handler_getter_callback,
        setter = simple_event_target_handler_setter_callback,
        data = v8str(scope, "onerror"),
        setter_data = v8str(scope, "onerror"),
        enumerable,
        receiver = web_api_interfaces::IDBTransaction::is_instance
    )]
    onerror: (),
}

pub(in crate::context_bootstrap::indexed_db) fn create_transaction_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    db: v8::Local<'s, v8::Object>,
    handle: Option<TransactionHandle>,
    mode: TransactionMode,
    store_names: &[moli_indexeddb::IndexedDbName],
) -> Option<v8::Local<'s, v8::Object>> {
    let mode_label: &'static str = mode.into();
    let object_store_names = new_idb_name_list(scope, store_names);
    let tx = IdbTransactionObjectDeclaration::new(db, mode_label, object_store_names)
        .bind(scope)
        .ok()?;
    let storage_scope = indexed_db_typed_storage_scope(scope, db);
    let owner = indexed_db_typed_execution_owner(scope, db)
        .expect("IDBTransaction should inherit typed owner from database");
    register_indexed_db_wrapper_with_owner(
        scope,
        tx,
        IndexedDbWrapperKind::Transaction,
        owner,
        storage_scope,
    );
    register_indexed_db_transaction_lifecycle(scope, tx, db, handle, mode, handle.is_some());
    Some(tx)
}
