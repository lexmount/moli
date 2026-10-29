use super::*;
use std::sync::Arc;

#[tokio::test]
async fn idb_sqlite_abort_from_write_listener_restores_schema_records_and_generator() {
    let (mut vm, loader, backend) = fixture();
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        globalThis.events = [];
        const store = upgrade.objectStore('records');
        store.put({group:'rollback'}).onsuccess = event => {
            events.push('written:' + event.target.result);
            store.createIndex('later', 'group');
            // A public abort restores the schema candidate and the preceding write.
            upgrade.abort();
        };
        upgrade.onabort = () => events.push('abort:' + upgrade.error);
        upgrade.oncomplete = () => events.push('bad-complete');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["written:1","abort:null"]"#
    );
    assert_eq!(vm.eval("String(db.version)").unwrap(), "1");
    let metadata = backend.client.committed().unwrap();
    assert_eq!(metadata.version, 1);
    assert_eq!(metadata.stores["records"].indexes.len(), 1);
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
        r#"["written:1","abort:null","complete"]"#
    );
    assert_eq!(backend._service.queue_usage().data_tasks, 0);
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
}

#[tokio::test]
async fn idb_sqlite_same_task_schema_abort_cancels_pending_writes_in_fifo_order() {
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
            upgrade.onabort = () => events.push('abort:' + upgrade.error);
            upgrade.oncomplete = () => events.push('bad-complete');
            upgrade.abort();
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
            r#"["returned","write-error:AbortError","abort:null"]"#
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
        vm.eval("globalThis.savedCount = -1; db.transaction('records').objectStore('records').count().onsuccess = event => savedCount = event.target.result").unwrap();
        backend.drain(&mut vm, &loader).await;
        assert_eq!(vm.eval("String(savedCount)").unwrap(), "0");
        assert_eq!(backend._service.queue_usage().data_tasks, 0);
        assert_eq!(backend._service.queue_usage().control_tasks, 0);
    }
}

#[tokio::test]
async fn idb_sqlite_readonly_commit_reuses_committed_metadata() {
    let (mut vm, loader, backend) = fixture();
    let before = backend.client.committed().unwrap();
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
    assert!(Arc::ptr_eq(&before, &backend.client.committed().unwrap()));
}

#[tokio::test]
async fn idb_sqlite_committed_upgrade_promotes_shared_mirror_before_later_upgrade_abort() {
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
