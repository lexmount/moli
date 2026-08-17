use super::*;
use crate::context_bootstrap::indexed_db::ensure_indexed_db_runtime_state;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBFactory.deleteDatabase")]
struct IdbFactoryDeleteDatabaseArgs {
    #[webidl(required)]
    name: String,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_factory_delete_database_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbFactoryDeleteDatabaseArgs>(scope, &args) else {
        return;
    };
    let name = parsed.name;
    let Some(owner) = idb_factory_effective_execution_owner(scope, args.this()) else {
        let exception = dom_exception_value(
            scope,
            "Failed to execute 'deleteDatabase' on 'IDBFactory': access to the Indexed Database API is denied in this context.",
            "SecurityError",
        );
        scope.throw_exception(exception);
        return;
    };
    let Some(storage_scope) = idb_factory_effective_storage_scope(scope, args.this(), owner) else {
        let exception = dom_exception_value(
            scope,
            "Failed to execute 'deleteDatabase' on 'IDBFactory': access to the Indexed Database API is denied in this context.",
            "SecurityError",
        );
        scope.throw_exception(exception);
        return;
    };
    let _ = ensure_indexed_db_runtime_state(scope);
    let origin = storage_scope.storage_key().to_owned();
    let request_storage_scope = storage_scope.clone();
    let Some(request) =
        create_open_request_object(scope, args.this(), owner, request_storage_scope)
    else {
        rv.set_undefined();
        return;
    };
    if let Err(error) = validate_storage_bucket_scope(scope, &storage_scope) {
        let error = request_error_object(scope, &error);
        store_request_error(scope, request, error);
        rv.set(request.into());
        return;
    }
    let registry_key = database_registry_key(&origin, &name);
    let has_open_connections = has_open_database_connections_for_key(scope, &registry_key);
    let delete_blocked =
        match with_indexed_db_manager(scope, |manager| manager.database_version(&origin, &name)) {
            Ok(version) if has_open_connections => Some(
                version
                    .or_else(|| open_database_connection_version_for_key(scope, &registry_key))
                    .unwrap_or(0),
            ),
            Ok(_) => None,
            Err(error) => {
                let error = request_error_object(scope, &error);
                store_request_error(scope, request, error);
                rv.set(request.into());
                return;
            }
        };
    if let Some(old_version) = delete_blocked {
        enqueue_blocked_delete_task(scope, request, &origin, &name, old_version);
    } else {
        execute_delete_database_request(scope, request, storage_scope, name);
    }
    rv.set(request.into());
}
