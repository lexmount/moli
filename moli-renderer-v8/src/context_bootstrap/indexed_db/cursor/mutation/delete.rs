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
    match with_indexed_db_manager(scope, |manager| {
        manager.delete(target.handle, &target.store_name, &target.primary_key)
    }) {
        Ok(()) => store_request_success(scope, request, v8::undefined(scope).into()),
        Err(error) => {
            let error = request_error_object(scope, &error);
            store_request_error(scope, request, error);
        }
    }
    rv.set(request.into());
}
