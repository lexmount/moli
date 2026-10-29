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
        if task_capacity {
            // A full execution lane waits without converting accepted schema
            // work into an error. Releasing slots must finish the same upgrade.
            // Run ready turns without waiting for the slot held by this test.
            for turn in 0..200 {
                backend.barrier();
                if !vm
                    .run_one_oldest_ready_page_task_executor_turn(&loader)
                    .await
                    .unwrap()
                {
                    break;
                }
                assert!(turn < 199, "ready schema turns must remain bounded");
            }
            assert!(vm.has_in_flight_indexed_db_tasks_for_test().unwrap());
            assert_eq!(backend._service.queue_usage().data_tasks, held.len());
            assert_eq!(backend.client.committed().unwrap().version, 1);
            assert_eq!(
                vm.eval("JSON.stringify(events)").unwrap(),
                r#"["returned"]"#
            );
            drop(held);
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
            assert_eq!(backend._service.queue_usage().data_tasks, 0);
            assert_eq!(backend._service.queue_usage().control_tasks, 0);
            continue;
        }
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
async fn idb_sqlite_schema_task_pressure_waits_then_commits() {
    schema_admission_failure(true).await;
}

#[tokio::test]
async fn idb_sqlite_schema_byte_admission_failure_returns_then_aborts() {
    schema_admission_failure(false).await;
}
