use super::*;
use crate::context_bootstrap::indexed_db::{
    IdbTransactionDurability, TRANSACTION_DB_SLOT, TRANSACTION_ERROR_SLOT, TRANSACTION_MODE_SLOT,
    initialize_indexed_db_event_target,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IDBTransaction, require_prototype)]
struct IdbTransactionObjectDeclaration<'scope> {
    #[webapi(slot = INDEXED_DB_EVENT_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    // Heap-owned references remain observable after dispatch roots are released,
    // without introducing Rust Global roots for transaction/database cycles.
    #[webapi(slot = TRANSACTION_DB_SLOT)]
    db: v8::Local<'scope, v8::Object>,
    #[webapi(slot = TRANSACTION_MODE_SLOT)]
    mode: &'static str,
    #[webapi(slot = TRANSACTION_ERROR_SLOT, init = "null")]
    error: (),
}

pub(in crate::context_bootstrap::indexed_db) fn create_transaction_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    db: v8::Local<'s, v8::Object>,
    handle: Option<TransactionHandle>,
    mode: TransactionMode,
    durability: IdbTransactionDurability,
    store_names: &[IndexedDbName],
) -> Option<v8::Local<'s, v8::Object>> {
    let db_key = object_string_property(scope, db, INDEXED_DB_DATABASE_KEY_SLOT);
    let tx = IdbTransactionObjectDeclaration::new(db, mode.into())
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
    register_indexed_db_transaction_lifecycle(scope, tx, db, handle, mode, durability, db_key);
    crate::context_bootstrap::indexed_db::set_indexed_db_transaction_store_names(
        scope,
        tx,
        store_names,
    );
    initialize_indexed_db_event_target(scope, tx, Some(db));
    Some(tx)
}
