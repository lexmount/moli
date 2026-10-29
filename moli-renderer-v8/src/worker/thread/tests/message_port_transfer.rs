use super::*;

#[tokio::test]
async fn worker_message_port_transfer_validates_after_serialization_across_global_kinds() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://port-transfer.test".to_owned(),
        "https://port-transfer.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for kind in [
        super::super::WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        super::super::WorkerGlobalKind::Shared {
            name: "port-transfer".to_owned(),
            storage_key,
        },
        super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://port-transfer.test/").unwrap(),
        },
    ] {
        let source = format!(
            "const probe={};probe(globalThis,globalThis,'worker').then(rows=>console.log(JSON.stringify({{total:rows.length,failures:rows.filter(row=>!row.passed)}})));",
            include_str!("../../../script_vm/tests/message_port_transfer.js")
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(source, "https://port-transfer.test/worker.js".to_owned())
                .with_global_kind(kind),
        );
        let message = timeout(Duration::from_secs(30), handle.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkerToParentMessage::Console(console) = message else {
            panic!("expected MessagePort checks, got {message:?}");
        };
        let result: serde_json::Value =
            serde_json::from_str(console.message.strip_prefix("log: ").unwrap()).unwrap();
        assert_eq!(result["total"], 40, "{result}");
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        handle.terminate_and_join();
    }
}
