use super::*;

async fn schema_admission_failure(task_capacity: bool) {
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
        let mut held = Vec::new();
        if task_capacity {
            while let Ok(permit) = backend.client.try_reserve(TaskKind::Data, 0) {
                held.push(permit);
            }
            assert_eq!(held.len(), ServiceConfig::default().executor.data.tasks);
        } else {
            held.push(
                backend
                    .client
                    .try_reserve(TaskKind::Data, ServiceConfig::default().executor.data.bytes)
                    .unwrap(),
            );
        }
        assert_eq!(
            vm.eval(&format!(
                r#"(() => {{
                    globalThis.events = [];
                    const visible = (() => {{ {operation} }})();
                    events.push('returned');
                    upgrade.oncomplete = () => events.push('bad-complete');
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
        // Keep the Data lane exhausted through ROLLBACK and abort delivery:
        // lifecycle work must use its independently reserved Control capacity.
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
        assert_eq!(backend._service.queue_usage().data_tasks, 0);
        assert_eq!(backend._service.queue_usage().control_tasks, 0);
    }
}

#[tokio::test]
async fn idb_sqlite_schema_task_admission_failure_returns_then_aborts() {
    schema_admission_failure(true).await;
}

#[tokio::test]
async fn idb_sqlite_schema_byte_admission_failure_returns_then_aborts() {
    schema_admission_failure(false).await;
}
