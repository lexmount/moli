use super::*;

#[tokio::test]
async fn idb_sqlite_small_write_shares_large_schema() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idb-sql-shared-write-schema.test/",
        &loader,
    );
    let config = ServiceConfig::default();

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
            name: format!("unused{index}").into(),
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
        backend.client.committed().unwrap().stores[&moli_indexeddb::IndexedDbName::from("records")]
            .indexes
            .len(),
        11
    );
}
