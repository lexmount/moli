use super::*;

pub(in crate::context_bootstrap::indexed_db) fn submit_cursor_open_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transaction: v8::Local<'s, v8::Object>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    store_name: &str,
    operation: IndexedDbCursorOpenOperation,
) {
    schedule_transaction_operation(
        scope,
        transaction,
        source,
        request,
        store_name,
        IndexedDbTransactionOperation::OpenCursor(operation),
    );
}

pub(in crate::context_bootstrap::indexed_db) fn execute_cursor_open_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handle: TransactionHandle,
    store_name: &str,
    operation: &IndexedDbCursorOpenOperation,
) {
    let entries = match &operation.source {
        IndexedDbCursorSource::ObjectStore => object_store_cursor_snapshot(
            scope,
            handle,
            store_name,
            operation.query.as_ref(),
            operation.direction,
            operation.key_only,
        )
        .map_err(KeyConversionError::from),
        IndexedDbCursorSource::Index(index_info) => index_cursor_snapshot(
            scope,
            handle,
            store_name,
            index_info,
            operation.query.as_ref(),
            operation.direction,
            operation.key_only,
        ),
    };
    settle_cursor_open_request(scope, source, request, entries, operation);
}

fn settle_cursor_open_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    entries: std::result::Result<Vec<CursorSnapshotEntry>, KeyConversionError>,
    operation: &IndexedDbCursorOpenOperation,
) {
    match entries {
        Ok(entries) if entries.is_empty() => {
            store_request_success(scope, request, v8::null(scope).into());
        }
        Ok(entries) => {
            let result = materialize_cursor_result_in_request_realm(
                scope,
                source,
                request,
                entries,
                operation.direction,
                operation.key_only,
            )
            .map_or_else(|| v8::null(scope).into(), Into::into);
            store_request_success(scope, request, result);
        }
        Err(error) => {
            if let Some(error) = error.into_value(scope) {
                store_request_error(scope, request, error);
            }
        }
    }
}
