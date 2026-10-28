use super::*;

#[tokio::test]
async fn idb_sqlite_metadata_budget_commit_failure_restores_schema_and_records() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-metadata-budget.test/",
        &loader,
    );
    let settings = ServiceConfig {
        metadata_bytes: 16 * 1024,
        ..Default::default()
    };
    let backend = SqlBackend::with_config(settings);
    backend.bind(&mut vm);
    backend.upgrade(&mut vm);
    vm.eval(r#"
        globalThis.events = [];
        const store = upgrade.objectStore('records');
        store.put({group:'rollback'}).onsuccess = event => events.push('written:' + event.target.result);
        store.createIndex('x'.repeat(32 * 1024), 'group');
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        upgrade.oncomplete = () => events.push('bad-complete');
        'queued'
    "#).unwrap();
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
