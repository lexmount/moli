use super::*;

#[tokio::test]
async fn worker_user_timing_implicit_timestamps_share_the_performance_now_clock() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const privateStateProbe = {};
            const checks = [...probe(), ...privateStateProbe()];
            postMessage({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../../tests/fixtures/user-timing-clock.js"),
        include_str!("../../../../tests/fixtures/user-timing-private-state.js"),
    );
    let mut worker = spawn_worker(source, "test://user_timing_clock".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["total"], 200);
    assert_eq!(result["failures"], serde_json::json!([]), "{result:#}");
}
