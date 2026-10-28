use super::*;
use moli_indexeddb::sqlite::{
    DatabaseIdentity,
    service::{Command, CommandResult, DatabaseClient, Location, Service, ServiceConfig, TaskKind},
};
use moli_indexeddb::{DatabaseHandle, IndexOptions, KeyPath, ObjectStoreOptions, TransactionMode};
use std::{collections::BTreeSet, sync::mpsc, time::Duration};

struct SqlBackend {
    client: DatabaseClient,
    _service: Service,
}

impl SqlBackend {
    fn new() -> Self {
        let mut config = ServiceConfig::default();
        config.executor.workers = 1;
        let service = Service::new(config).unwrap();
        let (sender, receiver) = mpsc::channel();
        let client = service
            .open(
                DatabaseIdentity {
                    owner_key: b"sql-web-lifecycle-test".to_vec(),
                    name: "sql-web-lifecycle".into(),
                    generation: [43; 16],
                },
                Location::Memory,
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
        let backend = Self {
            client,
            _service: service,
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
        backend.execute(Command::Commit { transaction });
        backend
    }

    fn execute(&self, command: Command) -> CommandResult {
        let (sender, receiver) = mpsc::channel();
        self.client
            .submit(command, 0, move |reply| {
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
        let (sender, receiver) = mpsc::channel();
        self.client
            .try_reserve(TaskKind::Control, 0)
            .unwrap()
            .submit(0, move |permit| {
                drop(permit);
                sender.send(()).unwrap();
            })
            .unwrap();
        receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    }

    fn bind(&self, vm: &mut ScriptVm) {
        let context_ptr = &vm.page_default_runtime.context as *const _;
        vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let database = crate::context_bootstrap::create_sql_database_for_test(
                scope,
                self.client.clone(),
                DatabaseHandle::from_raw((1u64 << 53) + 1),
            )
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
                return;
            }
        }
        panic!(
            "SQL Web lifecycle did not become idle: usage={:?}",
            self._service.queue_usage()
        );
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
    let backend = SqlBackend::new();
    backend.bind(&mut vm);
    (vm, loader, backend)
}

#[tokio::test]
async fn idb_sqlite_lifecycle_waiting_transactions_do_not_hold_control_capacity() {
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
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
    // Every transaction needs its own admission wake, start, BEGIN receipt,
    // commit task and COMMIT receipt. Assert bounded progress per transaction
    // instead of applying the single-workflow drain budget to all forty.
    for expected in 1..=40 {
        let mut completed = 0;
        for _ in 0..8 {
            backend.barrier();
            assert!(
                vm.run_one_oldest_ready_page_task_executor_turn(&loader)
                    .await
                    .unwrap()
            );
            completed = vm
                .eval("String(completed)")
                .unwrap()
                .parse::<usize>()
                .unwrap();
            if completed >= expected {
                break;
            }
        }
        assert_eq!(
            completed, expected,
            "transaction must advance within its bounded task sequence"
        );
    }
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("String(completed)").unwrap(), "40");
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
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
