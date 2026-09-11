use super::*;
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
    #[webapi(slot = INDEXED_DB_EVENT_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    db: v8::Local<'scope, v8::Object>,
    mode: &'static str,
    #[webapi(init = "null")]
    error: (),
    object_store_names: v8::Local<'scope, v8::Object>,
    #[webapi(init = "null")]
    onabort: (),
    #[webapi(init = "null")]
    oncomplete: (),
    #[webapi(init = "null")]
    onerror: (),
}

pub(in crate::context_bootstrap::indexed_db) fn create_transaction_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    db: v8::Local<'s, v8::Object>,
    handle: Option<TransactionHandle>,
    mode: TransactionMode,
    store_names: &[String],
) -> Option<v8::Local<'s, v8::Object>> {
    let handle_raw = handle.map(|handle| handle.into_raw() as f64);
    let mode: &'static str = mode.into();
    let db_key = object_string_property(scope, db, INDEXED_DB_DATABASE_KEY_SLOT);
    let object_store_names = new_idb_dom_string_list(scope, store_names);
    let tx = IdbTransactionObjectDeclaration::new(db, mode, object_store_names)
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
    register_indexed_db_transaction_lifecycle(scope, tx, handle, handle_raw.is_some(), db_key);
    Some(tx)
}
