use super::*;
use crate::context_bootstrap::SIMPLE_EVENT_TARGET_LISTENERS_SLOT;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::IDBTransaction,
    require_prototype,
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
}

pub(in crate::context_bootstrap::indexed_db) fn create_transaction_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    db: v8::Local<'s, v8::Object>,
    handle: Option<TransactionHandle>,
    mode: TransactionMode,
    store_names: &[moli_indexeddb::IndexedDbName],
) -> Option<v8::Local<'s, v8::Object>> {
    let mode_label: &'static str = mode.into();
    let tx = IdbTransactionObjectDeclaration::new(db, mode_label)
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
    register_indexed_db_transaction_lifecycle(
        scope,
        tx,
        db,
        handle,
        mode,
        store_names,
        handle.is_some(),
    );
    crate::context_bootstrap::indexed_db::initialize_indexed_db_event_target(scope, tx, Some(db));
    Some(tx)
}
