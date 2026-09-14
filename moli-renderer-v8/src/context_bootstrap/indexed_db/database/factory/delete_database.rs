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
    let request_storage_scope = storage_scope.clone();
    let Some(request) =
        create_open_request_object(scope, args.this(), owner, request_storage_scope)
    else {
        rv.set_undefined();
        return;
    };
    enqueue_connection_request(
        scope,
        request,
        storage_scope,
        name,
        ConnectionOperation::Delete,
    );
    rv.set(request.into());
}
