use super::*;

pub(in crate::context_bootstrap::indexed_db) fn prepare_cursor_request(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
) {
    let pending = v8str(scope, "pending").into();
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT, pending);
    let undefined = v8::undefined(scope).into();
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_RESULT_SLOT, undefined);
    let null = v8::null(scope).into();
    set_indexed_db_slot_value(scope, request, INDEXED_DB_REQUEST_ERROR_SLOT, null);
}
