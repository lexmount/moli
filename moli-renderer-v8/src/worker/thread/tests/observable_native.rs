use super::*;

#[tokio::test]
async fn worker_observable_core_lifecycle_conversion_brands_and_exceptions() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "postMessage({}); close();",
            include_str!("../../../../tests/fixtures/observable-core.js")
        ),
        "https://observable.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 65, "{result}");
}
