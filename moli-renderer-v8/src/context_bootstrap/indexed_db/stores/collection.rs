use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbValue, Key, execute_index_get_all_request, execute_object_store_get_all_request,
    indexed_db_index_is_deleted, object_store_active_transaction,
};
use moli_indexeddb::{GetAllOptionsCandidate, should_parse_get_all_options};

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap::indexed_db) enum CollectionKind {
    Value,
    Key,
    Record,
}

impl CollectionKind {
    pub(in crate::context_bootstrap::indexed_db) fn result<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        key: &Key,
        primary_key: &Key,
        bytes: &IndexedDbValue,
    ) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::Key => Some(key_to_js_value(scope, primary_key)),
            Self::Value => deserialize_js_value(scope, bytes),
            Self::Record => {
                let value = deserialize_js_value(scope, bytes)?;
                super::super::record::create_record(scope, key, primary_key, value).map(Into::into)
            }
        }
    }
}

// This snapshot is shared by immediate execution and transactions waiting to start.
// It contains no author objects or getters that could be observed a second time.
pub(in crate::context_bootstrap::indexed_db) struct CollectionRequest {
    pub(in crate::context_bootstrap::indexed_db) query: Option<IdbKeyRangeQuery>,
    pub(in crate::context_bootstrap::indexed_db) count: Option<usize>,
    pub(in crate::context_bootstrap::indexed_db) direction: CursorDirection,
    pub(in crate::context_bootstrap::indexed_db) kind: CollectionKind,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "IDBGetAllOptions")]
struct GetAllOptions<'s> {
    // WebIDL dictionary members are evaluated in lexicographic order.
    #[webidl(converter = "enforce_range_unsigned_long")]
    count: Option<u32>,
    #[webidl(with = parse_direction_member)]
    direction: CursorDirection,
    query: Option<v8::Local<'s, v8::Value>>,
}

fn parse_direction_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<CursorDirection, webidl::WebIdlError> {
    let context = webidl::Context::member("IDBGetAllOptions", name);
    let value = webidl::property_result(scope, object, name, context)?
        .unwrap_or_else(|| v8::undefined(scope).into());
    parse_cursor_direction_with_context(scope, value, context)
}

fn parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    operation: &'static str,
) -> Result<GetAllOptions<'s>, webidl::WebIdlError> {
    match webidl::dictionary_value(value, webidl::Context::argument(operation, 1))? {
        Some(object) => webidl::parse_dictionary_object(scope, object),
        None => Ok(GetAllOptions {
            count: None,
            direction: CursorDirection::default_next(),
            query: None,
        }),
    }
}

pub(super) fn collection_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    kind: CollectionKind,
    is_index: bool,
    operation: &'static str,
) {
    // WebIDL conversion precedes source/transaction validation. The legacy
    // APIs also convert the positional count even when a dictionary overrides
    // it. getAllRecords has only one argument and ignores an extra count.
    let converted = if matches!(kind, CollectionKind::Record) {
        parse_options(scope, args.get(0), operation)
    } else {
        parse_optional_count(scope, args.get(1), operation).map(|count| GetAllOptions {
            query: Some(args.get(0)),
            count: count.map(|count| count as u32),
            direction: CursorDirection::default_next(),
        })
    };
    let mut options = match converted {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !matches!(kind, CollectionKind::Record) && should_parse_options(scope, args.get(0)) {
        options = match parse_options(scope, args.get(0), operation) {
            Ok(options) => options,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        };
    }
    let source = args.this();
    let store = if is_index {
        if indexed_db_index_is_deleted(scope, source) {
            let error =
                dom_exception_value(scope, "The index has been deleted.", "InvalidStateError");
            scope.throw_exception(error);
            return;
        }
        let Some(store) = indexed_db_index_object_store(scope, source) else {
            return;
        };
        store
    } else {
        source
    };
    let Some(transaction) = object_store_active_transaction(scope, store) else {
        return;
    };
    let query_value = options.query.unwrap_or_else(|| v8::null(scope).into());
    let query = match parse_key_or_range(scope, query_value) {
        Ok(query) => query,
        Err(error) => {
            error.throw(scope);
            return;
        }
    };
    if object_store_active_transaction(scope, store).is_none() {
        return;
    }
    let collection = CollectionRequest {
        query,
        count: options.count.map(|count| count as usize),
        direction: options.direction,
        kind,
    };
    let Some(store_name) = indexed_db_object_store_name(scope, store) else {
        return;
    };
    let Some(context) = transaction.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(request) = create_request_object(scope, source.into(), transaction) else {
        return;
    };
    if !object_bool_property(scope, transaction, INDEXED_DB_TRANSACTION_STARTED_SLOT)
        .unwrap_or(false)
    {
        let operation = if is_index {
            IndexedDbTransactionOperation::IndexGetAll(collection)
        } else {
            IndexedDbTransactionOperation::ObjectStoreGetAll(collection)
        };
        enqueue_transaction_operation(scope, transaction, source, request, &store_name, operation);
    } else if let Some(handle) = transaction_handle_from_value(scope, transaction.into()) {
        if is_index {
            execute_index_get_all_request(scope, source, request, handle, &store_name, &collection);
        } else {
            execute_object_store_get_all_request(scope, request, handle, &store_name, &collection);
        }
    }
    rv.set(request.into());
}

fn should_parse_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    should_parse_get_all_options(GetAllOptionsCandidate {
        is_object: value.is_object(),
        // Test the native brand here; reading a key range's bounds would
        // perform conversion twice before the actual query snapshot.
        is_key_range: v8::Local::<v8::Object>::try_from(value)
            .is_ok_and(|object| crate::web_api_interfaces::IDBKeyRange::is_instance(scope, object)),
        is_date: value.is_date(),
        is_array: value.is_array(),
        is_buffer_source: value.is_array_buffer() || value.is_array_buffer_view(),
    })
}
