use super::*;

async fn observable_worker_collect_garbage(handle: &WorkerTestHandle) {
    let (tx, rx) = oneshot::channel();
    assert!(
        handle
            .devtools_handle()
            .inspector_tasks()
            .collect_garbage_for_test(tx)
    );
    timeout(TIMEOUT, rx).await.unwrap().unwrap();
}

async fn observable_worker_evaluate(
    handle: &WorkerTestHandle,
    expression: &str,
) -> serde_json::Value {
    let (tx, rx) = oneshot::channel();
    let request = serde_json::json!({"id": 1, "method": "Runtime.evaluate", "params": {
        "expression": expression, "returnByValue": true
    }});
    assert!(handle.dispatch_runtime_protocol_message(None, request.to_string(), None, tx));
    let messages = timeout(TIMEOUT, rx).await.unwrap().unwrap().unwrap();
    let response = messages
        .into_iter()
        .map(crate::runtime::RendererRuntimeInspectorMessage::into_v8_inspector_message)
        .find(|message| message["id"] == 1)
        .expect("worker evaluation must return its response");
    assert!(response.get("error").is_none(), "{response}");
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response}"
    );
    response["result"]["result"]["value"].clone()
}

#[tokio::test]
async fn worker_observable_async_iterator_stays_cancellable_across_gc() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "postMessage({});",
            include_str!("../../../../tests/fixtures/observable-iterator-gc.js")
        ),
        "https://observable-iterator-gc.test/worker.js".into(),
    );
    assert_eq!(recv_post_json(&mut handle).await, r#""ready""#);
    assert!(handle.attach_runtime_inspector_session(None));
    observable_worker_collect_garbage(&handle).await;
    let result = observable_worker_evaluate(&handle,
        "observableIteratorGc.live.controller.abort('stop'); [observableIteratorGc.live.closes, observableIteratorGc.abandoned.iterator.deref() === undefined]").await;
    assert_eq!(result, serde_json::json!([1, true]));
    observable_worker_collect_garbage(&handle).await;
    let result = observable_worker_evaluate(
        &handle,
        "observableIteratorGc.live.iterator.deref() === undefined",
    )
    .await;
    assert_eq!(result, true);
}

#[tokio::test]
async fn worker_observable_abort_steps_finish_after_iterator_close_errors() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "postMessage({}); close();",
            include_str!("../../../../tests/fixtures/observable-abort-steps.js")
        ),
        "https://observable-abort-steps.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result, serde_json::json!({"checks": 10}));
}

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

#[tokio::test]
async fn worker_observable_inspect_preserves_conversion_callbacks_cancellation_and_error_order() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-inspect.js")
        ),
        "https://observable-inspect.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["checks"], 169, "{result}");
}

#[tokio::test]
async fn worker_observable_finally_preserves_teardown_order_sharing_and_reentrant_cancellation() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-finally.js")
        ),
        "https://observable-finally.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 120, "{result}");
}

#[tokio::test]
async fn worker_observable_flat_map_preserves_serial_order_conversion_reentrancy_and_cancellation()
{
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-flat-map.js")
        ),
        "https://observable-flat-map.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 120, "{result}");
}

#[tokio::test]
async fn worker_observable_switch_map_preserves_switch_order_conversion_reentrancy_and_cancellation()
 {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-switch-map.js")
        ),
        "https://observable-switch-map.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 170, "{result}");
}

#[tokio::test]
async fn worker_observable_catch_preserves_recovery_order_conversion_reentrancy_and_cancellation() {
    ensure_v8();
    let mut handle = spawn_worker(
        format!(
            "({}).then(value => {{ postMessage(value); close(); }});",
            include_str!("../../../../tests/fixtures/observable-catch.js")
        ),
        "https://observable-catch.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert!(result["checks"].as_u64().unwrap() >= 130, "{result}");
}
