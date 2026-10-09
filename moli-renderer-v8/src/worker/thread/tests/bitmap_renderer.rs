use super::*;

#[tokio::test]
async fn worker_bitmap_renderer_exposes_native_surface_and_nullable_transfer() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const checks = probe(globalThis, globalThis, 'offscreen', 'worker');
            postMessage({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../../tests/fixtures/canvas-bitmap-renderer-surface.js"),
    );
    let mut worker = spawn_worker(source, "test://bitmap_renderer".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), r#"{"total":31,"failures":[]}"#);
}
