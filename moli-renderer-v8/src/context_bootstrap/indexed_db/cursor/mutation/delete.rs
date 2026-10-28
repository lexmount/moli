use super::*;

pub(in crate::context_bootstrap::indexed_db) fn idb_cursor_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let cursor = args.this();
    let Some(target) = prepare_cursor_mutation(scope, cursor) else {
        return;
    };
    let Some(request) = target.create_request(scope, cursor) else {
        return;
    };
    schedule_transaction_operation(
        scope,
        target.transaction,
        target.store,
        request,
        &target.store_name,
        IndexedDbTransactionOperation::CursorDelete {
            key: target.primary_key,
        },
    );
    rv.set(request.into());
}
