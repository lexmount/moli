use super::*;

#[tokio::test]
async fn worker_fill_rect_applies_transforms_and_alpha_with_native_receivers() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const checks = probe(globalThis,globalThis,'offscreen','worker');
            postMessage({{total:checks.length,failures:checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../script_vm/tests/canvas_fill_rect.js"),
    );
    let mut worker = spawn_worker(source, "test://canvas_fill_rect".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), r#"{"total":41,"failures":[]}"#);
}
