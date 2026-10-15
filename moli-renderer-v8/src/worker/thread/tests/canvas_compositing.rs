use super::*;

#[tokio::test]
async fn worker_canvas_compositing_applies_native_operators_across_global_kinds() {
    let vectors = serde_json::to_string(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-canvas/src/composite_chromium_145.tsv"
    )))
    .unwrap();
    let source = format!(
        "const probe={};const rows=probe(globalThis,globalThis,'offscreen','worker',{});console.log(JSON.stringify({{total:rows.length,failures:rows.filter(row=>!row.passed)}}));",
        include_str!("../../../script_vm/tests/canvas_compositing.js"),
        vectors
    );
    assert_canvas_fixture_worker_kinds(source, 656).await;
}

#[tokio::test]
async fn worker_canvas_shadows_preserve_unclipped_sources_and_ordered_compositing() {
    let source = format!(
        "const probe={};const rows=probe(globalThis,globalThis,'offscreen','worker');console.log(JSON.stringify({{total:rows.length,failures:rows.filter(row=>!row.passed)}}));",
        include_str!("../../../script_vm/tests/canvas_shadows.js")
    );
    assert_canvas_fixture_worker_kinds(source, 25).await;
}

#[tokio::test]
async fn worker_draw_image_transforms_crop_and_sample_across_global_kinds() {
    let source = format!(
        "const probe={};const rows=probe(globalThis,globalThis,'offscreen','worker',workerVideoFrameExposed);console.log(JSON.stringify({{total:rows.length,failures:rows.filter(row=>!row.passed)}}));",
        include_str!("../../../script_vm/tests/canvas_draw_image.js")
    );
    assert_canvas_fixture_worker_kinds(source, 50).await;
}

async fn assert_canvas_fixture_worker_kinds(source: String, expected: u64) {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://canvas-compositing.test".to_owned(),
        "https://canvas-compositing.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for kind in [
        super::super::WorkerGlobalKind::Dedicated {
            name: String::new(),
        },
        super::super::WorkerGlobalKind::Shared {
            name: "compositing".to_owned(),
            storage_key,
        },
        super::super::WorkerGlobalKind::Service {
            registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
            version_id: ServiceWorkerVersionId::from_u64_for_test(1),
            scope_url: url::Url::parse("https://canvas-compositing.test/").unwrap(),
        },
    ] {
        let source = format!(
            "const workerVideoFrameExposed={};\n{}",
            matches!(&kind, super::super::WorkerGlobalKind::Dedicated { .. }),
            source
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                source,
                "https://canvas-compositing.test/worker.js".to_owned(),
            )
            .with_global_kind(kind),
        );
        let message = timeout(Duration::from_secs(30), handle.recv())
            .await
            .unwrap()
            .unwrap();
        let WorkerToParentMessage::Console(console) = message else {
            panic!("expected compositing checks, got {message:?}");
        };
        let result: serde_json::Value =
            serde_json::from_str(console.message.strip_prefix("log: ").unwrap()).unwrap();
        assert_eq!(result["total"], expected, "{result}");
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        handle.terminate_and_join();
    }
}
