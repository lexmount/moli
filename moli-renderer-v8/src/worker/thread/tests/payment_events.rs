use super::*;

#[tokio::test]
async fn payment_update_event_interfaces_are_not_exposed_in_workers() {
    ensure_v8();
    let mut worker = spawn_worker(
        "postMessage([typeof PaymentRequestUpdateEvent, typeof PaymentMethodChangeEvent]);close();"
            .into(),
        "https://payment-worker.test/".into(),
    );
    let message = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&expect_post_json(message)).unwrap(),
        serde_json::json!(["undefined", "undefined"])
    );
}
