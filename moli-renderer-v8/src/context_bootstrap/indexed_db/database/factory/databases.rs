use super::*;
use moli_webapi_declare::ObjectLiteralDeclaration;

pub(in crate::context_bootstrap::indexed_db) fn idb_factory_databases_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(owner) = idb_factory_effective_execution_owner(scope, args.this()) else {
        let exception = dom_exception_value(
            scope,
            "Failed to execute 'databases' on 'IDBFactory': access to the Indexed Database API is denied in this context.",
            "SecurityError",
        );
        let _ = resolver.reject(scope, exception);
        return;
    };
    let Some(storage_scope) = idb_factory_effective_storage_scope(scope, args.this(), owner) else {
        let exception = dom_exception_value(
            scope,
            "Failed to execute 'databases' on 'IDBFactory': access to the Indexed Database API is denied in this context.",
            "SecurityError",
        );
        schedule_databases_promise_settlement(scope, owner, None, resolver, exception, true);
        return;
    };
    let origin = storage_scope.storage_key().to_owned();
    if let Err(error) = validate_storage_bucket_scope(scope, &storage_scope) {
        let error = request_error_object(scope, &error);
        schedule_databases_promise_settlement(
            scope,
            owner,
            Some(storage_scope.clone()),
            resolver,
            error,
            true,
        );
        return;
    }

    let infos = match with_indexed_db_manager(scope, |manager| manager.databases(&origin)) {
        Ok(infos) => infos,
        Err(error) => {
            let error = request_error_object(scope, &error);
            schedule_databases_promise_settlement(
                scope,
                owner,
                Some(storage_scope.clone()),
                resolver,
                error,
                true,
            );
            return;
        }
    };
    let array = v8::Array::new(scope, infos.len() as i32);
    for (index, info) in infos.into_iter().enumerate() {
        let object = ObjectLiteralDeclaration::bind(scope);
        if let Some(name) = v8_string(scope, &info.name) {
            object.set_string_property(scope, "name", name.into());
        }
        let version = v8::Number::new(scope, info.version as f64);
        object.set_string_property(scope, "version", version.into());
        let _ = array.set_index(scope, index as u32, object.into_value());
    }
    schedule_databases_promise_settlement(
        scope,
        owner,
        Some(storage_scope),
        resolver,
        array.into(),
        false,
    );
}

fn schedule_databases_promise_settlement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: IndexedDbExecutionOwner,
    storage_scope: Option<IndexedDbStorageScope>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    value: v8::Local<'s, v8::Value>,
    reject: bool,
) {
    let task = v8::Object::new(scope);
    register_indexed_db_databases_settle_task(
        scope,
        task,
        owner,
        storage_scope,
        resolver,
        value,
        reject,
    );
    enqueue_indexed_db_task(scope, task);
}

pub(in crate::context_bootstrap::indexed_db) fn flush_databases_settle_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let Some((resolver, value, reject)) = indexed_db_databases_settle_task_payload(scope, task)
    else {
        return;
    };
    if reject {
        let _ = resolver.reject(scope, value);
    } else {
        let _ = resolver.resolve(scope, value);
    }
}
