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

#[tokio::test]
async fn worker_observable_event_target_listener_lifecycle() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "postMessage({}); close();",
            include_str!("../../../../tests/fixtures/observable-event-target.js")
        ),
        "https://observable-events.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 44, "{result}");
}

#[tokio::test]
async fn worker_observable_from_iterables_promises_cancellation_and_exception_timing() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-from.js")
        ),
        "https://observable-from.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 80, "{result}");
}

#[tokio::test]
async fn worker_observable_first_promises_cancellation_reentrancy_and_native_observers() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-first.js")
        ),
        "https://observable-first.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 72, "{result}");
}

#[tokio::test]
async fn worker_observable_collect_values_abort_order_and_native_promise_observers() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-collect.js")
        ),
        "https://observable-collect.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 110, "{result}");
}

#[tokio::test]
async fn worker_observable_callback_consumers_conversion_reentrancy_cancellation_and_exception_identity()
 {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-callback-consumers.js")
        ),
        "https://observable-consumers.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 132, "{result}");
}

#[tokio::test]
async fn worker_observable_predicate_consumers_short_circuit_conversion_reentrancy_and_cancellation()
 {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-predicate-consumers.js")
        ),
        "https://observable-predicates.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 302, "{result}");
}

#[tokio::test]
async fn worker_observable_transforms_preserve_lazy_sharing_cancellation_and_callback_semantics() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-transforms.js")
        ),
        "https://observable-transforms.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 201, "{result}");
}

#[tokio::test]
async fn worker_observable_count_operators_preserve_conversion_sharing_reentrancy_and_cancellation()
{
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-count-operators.js")
        ),
        "https://observable-count-operators.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 194, "{result}");
}

#[tokio::test]
async fn worker_observable_take_until_preserves_conversion_notifier_order_sharing_and_cancellation()
{
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-take-until.js")
        ),
        "https://observable-take-until.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 154, "{result}");
}
