use super::*;

#[tokio::test]
async fn worker_path2d_geometry_and_offscreen_drawing_use_native_brands() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const checks = probe(globalThis,globalThis,'offscreen','worker');
            postMessage({{total:checks.length,failures:checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../script_vm/tests/path2d_native.js"),
    );
    let mut worker = spawn_worker(source, "test://path2d_native".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), r#"{"total":160,"failures":[]}"#);
}
