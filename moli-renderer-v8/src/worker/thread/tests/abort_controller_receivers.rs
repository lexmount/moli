use super::*;

#[tokio::test]
async fn worker_abort_controller_receivers_use_native_brands() {
    ensure_v8();
    let probe = include_str!("../../../../tests/fixtures/abort-controller-receivers.js");
    let mut handle = spawn_worker(
        format!("{probe}\npostMessage(__uiEventResults); close();"),
        "https://abort-controller-receivers.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["complete"], true, "{result}");
    assert_eq!(result["total"], 62, "{result}");
    assert_eq!(result["passed"], result["total"], "{result}");
}
