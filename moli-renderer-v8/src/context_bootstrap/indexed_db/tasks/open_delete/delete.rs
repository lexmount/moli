use super::*;

pub(in crate::context_bootstrap::indexed_db) fn execute_delete_database_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    storage_scope: IndexedDbStorageScope,
    name: String,
) {
    if let Err(error) = validate_storage_bucket_scope(scope, &storage_scope) {
        let error = request_error_object(scope, &error);
        store_request_error(scope, request, error);
        return;
    }
    let origin = storage_scope.storage_key().to_owned();
    match with_indexed_db_manager(scope, |manager| manager.delete_database(&origin, &name)) {
        Ok(()) => store_request_success(scope, request, v8::undefined(scope).into()),
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
