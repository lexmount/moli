use super::*;
use crate::context_bootstrap::indexed_db::set_indexed_db_deleted_database_version;

pub(in crate::context_bootstrap::indexed_db) fn execute_delete_database_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    storage_scope: IndexedDbStorageScope,
    name: IndexedDbName,
) {
    if let Err(error) = validate_storage_bucket_scope(scope, &storage_scope) {
        let error = request_error_object(scope, &error);
        store_request_error(scope, request, error);
        return;
    }
    let origin = storage_scope.storage_key().to_owned();
    // Capture the old version under the same manager lock as deletion. Another
    // queued open can recreate the database before this success event is sent.
    match with_indexed_db_manager(scope, |manager| {
        let old_version = manager.database_version(&origin, &name)?.unwrap_or(0);
        manager.delete_database(&origin, &name)?;
        Ok(old_version)
    }) {
        Ok(old_version) => {
            set_indexed_db_deleted_database_version(scope, request, old_version);
            store_request_success(scope, request, v8::undefined(scope).into());
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
