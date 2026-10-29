use super::*;

async fn schema_admission(byte_pressure: bool) {
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
        let held = byte_pressure.then(|| {
            backend
                .client
                .try_reserve(ServiceConfig::default().executor.max_in_flight_bytes)
                .unwrap()
        });
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
            "{operation} must return before the asynchronous storage failure",
        );
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["returned"]"#
        );
        if !byte_pressure {
            backend.drain(&mut vm, &loader).await;
            assert_eq!(
                vm.eval("JSON.stringify(events)").unwrap(),
                r#"["returned","complete"]"#
            );
            let metadata = backend.client.committed().unwrap();
            assert_eq!(metadata.version, 2);
            if operation.contains("createObjectStore") {
                assert!(metadata.stores.contains_key("added"));
            } else if operation.contains("createIndex") {
                assert!(metadata.stores["records"].indexes.contains_key("added"));
            } else if operation.contains("deleteIndex") {
                assert!(metadata.stores["records"].indexes.is_empty());
            } else {
                assert!(metadata.stores.is_empty());
            }
            assert_eq!(backend._service.in_flight_bytes(), 0);

            continue;
        }
        // Keep all data bytes retained through rollback and abort delivery;
        // lifecycle work uses ordinary FIFO cleanup without a reservation.
        backend.drain(&mut vm, &loader).await;
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["returned","abort:UnknownError"]"#,
        );
        let metadata = backend.client.committed().unwrap();
        assert_eq!(metadata.version, 1);
        assert_eq!(metadata.stores.len(), 1);
        assert_eq!(metadata.stores["records"].indexes.len(), 1);
        assert!(metadata.stores["records"].indexes.contains_key("unique"));
        assert_eq!(
            vm.eval("[db.version,Array.from(db.objectStoreNames)].join('|')")
                .unwrap(),
            "1|records",
        );
        drop(held);
        assert_eq!(backend._service.in_flight_bytes(), 0);
    }
}

#[tokio::test]
async fn idb_sqlite_schema_commands_commit_without_task_slot_reservations() {
    schema_admission(false).await;
}

#[tokio::test]
async fn idb_sqlite_schema_byte_admission_failure_returns_then_aborts() {
    schema_admission(true).await;
}
