use super::*;

#[tokio::test]
async fn worker_image_bitmap_snapshots_decodes_and_rejections_across_global_kinds() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://bitmap.test".to_owned(),
        "https://bitmap.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for kind in [
        super::super::WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        super::super::WorkerGlobalKind::Shared {
            name: "bitmap".to_owned(),
            storage_key,
        },
        super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://bitmap.test/").unwrap(),
        },
    ] {
        let expected_total = if matches!(&kind, super::super::WorkerGlobalKind::Dedicated { .. }) {
            27
        } else {
            25
        };
        let source = format!(
            "const run = {}; run().then(result => console.log(JSON.stringify({{total:result.total,failures:result.checks.filter(row=>!row.passed)}})));",
            include_str!("image_bitmap.js"),
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(source, "https://bitmap.test/worker.js".to_owned())
                .with_global_kind(kind),
        );
        let message = timeout(Duration::from_secs(15), handle.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkerToParentMessage::Console(console) = message else {
            panic!("expected the completed bitmap contract, got {message:?}");
        };
        let result: serde_json::Value =
            serde_json::from_str(console.message.strip_prefix("log: ").unwrap()).unwrap();
        assert_eq!(result["total"], expected_total, "{result}");
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_image_bitmap_completion_unblocks_module_top_level_await() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_kind(
        r#"
        const source = new OffscreenCanvas(1, 1);
        source.getContext('2d').fillStyle = '#ff0000';
        source.getContext('2d').fillRect(0, 0, 1, 1);
        const blob = await source.convertToBlob();
        const bitmap = await createImageBitmap(blob);
        const context = source.getContext('2d');
        context.clearRect(0, 0, 1, 1);
        context.drawImage(bitmap, 0, 0);
        postMessage(Array.from(context.getImageData(0, 0, 1, 1).data));
        bitmap.close(); close();
        "#
        .to_owned(),
        "https://bitmap.test/module.js".to_owned(),
        worker_test_request_client(),
        WorkerScriptKind::Module,
    );
    assert_eq!(recv_post_json(&mut handle).await, "[255,0,0,255]");
}
