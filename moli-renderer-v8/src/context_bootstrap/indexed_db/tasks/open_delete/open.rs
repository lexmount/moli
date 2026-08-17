use super::*;

mod info;
mod upgrade;

pub(in crate::context_bootstrap::indexed_db) fn execute_open_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    storage_scope: IndexedDbStorageScope,
    name: String,
    version: Option<u64>,
) {
    let database_name = name.clone();
    let storage_key = storage_scope.storage_key().to_owned();
    if let Err(error) = validate_storage_bucket_scope(scope, &storage_scope) {
        let error = request_error_object(scope, &error);
        store_request_error(scope, request, error);
        return;
    }
    match with_indexed_db_manager(scope, |manager| {
        manager.open(OpenOptions {
            origin: storage_key.clone(),
            name,
            version,
        })
    }) {
        Ok(opened) => {
            let info = match info::opened_database_info(scope, &database_name, &opened) {
                Ok(info) => info,
                Err(error) => {
                    let error = request_error_object(scope, &error);
                    store_request_error(scope, request, error);
                    return;
                }
            };
            let owner = indexed_db_typed_execution_owner(scope, request)
                .expect("IDB open request should have typed owner state");
            let Some(database) =
                create_database_object(scope, storage_scope.clone(), owner, opened.database, &info)
            else {
                return;
            };
            match opened.disposition {
                OpenDisposition::Existing => {
                    store_request_success(scope, request, database.into());
                }
                OpenDisposition::UpgradeNeeded {
                    old_version,
                    new_version,
                } => {
                    upgrade::enqueue_upgrade_needed_open_task(
                        scope,
                        request,
                        database,
                        opened.upgrade_transaction,
                        &info,
                        old_version,
                        new_version,
                    );
                }
            }
        }
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
}
