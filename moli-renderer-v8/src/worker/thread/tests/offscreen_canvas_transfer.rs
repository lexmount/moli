use super::*;

#[tokio::test]
async fn worker_offscreen_canvas_transfer_owns_pixels_without_image_decode_tasks() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const result = probe(globalThis, globalThis, 'worker');
            postMessage({{total: result.checks.length, failures: result.checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../../tests/fixtures/canvas-offscreen-transfer-checks.js"),
    );
    let mut worker = spawn_worker(source, "test://offscreen_canvas_transfer".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), r#"{"total":29,"failures":[]}"#);
}
