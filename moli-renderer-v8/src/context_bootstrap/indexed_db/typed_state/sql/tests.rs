use super::*;
use crate::context_bootstrap::indexed_db::{
    IndexedDbTransactionOperation, IndexedDbValue, ObjectStoreOptions,
    indexed_db_has_pending_tasks, schedule_transaction_operation,
    stores::collection::{CollectionKind, CollectionRequest},
};
use crate::util::get_private_value;
use moli_indexeddb::sqlite::{
    DatabaseIdentity, IndexKeys,
    service::{CommandResult, Location, Service, ServiceConfig},
};
use moli_indexeddb::{IndexOptions, KeyPath};
use std::{cell::Cell, sync::mpsc, time::Duration};

const TX: TransactionHandle = TransactionHandle::from_raw(19);

struct Backend {
    client: DatabaseClient,
    _service: Service,
}

impl Backend {
    fn new() -> Self {
        let mut config = ServiceConfig::default();
        config.executor.workers = 1;
        let service = Service::new(config).unwrap();
        let (sender, receiver) = mpsc::channel();
        let client = service
            .open(
                DatabaseIdentity {
                    owner_key: b"test-partition/storage-key/default-bucket".to_vec(),
                    name: "sql-routing".into(),
                    generation: [5; 16],
                },
                Location::Memory,
                move |reply| {
                    let _ = sender.send(reply);
                },
            )
            .unwrap();
        assert!(receive(&receiver).result.is_ok());
        let backend = Self {
            client,
            _service: service,
        };
        backend.execute(Command::Begin {
            transaction: TX,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(1),
            admission: None,
        });
        backend.execute(Command::CreateStore {
            transaction: TX,
            name: "records".into(),
            options: ObjectStoreOptions::default(),
        });
        backend.execute(Command::CreateIndex {
            transaction: TX,
            store: "records".into(),
            name: "group".into(),
            options: IndexOptions {
                key_path: KeyPath::String("group".into()),
                unique: false,
                multi_entry: false,
            },
        });
        backend
    }

    fn execute(&self, command: Command) -> CommandResult {
        let (sender, receiver) = mpsc::channel();
        self.client
            .submit(command, 0, move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        receive(&receiver).result.unwrap()
    }

    fn count(&self) -> Command {
        Command::Count {
            transaction: TX,
            store: "records".into(),
            source: Default::default(),
            range: Default::default(),
        }
    }

    fn barrier(&self) {
        self.execute(self.count());
    }

    fn put(&self, value: IndexedDbValue) {
        self.execute(Command::Write {
            transaction: TX,
            store: "records".into(),
            key: Some(Key::from("primary")),
            value,
            indexes: vec![IndexKeys {
                name: "group".into(),
                keys: vec![Key::from("group-key")],
            }],
            add_only: false,
            quota: None,
        });
    }

    fn block(&self) -> WorkerGate {
        let (entered, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        self.client
            .try_reserve(self.count().kind(), 0)
            .unwrap()
            .submit(0, move |_| {
                entered.send(()).unwrap();
                resume
                    .recv_timeout(Duration::from_secs(10))
                    .expect("test releases SQL worker");
            })
            .unwrap();
        waiting.recv_timeout(Duration::from_secs(10)).unwrap();
        WorkerGate(Some(release))
    }
}

struct WorkerGate(Option<mpsc::Sender<()>>);
impl Drop for WorkerGate {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

fn receive(receiver: &mpsc::Receiver<Completion>) -> Completion {
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("bounded SQL worker completion")
}

fn runtime(scope: &mut v8::PinScope<'_, '_>) {
    // These transport tests need an internal queue, without materializing the
    // Window/Worker Web API prototypes in a bare V8 context.
    let state = v8::Object::new(scope);
    let factory = v8::Object::new(scope);
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, "moli.IndexedDb.runtimeState", state.into());
    set_private_value(
        scope,
        state,
        "moli.IndexedDb.runtime.factory",
        factory.into(),
    );
    define_non_enumerable_bool_property(
        scope,
        state,
        "moli.IndexedDb.runtime.factoryInitialized",
        true,
    );
    assert!(indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue).is_some());
}

fn source<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let source = v8::Object::new(scope);
    register_indexed_db_wrapper(scope, source, IndexedDbWrapperKind::Factory, None);
    source
}

fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: &DatabaseClient,
) -> v8::Local<'s, v8::Object> {
    let database = v8::Object::new(scope);
    let transaction = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        transaction,
        IndexedDbWrapperKind::Transaction,
        Some(storage_scope()),
    );
    register_indexed_db_transaction_lifecycle(
        scope,
        transaction,
        database,
        Some(TX),
        TransactionMode::VersionChange,
        true,
        None,
    );
    let id = indexed_db_typed_state_id(scope, transaction).unwrap();
    let table = indexed_db_runtime_state_table_for_object(scope, transaction);
    table
        .borrow_mut()
        .transactions
        .get_mut(&id)
        .unwrap()
        .sql_client = Some(client.clone());
    let request = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        request,
        IndexedDbWrapperKind::Request,
        Some(storage_scope()),
    );
    let store = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        store,
        IndexedDbWrapperKind::ObjectStore,
        Some(storage_scope()),
    );
    register_indexed_db_request_lifecycle(scope, request, store.into(), transaction.into(), false);
    request
}

fn storage_scope() -> IndexedDbStorageScope {
    IndexedDbStorageScope::new("https://sql.example", "test-browser", "test-partition")
}

fn schedule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    kind: IndexedDbTransactionOperation,
) {
    let transaction = indexed_db_request_transaction_object(scope, request).unwrap();
    let source = object_hidden_value(scope, request, INDEXED_DB_REQUEST_SOURCE_SLOT)
        .unwrap()
        .to_object(scope)
        .unwrap();
    schedule_transaction_operation(scope, transaction, source, request, "records", kind);
}

fn result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    if let Some(value) = object_hidden_value(scope, request, INDEXED_DB_PENDING_RESULT_SLOT) {
        return value;
    }
    let error = object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT);
    let description = error.map(|error| error.to_rust_string_lossy(scope));
    let ready = object_string_property(scope, request, INDEXED_DB_REQUEST_READY_STATE_SLOT);
    let final_value = object_hidden_value(scope, request, INDEXED_DB_REQUEST_RESULT_SLOT)
        .map(|value| value.to_rust_string_lossy(scope));
    panic!(
        "missing SQL request result; pending error: {description:?}, ready: {ready:?}, final result: {final_value:?}"
    );
}

fn finish_sql(scope: &mut v8::PinScope<'_, '_>, backend: &Backend) {
    let table = ensure_indexed_db_runtime_state_table(scope);
    let id = *table.borrow().sql_tasks.keys().next().unwrap();
    backend.barrier();
    assert!(flush_indexed_db_task_by_id(scope, id));
    assert!(table.borrow().sql_tasks.is_empty());
}

#[test]
fn sql_read_results_materialize_without_a_legacy_manager_or_record_scan() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope);
    let poison = v8str(
        scope,
        "Object.defineProperty(Array.prototype, '0', {set() {throw new Error('internal queue invoked author setter')}, configurable: true})",
    );
    let script = v8::Script::compile(scope, poison, None).unwrap();
    crate::script_execution::execute_compiled_script(scope, script).unwrap();
    let value = v8str(scope, "from SQLite");
    backend.put(serialize_js_value(scope, value.into()).unwrap());
    let query = IdbKeyRangeQuery::default();

    let get = request(scope, &backend.client);
    schedule(
        scope,
        get,
        IndexedDbTransactionOperation::ObjectStoreGet {
            query: query.clone(),
        },
    );
    finish_sql(scope, &backend);
    assert_eq!(
        result(scope, get).to_rust_string_lossy(scope),
        "from SQLite"
    );
    // Materialization is separate from the subsequent request success event.
    assert_eq!(
        object_string_property(scope, get, INDEXED_DB_REQUEST_READY_STATE_SLOT).as_deref(),
        Some("pending")
    );

    let get_key = request(scope, &backend.client);
    schedule(
        scope,
        get_key,
        IndexedDbTransactionOperation::ObjectStoreGetKey {
            query: query.clone(),
        },
    );
    finish_sql(scope, &backend);
    assert_eq!(
        result(scope, get_key).to_rust_string_lossy(scope),
        "primary"
    );

    let count = request(scope, &backend.client);
    schedule(
        scope,
        count,
        IndexedDbTransactionOperation::ObjectStoreCount { query: None },
    );
    finish_sql(scope, &backend);
    assert_eq!(result(scope, count).number_value(scope), Some(1.0));

    for kind in [CollectionKind::Value, CollectionKind::Key] {
        let get_all = request(scope, &backend.client);
        schedule(
            scope,
            get_all,
            IndexedDbTransactionOperation::ObjectStoreGetAll(CollectionRequest {
                query: None,
                count: None,
                direction: CursorDirection::Prev,
                kind,
            }),
        );
        finish_sql(scope, &backend);
        let array = v8::Local::<v8::Array>::try_from(result(scope, get_all)).unwrap();
        assert_eq!(array.length(), 1);
        let expected = if matches!(kind, CollectionKind::Key) {
            "primary"
        } else {
            "from SQLite"
        };
        assert_eq!(
            array
                .get_index(scope, 0)
                .unwrap()
                .to_rust_string_lossy(scope),
            expected
        );
    }
    let index_key = request(scope, &backend.client);
    let store = object_hidden_value(scope, index_key, INDEXED_DB_REQUEST_SOURCE_SLOT)
        .unwrap()
        .to_object(scope)
        .unwrap();
    let index = v8::Object::new(scope);
    register_indexed_db_wrapper(
        scope,
        index,
        IndexedDbWrapperKind::Index,
        Some(storage_scope()),
    );
    register_indexed_db_index_lifecycle(
        scope,
        index,
        store,
        IndexInfo {
            name: "group".into(),
            key_path: KeyPath::String("group".into()),
            unique: false,
            multi_entry: false,
        },
    );
    set_private_value(
        scope,
        index_key,
        INDEXED_DB_REQUEST_SOURCE_SLOT,
        index.into(),
    );
    schedule(
        scope,
        index_key,
        IndexedDbTransactionOperation::IndexGetKey {
            query: IdbKeyRangeQuery {
                lower: Some(Key::from("group-key")),
                upper: Some(Key::from("group-key")),
                ..Default::default()
            },
        },
    );
    finish_sql(scope, &backend);
    assert_eq!(
        result(scope, index_key).to_rust_string_lossy(scope),
        "primary"
    );
}

#[test]
fn waiting_sql_tasks_do_not_spin_or_block_ready_owner_tasks() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope);
    let (wake, mut wakes) = tokio::sync::mpsc::unbounded_channel();
    set_worker_indexed_db_task_wake_for_context(context, wake);
    let gate = backend.block();
    let source = source(scope);
    let completed = Rc::new(Cell::new(false));
    let observed = completed.clone();
    submit_indexed_db_sql_task(
        scope,
        source,
        &backend.client,
        backend.count(),
        0,
        move |_, _, reply| {
            assert!(matches!(reply.result, Ok(CommandResult::Count(0))));
            observed.set(true);
        },
    )
    .unwrap();
    let table = ensure_indexed_db_runtime_state_table(scope);
    let id = *table.borrow().sql_tasks.keys().next().unwrap();
    assert!(!indexed_db_has_pending_tasks(scope));
    assert!(!flush_next_indexed_db_task(scope));
    assert!(!flush_indexed_db_task_by_id(scope, id));
    assert!(wakes.try_recv().is_err());
    // An unrelated ready task may run while the SQL command is queued.
    let task = v8::Object::new(scope);
    register_indexed_db_task(scope, task, IndexedDbTaskKind::DrainBlockedOpens, None);
    push_object_to_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue, task);
    assert!(
        pop_first_indexed_db_task(scope)
            .unwrap()
            .strict_equals(task.into())
    );
    unregister_indexed_db_task(scope, task);
    drop(gate);
    backend.barrier();
    assert!(wakes.try_recv().is_ok());
    assert!(indexed_db_has_pending_tasks(scope));
    assert!(flush_next_indexed_db_task(scope));
    assert!(completed.get());
    assert!(!flush_next_indexed_db_task(scope));
}

#[test]
fn sql_completion_returns_to_the_accepting_realm() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let first = v8::Context::new(scope, Default::default());
    let second = v8::Context::new(scope, Default::default());
    let source = {
        let scope = &mut v8::ContextScope::new(scope, first);
        runtime(scope);
        source(scope)
    };
    let completed = Rc::new(Cell::new(false));
    {
        let scope = &mut v8::ContextScope::new(scope, second);
        runtime(scope);
        let observed = completed.clone();
        submit_indexed_db_sql_task(
            scope,
            source,
            &backend.client,
            backend.count(),
            0,
            move |scope, _, _| {
                let global = scope.get_current_context().global(scope);
                assert!(
                    get_private_value(scope, global, "sqlRealmMarker")
                        .unwrap()
                        .is_true()
                );
                observed.set(true);
            },
        )
        .unwrap();
        assert_eq!(scope.get_current_context(), second);
        backend.barrier();
        assert!(!flush_next_indexed_db_task(scope));
    }
    {
        let scope = &mut v8::ContextScope::new(scope, first);
        let global = first.global(scope);
        let marker = v8::Boolean::new(scope, true);
        set_private_value(scope, global, "sqlRealmMarker", marker.into());
        assert!(flush_next_indexed_db_task(scope));
    }
    assert!(completed.get());
}

#[test]
fn discarding_sql_tasks_releases_ready_and_late_result_permits() {
    crate::ensure_v8_for_test();
    for ready in [false, true] {
        let backend = Backend::new();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        runtime(scope);
        let gate = (!ready).then(|| backend.block());
        let source = source(scope);
        submit_indexed_db_sql_task(
            scope,
            source,
            &backend.client,
            backend.count(),
            8192,
            |_, _, _| panic!("discarded SQL tasks must not deliver results"),
        )
        .unwrap();
        if ready {
            backend.barrier();
        }
        let table = ensure_indexed_db_runtime_state_table(scope);
        let receiver =
            Arc::downgrade(&table.borrow().sql_tasks.values().next().unwrap().completion);
        let id = *table.borrow().sql_tasks.keys().next().unwrap();
        assert!(discard_indexed_db_task_by_id(scope, id));
        assert!(receiver.upgrade().is_none());
        drop(gate);
        backend.barrier();
        assert!(table.borrow().sql_tasks.is_empty());
        let available = backend
            .client
            .try_reserve(
                backend.count().kind(),
                ServiceConfig::default().executor.data.bytes,
            )
            .unwrap();
        drop(available);
    }
}

#[test]
fn malformed_sql_wire_is_a_request_error_instead_of_successful_undefined() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    backend.put(vec![1, 2, 3].into());
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope);
    let get_key = request(scope, &backend.client);
    schedule(
        scope,
        get_key,
        IndexedDbTransactionOperation::ObjectStoreGetKey {
            query: IdbKeyRangeQuery::default(),
        },
    );
    finish_sql(scope, &backend);
    assert_eq!(
        result(scope, get_key).to_rust_string_lossy(scope),
        "primary"
    );
    let request = request(scope, &backend.client);
    schedule(
        scope,
        request,
        IndexedDbTransactionOperation::ObjectStoreGet {
            query: IdbKeyRangeQuery::default(),
        },
    );
    finish_sql(scope, &backend);
    assert!(object_hidden_value(scope, request, INDEXED_DB_PENDING_ERROR_SLOT).is_some());
    assert!(object_hidden_value(scope, request, INDEXED_DB_PENDING_RESULT_SLOT).is_none());
}

#[test]
fn sql_delete_commands_use_the_same_dispatcher_and_return_undefined() {
    crate::ensure_v8_for_test();
    let backend = Backend::new();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    runtime(scope);
    for kind in [
        IndexedDbTransactionOperation::CursorDelete {
            key: Key::from("primary"),
        },
        IndexedDbTransactionOperation::ObjectStoreDelete {
            query: IdbKeyRangeQuery {
                lower: Some(Key::from("primary")),
                upper: Some(Key::from("primary")),
                ..Default::default()
            },
        },
        IndexedDbTransactionOperation::ObjectStoreClear,
    ] {
        backend.put(vec![1, 2, 3].into());
        let request = request(scope, &backend.client);
        schedule(scope, request, kind);
        finish_sql(scope, &backend);
        assert!(result(scope, request).is_undefined());
        assert!(matches!(
            backend.execute(backend.count()),
            CommandResult::Count(0)
        ));
    }
}
