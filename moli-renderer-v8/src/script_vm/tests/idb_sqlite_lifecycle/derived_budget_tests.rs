use super::*;

fn bounded_fixture() -> (
    crate::runtime::PageVmTaskExecutorTestHarness,
    crate::network::ResourceRequestClientOwner,
    SqlBackend,
) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-derived-budget.test/",
        &loader,
    );
    let mut config = ServiceConfig::default();
    config.executor.data.bytes = 32 * 1024;
    let backend = SqlBackend::with_config(config, &mut vm);
    backend.bind(&mut vm);
    (vm, loader, backend)
}

#[tokio::test]
async fn idb_sqlite_derived_key_capacity_failure_cancels_inline_key_and_can_be_prevented() {
    let (mut vm, loader, backend) = bounded_fixture();
    backend.upgrade(&mut vm);
    vm.eval("upgrade.objectStore('records').createIndex('tags','tags',{multiEntry:true})")
        .unwrap();
    backend.drain(&mut vm, &loader).await;
    vm.eval(r#"(() => {
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        const store = tx.objectStore('records');
        store.put({group:'large',tags:Array.from({length:200},(_,i)=>'key'+i)}).onerror = event => {
            event.preventDefault();
            events.push('error:' + event.target.error.name);
            store.put({group:'small',tags:['one']}).onsuccess = event => events.push('key:' + event.target.result);
        };
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('bad-abort:' + tx.error.name);
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["error:UnknownError","key:1","complete"]"#
    );
    vm.eval(
        r#"(() => {
        const tx = db.transaction('records');
        const store = tx.objectStore('records');
        store.count().onsuccess = event => events.push('records:' + event.target.result);
        store.index('tags').count().onsuccess = event => events.push('tags:' + event.target.result);
        return 'queued';
    })()"#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["error:UnknownError","key:1","complete","records:1","tags:1"]"#
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
}

#[tokio::test]
async fn idb_sqlite_backfill_derived_key_capacity_failure_aborts_upgrade_without_losing_records() {
    let (mut vm, loader, backend) = bounded_fixture();
    vm.eval(r#"(() => {
        const tx = db.transaction('records','readwrite');
        tx.objectStore('records').put({group:'existing',tags:Array.from({length:200},(_,i)=>'key'+i)});
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    backend.upgrade(&mut vm);
    vm.eval(
        r#"
        globalThis.events = [];
        upgrade.objectStore('records').createIndex('tags','tags',{multiEntry:true});
        upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
        upgrade.oncomplete = () => events.push('bad-complete');
        'queued'
    "#,
    )
    .unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["abort:UnknownError"]"#
    );
    assert_eq!(backend.client.committed().unwrap().version, 1);
    assert_eq!(
        backend.client.committed().unwrap().stores["records"]
            .indexes
            .len(),
        1
    );
    vm.eval(r#"
        db.transaction('records').objectStore('records').count().onsuccess = event => events.push('records:' + event.target.result);
        'queued'
    "#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["abort:UnknownError","records:1"]"#
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
}

#[tokio::test]
async fn idb_sqlite_small_write_shares_schema_larger_than_its_data_capacity() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-shared-write-schema.test/",
        &loader,
    );
    let mut config = ServiceConfig::default();
    config.executor.data.bytes = 32 * 1024;
    let backend = SqlBackend::with_config(config, &mut vm);
    let transaction = backend.client.allocate_transaction_handle().unwrap();
    backend.execute(Command::Begin {
        transaction,
        mode: TransactionMode::VersionChange,
        stores: BTreeSet::new(),
        version: Some(2),
        admission: None,
    });
    for index in 0..10 {
        backend.execute(Command::CreateIndex {
            transaction,
            store: "records".into(),
            name: format!("unused{index}"),
            options: IndexOptions {
                key_path: KeyPath::String(format!("missing{index}{}", "x".repeat(8192))),
                unique: false,
                multi_entry: false,
            },
        });
    }
    backend.execute(Command::Commit {
        transaction,
        quota: None,
    });
    backend.bind(&mut vm);
    vm.eval(r#"(() => {
        globalThis.events = [];
        const tx = db.transaction('records','readwrite');
        tx.objectStore('records').put({group:'small'}).onsuccess = event => events.push('key:' + event.target.result);
        tx.oncomplete = () => events.push('complete');
        tx.onabort = () => events.push('bad-abort:' + tx.error.name);
        return 'queued';
    })()"#).unwrap();
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"["key:1","complete"]"#
    );
    assert_eq!(
        backend.client.committed().unwrap().stores["records"]
            .indexes
            .len(),
        11
    );
    assert_eq!(backend._service.queue_usage().data_bytes, 0);
    assert_eq!(backend._service.queue_usage().control_tasks, 0);
}
