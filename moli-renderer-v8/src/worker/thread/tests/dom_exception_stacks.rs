use super::*;

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
