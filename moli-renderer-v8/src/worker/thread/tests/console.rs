use super::*;

#[tokio::test]
async fn worker_console_namespace_uses_realm_intrinsics_across_global_kinds() {
    let source = format!(
        "const probe={};console.log(probe(globalThis));",
        include_str!("../../../script_vm/tests/browser_api/console_namespace.js")
    );
    assert_worker_console_script(&source, "log: true").await;
}

#[tokio::test]
async fn worker_console_labels_preserve_conversion_and_exceptions_across_global_kinds() {
    let source = format!(
        "const probe={};const rows=probe(globalThis);if(rows.length!==70)throw new Error('incomplete label matrix');console.log(JSON.stringify(rows.filter(row=>!row.passed)));",
        include_str!("../../../script_vm/tests/browser_api/console_labels.js")
    );
    assert_worker_console_script(&source, "log: []").await;
}

async fn assert_worker_console_script(source: &str, expected: &str) {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://console-namespace.test".to_owned(),
        "https://console-namespace.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for kind in [
        super::super::WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        super::super::WorkerGlobalKind::Shared {
            name: "console".to_owned(),
            storage_key,
        },
        super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://console-namespace.test/").unwrap(),
        },
    ] {
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                source.to_owned(),
                "https://console-namespace.test/worker.js".to_owned(),
            )
            .with_global_kind(kind),
        );
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        let WorkerToParentMessage::Console(console) = message else {
            panic!("expected console result, got {message:?}");
        };
        assert_eq!(console.message, expected);
        handle.terminate_and_join();
    }
}
