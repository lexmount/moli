use super::*;
use crate::context_bootstrap::indexed_db::{
    INDEXED_DB_TRANSACTION_FINISHED_SLOT, indexed_db_object_store_is_deleted,
};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBObjectStore.index")]
struct IdbObjectStoreIndexArgs {
    #[webidl(required, with = parse_index_name_arg)]
    index_name: IndexedDbName,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_object_store_index_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbObjectStoreIndexArgs>(scope, &args) else {
        return;
    };
    let index_name = parsed.index_name;
    let store = args.this();
    let finished = indexed_db_object_store_transaction(scope, store).is_none_or(|transaction| {
        object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_FINISHED_SLOT)
            .unwrap_or(false)
    });
    if indexed_db_object_store_is_deleted(scope, store) || finished {
        let error = dom_exception_value(
            scope,
            "The object store has been deleted or its transaction has finished.",
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return;
    }
    let Some(info) = index_info_from_store_metadata(scope, store, &index_name) else {
        let error =
            dom_exception_value(scope, "The requested index was not found.", "NotFoundError");
        scope.throw_exception(error);
        return;
    };
    let Some(context) = store.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(index) = create_index_object(scope, store, &info) {
        rv.set(index.into());
    } else {
        rv.set_undefined();
    }
}

fn parse_index_name_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<IndexedDbName, webidl::WebIdlError> {
    webidl::convert::<webidl::DomString16>(
        scope,
        args.get(index),
        webidl::Context::argument("IDBObjectStore.index", (index + 1) as usize),
    )
    .map(|name| IndexedDbName::from_utf16(name.0))
}
