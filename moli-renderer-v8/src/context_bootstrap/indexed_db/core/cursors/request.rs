use crate::context_bootstrap::indexed_db::{
    IndexedDbRequestReadyState, IndexedDbRequestValue, set_indexed_db_request_ready_state,
    set_request_value,
};

pub(in crate::context_bootstrap::indexed_db) fn prepare_cursor_request(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
) {
    set_indexed_db_request_ready_state(scope, request, IndexedDbRequestReadyState::Pending);
    let undefined = v8::undefined(scope).into();
    set_request_value(scope, request, IndexedDbRequestValue::Result, undefined);
    let null = v8::null(scope).into();
    set_request_value(scope, request, IndexedDbRequestValue::Error, null);
}
