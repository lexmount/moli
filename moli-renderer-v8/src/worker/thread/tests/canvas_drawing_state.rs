use super::*;

#[tokio::test]
async fn worker_offscreen_canvas_drawing_stack_preserves_native_state_and_bitmap_boundaries() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const checks = probe(globalThis, globalThis, 'offscreen', 'worker');
            postMessage({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../script_vm/tests/canvas_drawing_state.js"),
    );
    let mut worker = spawn_worker(source, "test://canvas_drawing_state".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let value: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(value["total"].as_u64(), Some(166));
    assert_eq!(value["failures"], serde_json::json!([]));
}
