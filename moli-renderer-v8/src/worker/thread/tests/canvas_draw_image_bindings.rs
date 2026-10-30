use super::*;
#[tokio::test]
async fn worker_draw_image_uses_native_source_and_argument_bindings() {
    ensure_v8();
    let source = format!(
        "const probe={};const checks=probe(globalThis,globalThis,'offscreen','worker');postMessage({{total:checks.length,failures:checks.filter(row=>!row.passed)}});close();",
        include_str!("../../../../tests/fixtures/canvas-draw-image-bindings.js")
    );
    let mut worker = spawn_worker(source, "https://draw-image-worker.test/worker.js".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("worker deadline")
        .expect("worker result");
    assert_eq!(expect_post_json(message), r#"{"total":13,"failures":[]}"#);
}
