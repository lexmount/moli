use super::*;

#[tokio::test]
async fn exception_constructors_preserve_utf16_and_conversion_order_across_worker_kinds() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://exception-constructor-strings.test".to_owned(),
        "https://exception-constructor-strings.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for (kind, script_kind) in [
        (
            super::super::WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            WorkerScriptKind::Classic,
        ),
        (
            super::super::WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            WorkerScriptKind::Module,
        ),
        (
            super::super::WorkerGlobalKind::Shared {
                name: "errors".to_owned(),
                storage_key,
            },
            WorkerScriptKind::Classic,
        ),
        (
            super::super::WorkerGlobalKind::Service {
                registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
                version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                scope_url: url::Url::parse("https://exception-constructor-strings.test/").unwrap(),
            },
            WorkerScriptKind::Classic,
        ),
    ] {
        let source = format!(
            "{}\nconsole.log(JSON.stringify(exceptionConstructorStringsProbe(globalThis)));",
            include_str!("../../../../tests/fixtures/exception-constructor-strings.js"),
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                source,
                "https://exception-constructor-strings.test/worker.js".to_owned(),
            )
            .with_global_kind(kind)
            .with_script_kind(script_kind),
        );
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        handle.terminate_and_join();
        let WorkerToParentMessage::Console(console) = message else {
            panic!("expected completed exception constructor argument checks, got {message:?}");
        };
        let result: serde_json::Value =
            serde_json::from_str(console.message.strip_prefix("log: ").unwrap()).unwrap();
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        assert_eq!(result["rows"].as_array().unwrap().len(), 188, "{result}");
    }
}

#[tokio::test]
async fn native_error_brand_uses_v8_classification_across_worker_kinds() {
    ensure_v8();
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://native-error-brand.test".to_owned(),
        "https://native-error-brand.test".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    for (kind, script_kind) in [
        (
            super::super::WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            WorkerScriptKind::Classic,
        ),
        (
            super::super::WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            WorkerScriptKind::Module,
        ),
        (
            super::super::WorkerGlobalKind::Shared {
                name: "errors".to_owned(),
                storage_key,
            },
            WorkerScriptKind::Classic,
        ),
        (
            super::super::WorkerGlobalKind::Service {
                registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
                version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                scope_url: url::Url::parse("https://native-error-brand.test/").unwrap(),
            },
            WorkerScriptKind::Classic,
        ),
    ] {
        let source = format!(
            "{}\nconsole.log(JSON.stringify(nativeErrorBrandProbe(globalThis)));",
            include_str!("../../../../tests/fixtures/native-error-brand.js"),
        );
        let mut handle = spawn_test_worker_with_options(
            WorkerSpawnOptions::new(
                source,
                "https://native-error-brand.test/worker.js".to_owned(),
            )
            .with_global_kind(kind)
            .with_script_kind(script_kind),
        );
        let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
        handle.terminate_and_join();
        let WorkerToParentMessage::Console(console) = message else {
            panic!("expected completed native error brand checks, got {message:?}");
        };
        let result: serde_json::Value =
            serde_json::from_str(console.message.strip_prefix("log: ").unwrap()).unwrap();
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        assert_eq!(result["rows"].as_array().unwrap().len(), 64, "{result}");
    }
}

#[tokio::test]
async fn worker_dom_exception_data_and_native_failures_have_distinct_stack_boundaries() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let source = format!(
            "{}\npostMessage(domExceptionStackBoundariesProbe(globalThis)); close();",
            include_str!("../../../../tests/fixtures/dom-exception-stack-boundaries.js"),
        );
        let mut handle = spawn_worker_with_request_client_and_kind(
            source,
            "https://worker-error-stack.test/worker.js".to_owned(),
            worker_test_request_client(),
            kind,
        );
        let actual: serde_json::Value =
            serde_json::from_str(&recv_post_json(&mut handle).await).unwrap();
        assert_eq!(
            actual["failures"],
            serde_json::json!([]),
            "{kind:?}: {actual}"
        );
        assert_eq!(
            actual["rows"].as_array().unwrap().len(),
            28,
            "{kind:?}: {actual}"
        );
    }
}
