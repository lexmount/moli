use super::*;

#[tokio::test]
async fn idb_sqlite_schema_commands_commit_after_synchronous_metadata_update() {
    for (operation, visible) in [
        (
            "return db.createObjectStore('added') instanceof IDBObjectStore",
            "true",
        ),
        (
            "return upgrade.objectStore('records').createIndex('added','group') instanceof IDBIndex",
            "true",
        ),
        (
            "upgrade.objectStore('records').deleteIndex('unique'); return upgrade.objectStore('records').indexNames.length",
            "0",
        ),
        (
            "db.deleteObjectStore('records'); return db.objectStoreNames.length",
            "0",
        ),
    ] {
        let (mut vm, loader, backend) = fixture();
        backend.upgrade(&mut vm);

        assert_eq!(
            vm.eval(&format!(
                r#"(() => {{
                    globalThis.events = [];
                    const visible = (() => {{ {operation} }})();
                    events.push('returned');
                    upgrade.oncomplete = () => events.push('complete');
                    upgrade.onabort = () => events.push('abort:' + upgrade.error.name);
                    const error = String(upgrade.error);
                    upgrade.commit();
                    return visible + ',' + error;
                }})()"#,
            ))
            .unwrap(),
            format!("{visible},null"),
            "{operation} must return before SQL completes",
        );
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["returned"]"#
        );
        backend.drain(&mut vm, &loader).await;
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["returned","complete"]"#
        );
        let metadata = backend.client.committed().unwrap();
        assert_eq!(metadata.version, 2);
        if operation.contains("createObjectStore") {
            assert!(
                metadata
                    .stores
                    .contains_key(&moli_indexeddb::IndexedDbName::from("added"))
            );
        } else if operation.contains("createIndex") {
            assert!(
                metadata.stores[&moli_indexeddb::IndexedDbName::from("records")]
                    .indexes
                    .contains_key(&moli_indexeddb::IndexedDbName::from("added"))
            );
        } else if operation.contains("deleteIndex") {
            assert!(
                metadata.stores[&moli_indexeddb::IndexedDbName::from("records")]
                    .indexes
                    .is_empty()
            );
        } else {
            assert!(metadata.stores.is_empty());
        }
    }
}
