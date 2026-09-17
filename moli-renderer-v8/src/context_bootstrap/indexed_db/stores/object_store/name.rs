use super::*;
use crate::context_bootstrap::indexed_db::{idb_name_to_v8, rename_indexed_db_store_metadata};

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(name) = indexed_db_object_store_name(scope, args.this()) {
        rv.set(idb_name_to_v8(scope, &name).into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_name_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    // Generated receiver validation precedes conversion. Conversion can run
    // script, so read schema and transaction state only after it succeeds.
    let name = match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::member("IDBObjectStore", "name"),
    ) {
        Ok(name) => IndexedDbName::from_utf16(name.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Err(error) = rename_store(scope, args.this(), name) {
        let error = request_error_object(scope, &error);
        scope.throw_exception(error);
    }
}

fn rename_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    store: v8::Local<'s, v8::Object>,
    name: IndexedDbName,
) -> Result<(), IndexedDbError> {
    // https://w3c.github.io/IndexedDB/#dom-idbobjectstore-name
    // Deleted stores fail InvalidState even after their upgrade aborts.
    let (_, _, handle, old_name) = object_store_versionchange_common(scope, store)?;
    if old_name == name {
        return Ok(());
    }
    let context = store.get_creation_context(scope).ok_or_else(|| {
        IndexedDbError::InvalidState("The object store is unavailable.".to_owned())
    })?;
    let scope = &mut v8::ContextScope::new(scope, context);
    with_indexed_db_manager(scope, |manager| {
        manager.rename_object_store(handle, &old_name, name.clone())
    })?;
    rename_indexed_db_store_metadata(scope, store, &old_name, &name);
    Ok(())
}
