use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = "IDBRequest")]
struct IdbRequestObjectDeclaration<'scope> {
    #[webapi(slot = INDEXED_DB_EVENT_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    transaction: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable, init = "undefined")]
    result: (),

    #[webapi(data_property, enumerable, init = "null")]
    error: (),

    #[webapi(data_property, enumerable)]
    ready_state: &'static str,

    #[webapi(data_property, enumerable, init = "null")]
    onsuccess: (),

    #[webapi(data_property, enumerable, init = "null")]
    onerror: (),
}

#[derive(Default, WebApiObject)]
#[webapi(prototype = "Object", interface = "IDBOpenDBRequest")]
struct IdbOpenRequestHandlersDeclaration {
    #[webapi(data_property, enumerable, init = "null")]
    onupgradeneeded: (),

    #[webapi(data_property, enumerable, init = "null")]
    onblocked: (),
}

pub(in crate::context_bootstrap::indexed_db) fn create_request_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Value>,
    transaction: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let relevant_context = transaction.get_creation_context(scope)?;
    let owner = indexed_db_typed_execution_owner(scope, transaction)
        .expect("IDBRequest should inherit typed owner from transaction");
    let storage_scope = indexed_db_typed_storage_scope(scope, transaction)
        .expect("IDBRequest should inherit typed storage scope from transaction");
    create_request_object_in_relevant_context(
        scope,
        relevant_context,
        source,
        Some(transaction),
        false,
        owner,
        Some(storage_scope),
    )
}

pub(in crate::context_bootstrap::indexed_db) fn create_open_request_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    factory: v8::Local<'s, v8::Object>,
    owner: IndexedDbExecutionOwner,
    storage_scope: IndexedDbStorageScope,
) -> Option<v8::Local<'s, v8::Object>> {
    let relevant_context = factory.get_creation_context(scope)?;
    let null_source: v8::Local<'s, v8::Value> = v8::null(scope).into();
    create_request_object_in_relevant_context(
        scope,
        relevant_context,
        null_source,
        None,
        true,
        owner,
        Some(storage_scope),
    )
}

fn create_request_object_in_relevant_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    relevant_context: v8::Local<'s, v8::Context>,
    source: v8::Local<'s, v8::Value>,
    transaction: Option<v8::Local<'s, v8::Object>>,
    is_open: bool,
    owner: IndexedDbExecutionOwner,
    storage_scope: Option<IndexedDbStorageScope>,
) -> Option<v8::Local<'s, v8::Object>> {
    if relevant_context == scope.get_current_context() {
        return create_request_object_in_current_context(
            scope,
            source,
            transaction,
            is_open,
            owner,
            storage_scope,
        );
    }

    let source = v8::Global::new(scope, source);
    let transaction = transaction.map(|transaction| v8::Global::new(scope, transaction));
    let request = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let source = v8::Local::new(target_scope, &source);
        let transaction = transaction
            .as_ref()
            .map(|transaction| v8::Local::new(target_scope, transaction));
        create_request_object_in_current_context(
            target_scope,
            source,
            transaction,
            is_open,
            owner,
            storage_scope,
        )
        .map(|request| v8::Global::new(target_scope, request))
    }?;
    Some(v8::Local::new(scope, &request))
}

fn create_request_object_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Value>,
    transaction: Option<v8::Local<'s, v8::Object>>,
    is_open: bool,
    owner: IndexedDbExecutionOwner,
    storage_scope: Option<IndexedDbStorageScope>,
) -> Option<v8::Local<'s, v8::Object>> {
    let transaction_value = transaction
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let request = IdbRequestObjectDeclaration::new(source, transaction_value, "pending")
        .bind(scope)
        .ok()?;
    if is_open {
        IdbOpenRequestHandlersDeclaration::default()
            .initialize(scope, request)
            .ok()?;
    }
    let prototype = if is_open {
        global_constructor_prototype(scope, "IDBOpenDBRequest")?
    } else {
        global_constructor_prototype(scope, "IDBRequest")?
    };
    let _ = request.set_prototype(scope, prototype.into());
    let kind = if is_open {
        IndexedDbWrapperKind::OpenRequest
    } else {
        IndexedDbWrapperKind::Request
    };
    register_indexed_db_wrapper_with_owner(scope, request, kind, owner, storage_scope);
    register_indexed_db_request_lifecycle(scope, request, source, transaction_value, false);
    Some(request)
}
