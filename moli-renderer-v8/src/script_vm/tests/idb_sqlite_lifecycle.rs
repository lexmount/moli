use super::*;
use moli_indexeddb::sqlite::service::{
    Command, CommandResult, DatabaseClient, DatabaseConnection, Service, ServiceClient,
    ServiceConfig, WeakDatabaseConnection,
};
use moli_indexeddb::{IndexOptions, KeyPath, ObjectStoreOptions, TransactionMode};
use std::{cell::RefCell, collections::BTreeSet, sync::mpsc, time::Duration};

mod admission_tests;
mod blob_tests;
mod cursor_tests;
mod fatal_tests;
mod index_tests;
mod metadata_tests;
mod sharing_tests;

struct SqlBackend {
    client: DatabaseClient,
    _service: ServiceClient,
    manager: crate::SharedIndexedDbManager,
    buckets: moli_storage_service::SharedStorageBucketStore,
    connection: RefCell<Option<WeakDatabaseConnection>>,
}

impl Drop for SqlBackend {
    fn drop(&mut self) {
        self.manager.lock().shutdown_sqlite().unwrap();
    }
}

impl SqlBackend {
    fn upgrade(&self, vm: &mut ScriptVm) {
        let handle = self.client.allocate_transaction_handle().unwrap();
        let version = self.client.committed().unwrap().version + 1;
        self.execute(Command::Begin {
            transaction: handle,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(version),
            admission: None,
        });
        vm.with_default_context_scope(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let database = v8::Local::<v8::Object>::try_from(
                global
                    .get(scope, crate::util::v8str(scope, "db").into())
                    .unwrap(),
            )
            .unwrap();
            let transaction = crate::context_bootstrap::create_sql_upgrade_for_test(
                scope, database, handle, version,
            )
            .unwrap();
            assert_eq!(
                global.set(
                    scope,
                    crate::util::v8str(scope, "upgrade").into(),
                    transaction.into()
                ),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
    }
    fn new(vm: &mut ScriptVm) -> Self {
        Self::with_config(ServiceConfig::default(), vm)
    }

    fn with_config(mut config: ServiceConfig, vm: &mut ScriptVm) -> Self {
        config.executor.workers = 1;
        let service = Service::new(config).unwrap();
        let (sender, receiver) = mpsc::channel();
        let client = service
            .resolve(
                vm._context_host
                    .borrow_mut()
                    .top_document_storage_context()
                    .storage_key()
                    .serialized_storage_key()
                    .into_bytes(),
                "sql-web-lifecycle".into(),
                move |reply| {
                    sender.send(reply).unwrap();
                },
            )
            .unwrap();
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .result
                .is_ok()
        );
        let service_client = service.client();
        let manager = std::sync::Arc::new(parking_lot::Mutex::new(
            moli_indexeddb::IndexedDbManager::with_sqlite_service(service),
        ));
        let buckets = crate::new_shared_storage_bucket_store_with_indexed_db_manager(&manager);
        let backend = Self {
            client,
            _service: service_client,
            manager,
            buckets,
            connection: RefCell::new(None),
        };
        let transaction = backend.client.allocate_transaction_handle().unwrap();
        backend.execute(Command::Begin {
            transaction,
            mode: TransactionMode::VersionChange,
            stores: BTreeSet::new(),
            version: Some(1),
            admission: None,
        });
        backend.execute(Command::CreateStore {
            transaction,
            name: "records".into(),
            options: ObjectStoreOptions {
                key_path: Some(KeyPath::String("id".into())),
                auto_increment: true,
            },
        });
        backend.execute(Command::CreateIndex {
            transaction,
            store: "records".into(),
            name: "unique".into(),
            options: IndexOptions {
                key_path: KeyPath::String("group".into()),
                unique: true,
                multi_entry: false,
            },
        });
        backend.execute(Command::Commit {
            transaction,
            quota: None,
        });
        backend
    }

    fn execute(&self, command: Command) -> CommandResult {
        let (sender, receiver) = mpsc::channel();
        self.client
            .submit(command, move |reply| {
                sender.send(reply).unwrap();
            })
            .unwrap();
        receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .result
            .unwrap()
    }

    fn barrier(&self) {
        if self.client.is_closed() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.client
            .submit(
                Command::Count {
                    transaction: moli_indexeddb::TransactionHandle::from_raw(0),
                    store: "records".into(),
                    source: Default::default(),
                    range: Default::default(),
                },
                move |_| {
                    sender.send(()).unwrap();
                },
            )
            .unwrap();
        receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    }

    fn connection(&self) -> DatabaseConnection {
        self.connection
            .borrow()
            .as_ref()
            .and_then(WeakDatabaseConnection::upgrade)
            .expect("fixture's Web database owns a live logical connection")
    }

    fn bind(&self, vm: &mut ScriptVm) {
        vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&self.manager)));
        vm.set_storage_bucket_store(self.buckets.clone());
        let connection = self
            .client
            .open_connection(std::sync::Arc::new(|| {}))
            .unwrap();
        self.connection.borrow_mut().replace(connection.downgrade());
        let context_ptr = &vm.page_default_runtime.context as *const _;
        vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let database =
                crate::context_bootstrap::create_sql_database_for_test(scope, connection)
                    .expect("real SQL IDBDatabase wrapper");
            let global = scope.get_current_context().global(scope);
            assert_eq!(
                global.set(
                    scope,
                    crate::util::v8str(scope, "db").into(),
                    database.into()
                ),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
    }

    async fn drain(
        &self,
        vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
        loader: &ResourceRequestClient,
    ) {
        for _ in 0..200 {
            // Wait for actual worker completion instead of sleeping or racing
            // a fixed number of JavaScript pumps against the native thread.
            self.barrier();
            if !vm
                .run_one_oldest_ready_page_task_executor_turn(loader)
                .await
                .unwrap()
            {
                if vm.has_in_flight_indexed_db_tasks_for_test().unwrap() {
                    wait_for_one_selected_page_task_executor_test_turn(vm, loader)
                        .await
                        .unwrap();
                } else {
                    return;
                }
            }
        }
        panic!("SQL Web lifecycle did not become idle");
    }
}

fn fixture() -> (
    crate::runtime::PageVmTaskExecutorTestHarness,
    crate::network::ResourceRequestClientOwner,
    SqlBackend,
) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-lifecycle.test/",
        &loader,
    );
    let backend = SqlBackend::new(&mut vm);
    backend.bind(&mut vm);
    (vm, loader, backend)
}

#[tokio::test]
async fn idb_sqlite_lifecycle_waiting_transactions_do_not_precharge_data_capacity() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(
        r#"
        globalThis.completed = 0;
        for(let index = 0; index < 40; index++) {
            db.transaction('records').oncomplete = () => completed++;
        }
        'queued'
    "#,
    )
    .unwrap();

    // Concurrent readers can interleave their starts before any completion.
    // Keep the original total task budget without requiring serial scheduling.
    let mut previous = 0;
    for _ in 0..40 * 8 {
        backend.barrier();
        assert!(
            vm.run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
                .unwrap()
        );
        let completed = vm
            .eval("String(completed)")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert!(completed >= previous && completed <= 40);
        previous = completed;
        if completed == 40 {
            break;
        }
    }
    assert_eq!(previous, 40, "all admitted readers must complete");
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(completed)").unwrap(), "40");
}

#[tokio::test]
async fn idb_sqlite_lifecycle_waits_for_events_microtasks_and_queued_transaction() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(
        r#"
        globalThis.events = [];
        const first = db.transaction('records', 'readwrite');
        const store = first.objectStore('records');
        const snapshot = {group:'first'};
        store.put(snapshot).onsuccess = event => {
            events.push('put:' + event.target.result);
            store.put({group:'event'}).onsuccess = () => events.push('event');
            Promise.resolve().then(() => {
                events.push('microtask');
                store.put({group:'microtask'}).onsuccess = () => events.push('last');
            });
        };
        snapshot.group = 'mutated';
        first.oncomplete = () => events.push('committed');
        const second = db.transaction('records');
        second.objectStore('records').getAll().onsuccess = event => {
            events.push(event.target.result.map(value => value.id + ':' + value.group).join(','));
        };
        second.oncomplete = () => events.push('read-complete');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["put:1","microtask","event","last","committed","1:first,2:event,3:microtask","read-complete"]"#
    );
    assert!(backend.client.committed().unwrap().usage > 0);
}

#[tokio::test]
async fn idb_sqlite_lifecycle_explicit_commit_drains_inline_write_and_rejects_new_requests() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(
        r#"
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        store.put({group:'a'}).onsuccess = () => {
            events.push('success');
            try { store.count(); } catch(error) { events.push(error.name); }
        };
        store.put({group:'b'});
        tx.commit();
        try { store.put({group:'late'}); } catch(error) { events.push(error.name); }
        try { tx.commit(); } catch(error) { events.push(error.name); }
        try { tx.abort(); } catch(error) { events.push(error.name); }
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('abort');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["TransactionInactiveError","InvalidStateError","InvalidStateError","success","TransactionInactiveError","complete"]"#
    );
    vm.eval("globalThis.count = -1; db.transaction('records').objectStore('records').count().onsuccess = event => count = event.target.result").unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(count)").unwrap(), "2");
}

#[tokio::test]
async fn idb_sqlite_lifecycle_prevent_default_keeps_transaction_and_generator() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(
        r#"
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        store.put({group:'a'});
        store.put({group:'a'}).onerror = event => {
            events.push(event.target.error.name);
            event.preventDefault();
            store.put({group:'b'}).onsuccess = event => events.push('key:' + event.target.result);
        };
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('abort');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["ConstraintError","key:2","complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_lifecycle_unhandled_error_rolls_back_before_next_transaction() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(r#"
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        store.put({group:'a'});
        store.put({group:'a'}).onerror = event => events.push(event.target.error.name);
        tx.onabort = () => events.push('abort');
        tx.oncomplete = () => events.push('bad-complete');
        const next = db.transaction('records','readwrite');
        next.objectStore('records').put({group:'next'}).onsuccess = event => events.push('key:' + event.target.result);
        next.oncomplete = () => events.push('next-complete');
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["ConstraintError","abort","key:1","next-complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_lifecycle_explicit_abort_cancels_unstarted_requests() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(r#"
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        tx.objectStore('records').put({group:'aborted'}).onerror = event => {
            event.preventDefault(); events.push(event.target.error.name);
        };
        tx.onabort = () => events.push('abort:' + tx.error);
        tx.abort();
        const next = db.transaction('records','readwrite');
        next.objectStore('records').put({group:'next'}).onsuccess = event => events.push('key:' + event.target.result);
        next.oncomplete = () => events.push('complete');
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["AbortError","abort:null","key:1","complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_lifecycle_abort_during_inline_round_trip_releases_admission() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(r#"
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        store.put({group:'first'}).onsuccess = () => { events.push('first'); tx.abort(); };
        store.put({group:'second'}).onerror = event => { event.preventDefault(); events.push(event.target.error.name); };
        tx.onabort = () => events.push('abort');
        const next = db.transaction('records','readwrite');
        next.objectStore('records').put({group:'next'}).onsuccess = event => events.push('key:' + event.target.result);
        next.oncomplete = () => events.push('complete');
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["first","AbortError","abort","key:1","complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_lifecycle_explicit_commit_does_not_hide_failed_request() {
    let (mut vm, loader, backend) = fixture();
    vm.eval(
        r#"
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        store.put({group:'same'});
        store.put({group:'same'}).onerror = event => {
            events.push(event.target.error.name); event.preventDefault();
        };
        tx.commit();
        tx.onabort = () => events.push('abort:' + tx.error.name);
        tx.oncomplete = () => events.push('bad-complete');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["AbortError","abort:ConstraintError"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_lifecycle_listener_exception_aborts_only_automatic_commit() {
    for explicit in [false, true] {
        let (mut vm, loader, backend) = fixture();
        vm.eval(&format!(r#"
            globalThis.events = [];
            const tx = db.transaction('records','readwrite');
            tx.objectStore('records').put({{group:'first'}}).onsuccess = () => {{
                events.push('success'); throw new Error('listener failure');
            }};
            tx.onabort = () => events.push('abort:' + tx.error.name);
            tx.oncomplete = () => events.push('complete');
            if ({explicit}) tx.commit();
            const next = db.transaction('records','readwrite');
            next.objectStore('records').put({{group:'next'}}).onsuccess = event => events.push('key:' + event.target.result);
            next.oncomplete = () => events.push('next-complete');
            'queued'
        "#)).unwrap();
        backend.drain(&mut vm, &loader).await;
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            if explicit {
                r#"["success","complete","key:2","next-complete"]"#
            } else {
                r#"["success","abort:AbortError","key:1","next-complete"]"#
            }
        );
    }
}

#[tokio::test]
async fn idb_sqlite_schema_creates_deletes_and_mirrors_before_native_completion() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    assert_eq!(vm.eval(r#"
        globalThis.events = [];
        const created = db.createObjectStore('new', {keyPath:'id', autoIncrement:true});
        created.createIndex('id','id');
        created.createIndex('tags','tags',{multiEntry:true});
        created.put({tags:['a','a','b']});
        created.index('tags').count().onsuccess = event => events.push('count:' + event.target.result);
        const old = upgrade.objectStore('records');
        old.deleteIndex('unique');
        try { old.index('unique'); } catch(error) { events.push(error.name); }
        db.createObjectStore('temporary');
        db.deleteObjectStore('temporary');
        try { db.deleteObjectStore('absent'); } catch(error) { events.push(error.name); }
        try { db.createObjectStore('new'); } catch(error) { events.push(error.name); }
        upgrade.oncomplete = () => events.push('complete');
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        [created instanceof IDBObjectStore, Array.from(created.indexNames), Array.from(db.objectStoreNames), Array.from(old.indexNames)].join('|')
    "#).unwrap(), "true|id,tags|new,records|");
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["NotFoundError","NotFoundError","ConstraintError","count:2","complete"]"#
    );
    let metadata = backend.client.committed().unwrap();
    assert_eq!(metadata.version, 2);
    assert!(
        metadata.stores[&moli_indexeddb::IndexedDbName::from("records")]
            .indexes
            .is_empty()
    );
    assert_eq!(
        metadata.stores[&moli_indexeddb::IndexedDbName::from("new")]
            .indexes
            .len(),
        2
    );
    assert!(
        !metadata
            .stores
            .contains_key(&moli_indexeddb::IndexedDbName::from("temporary"))
    );
}

#[tokio::test]
async fn idb_sqlite_schema_backfills_existing_and_prior_queued_values_in_order() {
    let (mut vm, loader, backend) = fixture();
    vm.eval("const seed = db.transaction('records','readwrite').objectStore('records'); seed.put({group:'a',tags:['a','a','b']}); seed.put({group:'b',tags:['a','c']})").unwrap();
    backend.drain(&mut vm, &loader).await;
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        globalThis.events = [];
        const store = upgrade.objectStore('records');
        store.put({group:'before',tags:['a','d']});
        const index = store.createIndex('tags','tags',{multiEntry:true});
        events.push(index instanceof IDBIndex);
        store.put({group:'after',tags:['a','e']});
        index.getAllKeys('a').onsuccess = event => events.push(event.target.result.join(','));
        upgrade.commit();
        upgrade.oncomplete = () => events.push('complete');
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[true,"1,2,3,4","complete"]"#
    );
}

#[tokio::test]
async fn idb_sqlite_schema_unique_backfill_failure_is_async_and_restores_everything() {
    let (mut vm, loader, backend) = fixture();
    vm.eval("const seed = db.transaction('records','readwrite').objectStore('records'); seed.put({group:'a',slug:'same'}); seed.put({group:'b',slug:'same'})").unwrap();
    backend.drain(&mut vm, &loader).await;
    let committed = backend.client.committed().unwrap();
    backend.upgrade(&mut vm);
    vm.eval(r#"
        globalThis.events = [];
        globalThis.changedStore = db.createObjectStore('new');
        const store = upgrade.objectStore('records');
        globalThis.changedIndex = store.createIndex('slug','slug',{unique:true});
        events.push(changedIndex instanceof IDBIndex);
        store.put({group:'later',slug:'later'}).onerror = event => { event.preventDefault(); events.push(event.target.error.name); };
        upgrade.oncomplete = () => events.push('bad-complete');
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[true,"AbortError","abort:ConstraintError"]"#
    );
    assert_eq!(backend.client.committed().as_ref(), Some(&committed));
    assert_eq!(vm.eval("[db.version, Array.from(db.objectStoreNames), Array.from(changedIndex.objectStore.indexNames)].join('|')").unwrap(), "1|records|unique");
    vm.eval("globalThis.savedCount = -1; db.transaction('records').objectStore('records').count().onsuccess = event => savedCount = event.target.result").unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(savedCount)").unwrap(), "2");
}

#[tokio::test]
async fn idb_sqlite_schema_unique_index_does_not_retroactively_reject_prior_puts() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        globalThis.events = [];
        const store = upgrade.objectStore('records');
        store.put({group:'a',slug:'same'}).onsuccess = event => events.push(event.target.result);
        store.put({group:'b',slug:'same'}).onsuccess = event => events.push(event.target.result);
        events.push(store.createIndex('slug','slug',{unique:true}) instanceof IDBIndex);
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        upgrade.oncomplete = () => events.push('bad-complete');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[true,1,2,"abort:ConstraintError"]"#
    );
    vm.eval("globalThis.savedCount = -1; db.transaction('records').objectStore('records').count().onsuccess = event => savedCount = event.target.result").unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(savedCount)").unwrap(), "0");
}

#[tokio::test]
async fn idb_sqlite_schema_backfill_recycles_budget_instead_of_accumulating_records() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-schema-budget.test/",
        &loader,
    );
    let config = ServiceConfig::default();

    let backend = SqlBackend::with_config(config, &mut vm);
    backend.bind(&mut vm);
    for index in 0..20 {
        vm.eval(&format!("db.transaction('records','readwrite').objectStore('records').put({{group:'g{index}',payload:'x'.repeat(16384)}})")).unwrap();
        backend.drain(&mut vm, &loader).await;
    }
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        globalThis.events = [];
        upgrade.objectStore('records').createIndex('by_group','group');
        upgrade.oncomplete = () => events.push('complete');
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["complete"]"#
    );
    vm.eval("globalThis.savedCount = -1; db.transaction('records').objectStore('records').index('by_group').count().onsuccess = event => savedCount = event.target.result").unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(savedCount)").unwrap(), "20");
}

#[tokio::test]
async fn idb_sqlite_schema_corrupt_wire_aborts_backfill_instead_of_skipping_a_row() {
    let (mut vm, loader, backend) = fixture();
    let handle = backend.client.allocate_transaction_handle().unwrap();
    let admission = backend
        .connection()
        .queue_transaction_start(
            &["records".into()],
            TransactionMode::ReadWrite,
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
    backend.execute(Command::Begin {
        transaction: handle,
        mode: TransactionMode::ReadWrite,
        stores: BTreeSet::from(["records".into()]),
        version: None,
        admission: Some(admission.handle().clone()),
    });
    backend.execute(Command::Write {
        transaction: handle,
        store: "records".into(),
        key: Some(1.into()),
        value: vec![0xff].into(),
        indexes: vec![moli_indexeddb::sqlite::IndexKeys {
            name: "unique".into(),
            keys: vec!["g".into()],
        }],
        add_only: false,
        quota: None,
    });
    backend.execute(Command::Commit {
        transaction: handle,
        quota: None,
    });
    // Index reads must report a corrupt stored value instead of silently
    // discarding its entry. Run through the same native query as a real page.
    vm.eval(
        r#"
        globalThis.corruptIndexRead = 'pending';
        const read = db.transaction('records').objectStore('records').index('unique').get('g');
        read.onsuccess = () => corruptIndexRead = 'unexpected-success';
        read.onerror = event => { event.preventDefault(); corruptIndexRead = read.error.name; };
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("corruptIndexRead").unwrap(), "Error");
    let committed = backend.client.committed().unwrap();
    backend.upgrade(&mut vm);
    vm.eval(r#"
        globalThis.events = [];
        events.push(upgrade.objectStore('records').createIndex('broken','field') instanceof IDBIndex);
        upgrade.oncomplete = () => events.push('bad-complete');
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[true,"abort:UnknownError"]"#
    );
    assert_eq!(backend.client.committed().as_ref(), Some(&committed));
}

#[tokio::test]
async fn idb_sqlite_schema_inactive_upgrade_cannot_queue_late_schema_changes() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval("globalThis.upgradeStore = upgrade.objectStore('records'); upgradeStore.createIndex('created','group'); globalThis.events = []; upgrade.oncomplete = () => events.push('complete')").unwrap();
    assert_eq!(
        vm.eval(
            r#"
        const errors = [];
        for(const change of [
            () => db.createObjectStore('late'),
            () => db.deleteObjectStore('records'),
            () => upgradeStore.createIndex('late','group'),
            () => upgradeStore.deleteIndex('created')
        ]) {
            try { change(); errors.push('accepted'); } catch(error) { errors.push(error.name); }
        }
        JSON.stringify(errors)
    "#
        )
        .unwrap(),
        r#"["TransactionInactiveError","TransactionInactiveError","TransactionInactiveError","TransactionInactiveError"]"#
    );
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["complete"]"#
    );
    let metadata = backend.client.committed().unwrap();
    assert!(
        !metadata
            .stores
            .contains_key(&moli_indexeddb::IndexedDbName::from("late"))
    );
    assert!(
        metadata.stores[&moli_indexeddb::IndexedDbName::from("records")]
            .indexes
            .contains_key(&moli_indexeddb::IndexedDbName::from("created"))
    );
}
