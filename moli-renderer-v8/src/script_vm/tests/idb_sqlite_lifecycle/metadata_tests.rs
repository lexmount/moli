use super::*;
use moli_indexeddb::sqlite::MetadataReservation;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone)]
struct MetadataControl {
    client: DatabaseClient,
    limit: usize,
    held: Arc<Mutex<Option<MetadataReservation>>>,
}

fn exhaust_metadata(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let control = scope.get_slot::<MetadataControl>().unwrap().clone();
    let remaining = control.limit - control.client.metadata_bytes();
    *control.held.lock() = Some(control.client.try_reserve_metadata(remaining).unwrap());
}

fn install_metadata_control(
    vm: &mut ScriptVm,
    backend: &SqlBackend,
    limit: usize,
) -> MetadataControl {
    let control = MetadataControl {
        client: backend.client.clone(),
        limit,
        held: Arc::default(),
    };
    vm.with_default_context_scope(|scope, _| {
        scope.set_slot(control.clone());
        let callback = v8::Function::new(scope, exhaust_metadata).unwrap();
        let global = scope.get_current_context().global(scope);
        global
            .set(
                scope,
                crate::util::v8str(scope, "exhaustMetadata").into(),
                callback.into(),
            )
            .unwrap();
        Ok(())
    })
    .unwrap();
    control
}

#[tokio::test]
async fn idb_sqlite_metadata_budget_commit_failure_restores_schema_and_records() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-metadata-budget.test/",
        &loader,
    );
    let settings = ServiceConfig {
        metadata_bytes: 64 * 1024,
        ..Default::default()
    };
    let backend = SqlBackend::with_config(settings, &mut vm);
    backend.bind(&mut vm);
    backend.upgrade(&mut vm);
    let control = install_metadata_control(&mut vm, &backend, settings.metadata_bytes);
    vm.eval(
        r#"
        globalThis.events = [];
        const store = upgrade.objectStore('records');
        store.put({group:'rollback'}).onsuccess = event => {
            events.push('written:' + event.target.result);
            store.createIndex('later', 'group');
            // The candidate mirror already exists. Force the native metadata
            // publication allocation to fail before the irreversible COMMIT.
            exhaustMetadata();
        };
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        upgrade.oncomplete = () => events.push('bad-complete');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["written:1","abort:UnknownError"]"#
    );
    assert_eq!(vm.eval("String(db.version)").unwrap(), "1");
    let metadata = backend.client.committed().unwrap();
    assert_eq!(metadata.version, 1);
    assert_eq!(metadata.stores["records"].indexes.len(), 1);
    assert!(backend._service.metadata_bytes() <= settings.metadata_bytes);
    control.held.lock().take();
    vm.eval(
        r#"
        globalThis.saved = -1;
        globalThis.nextKey = -1;
        const tx = db.transaction('records','readwrite');
        const records = tx.objectStore('records');
        records.count().onsuccess = event => saved = event.target.result;
        records.put({group:'next'}).onsuccess = event => nextKey = event.target.result;
        tx.oncomplete = () => events.push('complete');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(vm.eval("[saved,nextKey].join(',')").unwrap(), "0,1");
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["written:1","abort:UnknownError","complete"]"#
    );
    assert_eq!(backend._service.queue_usage().data_tasks, 0);
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
}

#[tokio::test]
async fn idb_sqlite_mirror_budget_schema_methods_return_then_abort_in_fifo_order() {
    for (operation, expected) in [
        (
            r#"globalThis.failed = db.createObjectStore('added',{keyPath:['id']});
            return [failed instanceof IDBObjectStore, failed.name, failed.keyPath[0],
                failed.transaction === upgrade, failed.get instanceof Function].join(',')"#,
            "true,added,id,true,true",
        ),
        (
            r#"globalThis.failed = store.createIndex('added',['group']);
            return [failed instanceof IDBIndex, failed.name, failed.keyPath[0],
                failed.objectStore === store, failed.get instanceof Function].join(',')"#,
            "true,added,group,true,true",
        ),
        (
            "db.deleteObjectStore('records'); return 'deleted'",
            "deleted",
        ),
        ("store.deleteIndex('unique'); return 'deleted'", "deleted"),
    ] {
        let (mut vm, loader, backend) = fixture();
        backend.upgrade(&mut vm);
        // Exhausting metadata also rejects the quota job's bounded error DTO.
        // The schema abort cancels that pending write; its AbortError must be
        // delivered before the transaction abort event.
        let held = backend
            .client
            .try_reserve_metadata(
                ServiceConfig::default().metadata_bytes - backend.client.metadata_bytes(),
            )
            .unwrap();
        assert_eq!(
            vm.eval(&format!(
                r#"(() => {{
            globalThis.events = [];
            const store = upgrade.objectStore('records');
            const pendingWrite = store.put({{group:'before'}});
            pendingWrite.onsuccess = event => events.push('written:' + event.target.result);
            pendingWrite.onerror = event => events.push('write-error:' + event.target.error.name);
            const result = (() => {{ {operation} }})();
            events.push('returned');
            upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
            upgrade.oncomplete = () => events.push('bad-complete');
            upgrade.commit();
            return result + ',' + String(upgrade.error);
        }})()"#
            ))
            .unwrap(),
            format!("{expected},null")
        );
        if expected != "deleted" {
            assert_eq!(
                vm.eval("(() => { try { failed.get(1); } catch(error) { return error.name; } })()")
                    .unwrap(),
                "InvalidStateError"
            );
        }
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["returned"]"#
        );
        backend.drain(&mut vm, &loader).await;
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["returned","write-error:AbortError","abort:UnknownError"]"#
        );
        assert_eq!(
            vm.eval("[db.version,Array.from(db.objectStoreNames)].join('|')")
                .unwrap(),
            "1|records"
        );
        let metadata = backend.client.committed().unwrap();
        assert_eq!(metadata.version, 1);
        assert_eq!(metadata.stores["records"].indexes.len(), 1);
        assert!(metadata.stores["records"].indexes.contains_key("unique"));
        assert!(backend.client.metadata_bytes() <= ServiceConfig::default().metadata_bytes);
        drop(held);
        vm.eval("globalThis.savedCount = -1; db.transaction('records').objectStore('records').count().onsuccess = event => savedCount = event.target.result").unwrap();
        backend.drain(&mut vm, &loader).await;
        assert_eq!(vm.eval("String(savedCount)").unwrap(), "0");
        assert_eq!(backend._service.queue_usage().data_tasks, 0);
        assert_eq!(backend._service.queue_usage().control_tasks, 0);
    }
}

#[tokio::test]
async fn idb_sqlite_readonly_commit_needs_no_new_mirror_metadata_capacity() {
    let (mut vm, loader, backend) = fixture();
    let held = backend
        .client
        .try_reserve_metadata(
            ServiceConfig::default().metadata_bytes - backend.client.metadata_bytes(),
        )
        .unwrap();
    vm.eval(
        r#"
        globalThis.events = [];
        const tx = db.transaction('records');
        globalThis.store = tx.objectStore('records');
        globalThis.index = store.index('unique');
        index.count().onsuccess = event => events.push('count:' + event.target.result);
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('bad-abort');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["count:0","complete"]"#
    );
    assert_eq!(
        backend.client.metadata_bytes(),
        ServiceConfig::default().metadata_bytes
    );
    drop(held);
}

#[tokio::test]
async fn idb_sqlite_committed_upgrade_promotes_mirror_with_metadata_capacity_exhausted() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        if (db.version !== 2) throw new Error('upgrade must expose version 2');
        globalThis.events = [];
        const store = db.createObjectStore('added',{keyPath:'id'});
        globalThis.kept = store.createIndex('by-value','value');
        upgrade.oncomplete = () => events.push('complete');
        upgrade.onabort = () => events.push('bad-abort');
        'queued'
    "#,
    )
    .unwrap();
    // Stop between the actual SQL COMMIT and its renderer receipt. No callback
    // waits for SQL, and no source can change after COMMIT has been observed.
    for step in 0..200 {
        backend.barrier();
        if backend.client.committed().unwrap().version == 2 {
            break;
        }
        assert!(step < 199);
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .unwrap()
        {
            // Quota authorization may still be running outside the SQL worker.
            // Only await its wake here: executing the next turn could consume
            // the COMMIT receipt before we exhaust the mirror budget below.
            assert!(vm.has_in_flight_indexed_db_tasks_for_test().unwrap());
            assert!(
                tokio::time::timeout(
                    Duration::from_secs(2),
                    vm.wait_for_task_executor_work_arrival(),
                )
                .await
                .expect("quota/SQL completion wakes the fixture")
            );
        }
    }
    assert_eq!(backend.client.committed().unwrap().version, 2);
    assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), "[]");
    let held = backend
        .client
        .try_reserve_metadata(
            ServiceConfig::default().metadata_bytes - backend.client.metadata_bytes(),
        )
        .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["complete"]"#
    );
    assert_eq!(
        vm.eval("[db.version,kept.name,Array.from(db.objectStoreNames)].join('|')")
            .unwrap(),
        "2|by-value|added,records"
    );
    drop(held);
    // Successful promotion must also clear created-in-upgrade markers for a
    // later upgrade. Abort restores these existing definitions, not deletion.
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        if (db.version !== 3) throw new Error('upgrade must expose version 3');
        globalThis.existing = upgrade.objectStore('added');
        existing.createIndex('later','other');
        upgrade.abort();
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("Array.from(existing.indexNames).join(',')")
            .unwrap(),
        "by-value"
    );
    assert_eq!(backend.client.committed().unwrap().version, 2);
    assert_eq!(vm.eval("String(db.version)").unwrap(), "2");
}

#[tokio::test]
async fn idb_sqlite_failed_database_mirror_admission_refunds_partial_nodes_and_can_retry() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-mirror-open-budget.test/",
        &loader,
    );
    let backend = SqlBackend::new(&mut vm);
    let transaction = backend.client.allocate_transaction_handle().unwrap();
    backend.execute(Command::Begin {
        transaction,
        mode: TransactionMode::VersionChange,
        stores: BTreeSet::new(),
        version: Some(2),
        admission: None,
    });
    backend.execute(Command::CreateStore {
        transaction,
        name: "zzz".into(),
        options: ObjectStoreOptions {
            key_path: Some(KeyPath::String("x".repeat(32 * 1024))),
            auto_increment: false,
        },
    });
    backend.execute(Command::Commit {
        transaction,
        quota: None,
    });
    let native_bytes = backend.client.metadata_bytes();
    let held = backend
        .client
        .try_reserve_metadata(ServiceConfig::default().metadata_bytes - native_bytes - 8 * 1024)
        .unwrap();
    let before = backend.client.metadata_bytes();
    vm.with_default_context_scope(|scope, _| {
        assert!(
            crate::context_bootstrap::create_sql_database_for_test(
                scope,
                backend
                    .client
                    .open_connection(std::sync::Arc::new(|| {}))
                    .unwrap(),
            )
            .is_none()
        );
        Ok(())
    })
    .unwrap();
    backend.barrier();
    // The small 'records' definition was constructed before 'zzz' exceeded the
    // allowance. No partial mirror or lifecycle registration can retain it.
    assert_eq!(backend.client.metadata_bytes(), before);
    drop(held);
    assert_eq!(backend.client.metadata_bytes(), native_bytes);
    backend.bind(&mut vm);
    assert_eq!(
        vm.eval("Array.from(db.objectStoreNames).join(',')")
            .unwrap(),
        "records,zzz"
    );
    assert!(backend.client.metadata_bytes() > native_bytes);
}
