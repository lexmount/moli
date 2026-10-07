use super::*;

#[tokio::test]
async fn worker_observer_options_modes_and_disconnect_match_window_semantics() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const checks = probe(globalThis, globalThis, 'worker');
            postMessage({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../script_vm/tests/performance_observer_contract.js"),
    );
    let mut worker = spawn_worker(source, "test://performance_observer_contract".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), r#"{"total":91,"failures":[]}"#);
}
