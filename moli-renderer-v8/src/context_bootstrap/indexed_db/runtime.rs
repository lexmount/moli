use super::*;
use crate::util::{get_private_object, get_private_value, set_private_value};
use moli_webapi_declare::WebApiObject;
use std::rc::Rc;

const INDEXED_DB_RUNTIME_STATE_SLOT: &str = "moli.IndexedDb.runtimeState";
const INDEXED_DB_FACTORY_FIELD: &str = "moli.IndexedDb.runtime.factory";
const INDEXED_DB_FACTORY_INITIALIZED_FIELD: &str = "moli.IndexedDb.runtime.factoryInitialized";
const INDEXED_DB_TASK_QUEUE_FIELD: &str = "moli.IndexedDb.runtime.taskQueue";
const INDEXED_DB_OPEN_DATABASES_FIELD: &str = "moli.IndexedDb.runtime.openDatabases";
const INDEXED_DB_BLOCKED_OPEN_QUEUE_FIELD: &str = "moli.IndexedDb.runtime.blockedOpenQueue";
const INDEXED_DB_READWRITE_TRANSACTION_QUEUE_FIELD: &str =
    "moli.IndexedDb.runtime.readwriteTransactionQueue";

#[derive(Default, WebApiObject)]
#[webapi(interface = "IDBFactory", require_prototype)]
struct IndexedDbFactoryRuntimeDeclaration {
    #[webapi(slot = INDEXED_DB_EVENT_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),
}

struct IndexedDbWorkerTaskWake {
    tx: tokio::sync::mpsc::UnboundedSender<()>,
}

#[derive(Clone, Copy)]
pub(in crate::context_bootstrap::indexed_db) enum IndexedDbRuntimeArray {
    TaskQueue,
    OpenDatabases,
    BlockedOpenQueue,
    ReadwriteTransactions,
}

impl IndexedDbRuntimeArray {
    fn field(self) -> &'static str {
        match self {
            Self::TaskQueue => INDEXED_DB_TASK_QUEUE_FIELD,
            Self::OpenDatabases => INDEXED_DB_OPEN_DATABASES_FIELD,
            Self::BlockedOpenQueue => INDEXED_DB_BLOCKED_OPEN_QUEUE_FIELD,
            Self::ReadwriteTransactions => INDEXED_DB_READWRITE_TRANSACTION_QUEUE_FIELD,
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_runtime_factory<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = ensure_indexed_db_runtime_state_object(scope)?;
    ensure_runtime_factory_field(scope, state)
}

pub(crate) fn materialized_indexed_db_factory_for_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = get_private_object(scope, window, INDEXED_DB_RUNTIME_STATE_SLOT)?;
    runtime_object_field(scope, state, INDEXED_DB_FACTORY_FIELD)
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_runtime_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: IndexedDbRuntimeArray,
) -> Option<v8::Local<'s, v8::Array>> {
    let state = ensure_indexed_db_runtime_state_object(scope)?;
    runtime_array_field(scope, state, array)
}

pub(in crate::context_bootstrap::indexed_db) fn push_object_to_indexed_db_runtime_array(
    scope: &mut v8::PinScope<'_, '_>,
    array: IndexedDbRuntimeArray,
    object: v8::Local<'_, v8::Object>,
) {
    if let Some(queue) = indexed_db_runtime_array(scope, array) {
        array_push_value(scope, queue, object.into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn push_unique_object_to_indexed_db_runtime_array(
    scope: &mut v8::PinScope<'_, '_>,
    array: IndexedDbRuntimeArray,
    object: v8::Local<'_, v8::Object>,
) {
    if let Some(queue) = indexed_db_runtime_array(scope, array)
        && !array_contains_strict(scope, queue, object.into())
    {
        array_push_value(scope, queue, object.into());
    }
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_runtime_array_contains_object(
    scope: &mut v8::PinScope<'_, '_>,
    array: IndexedDbRuntimeArray,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    indexed_db_runtime_array(scope, array)
        .map(|queue| array_contains_strict(scope, queue, object.into()))
        .unwrap_or(false)
}

pub(crate) fn indexed_db_has_pending_tasks(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let global = scope.get_current_context().global(scope);
    let Some(state) = get_private_object(scope, global, INDEXED_DB_RUNTIME_STATE_SLOT) else {
        return false;
    };
    runtime_array_field(scope, state, IndexedDbRuntimeArray::TaskQueue)
        .map(|queue| queue.length() > 0)
        .unwrap_or(false)
}

pub(crate) fn set_worker_indexed_db_task_wake_for_context(
    context: v8::Local<'_, v8::Context>,
    tx: tokio::sync::mpsc::UnboundedSender<()>,
) {
    let _previous = context.set_slot(Rc::new(IndexedDbWorkerTaskWake { tx }));
}

pub(in crate::context_bootstrap::indexed_db) fn signal_worker_indexed_db_task_wake(
    scope: &mut v8::PinScope<'_, '_>,
) -> bool {
    let context = scope.get_current_context();
    let Some(wake) = context.get_slot::<IndexedDbWorkerTaskWake>() else {
        return false;
    };
    wake.tx.send(()).is_ok()
}

pub(in crate::context_bootstrap::indexed_db) fn pop_first_indexed_db_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let queue = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue)?;
    let first = queue.get_index(scope, 0)?;
    let first = v8::Local::<v8::Object>::try_from(first).ok()?;
    let next = v8::Array::new(scope, 0);
    for index in 1..queue.length() {
        let Some(value) = queue.get_index(scope, index) else {
            continue;
        };
        array_push_value(scope, next, value);
    }
    replace_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue, next);
    Some(first)
}

pub(in crate::context_bootstrap::indexed_db) fn take_indexed_db_task_by_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    expected: IndexedDbTaskId,
) -> Option<v8::Local<'s, v8::Object>> {
    let queue = indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue)?;
    let next = v8::Array::new(scope, 0);
    let mut selected = None;
    for index in 0..queue.length() {
        let Some(value) = queue.get_index(scope, index) else {
            continue;
        };
        let Ok(task) = v8::Local::<v8::Object>::try_from(value) else {
            array_push_value(scope, next, value);
            continue;
        };
        if selected.is_none() && indexed_db_typed_task_id(scope, task) == Some(expected) {
            selected = Some(task);
        } else {
            array_push_value(scope, next, value);
        }
    }
    if selected.is_some() {
        replace_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue, next);
    }
    selected
}

pub(in crate::context_bootstrap::indexed_db) fn replace_indexed_db_runtime_array(
    scope: &mut v8::PinScope<'_, '_>,
    array: IndexedDbRuntimeArray,
    next: v8::Local<'_, v8::Array>,
) {
    if let Some(state) = ensure_indexed_db_runtime_state_object(scope) {
        define_non_enumerable_value_property(scope, state, array.field(), next.into());
    }
}

fn ensure_indexed_db_runtime_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let _ = ensure_indexed_db_runtime_state_table(scope);
    let global = scope.get_current_context().global(scope);
    if let Some(state) = get_private_object(scope, global, INDEXED_DB_RUNTIME_STATE_SLOT) {
        ensure_runtime_state_fields(scope, state)?;
        return Some(state);
    }

    // Private IDB owner state is an internal dictionary, not a Web API object.
    // Keep it null-prototype so page `Object.prototype` pollution cannot fake
    // readiness flags or queue fields during lazy initialization/re-entry.
    let state = new_null_prototype_object(scope);
    // Publish the owner before filling fields so any synchronous V8 re-entry
    // completes the same state object instead of constructing a competing one.
    set_private_value(scope, global, INDEXED_DB_RUNTIME_STATE_SLOT, state.into());
    ensure_runtime_state_fields(scope, state)?;
    Some(state)
}

fn ensure_runtime_state_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<()> {
    ensure_runtime_factory_field(scope, state)?;
    ensure_runtime_array_field(scope, state, IndexedDbRuntimeArray::TaskQueue)?;
    ensure_runtime_array_field(scope, state, IndexedDbRuntimeArray::OpenDatabases)?;
    ensure_runtime_array_field(scope, state, IndexedDbRuntimeArray::BlockedOpenQueue)?;
    ensure_runtime_array_field(scope, state, IndexedDbRuntimeArray::ReadwriteTransactions)?;
    Some(())
}

fn ensure_runtime_factory_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let factory =
        if let Some(factory) = runtime_object_field(scope, state, INDEXED_DB_FACTORY_FIELD) {
            factory
        } else {
            let factory = build_indexed_db_factory_object(scope)?;
            set_private_value(scope, state, INDEXED_DB_FACTORY_FIELD, factory.into());
            define_non_enumerable_bool_property(
                scope,
                state,
                INDEXED_DB_FACTORY_INITIALIZED_FIELD,
                true,
            );
            factory
        };
    if object_bool_property(scope, state, INDEXED_DB_FACTORY_INITIALIZED_FIELD).unwrap_or(false) {
        return Some(factory);
    };

    let factory_proto = global_constructor_prototype(scope, "IDBFactory")?;
    if object_bool_property(scope, state, INDEXED_DB_FACTORY_INITIALIZED_FIELD).unwrap_or(false) {
        return Some(factory);
    }
    let _ = factory.set_prototype(scope, factory_proto.into());
    if object_property_as_object(scope, factory, INDEXED_DB_EVENT_LISTENERS_SLOT).is_none() {
        // Listener maps use event type strings as dictionary keys, so they must
        // not inherit attacker-controlled Object.prototype properties.
        let listeners = new_null_prototype_object(scope);
        set_indexed_db_slot_value(
            scope,
            factory,
            INDEXED_DB_EVENT_LISTENERS_SLOT,
            listeners.into(),
        );
    }
    define_non_enumerable_bool_property(scope, state, INDEXED_DB_FACTORY_INITIALIZED_FIELD, true);
    Some(factory)
}

pub(crate) fn scoped_indexed_db_factory<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    storage_key: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let storage_scope = storage_scope_for_current_partition(scope, storage_key)?;
    build_scoped_indexed_db_factory(scope, storage_scope)
}

pub(super) fn build_scoped_indexed_db_factory<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    storage_scope: IndexedDbStorageScope,
) -> Option<v8::Local<'s, v8::Object>> {
    let factory = build_indexed_db_factory_object(scope)?;
    register_indexed_db_wrapper(
        scope,
        factory,
        IndexedDbWrapperKind::Factory,
        Some(storage_scope),
    );
    Some(factory)
}

pub(crate) fn bind_indexed_db_factory_to_window_execution_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    factory: v8::Local<'s, v8::Object>,
    execution_context: crate::native_bridge::WindowExecutionContextIdentity,
) -> bool {
    if !indexed_db_typed_wrapper_is(scope, factory, IndexedDbWrapperKind::Factory) {
        return false;
    }
    let storage_scope = indexed_db_typed_storage_scope(scope, factory)
        .or_else(|| storage_scope_for_window_execution_context(scope, execution_context));
    register_indexed_db_wrapper_with_owner(
        scope,
        factory,
        IndexedDbWrapperKind::Factory,
        IndexedDbExecutionOwner::for_window_execution_context(execution_context),
        storage_scope,
    );
    true
}

pub(in crate::context_bootstrap::indexed_db) fn indexed_db_factory_storage_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    factory: v8::Local<'s, v8::Object>,
) -> Option<IndexedDbStorageScope> {
    indexed_db_typed_storage_scope(scope, factory)
}

fn build_indexed_db_factory_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    // The factory is script-visible and must receive IDBFactory.prototype;
    // only the private owner state around it is null-prototype.
    let factory = IndexedDbFactoryRuntimeDeclaration {
        event_listeners: (),
    }
    .bind(scope)
    .ok()?;
    let factory_proto = global_constructor_prototype(scope, "IDBFactory")?;
    let _ = factory.set_prototype(scope, factory_proto.into());
    let listeners = new_null_prototype_object(scope);
    set_indexed_db_slot_value(
        scope,
        factory,
        INDEXED_DB_EVENT_LISTENERS_SLOT,
        listeners.into(),
    );
    let owner = indexed_db_execution_owner_for_object(scope, factory);
    let storage_scope = owner
        .execution_context()
        .and_then(|execution_context| {
            storage_scope_for_window_execution_context(scope, execution_context)
        })
        .or_else(|| {
            context_host_ptr_from_global_bridge(scope)
                .is_none()
                .then(|| current_storage_scope(scope))
                .flatten()
        });
    register_indexed_db_wrapper_with_owner(
        scope,
        factory,
        IndexedDbWrapperKind::Factory,
        owner,
        storage_scope,
    );
    Some(factory)
}

fn ensure_runtime_array_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    array: IndexedDbRuntimeArray,
) -> Option<v8::Local<'s, v8::Array>> {
    if let Some(existing) = runtime_array_field(scope, state, array) {
        return Some(existing);
    }
    let next = v8::Array::new(scope, 0);
    set_private_value(scope, state, array.field(), next.into());
    Some(next)
}

fn runtime_object_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    field: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, state, field)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn runtime_array_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    array: IndexedDbRuntimeArray,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, state, array.field())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}
