use super::*;
use crate::context_bootstrap::indexed_db::ensure_indexed_db_runtime_state;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBFactory.open")]
struct IdbFactoryOpenArgs {
    #[webidl(required)]
    name: String,
    #[webidl(converter = "enforce_range_unsigned_long_long")]
    version: Option<u64>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_factory_open_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbFactoryOpenArgs>(scope, &args) else {
        return;
    };
    let name = parsed.name;
    let version = parsed.version;
    if version == Some(0) {
        throw_type_error(
            scope,
            "Failed to execute 'open' on 'IDBFactory': The version provided must not be 0.",
        );
        return;
    }
    let Some(owner) = idb_factory_effective_execution_owner(scope, args.this()) else {
        let exception = dom_exception_value(
            scope,
            "Failed to execute 'open' on 'IDBFactory': access to the Indexed Database API is denied in this context.",
            "SecurityError",
        );
        scope.throw_exception(exception);
        return;
    };
    let Some(storage_scope) = idb_factory_effective_storage_scope(scope, args.this(), owner) else {
        let exception = dom_exception_value(
            scope,
            "Failed to execute 'open' on 'IDBFactory': access to the Indexed Database API is denied in this context.",
            "SecurityError",
        );
        scope.throw_exception(exception);
        return;
    };
    let request_storage_scope = storage_scope.clone();
    let _ = ensure_indexed_db_runtime_state(scope);

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
        ConnectionOperation::Open(version),
    );
    rv.set(request.into());
}
