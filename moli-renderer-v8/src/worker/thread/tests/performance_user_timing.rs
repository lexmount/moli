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

#[tokio::test]
async fn worker_performance_measure_converts_arguments_and_members_in_webidl_order() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const checks = probe();
            postMessage({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../../tests/fixtures/user-timing-measure-conversion.js"),
    );
    let mut worker = spawn_worker(source, "test://measure_conversion_order".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["total"], 32);
    assert_eq!(result["failures"], serde_json::json!([]), "{result:#}");
}

#[tokio::test]
async fn worker_performance_names_retain_utf16_identity_with_observers() {
    ensure_v8();
    let source = format!(
        r#"(async () => {{
            const probe = {};
            const checks = await probe();
            postMessage({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            close();
        }})()"#,
        include_str!("../../../../tests/fixtures/performance-name-utf16.js"),
    );
    let mut worker = spawn_worker(source, "test://performance_names".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["total"], 228);
    assert_eq!(result["failures"], serde_json::json!([]), "{result:#}");
}
