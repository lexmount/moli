use super::*;

#[tokio::test]
async fn worker_offscreen_canvas_transform_snapshots_preserve_native_state_and_identity() {
    ensure_v8();
    let source = format!(
        r#"const probe = {};
            const checks = probe(globalThis, globalThis, 'offscreen', 'worker');
            const point = new DOMMatrix().translate(3, 4).transformPoint(new DOMPoint(1, 2));
            if (!(point instanceof DOMPoint) || point.x !== 4 || point.y !== 6)
                throw Error('worker transform result types');
            postMessage({{total: checks.length, failures: checks.filter(row => !row.passed)}});
            close();"#,
        include_str!("../../../script_vm/tests/canvas_transform_snapshots.js"),
    );
    let mut worker = spawn_worker(source, "test://canvas_transform_snapshots".into());
    let message = timeout(TIMEOUT, worker.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(message), r#"{"total":74,"failures":[]}"#);
}
