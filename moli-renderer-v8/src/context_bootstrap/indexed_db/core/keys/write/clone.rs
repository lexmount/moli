use super::*;

pub(in crate::context_bootstrap::indexed_db) fn clone_value_for_transaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Value>, IndexedDbValue)> {
    set_indexed_db_slot_value(
        scope,
        transaction,
        INDEXED_DB_TRANSACTION_ACTIVE_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    // Restore activity even after a clone exception, but never resurrect a
    // transaction that an author getter aborted during serialization.
    let result = serialize_js_value(&mut scope, value)
        .and_then(|bytes| deserialize_js_value(&mut scope, &bytes).map(|clone| (clone, bytes)));
    if !object_bool_property(
        &mut scope,
        transaction,
        INDEXED_DB_TRANSACTION_FINISHED_SLOT,
    )
    .unwrap_or(false)
        && !object_bool_property(&mut scope, transaction, INDEXED_DB_TRANSACTION_ABORTED_SLOT)
            .unwrap_or(false)
        && !object_bool_property(
            &mut scope,
            transaction,
            INDEXED_DB_TRANSACTION_COMMITTING_SLOT,
        )
        .unwrap_or(false)
    {
        let active = v8::Boolean::new(&scope, true);
        set_indexed_db_slot_value(
            &mut scope,
            transaction,
            INDEXED_DB_TRANSACTION_ACTIVE_SLOT,
            active.into(),
        );
    }
    // Keep the exception caught until state restoration is complete: further
    // V8 property accesses must not consume a pending rethrow.
    if scope.has_caught() {
        scope.rethrow();
        return None;
    }
    result
}
