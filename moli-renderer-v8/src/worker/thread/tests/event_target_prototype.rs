use super::*;

#[tokio::test]
async fn worker_interfaces_inherit_canonical_event_target_operations() {
    ensure_v8();
    let probe = include_str!("../../../../tests/fixtures/event-target-inheritance.js");
    let mut handle = spawn_worker(
        format!(
            "{probe}; postMessage({{pending: __eventTargetInheritance.pending, rows: __eventTargetInheritance.rows}}); close();"
        ),
        "https://event-target-inheritance.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["pending"], false);
    assert!(
        result["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row[1] == true),
        "{result}"
    );
}

#[tokio::test]
async fn worker_event_target_prototype_methods_preserve_native_state() {
    ensure_v8();
    let probe = include_str!("../../../../tests/fixtures/event-target-prototype.js");
    let mut handle = spawn_worker(
        format!("{probe}\npostMessage(__uiEventResults); close();"),
        "https://event-target-prototype.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["complete"], true, "{result}");
    assert_eq!(result["total"], 56, "{result}");
    assert_eq!(result["passed"], result["total"], "{result}");
}
