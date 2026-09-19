use super::*;

#[tokio::test]
async fn worker_abort_signal_uses_shared_event_target_and_receiver_checks() {
    ensure_v8();
    let probe = include_str!("../../../script_vm/tests/browser_api/abort_signal_events.js");
    let mut handle = spawn_worker(
        format!(
            "{probe}\npostMessage({{events: abortSignalEventTargetProbe(), receivers: abortSignalReceiverProbe()}}); close();"
        ),
        "https://abort-signal-events.test/worker.js".into(),
    );
    let message = timeout(TIMEOUT, handle.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(
        result["events"]["failures"],
        serde_json::json!([]),
        "{result}"
    );
    assert_eq!(result["events"]["scenarios"].as_array().unwrap().len(), 6);
    assert_eq!(
        result["receivers"]["failures"],
        serde_json::json!([]),
        "{result}"
    );
    assert_eq!(result["receivers"]["checks"], 40);
}
