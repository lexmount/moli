use super::*;
use crate::worker::WorkerGlobalKind;

#[tokio::test]
async fn web_locks_worker_native_contract_across_global_kinds() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://locks.test".to_owned(),
        "https://locks.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for kind in [
        WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        WorkerGlobalKind::Shared {
            name: "locks".to_owned(),
            storage_key,
        },
        WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://locks.test/").unwrap(),
        },
    ] {
        let source = format!(
            "const run={};run().then(value=>console.log(value),error=>console.log(String(error)));",
            include_str!("web_locks.js")
        );
        let mut worker = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(source, "https://locks.test/worker.js".to_owned())
                .with_global_kind(kind),
        );
        let result = timeout(Duration::from_secs(15), worker.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkerToParentMessage::Console(console) = result else {
            panic!("expected contract completion: {result:?}");
        };
        assert_eq!(console.message, "log: complete");
        worker.terminate_and_join();
    }
}

#[tokio::test]
async fn web_locks_worker_callback_unblocks_module_top_level_await() {
    ensure_v8();
    let mut worker = spawn_worker_with_request_client_and_kind(
        "postMessage(await navigator.locks.request('module', lock=>lock.mode));close();".to_owned(),
        "https://locks.test/module.js".to_owned(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );
    assert_eq!(recv_post_json(&mut worker).await, "\"exclusive\"");
}
